//! Manual AWS Signature V4 signing for Linode Object Storage.
//!
//! Everything in this module is pure computation on strings and bytes: no network
//! calls, no filesystem, no AWS SDK. That is deliberate — the AWS SDK does not
//! compile to Wasm, and signing does not actually require it.
//!
//! [`sigv4_url`] is the only item the rest of the crate uses; the helpers above it
//! are the building blocks it needs.

use chrono::{DateTime, Utc};
use form_urlencoded::byte_serialize;
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

/// A type alias, so we can write `HmacSha256` instead of `Hmac<Sha256>` below.
type HmacSha256 = Hmac<Sha256>;

/// Computes HMAC-SHA256 of `data` using `key`, returning the raw bytes.
///
/// SigV4 chains several of these calls together to derive a signing key.
fn hmac_sha256(key: &[u8], data: &str) -> Vec<u8> {
    // `unwrap` is safe here: HMAC accepts a key of any length, so `new_from_slice`
    // cannot actually fail for this algorithm.
    let mut mac = HmacSha256::new_from_slice(key).unwrap();
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Hashes `data` with SHA-256 and returns it as a lowercase hex string,
/// which is the format SigV4 expects.
fn sha256_hex(data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    hex::encode(hasher.finalize())
}

/// Percent-encodes one path segment according to RFC 3986: every byte outside the
/// "unreserved" set becomes `%XX` with uppercase hex digits.
///
/// Do not be tempted to use `form_urlencoded::byte_serialize` for this. That encodes
/// for HTML form data, where a space becomes `+`. In a URL *path* a `+` means a
/// literal plus sign, so a key containing a space would be signed for the wrong
/// object and the request would fail or hit the wrong file.
fn uri_encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());

    // Iterating over `as_bytes()` rather than `chars()` keeps this correct for
    // non-ASCII text: a multi-byte character is encoded one byte at a time,
    // which is exactly what percent-encoding requires.
    for &b in segment.as_bytes() {
        match b {
            // The unreserved set: these are allowed to appear as-is.
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            // `{:02X}` formats the byte as two uppercase hex digits, e.g. ' ' -> "%20".
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Encodes an object key into a canonical URI: every segment is percent-encoded, but
/// the `/` separators are kept intact so the folder structure survives.
///
/// Both the signature and the outgoing URL must use this exact same encoding,
/// otherwise Linode computes a different signature and rejects the request.
fn s3_url_encode(path: &str) -> String {
    path.split('/')
        .map(uri_encode_segment)
        .collect::<Vec<String>>()
        .join("/")
}

/// Builds a pre-signed URL that grants the bearer permission to perform `method` on
/// `key` for the next 15 minutes. Nothing leaves the process while doing this: the
/// signature is derived from the secret key alone.
///
/// The AWS SigV4 recipe has four steps, followed below in order:
/// 1. Build a *canonical request* — a normalised description of the request.
/// 2. Build a *string to sign* from its hash.
/// 3. Derive a *signing key* by chaining HMACs over date, region, and service.
/// 4. Sign, and append the signature to the URL as a query parameter.
pub(crate) fn sigv4_url(
    method: &str,
    bucket: &str,
    key: &str,
    cluster: &str, // e.g. "nl-ams-1.linodeobjects.com"
    region: &str,  // Linode also accepts "us-east-1" regardless of the cluster
    access_key: &str,
    secret_key: &str,
) -> String {
    // Two timestamp formats are needed: a full one for the request, and a date-only
    // one for the credential scope. They must be consistent with each other.
    let now: DateTime<Utc> = Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let datestamp = now.format("%Y%m%d").to_string();

    // Linode uses the same virtual-hosted style as S3: the bucket is a subdomain.
    let host = format!("{}.{}", bucket, cluster);
    let canonical_uri = format!("/{}", s3_url_encode(key));

    // The scope limits the signature to one day, region, and service.
    let credential_scope = format!("{}/{}/s3/aws4_request", datestamp, region);

    // --- Step 1: the canonical request ---

    let mut query_params = vec![
        ("X-Amz-Algorithm", "AWS4-HMAC-SHA256".to_string()),
        (
            "X-Amz-Credential",
            format!("{}/{}", access_key, credential_scope),
        ),
        ("X-Amz-Date", amz_date.clone()),
        ("X-Amz-Expires", "900".to_string()), // Valid for 15 minutes.
        ("X-Amz-SignedHeaders", "host".to_string()),
    ];

    // SigV4 requires the query parameters in alphabetical order by name. Linode sorts
    // them the same way when it verifies, so any other order produces a mismatch.
    query_params.sort_by(|a, b| a.0.cmp(b.0));

    let canonical_query_string: String = query_params
        .iter()
        .map(|(k, v)| format!("{}={}", k, byte_serialize(v.as_bytes()).collect::<String>()))
        .collect::<Vec<String>>()
        .join("&");

    // Only `host` is signed. Any other header we send is therefore ignored by the
    // signature check, which is why forwarding a `content-type` on PUT is safe.
    let canonical_headers = format!("host:{}\n", host);
    let signed_headers = "host";

    // "UNSIGNED-PAYLOAD" tells Linode not to expect a hash of the body. That keeps us
    // from having to buffer and hash a whole upload in limited Wasm memory.
    let payload_hash = "UNSIGNED-PAYLOAD";

    // The field order and the newlines here are part of the specification.
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method, canonical_uri, canonical_query_string, canonical_headers, signed_headers, payload_hash
    );

    // --- Step 2: the string to sign ---

    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        amz_date,
        credential_scope,
        sha256_hex(&canonical_request)
    );

    // --- Step 3: derive the signing key ---

    // Each HMAC uses the previous result as its key. The chain means the final key is
    // only valid for this date, region, and service, so a leaked signature is limited
    // in scope. The secret key itself is never sent anywhere.
    let k_secret = format!("AWS4{}", secret_key);
    let k_date = hmac_sha256(k_secret.as_bytes(), &datestamp);
    let k_region = hmac_sha256(&k_date, region);
    let k_service = hmac_sha256(&k_region, "s3");
    let k_signing = hmac_sha256(&k_service, "aws4_request");

    // --- Step 4: sign, and assemble the final URL ---

    let signature = hex::encode(hmac_sha256(&k_signing, &string_to_sign));

    format!(
        "https://{}{}?{}&X-Amz-Signature={}",
        host, canonical_uri, canonical_query_string, signature
    )
}
