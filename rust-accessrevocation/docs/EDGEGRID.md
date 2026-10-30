# EdgeGrid signing in Rust on Akamai Functions

## Why signing is implemented here

No official Akamai Rust EdgeGrid package has been identified that supports
the Akamai Functions WebAssembly/WASI runtime used by this project
(`wasm32-wasip2`). Community Rust packages may exist, but that does not
establish official support or compatibility with this runtime.

The function therefore implements the required EdgeGrid signing directly in
[src/edgegrid.rs](./src/edgegrid.rs), using Rust SHA-256, HMAC, and Base64
libraries. HTTP transport is kept separate: [src/api.rs](./src/api.rs) sends
the signed request through Spin's outbound HTTP interface, rather than a
native socket-based client.

## Signing flow (pseudocode)

This describes this project's POST revocation request, not a general-purpose
EdgeGrid SDK. HMAC_SHA256 takes arguments in the order (key, message);
strings are encoded as UTF-8 bytes before hashing.

```text
url       = final HTTPS URL, including encoded path and optional query
body      = serialize the revocation request to its final JSON bytes
timestamp = current UTC time formatted as YYYYMMDDTHH:mm:ss+0000
nonce     = fresh random UUID

unsignedAuthorization =
    "EG1-HMAC-SHA256 client_token=" + clientToken + ";" +
    "access_token=" + accessToken + ";" +
    "timestamp=" + timestamp + ";" +
    "nonce=" + nonce + ";"

bodyHash = ""
if method is POST and body is not empty:
    bodyHash = Base64(SHA256(first min(length(body), 131072) bytes of body))

host = URL hostname, including explicit port if present
pathAndQuery = encoded URL path plus "?" + query when a query exists

dataToSign = joinWithTabs([
    uppercase(method),
    URL scheme,
    host,
    pathAndQuery,
    "",                 // No extra headers are signed by this implementation.
    bodyHash,
    unsignedAuthorization
])

signingKey = Base64(HMAC_SHA256(clientSecret, timestamp))
signature  = Base64(HMAC_SHA256(signingKey, dataToSign))

Authorization = unsignedAuthorization + "signature=" + signature
send the exact URL and body bytes used above, with this Authorization header
```

The Base64 text of `signingKey`, not the raw first HMAC digest, is used as the
second HMAC key. The client secret is used as supplied, not Base64-decoded.
Tabs, field order, and the trailing semicolon in the unsigned header matter.
The 131,072-byte body-hash limit matches the default used by this implementation;
it is not a truncation of the body sent to the API.

Credentials come from function variables, not source code. Never log the
credentials, derived signing key, or Authorization header. Each request gets
a new timestamp and nonce; do not modify the URL or reserialize the body
after signing.

The signer has a deterministic fixture test and the component has passed a
WASI build. These checks do not constitute official SDK certification.

## References

- [Akamai EdgeGrid authentication](https://techdocs.akamai.com/developer/docs/edgegrid)
- [Official Akamai EdgeGrid libraries](https://github.com/akamai?q=edgegrid&type=all&language=&sort=)
