
# Rust Access Revocation sample

A standalone Rust HTTP function for Akamai Functions. It accepts authenticated
Grafana alert webhooks and submits the firing alerts' session IDs to the
Akamai Access Revocation API using EdgeGrid authentication.

## What this demonstrates

```text
Players -> Protected media delivery -> DataStream -> TrafficPeak
                    ^                                    |
                    |                              Grafana alert
                    |                                    |
              Revocation list <- Access Revocation API <- Rust Function
                                                           ^
                                                    Webhook Property
                                                           ^
                                                    Grafana webhook
```

In an account-sharing POC, each signed playback token has a session ID such
as `sampleuser-session01`: the prefix before the first hyphen identifies an
opaque user reference, and the complete ID identifies one playback session.
TrafficPeak can estimate overlapping sessions from media-request logs.
An independently configured Grafana alert fires above x concurrent sessions
and supplies their full IDs.

**This function does not measure concurrency or enforce a threshold itself.**
It trusts the authenticated alert and revokes every supplied firing-alert ID.
The media delivery configuration must enforce the same revocation list.
Revocation blocks subsequent protected requests after propagation; it does not
withdraw buffered media or permanently ban an account or newly issued IDs.

## EdgeGrid authentication in Rust

This sample also demonstrates how to authenticate an Akamai API request with
EdgeGrid from within an Akamai Function written in Rust. No official Akamai
Rust EdgeGrid package has been identified for this WebAssembly/WASI runtime,
so signing is implemented directly and the signed request is sent using Spin's
outbound HTTP interface.

See [EDGEGRID.md](./docs/EDGEGRID.md) for the rationale and signing pseudocode.

## Prerequisites

- Rust and Cargo via rustup, including rustfmt and Clippy.
- The `wasm32-wasip2` Rust target.
- Spin CLI with support for this HTTP/WASI component.
- For deployment: Akamai Functions access and the Spin `aka` plugin.
- For a live revocation test: authorized EdgeGrid API credentials, an API host,
  a non-production revocation list, and media access enforcement using that list.

The export is validated with Rust/Cargo 1.97.0. No lower minimum compiler
version is claimed. Keep the included lockfile for reproducible dependencies.
The application name is `rust-accessrevocation`; the Rust crate, Spin component,
and output WASM filename intentionally retain the name `revokeaccess`.

## Build and validate

Run from this folder:

```sh
rustup target add wasm32-wasip2
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
spin build
```

`spin build` runs `cargo build --locked --target wasm32-wasip2 --release`.
The generated component is `target/wasm32-wasip2/release/revokeaccess.wasm`.
Cargo downloads dependencies when needed; no parent project files are required.

## Safe local smoke test

After building, start Spin with dummy configuration:

```sh
spin up --listen 127.0.0.1:3000 \
  --variable 'client_secret=local-dummy-secret' \
  --variable 'client_token=local-dummy-client' \
  --variable 'access_token=local-dummy-access' \
  --variable 'host=api.example.test' \
  --variable 'revocation_list_id=local-list' \
  --variable 'webhook_secret=local-only-test-secret'
```

In another terminal, from this folder:

```sh
curl -i http://127.0.0.1:3000/webhook \
  -H 'Content-Type: application/json' \
  -H 'X-Webhook-Secret: local-only-test-secret' \
  --data-binary @examples/webhook_resolved.json
```

Expect HTTP 200 and `{"reason":"no_firing_alerts","status":"ignored"}`.
An incorrect shared secret returns HTTP 401. Neither path calls the API.
Stop the server with Ctrl+C.

Do not use the firing fixture for this no-op test: a firing notification
attempts a real outbound revocation request. The fixture contains synthetic IDs
for unit tests, not actual credentials or valid production sessions.

## Akamai function test

After testing, deploy the function on Akamai Functions
```sh
spin aka login
spin aka deploy --variable @env.json
```

Once deployed the endpoint shown can be configured in the delivery configuration used to access the function 

The output of the function can be called with
```sh
spin aka logs
```

## Behavior

- Only `POST /webhook` with `Content-Type: application/json` and a matching
  `X-Webhook-Secret` is accepted. JSON content-type parameters are allowed.
- Each alert's own `status` and `labels.stream_ids` are used; resolved alerts
  are ignored. IDs are comma-separated, trimmed, merged, and deduplicated.
- Resolved-only notifications are successful no-ops, not revocation removals.
- At most 5,000 distinct IDs and a 1 MiB request body are accepted.
- IDs are revoked for 1,800 seconds by default; repeated firing notifications
  refresh that duration. Values below 1,800 seconds are rejected.
- API failures return non-success responses; there is no internal retry queue.
- Exactly one compact JSON request log is emitted to stderr, including the
  received API body when available. See the [reference](docs/REFERENCE.md).

## Files and documentation

```text
.
|-- .gitignore
|-- Cargo.toml
|-- Cargo.lock
|-- spin.toml
|-- README.md
|-- src/
|   |-- lib.rs          HTTP authentication, validation, responses and logs
|   |-- webhook.rs      Grafana alert extraction and fixture tests
|   |-- api.rs          Outbound HTTP and API response classification
|   `-- edgegrid.rs     API URL construction and EdgeGrid signing
|-- examples/
|   |-- webhook_fired.json
|   `-- webhook_resolved.json
`-- docs/
    |-- EDGEGRID.md
    |-- CONFIGURATION.md
    `-- REFERENCE.md
```

- [Configuration, Grafana/Property wiring, and deployment](docs/CONFIGURATION.md)
- [HTTP contract, logging, testing, and troubleshooting](docs/REFERENCE.md)

## Before publishing or deploying

1. Select and approve a license and attribution.
2. Review all files for private information. Never add real API credentials,
   tokens, account metadata, local `.edgerc` files, or deployment state.
3. Configure least-privilege API access and an independent webhook secret.
4. Validate in non-production with disposable session IDs and an unaffected
   control session. API success alone does not establish playback blocking.
5. Restrict log/diagnostic access and define retention: API responses and session
   IDs can be sensitive.

This is a sample, not a production availability or enforcement guarantee.
Deployment and any remote repository operations must be performed by an
authorized operator.
