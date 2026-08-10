use spin_sdk::http::{body::IncomingBodyExt, IntoResponse, Method, Request, Response, StatusCode};
use spin_sdk::http_service;
use spin_sdk::key_value::Store;

fn parse_request_path(path: &str) -> Option<(bool, Option<&str>)> {
    let path = path.trim_end_matches('/');
    if path == "/kv" {
        return Some((true, None));
    }

    if let Some(key) = path.strip_prefix("/kv/") {
        if !key.is_empty() {
            return Some((false, Some(key)));
        }
    }

    None
}

/// A simple Spin HTTP component.
#[http_service]
async fn handle_kvsample(req: Request) -> anyhow::Result<impl IntoResponse> {
    let path = req.uri().path().to_string();
    let Some((is_list, key)) = parse_request_path(&path) else {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("content-type", "text/plain")
            .body("Not found".to_string()));
    };

    let store = Store::open_default().await?;

    match (req.method(), is_list, key) {
        (method, true, _) if method == Method::GET => {
            let keys = store.get_keys().await;
            let keys: Vec<String> = keys.collect().await?;
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .body(format!("{:?}", keys)))
        }
        (method, false, Some(key)) if method == Method::GET => {
            let value = store.get(key).await?;
            match value {
                Some(value) => Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "text/plain")
                    .body(String::from_utf8_lossy(&value).to_string())),
                None => Ok(Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .header("content-type", "text/plain")
                    .body("Not found".to_string())),
            }
        }
        (method, false, Some(key)) if method == &Method::POST || method == &Method::PUT => {
            let body = req.into_body().bytes().await?;
            store.set(key, &body).await?;
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/plain")
                .body(String::from_utf8_lossy(&body).to_string()))
        }
        (method, false, Some(key)) if method == &Method::DELETE => {
            store.delete(key).await?;
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/plain")
                .body("Deleted".to_string()))
        }
        _ => Ok(Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .header("content-type", "text/plain")
            .body("Method not allowed".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_request_path;

    #[test]
    fn parses_the_list_route() {
        assert_eq!(parse_request_path("/kv"), Some((true, None)));
    }

    #[test]
    fn parses_a_named_key_route() {
        assert_eq!(parse_request_path("/kv/example"), Some((false, Some("example"))));
    }

    #[test]
    fn rejects_unknown_paths() {
        assert_eq!(parse_request_path("/hello"), None);
    }
}
