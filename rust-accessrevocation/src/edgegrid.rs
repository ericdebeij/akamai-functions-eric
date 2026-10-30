use anyhow::{bail, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

const MAX_SIGNED_BODY_BYTES: usize = 131_072;

#[derive(Debug)]
pub struct Credentials<'a> {
    pub client_secret: &'a str,
    pub client_token: &'a str,
    pub access_token: &'a str,
}

pub fn revocation_url(
    host: &str,
    revocation_list_id: &str,
    account_switch_key: Option<&str>,
) -> Result<Url> {
    if host.is_empty()
        || !host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        || revocation_list_id.trim().is_empty()
    {
        bail!("invalid API host or revocation list ID");
    }

    let mut url = Url::parse(&format!("https://{host}/"))?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("invalid API URL"))?;
        path.pop_if_empty()
            .push("taas")
            .push("v2")
            .push("revocation-lists")
            .push(revocation_list_id)
            .push("identifiers")
            .push("add");
    }

    if let Some(key) = account_switch_key.filter(|key| !key.is_empty()) {
        url.query_pairs_mut().append_pair("accountSwitchKey", key);
    }

    Ok(url)
}

pub fn authorization_header(
    credentials: &Credentials<'_>,
    method: &str,
    url: &Url,
    body: &[u8],
    timestamp: &str,
    nonce: &str,
) -> Result<String> {
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("request URL has no host"))?;
    let netloc = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    let path = match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_owned(),
    };

    let body_hash = if method.eq_ignore_ascii_case("POST") && !body.is_empty() {
        let bytes_to_sign = &body[..body.len().min(MAX_SIGNED_BODY_BYTES)];
        BASE64.encode(Sha256::digest(bytes_to_sign))
    } else {
        String::new()
    };

    let unsigned_header = format!(
        "EG1-HMAC-SHA256 client_token={};access_token={};timestamp={timestamp};nonce={nonce};",
        credentials.client_token, credentials.access_token
    );
    let data_to_sign = [
        method.to_ascii_uppercase(),
        url.scheme().to_owned(),
        netloc,
        path,
        String::new(),
        body_hash,
        unsigned_header.clone(),
    ]
    .join("\t");

    let signing_key = BASE64.encode(hmac_sha256(
        credentials.client_secret.as_bytes(),
        timestamp.as_bytes(),
    )?);
    let signature = BASE64.encode(hmac_sha256(
        signing_key.as_bytes(),
        data_to_sign.as_bytes(),
    )?);

    Ok(format!("{unsigned_header}signature={signature}"))
}

pub fn fresh_authorization_header(
    credentials: &Credentials<'_>,
    method: &str,
    url: &Url,
    body: &[u8],
) -> Result<String> {
    let timestamp = Utc::now().format("%Y%m%dT%H:%M:%S+0000").to_string();
    let nonce = Uuid::new_v4().to_string();
    authorization_header(credentials, method, url, body, &timestamp, &nonce)
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    let mut mac =
        HmacSha256::new_from_slice(key).map_err(|_| anyhow::anyhow!("invalid HMAC key"))?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_api_url_and_encodes_optional_account_switch_key() {
        let url = revocation_url("api.example.test", "list-1", Some("A-1:A-2")).expect("valid URL");

        assert_eq!(
            url.as_str(),
            "https://api.example.test/taas/v2/revocation-lists/list-1/identifiers/add?accountSwitchKey=A-1%3AA-2"
        );
    }

    #[test]
    fn omits_empty_account_switch_key() {
        let url = revocation_url("api.example.test", "list-1", Some("")).expect("valid URL");
        assert_eq!(
            url.as_str(),
            "https://api.example.test/taas/v2/revocation-lists/list-1/identifiers/add"
        );
    }

    #[test]
    fn matches_deterministic_edgegrid_signature_fixture() {
        let credentials = Credentials {
            client_secret: "test-secret",
            client_token: "test-client",
            access_token: "test-access",
        };
        let url = revocation_url("api.example.test", "list-1", Some("A-1:A-2")).expect("valid URL");
        let body = br#"[{"id":"test-id","durationSeconds":1800}]"#;

        let header = authorization_header(
            &credentials,
            "POST",
            &url,
            body,
            "20261021T12:00:00+0000",
            "d89947e5-8ead-4462-b6ec-2099d3e71c16",
        )
        .expect("signature");

        assert_eq!(
            header,
            "EG1-HMAC-SHA256 client_token=test-client;access_token=test-access;timestamp=20261021T12:00:00+0000;nonce=d89947e5-8ead-4462-b6ec-2099d3e71c16;signature=+Vvq+vY23zUSQ5W8HThIPVRI6hixWkxIXCth5lfpjNE="
        );
    }

    #[test]
    fn rejects_invalid_host_and_empty_list_id() {
        assert!(revocation_url("https://example.test", "list-1", None).is_err());
        assert!(revocation_url("api.example.test", "", None).is_err());
    }
}
