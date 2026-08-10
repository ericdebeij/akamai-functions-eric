# http-mirror

A [Spin](https://spinframework.dev) HTTP component (Rust → WASM) that echoes any
incoming request back as JSON. Deploys to Akamai Functions.

## What it does

Single handler in [src/lib.rs](src/lib.rs) (`handle_http_mirror`, route `/...`)
returns `200 application/json` describing the request:

```json
{ "method", "url", "headers", "body", "bodyEncoding" }
```

- **headers**: object; each name maps to an array of values (repeated headers
  grouped). Non-UTF-8 values are base64-encoded.
- **body/bodyEncoding**: UTF-8 string (`"utf8"`) when the body is empty, or
  when content-type looks textual (`text/*`, json, xml, x-www-form-urlencoded)
  and bytes are valid UTF-8; otherwise base64 (`"base64"`).
- **url**: prefers Spin's `spin-full-url` header, falls back to `req.uri()`.

## Stack

- Rust 2021, `crate-type = ["cdylib"]`, target `wasm32-wasip1`
- Deps: `spin-sdk`, `serde_json`, `base64`, `anyhow`
- Manifest: [spin.toml](spin.toml) — component `http-mirror`, no allowed
  outbound hosts

## Commands

```bash
cargo build --target wasm32-wasip1 --release   # build (via `spin build`)
spin build                                      # build using manifest
spin up                                         # run locally
```
