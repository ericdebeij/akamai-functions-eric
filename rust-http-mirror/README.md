# rust-http-mirror

A minimal Spin/Rust HTTP component that mirrors the incoming request back as JSON.
This example is useful for debugging request headers, bodies, and the URL information that Spin provides.

## What it does

The component accepts any HTTP request on the configured route and returns a JSON payload containing:

- `method` — original HTTP method
- `url` — the full request URL or raw request URI
- `headers` — all request headers, with repeated names preserved as arrays
- `body` — request body decoded as UTF-8 text when possible, otherwise base64
- `bodyEncoding` — `utf8` or `base64`

## Prerequisites

- [Spin CLI](https://spinframework.dev/install)
- Rust toolchain
- `wasm32-wasip1` target installed:

```sh
rustup target add wasm32-wasip1
```

## Build

From the `rust-http-mirror` folder:

```sh
spin build
```

This runs:

```sh
cargo build --target wasm32-wasip1 --release
```

## Run locally

Start the component locally with:

```sh
spin up
```

Then send test requests, for example:

```sh
curl http://127.0.0.1:3000/hello
curl -X POST http://127.0.0.1:3000/test -d 'hello world'
```

Each request will return a JSON document describing the original request.

## Deployment to Akamai Functions

This sample targets `akamai-functions` in `spin.toml`.
Deploy with the Spin Akamai plugin once configured:

```sh
spin aka deploy --build
```

If this is your first Akamai deploy, install the plugin and log in first:

```sh
spin plugins install aka
spin aka login
```

## Files

- `src/lib.rs` — the mirror handler implementation
- `spin.toml` — Spin manifest for route, component, and build configuration
- `Cargo.toml` — Rust package manifest

## Notes

- `allowed_outbound_hosts` is empty because this example does not make outbound network requests.
- The route is configured as `/...`, so the component receives all requests under the root path.
