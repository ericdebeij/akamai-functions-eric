//! A Spin HTTP component that proxies requests to Linode Object Storage (S3-compatible).
//!
//! The flow for every request is the same:
//!
//! 1. Work out the object key from the request path (`/storage/a/b.txt` -> `a/b.txt`),
//!    decoding it with [`percent::percent_decode`].
//! 2. Build a *pre-signed URL* for Linode with [`sigv4::sigv4_url`], which computes an
//!    AWS Signature V4 by hand. Signing is pure maths on strings and bytes, so no
//!    network call and no AWS SDK is needed — which matters here, because the AWS SDK
//!    does not compile to Wasm.
//! 3. Send the request to that URL and pass Linode's response back to the client.
//!
//! The signing lives in [`sigv4`] and path decoding in [`percent`]; this file is the
//! HTTP handling around them.

mod percent;
mod sigv4;

use bytes::Bytes;
use http::Response as HttpResponse;
use percent::percent_decode;
use sigv4::sigv4_url;
use spin_sdk::http::body::IncomingBodyExt;
use spin_sdk::http::{FullBody, HeaderValue, IntoResponse, Method, Request, StatusCode};
use spin_sdk::http_service;
use spin_sdk::variables;

/// The component's entry point. `#[http_service]` is what makes Spin route HTTP
/// requests here; the route itself (`/storage/...`) is configured in `spin.toml`.
///
/// Returning `anyhow::Result` means we can use the `?` operator on anything that can
/// fail: Spin turns an `Err` into a 500 response for us.
#[http_service]
async fn handle_s3sample(req: Request) -> anyhow::Result<impl IntoResponse> {
    // Spin puts the wildcard part of the route (everything after "/storage/") into a
    // `spin-path-info` header, so the route prefix does not have to be repeated here.
    //
    // Read this as a chain of "only if the previous step produced a value":
    // find the header, read it as text, strip the leading '/', then decode it.
    // The value arrives exactly as the client sent it, so it is still percent-encoded.
    // `unwrap_or_default()` turns a missing header into an empty string.
    let key = req
        .headers()
        .get("spin-path-info")
        .and_then(|v| v.to_str().ok())
        .map(|p| percent_decode(p.trim_start_matches('/')))
        .unwrap_or_default();

    if key.is_empty() {
        return Ok(HttpResponse::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(FullBody::new(Bytes::from_static(
                b"missing object key: use /storage/<path>",
            )))?);
    }

    // Only these three verbs are forwarded. `clone` is needed because `method()`
    // borrows from `req`, and `req` is consumed further down when we read the body.
    let method = req.method().clone();
    let sends_body = match method {
        Method::GET | Method::DELETE => false,
        Method::PUT => true,
        _ => {
            // `Allow` tells the client which methods it should have used instead.
            return Ok(HttpResponse::builder()
                .status(StatusCode::METHOD_NOT_ALLOWED)
                .header("allow", "GET, PUT, DELETE")
                .body(FullBody::new(Bytes::from_static(
                    b"method not allowed: use GET, PUT or DELETE",
                )))?);
        }
    };

    // Copy the client's content-type, otherwise an upload is stored as
    // binary/octet-stream. This has to happen before `req` is consumed below.
    let upload_content_type = req
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    // Configuration comes from Spin variables (declared in `spin.toml`), not from
    // `std::env::var`. Secrets stay out of the source this way.
    let access_key = variables::get("s3_access_key").await?;
    let secret_key = variables::get("s3_secret_key").await?;
    let bucket = variables::get("s3_bucket").await?;
    let cluster = variables::get("s3_endpoint").await?;
    let region = variables::get("s3_region").await?;

    let sigv4_url = sigv4_url(
        method.as_str(),
        &bucket,
        &key,
        &cluster,
        &region,
        &access_key,
        &secret_key,
    );

    // Log the key but never the signed URL: that URL contains a working signature and
    // would let anyone reading the logs access the object.
    eprintln!("{} bucket={} key={}", method, bucket, key);

    // Only PUT carries a payload. GET and DELETE send an empty body of the same type,
    // which keeps a single code path for building and sending the request.
    let upload_body = if sends_body {
        req.into_body().bytes().await?
    } else {
        Bytes::new()
    };

    let mut request = Request::builder()
        .method(method)
        .uri(&sigv4_url)
        .body(FullBody::new(upload_body))?;

    if let Some(content_type) = upload_content_type {
        request
            .headers_mut()
            .insert("content-type", HeaderValue::from_str(&content_type)?);
    }

    // Outbound requests only work if the host is listed in `allowed_outbound_hosts`
    // in `spin.toml`; Spin blocks anything else.
    let response = spin_sdk::http::send(request).await?;

    // Read the status and headers before `into_body()` consumes the response.
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = response.into_body().bytes().await?;

    // Pass Linode's answer straight back, success or failure. On failure that means the
    // original XML error body, so the difference between SignatureDoesNotMatch (we
    // signed something wrong) and NoSuchKey (the object is simply missing) is visible
    // from a plain curl. Note this buffers the whole body in memory, which is fine for
    // small objects but is the reason large ones are not supported yet.
    let mut builder = HttpResponse::builder().status(status);
    if let Some(content_type) = content_type {
        builder = builder.header("content-type", content_type);
    }

    Ok(builder.body(FullBody::new(body))?)
}
