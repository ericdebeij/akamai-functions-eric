# CLAUDE.md — Project Guidelines & Architecture

Server-side WebAssembly HTTP component built on the **Spin framework** (Spin SDK 6.0, Rust). It acts as a signing proxy in front of Linode Object Storage (S3-compatible): a client request comes in on `/storage/...`, the component generates an AWS SigV4 pre-signed URL in-memory, and forwards the request to Linode.

## 🧩 Runtime: Spin

This is **not** a generic WASI binary and **not** Akamai EdgeWorkers / Wasm Workers Server. It is a Spin app, which dictates most of the constraints below.

- **Crate type:** `cdylib` — the output is a Wasm component, never a binary.
- **Target:** `wasm32-wasip2` (component model). Not `wasm32-wasi`, not `wasm32-unknown-unknown`.
- **Entry point:** `#[http_service]` on an `async fn` taking `spin_sdk::http::Request` and returning `anyhow::Result<impl IntoResponse>` (see [src/lib.rs](src/lib.rs)). SDK 6 uses `#[http_service]`; older `#[http_component]` + `Params` examples do not apply.
- **HTTP types:** use `spin_sdk::http::*` (`Request`, `Response`, `Method`, `StatusCode`, `EmptyBody`, `IncomingBodyExt`) — not hand-rolled `http::Request<Vec<u8>>`.
- **Outbound HTTP:** `spin_sdk::http::send(request).await`. Every outbound host must be listed in `allowed_outbound_hosts` in [spin.toml](spin.toml) or the call is denied at runtime.
- **Manifest:** [spin.toml](spin.toml) is the source of truth for routing, variables, build command, and outbound permissions.

## ⚙️ Configuration & Secrets

- Config comes from **Spin variables**, read with `spin_sdk::variables::get("name").await` — *not* `std::env::var`.
- Declared in `[variables]` and wired per component in `[component.s3sample.variables]`. Adding a config value requires editing both blocks.
- Current variables: `s3_access_key`, `s3_secret_key` (secret), `s3_region`, `s3_endpoint`, `s3_bucket`.
- Locally they are supplied via the `SPIN_VARIABLE_`-prefixed uppercase names in [.env](.env) (e.g. `SPIN_VARIABLE_S3_ACCESS_KEY`).
- Never hardcode credentials or bucket names in Rust source.
- ⚠️ `.env` holds live keys. It is gitignored — keep it that way and never commit it.
- `allowed_outbound_hosts` is templated as `https://{{ s3_bucket }}.{{ s3_endpoint }}`, so it follows the variables automatically. Point the component at a different bucket/cluster by changing the variables only.

## 🚀 Architectural Principles

- **No heavy SDKs:** `aws-sdk-s3` is strictly forbidden — its network, async, and crypto layers do not compile for Wasm targets.
- **Pure-Rust in-memory crypto:** sign manually with `sha2`, `hmac`, `hex`.
- **Zero-network signing:** signatures are computed entirely in-memory; no auxiliary service calls.
- **Proxy, don't transform:** `GET`/`PUT`/`DELETE` are forwarded to Linode and the upstream response is passed back as-is. Bodies are currently buffered in memory; streaming them is the goal, since large objects otherwise sit in constrained Wasm memory.

## 📝 AWS SigV4 Rules for Linode

1. **Alphabetical query params:** sort strictly before building the canonical query string.
2. **Region scope:** comes from `s3_region` and is used for both the credential scope and the `k_region` signing key — the two must always match. The actual cluster region (`nl-ams-1`) is verified to work; `us-east-1` is also generally accepted by Linode regardless of cluster.
3. **Host format:** `{bucket}.{cluster}.linodeobjects.com` (e.g. `hdebeij1.nl-ams-1.linodeobjects.com`).
4. **Canonical URI encoding:** percent-encode each path segment with `uri_encode_segment` (RFC 3986 unreserved set only, uppercase hex), preserving `/` separators. Do **not** use `form_urlencoded::byte_serialize` here — it encodes a space as `+`, which in a path means a literal plus and breaks the signature.
5. **Payload hash:** always `UNSIGNED-PAYLOAD`, especially for `PUT` — avoids hashing large bodies in Wasm.

## 💻 Common Commands

```sh
spin build                 # runs: cargo build --target wasm32-wasip2 --release
spin up                    # serve on http://127.0.0.1:3000
spin watch                 # rebuild + reload on src/**/*.rs, Cargo.toml changes
cargo clippy --target wasm32-wasip2
cargo test                 # native only; pure signing helpers are testable without a host
```

Manual smoke tests live in [test.sh](test.sh) (`curl` against `/storage/test.txt`). The bucket contains `text1.txt` as a known-good GET fixture.

## 🔑 Object Key Derivation

The object key is everything after the route prefix, taken from Spin's **`spin-path-info`** request header — never by hardcoding `/storage/` in Rust, so the route stays a manifest concern. The header arrives percent-encoded exactly as the client sent it, so the handler `percent_decode`s it to the true key, and the signer re-encodes it canonically. An empty key returns `400`.

## 📌 Current State / Known Gaps

- **`GET`, `PUT` and `DELETE` all work** and are verified end-to-end against the live bucket (`PUT` → `GET` → `DELETE` → `404 NoSuchKey`). Anything else returns `405` with an `Allow: GET, PUT, DELETE` header.
- Upstream status, body and `content-type` are passed through unchanged, including Linode's XML `<Error>` body on failure, so `SignatureDoesNotMatch` vs `NoSuchKey` is readable from `curl` alone.
- On `PUT` the client's `content-type` is forwarded so objects don't land as `binary/octet-stream`. It is deliberately *not* in `X-Amz-SignedHeaders` — only `host` is signed, so extra headers are ignored by the signature check.
- Request and response bodies are buffered in memory (`FullBody<Bytes>`), not streamed — fine for small objects, a problem for large ones. This is the main remaining limitation.
- Toolchain: Rust edition 2024, `rust-version = 1.93`.
