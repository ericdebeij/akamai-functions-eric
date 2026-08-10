# s3sample

A [Spin](https://spinframework.dev) HTTP component (Rust → WebAssembly) that proxies requests to
Linode Object Storage. It signs each request with AWS Signature V4 in-memory — no AWS SDK, no
network round-trip to obtain credentials — and forwards it to the bucket.

Everything after the `/storage/` route prefix is the object key, so `/storage/sub/dir/a.txt`
maps to the key `sub/dir/a.txt`.

| Method   | Behaviour                                             |
| -------- | ----------------------------------------------------- |
| `GET`    | Returns the object, with its upstream `content-type`. |
| `PUT`    | Uploads the request body as the object.               |
| `DELETE` | Deletes the object (`204` on success).                |
| other    | `405` with an `Allow: GET, PUT, DELETE` header.       |

Upstream failures are passed through unchanged — status plus Linode's XML `<Error>` body — so a
`SignatureDoesNotMatch` is distinguishable from a `NoSuchKey` with nothing but `curl`.

## Prerequisites

- The [Spin CLI](https://spinframework.dev/install)
- Rust with the `wasm32-wasip2` target: `rustup target add wasm32-wasip2`

## Configuration

Config comes from Spin variables, declared in [spin.toml](spin.toml). Locally, supply them as
`SPIN_VARIABLE_`-prefixed environment variables in a `.env` file:

```sh
SPIN_VARIABLE_S3_ACCESS_KEY="..."
SPIN_VARIABLE_S3_SECRET_KEY="..."
SPIN_VARIABLE_S3_BUCKET="your-bucket"
SPIN_VARIABLE_S3_ENDPOINT="nl-ams-1.linodeobjects.com"
SPIN_VARIABLE_S3_REGION="nl-ams-1"
```

`.env` holds live credentials and is gitignored — don't commit it.

`allowed_outbound_hosts` in the manifest is templated as `https://{{ s3_bucket }}.{{ s3_endpoint }}`,
so pointing the component at another bucket or cluster only means changing these variables.

## Build and run

```sh
spin build                        # cargo build --target wasm32-wasip2 --release
set -a; . ./.env; set +a          # load the variables
spin up                           # http://127.0.0.1:3000
```

Then:

```sh
curl -X PUT http://127.0.0.1:3000/storage/test.txt \
  -H 'content-type: text/plain' --data-binary "Hello from Spin Wasm!"
curl http://127.0.0.1:3000/storage/test.txt
curl -X DELETE http://127.0.0.1:3000/storage/test.txt
```

[test.sh](test.sh) runs this sequence end to end against a real bucket, starting and stopping
its own `spin up`.

## Deploy to Akamai Functions

Needs the `aka` plugin (`spin plugins install aka`) and a one-off `spin aka login`.

The deployed app gets its variables from a `vars.json`. Note the names are the lowercase
ones from [spin.toml](spin.toml) — not the `SPIN_VARIABLE_`-prefixed form `.env` uses:

```json
{
  "s3_access_key": "YOUR-ACCESS-KEY",
  "s3_secret_key": "YOUR-SECRET-KEY",
  "s3_region": "nl-ams-1",
  "s3_endpoint": "nl-ams-1.linodeobjects.com",
  "s3_bucket": "YOUR-BUCKET-NAME"
}
```

Then deploy:

```sh
spin aka deploy --build --variable @vars.json
```

Add `--create-name s3sample` on the first deploy, or `--app-id <id>` to target an
existing app. `vars.json` holds your secret key, so it is gitignored — keep it that way.

Passing the values individually works too, but they end up visible in `ps` output:

```sh
spin aka deploy --build --variable s3_bucket=my-bucket --variable s3_access_key=...
```

## Notes

- Request and response bodies are buffered in memory, so this is suited to small objects.
  Streaming is not implemented yet.
- See [CLAUDE.md](CLAUDE.md) for the architecture constraints and the SigV4 details that matter
  for Linode specifically.
