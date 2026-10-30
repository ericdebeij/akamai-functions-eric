# HTTP, logging, and validation reference

## Inbound contract

`POST /webhook`, `Content-Type: application/json`, and a matching
`X-Webhook-Secret` are required. JSON media-type parameters are accepted.
Each firing alert supplies its own comma-separated `labels.stream_ids`.

```json
{"alerts":[{"status":"firing","labels":{"stream_ids":"sampleuser-session01, sampleuser-session02"}}]}
```

IDs are trimmed, merged, and deduplicated in encounter order. Resolved alerts
are ignored, including their IDs. Empty/missing IDs on a firing alert fail
validation. More than 5,000 distinct IDs or a body over 1 MiB is rejected
before an API call.

| HTTP status | Meaning / error |
| --- | --- |
| 200 | `{"status":"revoked","requested":2,"count":2,"limit":5000}` (illustrative API count/limit) |
| 200 | `{"reason":"no_firing_alerts","status":"ignored"}` |
| 400 | `invalid_json`, `invalid_request_body`, or `missing_stream_ids` |
| 401 | `unauthorized` |
| 405 | `method_not_allowed` |
| 413 | `payload_too_large` or `too_many_stream_ids` |
| 415 | `content_type_must_be_application_json` |
| 500 | `configuration_error` |
| 502 | `revocation_api_rejected_request`, `revocation_api_unavailable`, or `invalid_revocation_api_response` |

Non-success responses include an `error` field. An upstream HTTP rejection
also includes the upstream status, URL, request JSON, response headers, and body.
If the body could not be read, it is null and a body-read indicator is included.
Transport failures expose null upstream status/headers and a transport error.
Restrict access to these responses; they contain diagnostic information.

## Outbound contract

```text
POST https://{host}/taas/v2/revocation-lists/{revocation_list_id}/identifiers/add
```

Optional query: `accountSwitchKey`. Headers include JSON content type, JSON
accept, and an EdgeGrid Authorization signature. Example body:

```json
[{"id":"sampleuser-session01","durationSeconds":1800}]
```

EdgeGrid signs the request method, URL, body hash, timestamp, and nonce.
The API client uses the configured list and returns success only when it can
parse the expected `count` and `limit` response fields. The HTTP response to
Grafana retains the existing compact success summary, not the whole API body.

## Single-line logging

Every handled request emits one compact JSON line to stderr:

- `timestamp`: UTC RFC 3339, milliseconds.
- `status_code`: webhook HTTP status, not the upstream API status.
- `streams_revoked`: submitted IDs after API success; empty for no-ops.
- `error`: webhook error code instead of `streams_revoked` on failure.
- `revocationResponse`: the received body as a parsed JSON value if Content-Type
  is `application/json` or `application/problem+json`.
- `revocationError`: the received body as one string for other/missing content
  types, or malformed JSON advertised as one of those JSON types.

Media types are matched case-insensitively with parameters allowed.
Exactly one revocation-body field is added when a body was received; both are
omitted on transport/body-read failures or when no API call is made.
All JSON values, including arrays and null, remain structured JSON.
Newlines/control characters are escaped, keeping each entry on one physical line.

```json
{"timestamp":"2026-10-30T14:02:58.402Z","status_code":200,"streams_revoked":["sampleuser-session01"],"revocationResponse":{"count":1,"limit":5000}}
{"timestamp":"2026-10-30T14:02:59.402Z","status_code":502,"error":"revocation_api_rejected_request","revocationResponse":{"detail":"Denied"}}
{"timestamp":"2026-10-30T14:03:00.402Z","status_code":502,"error":"revocation_api_rejected_request","revocationError":"Upstream unavailable\nTry later"}
```

No headers, credentials, or outbound request payloads are explicitly added to
logs. Upstream bodies themselves can contain sensitive data, so restrict log
access/retention and redact diagnostics and session IDs before sharing.

## Tests and operational checks

Unit tests are colocated with the implementation and use only dummy credentials,
reserved example hosts, and synthetic fixtures. `cargo test --locked` does not
call the live API. The suite covers extraction/deduplication, limits, secret
comparison, deterministic signing, response preservation, media-type handling,
and single-line logging.

For live acceptance, record the alert time, webhook response/log, API outcome,
and first denied media request. Test five sessions (no threshold alert), six
sessions (firing), sequential stream switches, incorrect webhook secrets,
API failures, and unaffected control sessions. A historical midpoint estimate,
notification delivery, API processing, and revocation propagation all add delay.
Do not infer playback blocking solely from an HTTP 200 from the function.

## Troubleshooting and limits

- **401:** compare the configured webhook secret and header without logging them.
- **415:** send JSON with the required media type.
- **500:** check required variables, bare hostname, list ID, and duration.
- **502 with upstream status:** inspect restricted diagnostics and confirm
  EdgeGrid permissions/signing and revocation-list configuration. For upstream
  429, examine any `Retry-After` header in the HTTP response diagnostics.
- **502 without upstream status:** check connectivity, configured host, outbound
  permission, and the returned transport error; do not assume rate limiting.
- **API success but playback continues:** verify the same full session ID and
  list are enforced, propagation has occurred, and the player is not only
  consuming buffered content or obtaining fresh session IDs.
- **Alert resolves but session remains blocked:** expected; resolution is a
  no-op, and the ID remains revoked until its duration expires after the last
  refreshing firing request.

The function has no persistent queue or automatic API retry. Grafana delivery,
repeat intervals, API limits, telemetry health, and recovery need explicit
operational design. No-data in an alert is not proof of account compliance.
