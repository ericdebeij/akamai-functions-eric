use base64::Engine;
use serde_json::{json, Map, Value};
use spin_sdk::http::{IntoResponse, Request, Response};
use spin_sdk::http_component;

/// A Spin HTTP component that echoes the incoming request back as JSON.
#[http_component]
fn handle_http_mirror(req: Request) -> anyhow::Result<impl IntoResponse> {
    println!("Handling request to {:?}", req.header("spin-full-url"));

    // Collect headers into an object, grouping repeated names into an array of
    // values. Non-UTF-8 header values are base64-encoded.
    let mut headers: Map<String, Value> = Map::new();
    for (name, value) in req.headers() {
        let value = match value.as_str() {
            Some(s) => Value::String(s.to_string()),
            None => Value::String(base64::engine::general_purpose::STANDARD.encode(value.as_bytes())),
        };
        headers
            .entry(name.to_string())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .expect("header entry is always an array")
            .push(value);
    }

    // Decode the body as a UTF-8 string when the content-type indicates text or
    // JSON (and it is in fact valid UTF-8); otherwise fall back to base64.
    let content_type = req
        .header("content-type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let looks_textual = content_type.starts_with("text/")
        || content_type.contains("json")
        || content_type.contains("xml")
        || content_type.contains("x-www-form-urlencoded");
    let body_bytes = req.body();
    let (body, encoding) = match (looks_textual, std::str::from_utf8(body_bytes)) {
        _ if body_bytes.is_empty() => (String::new(), "utf8"),
        (true, Ok(s)) => (s.to_string(), "utf8"),
        _ => (
            base64::engine::general_purpose::STANDARD.encode(body_bytes),
            "base64",
        ),
    };

    // Prefer the full URL Spin exposes, falling back to the raw request URI.
    let url = req
        .header("spin-full-url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| req.uri().to_string());

    let payload = json!({
        "method": req.method().to_string(),
        "url": url,
        "headers": headers,
        "body": body,
        "bodyEncoding": encoding,
    });

    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(serde_json::to_vec(&payload)?)
        .build())
}
