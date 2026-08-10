# Akamai Functions Samples

This repository contains example Akamai Functions projects.
- It is an add-on to the [Akamai Functions samples repository](https://github.com/akamai-developers/akamai-functions-samples).

- Akamai functions documentation can be found the [Akamai Techdocs](https://techdocs.akamai.com/akamai-functions/docs/welcome)

Use at your own risk (review before use it).

## Samples in Rust

- [rust-http-mirror](rust-http-mirror/README.md)
  - A simple HTTP mirror component that echoes the incoming request as JSON.
  - Useful for debugging request headers, body encoding, and how Spin exposes the full URL.

- [rust-kvsample](rust-kvsample/README.md)
  - A minimal Spin/Rust example demonstrating the key-value store API.
  - Shows how to open the default store, read/write values, list keys, and delete entries.

- [rust-s3sample](rust-s3sample/README.md)
  - A Spin HTTP component that proxies requests to S3-compatible object storage.
  - Lightweight AWS Signature V4 implementation (no AWS SDK)
  - Implements `GET`, `PUT`, and `DELETE` for object keys.

## Getting started

Each sample contains its own `README.md` with build, run, and deployment instructions.

1. Choose a sample folder.
2. Open that folder and read its `README.md`.
3. Follow the sample-specific instructions for prerequisites and configuration.

## Notes

- These examples target the [Spin](https://spinframework.dev) framework for WebAssembly-based components.

## License

This repository is licensed under the terms in `LICENSE`.
