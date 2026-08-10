# Key-value store usage guide

This folder contains a minimal Spin / Rust example for using the key-value store.

## What is available

The example handler in [src/lib.rs](src/lib.rs) uses the Spin SDK key-value API:

- `Store::open_default()` to open the default store
- `store.get("name")` to read a value
- `store.set("name", data)` to write or overwrite a value
- `store.get_keys().await` to list all keys

## Basic pattern

```rust
use spin_sdk::key_value::Store;

let store = Store::open_default().await?;
store.set("message", b"hello").await?;

let value = store.get("message").await?;
if let Some(bytes) = value {
    let text = String::from_utf8_lossy(&bytes);
    println!("value: {text}");
}
```

## Routes implemented in this example

The current handler responds to:

- `GET /kv` → list the keys in the default store
- `GET /kv/name` → read the stored value for `name`
- `POST /kv/name` → set the value for `name`
- `PUT /kv/name` → overwrite the value for `name`
- `DELETE /kv/name` → remove the key-value entry for `name`

## Notes

- The store label used here is the default store, opened through `Store::open_default()`.
- The `spin.toml` manifest is the place to confirm which stores are allowed for the component.
- For this project, the relevant Rust dependency is already enabled in [Cargo.toml](Cargo.toml).
