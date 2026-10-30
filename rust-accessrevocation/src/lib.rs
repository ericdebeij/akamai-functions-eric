mod api;
mod edgegrid;
mod webhook;

use chrono::{SecondsFormat, Utc};
use futures_util::StreamExt;
use serde_json::{json, Value};
use spin_sdk::http::body::IncomingBodyExt;
use spin_sdk::http::{IntoResponse, Method, Request, Response};
use spin_sdk::{http_service, variables};
use subtle::ConstantTimeEq;
use webhook::{WebhookError, MAX_IDS, MAX_WEBHOOK_BODY_BYTES};

const WEBHOOK_SECRET_HEADER: &str = "x-webhook-secret";
const MIN_REVOCATION_SECONDS: u32 = 1800;

#[http_service]
async fn handle_revokeaccess(req: Request) -> anyhow::Result<impl IntoResponse> {
    if req.method() != Method::POST {
        return json_response(405, json!({ "error": "method_not_allowed" }));
    }

    let expected_secret = match variables::get("webhook_secret").await {
        Ok(secret) if !secret.is_empty() => secret,
        _ => {
            return json_response(500, json!({ "error": "configuration_error" }));
        }
    };

    let supplied_secret = req
        .headers()
        .get(WEBHOOK_SECRET_HEADER)
        .and_then(|value| value.to_str().ok());
    if !supplied_secret.is_some_and(|secret| secrets_match(&expected_secret, secret)) {
        return json_response(401, json!({ "error": "unauthorized" }));
    }

    let content_type = req
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !is_json_content_type(content_type) {
        return json_response(
            415,
            json!({ "error": "content_type_must_be_application_json" }),
        );
    }

    let mut body_stream = req.into_body().stream();
    let mut body = Vec::new();
    while let Some(chunk) = body_stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) => {
                return json_response(400, json!({ "error": "invalid_request_body" }));
            }
        };
        if body.len().saturating_add(chunk.len()) > MAX_WEBHOOK_BODY_BYTES {
            return json_response(413, json!({ "error": "payload_too_large" }));
        }
        body.extend_from_slice(&chunk);
    }

    let ids = match webhook::extract_firing_ids(&body) {
        Ok(ids) => ids,
        Err(WebhookError::InvalidJson) => {
            return json_response(400, json!({ "error": "invalid_json" }));
        }
        Err(WebhookError::MissingStreamIds) => {
            return json_response(400, json!({ "error": "missing_stream_ids" }));
        }
        Err(WebhookError::TooManyIds) => {
            return json_response(
                413,
                json!({ "error": "too_many_stream_ids", "maximum": MAX_IDS }),
            );
        }
    };

    if ids.is_empty() {
        return json_response(
            200,
            json!({ "status": "ignored", "reason": "no_firing_alerts" }),
        );
    }

    let config = match load_api_config().await {
        Ok(config) => config,
        Err(_) => {
            return json_response(500, json!({ "error": "configuration_error" }));
        }
    };

    match api::revoke_ids(&config, &ids).await {
        Ok(result) => logged_json_response(
            200,
            json!({
                "status": "revoked",
                "requested": ids.len(),
                "count": result.count,
                "limit": result.limit
            }),
            &ids,
            result.response.as_ref(),
        ),
        Err(api::ApiError::UpstreamStatus {
            status,
            request_url,
            request_body,
            response_headers,
            response_body,
        }) => upstream_rejection_response(
            status,
            request_url,
            request_body,
            response_headers,
            Some(response_body),
        ),
        Err(api::ApiError::UpstreamBodyRead {
            status,
            request_url,
            request_body,
            response_headers,
        }) => {
            upstream_rejection_response(status, request_url, request_body, response_headers, None)
        }
        Err(api::ApiError::Transport {
            request_url,
            request_body,
            message,
        }) => upstream_transport_error_response(request_url, request_body, message),
        Err(api::ApiError::InvalidResponse { response }) => logged_json_response(
            502,
            json!({ "error": "invalid_revocation_api_response" }),
            &[],
            response.as_ref(),
        ),
        Err(api::ApiError::RequestConfiguration) => {
            json_response(500, json!({ "error": "configuration_error" }))
        }
    }
}

async fn load_api_config() -> Result<api::ApiConfig, &'static str> {
    let client_secret = required_variable("client_secret").await?;
    let host = required_variable("host").await?;
    let access_token = required_variable("access_token").await?;
    let client_token = required_variable("client_token").await?;
    let revocation_list_id = required_variable("revocation_list_id").await?;

    let account_switch_key = variables::get("account_switch_key")
        .await
        .map_err(|_| "account_switch_key")?;
    let account_switch_key = (!account_switch_key.is_empty()).then_some(account_switch_key);

    let duration_text = required_variable("revocation_duration_seconds").await?;
    let duration_seconds = duration_text
        .parse::<u32>()
        .ok()
        .filter(|duration| *duration >= MIN_REVOCATION_SECONDS)
        .ok_or("revocation_duration_seconds")?;

    Ok(api::ApiConfig {
        client_secret,
        host,
        access_token,
        client_token,
        revocation_list_id,
        account_switch_key,
        duration_seconds,
    })
}

async fn required_variable(name: &'static str) -> Result<String, &'static str> {
    variables::get(name)
        .await
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or(name)
}

fn secrets_match(expected: &str, supplied: &str) -> bool {
    let expected = expected.as_bytes();
    let supplied = supplied.as_bytes();
    expected.len() == supplied.len() && bool::from(expected.ct_eq(supplied))
}

fn is_json_content_type(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("application/json"))
}

fn json_response(status: u16, body: Value) -> anyhow::Result<Response<String>> {
    logged_json_response(status, body, &[], None)
}

fn request_log_line(
    status: u16,
    body: &Value,
    ids: &[String],
    response: Option<&api::RevocationResponse>,
) -> String {
    let mut entry = json!({
        "timestamp": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        "status_code": status
    });
    if let Some(error) = body.get("error") {
        entry["error"] = error.clone();
    } else {
        entry["streams_revoked"] = json!(ids);
    }
    match response {
        Some(api::RevocationResponse::Json(value)) => entry["revocationResponse"] = value.clone(),
        Some(api::RevocationResponse::Error(text)) => entry["revocationError"] = json!(text),
        None => {}
    }
    entry.to_string()
}

fn logged_json_response(
    status: u16,
    body: Value,
    ids: &[String],
    response: Option<&api::RevocationResponse>,
) -> anyhow::Result<Response<String>> {
    eprintln!("{}", request_log_line(status, &body, ids, response));
    Ok(Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(body.to_string())?)
}

fn upstream_rejection_response(
    status: u16,
    request_url: String,
    request_body: Value,
    response_headers: Vec<api::ResponseHeader>,
    response_body: Option<String>,
) -> anyhow::Result<Response<String>> {
    let response = response_body
        .as_ref()
        .map(|body| api::RevocationResponse::from_body(&response_headers, body.as_bytes()));
    let body = match response_body {
        Some(response_body) => json!({
            "error": "revocation_api_rejected_request",
            "upstream_status": status,
            "response_status": status,
            "upstream_request_url": request_url,
            "upstream_request_body": request_body,
            "response_headers": response_headers,
            "upstream_response_body": response_body
        }),
        None => json!({
            "error": "revocation_api_rejected_request",
            "upstream_status": status,
            "response_status": status,
            "upstream_request_url": request_url,
            "upstream_request_body": request_body,
            "response_headers": response_headers,
            "upstream_response_body": null,
            "upstream_response_body_error": "could_not_read"
        }),
    };
    logged_json_response(502, body, &[], response.as_ref())
}

fn upstream_transport_error_response(
    request_url: String,
    request_body: Value,
    message: String,
) -> anyhow::Result<Response<String>> {
    json_response(
        502,
        json!({
            "error": "revocation_api_unavailable",
            "upstream_request_url": request_url,
            "upstream_request_body": request_body,
            "response_status": null,
            "response_headers": null,
            "transport_error": message
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_request_log_has_utc_timestamp_status_and_revoked_ids() {
        let ids = vec!["stream-1".to_owned(), "stream-2".to_owned()];
        let body = json!({ "status": "revoked", "requested": 2, "count": 2, "limit": 5000 });
        let before = Utc::now().timestamp_millis();
        let line = request_log_line(200, &body, &ids, None);
        let after = Utc::now().timestamp_millis();
        let entry: Value = serde_json::from_str(&line).expect("JSON log");
        let timestamp = entry["timestamp"].as_str().expect("timestamp");
        let parsed = chrono::DateTime::parse_from_rfc3339(timestamp).expect("RFC 3339 timestamp");

        assert!(timestamp.ends_with('Z'));
        assert_eq!(timestamp.len(), 24);
        assert!((before..=after).contains(&parsed.timestamp_millis()));
        assert_eq!(entry["status_code"], 200);
        assert_eq!(entry["streams_revoked"], json!(ids));
        assert_eq!(entry.as_object().expect("log object").len(), 3);
        assert_eq!(line.lines().count(), 1);
    }

    #[test]
    fn ignored_request_log_has_an_empty_revoked_list() {
        let body = json!({ "status": "ignored", "reason": "no_firing_alerts" });
        let entry: Value =
            serde_json::from_str(&request_log_line(200, &body, &[], None)).expect("JSON log");

        assert_eq!(entry["status_code"], 200);
        assert_eq!(entry["streams_revoked"], json!([]));
        assert!(entry.get("error").is_none());
        assert!(entry.get("revocationResponse").is_none());
        assert!(entry.get("revocationError").is_none());
    }

    #[test]
    fn upstream_body_is_logged_in_exactly_one_field_on_success_and_failure() {
        for status in [200, 502] {
            let body = if status == 200 {
                json!({ "status": "revoked" })
            } else {
                json!({ "error": "revocation_api_rejected_request" })
            };
            for (media_type, raw, expected_field) in [
                (
                    "application/json",
                    "{\"count\":1,\"limit\":5000}",
                    "revocationResponse",
                ),
                (
                    "application/problem+json; charset=utf-8",
                    "{\n\"detail\":\"Denied\\nTry later\"\n}",
                    "revocationResponse",
                ),
                (
                    "text/html",
                    "<html>Denied\nTry later</html>",
                    "revocationError",
                ),
                ("", "{\"detail\":\"no content type\"}", "revocationError"),
                (
                    "application/json",
                    "not JSON\nupstream failure",
                    "revocationError",
                ),
                ("application/json", "null", "revocationResponse"),
            ] {
                let headers = vec![api::ResponseHeader {
                    name: "content-type".to_owned(),
                    value: media_type.to_owned(),
                }];
                let upstream = api::RevocationResponse::from_body(&headers, raw.as_bytes());
                let line =
                    request_log_line(status, &body, &["stream-1".to_owned()], Some(&upstream));
                let entry: Value = serde_json::from_str(&line).expect("JSON log");
                let other_field = if expected_field == "revocationResponse" {
                    "revocationError"
                } else {
                    "revocationResponse"
                };
                let expected = if expected_field == "revocationResponse" {
                    serde_json::from_str(raw).expect("JSON fixture")
                } else {
                    json!(raw)
                };

                assert_eq!(entry[expected_field], expected);
                assert!(entry.get(expected_field).is_some());
                assert!(entry.get(other_field).is_none());
                assert_eq!(entry.as_object().expect("log object").len(), 4);
                assert_eq!(entry["status_code"], status);
                assert_eq!(line.lines().count(), 1);
                if status == 200 {
                    assert_eq!(entry["streams_revoked"], json!(["stream-1"]));
                } else {
                    assert_eq!(entry["error"], "revocation_api_rejected_request");
                    assert!(entry.get("streams_revoked").is_none());
                }
            }
        }
    }

    #[test]
    fn error_logs_include_only_the_error_and_not_response_diagnostics() {
        for (status, error) in [
            (405, "method_not_allowed"),
            (500, "configuration_error"),
            (401, "unauthorized"),
            (415, "content_type_must_be_application_json"),
            (400, "invalid_request_body"),
            (413, "payload_too_large"),
            (400, "invalid_json"),
            (400, "missing_stream_ids"),
            (413, "too_many_stream_ids"),
            (502, "revocation_api_rejected_request"),
            (502, "revocation_api_unavailable"),
            (502, "invalid_revocation_api_response"),
        ] {
            let body = json!({
                "error": error,
                "upstream_response_body": "must not appear in logs",
                "response_headers": [{"name": "sensitive", "value": "omit"}]
            });
            let line = request_log_line(status, &body, &["not-revoked".to_owned()], None);
            let entry: Value = serde_json::from_str(&line).expect("JSON log");

            assert_eq!(entry["status_code"], status);
            assert_eq!(entry["error"], error);
            assert_eq!(entry.as_object().expect("log object").len(), 3);
            assert!(entry.get("streams_revoked").is_none());
            assert_eq!(line.lines().count(), 1);
        }
    }

    #[test]
    fn request_logs_escape_newlines_and_control_characters() {
        let ids = vec!["stream\nwith\r\nnewlines\t\"quotes\"".to_owned()];
        let line = request_log_line(200, &json!({ "status": "revoked" }), &ids, None);
        let entry: Value = serde_json::from_str(&line).expect("JSON log");

        assert_eq!(line.lines().count(), 1);
        assert!(!line.contains('\r'));
        assert!(!line.contains('\t'));
        assert_eq!(entry["streams_revoked"], json!(ids));

        let error = "error\nwith\r\nnewlines";
        let line = request_log_line(500, &json!({ "error": error }), &[], None);
        let entry: Value = serde_json::from_str(&line).expect("JSON log");
        assert_eq!(line.lines().count(), 1);
        assert_eq!(entry["error"], error);
    }

    #[test]
    fn logged_success_preserves_the_http_response() {
        let body = json!({ "status": "revoked", "requested": 1, "count": 1, "limit": 5000 });
        let upstream = api::RevocationResponse::Json(json!({
            "count": 1,
            "limit": 5000,
            "extra": { "accepted": true }
        }));
        let response =
            logged_json_response(200, body.clone(), &["stream-1".to_owned()], Some(&upstream))
                .expect("valid success response");

        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], "application/json");
        assert_eq!(response.body(), &body.to_string());
    }

    #[test]
    fn non_json_upstream_rejection_preserves_the_http_response() {
        let original_body = "<html>Unavailable\nTry later</html>";
        let response = upstream_rejection_response(
            503,
            "https://api.example.test".to_owned(),
            json!([]),
            vec![api::ResponseHeader {
                name: "content-type".to_owned(),
                value: "text/html".to_owned(),
            }],
            Some(original_body.to_owned()),
        )
        .expect("valid error response");
        let body: Value = serde_json::from_str(response.body()).expect("JSON response");

        assert_eq!(response.status(), 502);
        assert_eq!(body["response_status"], 503);
        assert_eq!(body["upstream_response_body"], original_body);
        assert!(body.get("revocationError").is_none());
        assert!(body.get("revocationResponse").is_none());
    }

    #[test]
    fn shared_secret_comparison_matches_only_exact_values() {
        assert!(secrets_match("test-secret", "test-secret"));
        assert!(!secrets_match("test-secret", "test-secret "));
        assert!(!secrets_match("test-secret", "other-secret"));
    }

    #[test]
    fn json_content_type_allows_parameters_but_rejects_prefix_matches() {
        assert!(is_json_content_type("application/json"));
        assert!(is_json_content_type("Application/JSON; charset=utf-8"));
        assert!(!is_json_content_type("application/jsonx"));
        assert!(!is_json_content_type("text/json"));
    }

    #[test]
    fn upstream_rejection_response_includes_the_original_body() {
        let original_body = r#"{"detail":"Invalid EdgeGrid authorization"}"#;
        let request_url =
            "https://api.example.test/taas/v2/revocation-lists/list-1/identifiers/add";
        let request_body = json!([{ "id": "token-1", "durationSeconds": 1800 }]);
        let response_headers = vec![api::ResponseHeader {
            name: "content-type".to_owned(),
            value: "application/json".to_owned(),
        }];
        let response = upstream_rejection_response(
            403,
            request_url.to_owned(),
            request_body.clone(),
            response_headers,
            Some(original_body.to_owned()),
        )
        .expect("valid error response");
        let body: Value = serde_json::from_str(response.body()).expect("JSON response");

        assert_eq!(response.status(), 502);
        assert_eq!(body["upstream_status"], 403);
        assert_eq!(body["response_status"], 403);
        assert_eq!(body["upstream_request_url"], request_url);
        assert_eq!(body["upstream_request_body"], request_body);
        assert_eq!(
            body["response_headers"],
            json!([{ "name": "content-type", "value": "application/json" }])
        );
        assert_eq!(body["upstream_response_body"], original_body);
        assert!(body.get("upstream_response_body_error").is_none());
    }

    #[test]
    fn upstream_rejection_response_reports_unreadable_body() {
        let request_url =
            "https://api.example.test/taas/v2/revocation-lists/list-1/identifiers/add";
        let request_body = json!([{ "id": "token-1", "durationSeconds": 1800 }]);
        let response_headers = vec![api::ResponseHeader {
            name: "retry-after".to_owned(),
            value: "30".to_owned(),
        }];
        let response = upstream_rejection_response(
            403,
            request_url.to_owned(),
            request_body.clone(),
            response_headers,
            None,
        )
        .expect("valid error response");
        let body: Value = serde_json::from_str(response.body()).expect("JSON response");

        assert_eq!(body["response_status"], 403);
        assert_eq!(body["response_headers"][0]["name"], "retry-after");
        assert_eq!(body["response_headers"][0]["value"], "30");
        assert_eq!(body["upstream_request_url"], request_url);
        assert_eq!(body["upstream_request_body"], request_body);
        assert_eq!(body["upstream_response_body"], Value::Null);
        assert_eq!(body["upstream_response_body_error"], "could_not_read");
    }

    #[test]
    fn rate_limit_response_reports_status_and_retry_header() {
        let response = upstream_rejection_response(
            429,
            "https://api.example.test/taas/v2/revocation-lists/3827/identifiers/add".to_owned(),
            json!([{ "id": "token-1", "durationSeconds": 1800 }]),
            vec![api::ResponseHeader {
                name: "retry-after".to_owned(),
                value: "30".to_owned(),
            }],
            Some(r#"{"detail":"Rate limit exceeded"}"#.to_owned()),
        )
        .expect("valid rate-limit response");
        let body: Value = serde_json::from_str(response.body()).expect("JSON response");

        assert_eq!(body["response_status"], 429);
        assert_eq!(body["response_headers"][0]["name"], "retry-after");
        assert_eq!(body["response_headers"][0]["value"], "30");
    }

    #[test]
    fn transport_error_reports_that_no_response_metadata_exists() {
        let request_url = "https://api.example.test/taas/v2/revocation-lists/3827/identifiers/add";
        let request_body = json!([{ "id": "token-1", "durationSeconds": 1800 }]);
        let response = upstream_transport_error_response(
            request_url.to_owned(),
            request_body.clone(),
            "outbound request failed".to_owned(),
        )
        .expect("valid error response");
        let body: Value = serde_json::from_str(response.body()).expect("JSON response");

        assert_eq!(response.status(), 502);
        assert_eq!(body["error"], "revocation_api_unavailable");
        assert_eq!(body["upstream_request_url"], request_url);
        assert_eq!(body["upstream_request_body"], request_body);
        assert_eq!(body["response_status"], Value::Null);
        assert_eq!(body["response_headers"], Value::Null);
        assert_eq!(body["transport_error"], "outbound request failed");
    }
}
