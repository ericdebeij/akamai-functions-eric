use crate::edgegrid::{self, Credentials};
use anyhow::Result;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use spin_sdk::http::body::IncomingBodyExt;
use spin_sdk::http::{self, FullBody, Method, Request};
use url::Url;

#[derive(Debug)]
pub struct ApiConfig {
    pub client_secret: String,
    pub host: String,
    pub access_token: String,
    pub client_token: String,
    pub revocation_list_id: String,
    pub account_switch_key: Option<String>,
    pub duration_seconds: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ApiError {
    RequestConfiguration,
    Transport {
        request_url: String,
        request_body: Value,
        message: String,
    },
    UpstreamStatus {
        status: u16,
        request_url: String,
        request_body: Value,
        response_headers: Vec<ResponseHeader>,
        response_body: String,
    },
    UpstreamBodyRead {
        status: u16,
        request_url: String,
        request_body: Value,
        response_headers: Vec<ResponseHeader>,
    },
    InvalidResponse {
        response: Option<RevocationResponse>,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum RevocationResponse {
    Json(Value),
    Error(String),
}

impl RevocationResponse {
    pub fn from_body(headers: &[ResponseHeader], body: &[u8]) -> Self {
        let is_json = headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case("content-type"))
            .is_some_and(|header| {
                let media_type = header.value.split(';').next().unwrap_or("").trim();
                media_type.eq_ignore_ascii_case("application/json")
                    || media_type.eq_ignore_ascii_case("application/problem+json")
            });
        if is_json {
            if let Ok(value) = serde_json::from_slice(body) {
                return Self::Json(value);
            }
        }
        Self::Error(String::from_utf8_lossy(body).into_owned())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ResponseHeader {
    pub name: String,
    pub value: String,
}

#[derive(Serialize)]
struct TokenIdentifier<'a> {
    id: &'a str,
    #[serde(rename = "durationSeconds")]
    duration_seconds: u32,
}

#[derive(Deserialize)]
pub struct ApiSuccess {
    pub count: u64,
    pub limit: u64,
    #[serde(skip)]
    pub response: Option<RevocationResponse>,
}

pub async fn revoke_ids(config: &ApiConfig, ids: &[String]) -> Result<ApiSuccess, ApiError> {
    let PreparedRequest {
        url,
        authorization,
        body,
        body_json,
    } = prepare_request(config, ids).map_err(|_| ApiError::RequestConfiguration)?;
    let request_url = url.to_string();
    let request = Request::builder()
        .method(Method::POST)
        .uri(request_url.as_str())
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .header("authorization", authorization)
        .body(FullBody::new(Bytes::from(body)))
        .map_err(|_| ApiError::RequestConfiguration)?;

    let response = match http::send(request).await {
        Ok(response) => response,
        Err(error) => {
            return Err(ApiError::Transport {
                request_url,
                request_body: body_json,
                message: format!("{error:#}"),
            });
        }
    };

    let (parts, body) = response.into_parts();
    let status = parts.status.as_u16();
    let response_headers = parts
        .headers
        .iter()
        .map(|(name, value)| ResponseHeader {
            name: name.as_str().to_owned(),
            value: String::from_utf8_lossy(value.as_bytes()).into_owned(),
        })
        .collect();
    let body = match body.bytes().await {
        Ok(body) => body,
        Err(_) if parts.status.is_success() => {
            return Err(ApiError::InvalidResponse { response: None });
        }
        Err(_) => {
            return Err(ApiError::UpstreamBodyRead {
                status,
                request_url,
                request_body: body_json,
                response_headers,
            });
        }
    };
    parse_response(status, &body, request_url, body_json, response_headers)
}

fn parse_response(
    status: u16,
    body: &[u8],
    request_url: String,
    request_body: Value,
    response_headers: Vec<ResponseHeader>,
) -> Result<ApiSuccess, ApiError> {
    if !(200..300).contains(&status) {
        return Err(ApiError::UpstreamStatus {
            status,
            request_url,
            request_body,
            response_headers,
            response_body: String::from_utf8_lossy(body).into_owned(),
        });
    }
    let response = Some(RevocationResponse::from_body(&response_headers, body));
    match serde_json::from_slice::<ApiSuccess>(body) {
        Ok(mut result) => {
            result.response = response;
            Ok(result)
        }
        Err(_) => Err(ApiError::InvalidResponse { response }),
    }
}

struct PreparedRequest {
    url: Url,
    authorization: String,
    body: Vec<u8>,
    body_json: Value,
}

fn prepare_request(config: &ApiConfig, ids: &[String]) -> Result<PreparedRequest> {
    let url = edgegrid::revocation_url(
        &config.host,
        &config.revocation_list_id,
        config.account_switch_key.as_deref(),
    )?;
    let body = ids
        .iter()
        .map(|id| TokenIdentifier {
            id,
            duration_seconds: config.duration_seconds,
        })
        .collect::<Vec<_>>();
    let body_json = serde_json::to_value(&body)?;
    let body = serde_json::to_vec(&body)?;
    let authorization = edgegrid::fresh_authorization_header(
        &Credentials {
            client_secret: &config.client_secret,
            client_token: &config.client_token,
            access_token: &config.access_token,
        },
        "POST",
        &url,
        &body,
    )?;

    Ok(PreparedRequest {
        url,
        authorization,
        body,
        body_json,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ApiConfig {
        ApiConfig {
            client_secret: "test-secret".to_owned(),
            host: "api.example.test".to_owned(),
            access_token: "test-access".to_owned(),
            client_token: "test-client".to_owned(),
            revocation_list_id: "list-1".to_owned(),
            account_switch_key: None,
            duration_seconds: 1800,
        }
    }

    #[test]
    fn request_uses_expected_path_body_and_optional_query() {
        let mut config = config();
        config.account_switch_key = Some("A-1:A-2".to_owned());
        config.revocation_list_id = "3827".to_owned();
        let ids = vec!["token-1".to_owned(), "token-2".to_owned()];

        let request = prepare_request(&config, &ids).expect("valid request");

        assert_eq!(
            request.url.as_str(),
            "https://api.example.test/taas/v2/revocation-lists/3827/identifiers/add?accountSwitchKey=A-1%3AA-2"
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&request.body).expect("JSON body"),
            serde_json::json!([
                { "id": "token-1", "durationSeconds": 1800 },
                { "id": "token-2", "durationSeconds": 1800 }
            ])
        );
        assert_eq!(
            request.body_json,
            serde_json::json!([
                { "id": "token-1", "durationSeconds": 1800 },
                { "id": "token-2", "durationSeconds": 1800 }
            ])
        );
        assert!(request.authorization.starts_with("EG1-HMAC-SHA256 "));
    }

    #[test]
    fn upstream_error_preserves_the_response_body() {
        let body = br#"{"detail":"Invalid EdgeGrid authorization"}"#;
        let request_url =
            "https://api.example.test/taas/v2/revocation-lists/list-1/identifiers/add".to_owned();
        let request_body = serde_json::json!([
            { "id": "token-1", "durationSeconds": 1800 }
        ]);
        let response_headers = vec![ResponseHeader {
            name: "content-type".to_owned(),
            value: "application/json".to_owned(),
        }];

        assert!(matches!(
            parse_response(
                403,
                body,
                request_url.clone(),
                request_body.clone(),
                response_headers.clone()
            ),
            Err(ApiError::UpstreamStatus {
                status: 403,
                request_url: actual_request_url,
                request_body: actual_request_body,
                response_headers: actual_response_headers,
                response_body,
            }) if actual_request_body == request_body
                && actual_request_url == request_url
                && actual_response_headers == response_headers
                && response_body == String::from_utf8_lossy(body).into_owned()
        ));
    }

    #[test]
    fn successful_response_is_still_parsed() {
        let request_url =
            "https://api.example.test/taas/v2/revocation-lists/list-1/identifiers/add".to_owned();
        let result = parse_response(
            200,
            br#"{"count":2,"limit":5000}"#,
            request_url,
            serde_json::Value::Null,
            Vec::new(),
        )
        .expect("valid success response");

        assert_eq!(result.count, 2);
        assert_eq!(result.limit, 5000);
    }

    fn content_type(value: &str) -> Vec<ResponseHeader> {
        vec![ResponseHeader {
            name: "Content-Type".to_owned(),
            value: value.to_owned(),
        }]
    }

    #[test]
    fn response_logging_classifies_exact_json_media_types_and_parameters() {
        let body = br#"{"detail":"rejected","extra":[1,true,null]}"#;
        let expected = serde_json::from_slice::<Value>(body).expect("JSON fixture");
        for media_type in [
            "application/json",
            "application/problem+json",
            "Application/JSON; charset=utf-8",
            " application/problem+json ; charset=UTF-8",
        ] {
            assert_eq!(
                RevocationResponse::from_body(&content_type(media_type), body),
                RevocationResponse::Json(expected.clone())
            );
        }
        for media_type in [
            "text/plain",
            "text/html",
            "application/jsonx",
            "application/problem+jsonx",
            "application/vendor+json",
            "",
        ] {
            assert_eq!(
                RevocationResponse::from_body(&content_type(media_type), body),
                RevocationResponse::Error(String::from_utf8_lossy(body).into_owned())
            );
        }
        assert_eq!(
            RevocationResponse::from_body(&[], body),
            RevocationResponse::Error(String::from_utf8_lossy(body).into_owned())
        );
    }

    #[test]
    fn malformed_or_empty_json_is_logged_as_text() {
        for media_type in ["application/json", "application/problem+json"] {
            for body in [b"not JSON\nfailure".as_slice(), b"", b"{\"bad\":\"\xff\"}"] {
                assert_eq!(
                    RevocationResponse::from_body(&content_type(media_type), body),
                    RevocationResponse::Error(String::from_utf8_lossy(body).into_owned())
                );
            }
        }
    }

    #[test]
    fn success_retains_the_complete_body_for_logging_without_changing_parsing() {
        for media_type in ["application/json", "application/problem+json", "text/plain"] {
            let body = br#"{"count":2,"limit":5000,"extra":{"accepted":true}}"#;
            let headers = content_type(media_type);
            let expected = RevocationResponse::from_body(&headers, body);
            let result = parse_response(
                200,
                body,
                "https://api.example.test".to_owned(),
                Value::Null,
                headers,
            )
            .expect("valid success response");

            assert_eq!(result.count, 2);
            assert_eq!(result.limit, 5000);
            assert_eq!(result.response, Some(expected));
        }
    }

    #[test]
    fn invalid_success_retains_json_or_text_for_logging() {
        for body in [
            br#"{"detail":"missing count and limit"}"#.as_slice(),
            b"invalid JSON\nupstream failure",
            b"null",
        ] {
            let headers = content_type("application/json");
            let expected = RevocationResponse::from_body(&headers, body);
            assert!(matches!(
                parse_response(
                    200,
                    body,
                    "https://api.example.test".to_owned(),
                    Value::Null,
                    headers
                ),
                Err(ApiError::InvalidResponse { response: Some(actual) }) if actual == expected
            ));
        }
    }
}
