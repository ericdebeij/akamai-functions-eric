# Configuration and integration

## Application variables

[spin.toml](../spin.toml) declares and maps the variables below. Set real values
through approved runtime/deployment configuration, not source files. Application
variables on Akamai Functions are documented as encrypted at rest and in transit.

| Variable | Required | Meaning |
| --- | --- | --- |
| `client_secret` | Yes | EdgeGrid signing secret |
| `client_token` | Yes | EdgeGrid client token |
| `access_token` | Yes | EdgeGrid access token |
| `host` | Yes | Bare API hostname, without scheme or path |
| `revocation_list_id` | Yes | List enforced by the media access configuration |
| `webhook_secret` | Yes | Shared secret compared with `X-Webhook-Secret` |
| `account_switch_key` | No | Optional account selection; empty means no query parameter |
| `revocation_duration_seconds` | No | Default `1800`; integer at least `1800` |

Use authorized, least-privilege API credentials. The manifest permits outbound
HTTPS only to the configured API host. The function uses Spin's outbound HTTP
interface rather than a native socket client.

## Grafana webhook

Configure a webhook contact point and notification policy to deliver firing
alerts to `https://<WEBHOOK_HOST>/webhook`.

- Method: POST; body: Grafana alert JSON.
- Content-Type: `application/json`.
- Custom header: `X-Webhook-Secret`, matching the function variable.
- Each firing alert must include `labels.stream_ids`: full token/session IDs
  separated by commas. `commonLabels` alone is not sufficient.
- Preserve `alerts[].status`; resolved notifications are ignored.
- Decide notification grouping, repeat intervals, delivery retries, and error
  routing explicitly. The function does not provide a durable delivery queue.

The example payloads are intentionally minimal. A real Grafana envelope may
contain additional fields; the parser ignores fields it does not use.

The concurrency query and threshold are external dependencies, not embedded
in this export. A suitable account-sharing query groups by full signed session
ID and account prefix, counts overlapping activity rather than every stream
seen during a window, and emits the selected IDs as `stream_ids`.

The source POC evaluates every minute over six minutes ago through two minutes
ago, counting sessions whose first/last request span covers the midpoint
(four minutes ago), with at least two qualifying requests per session.
Only configured streaming hosts and successful requests are considered.
The rule fires strictly above five active sessions. This is a delayed estimate,
not proof of continuous playback. Do not equate dashboard filters with alert
configuration; configure the rule independently.

## Akamai Property and media enforcement

The webhook-facing Property must route HTTPS POST `/webhook` to the function,
preserving the body, content type, and secret header. Match the function route
when configuring any prefix/path rewriting. Do not cache webhook responses.
Verify unauthorized requests are rejected and error statuses reach Grafana.

The media-facing Property/access-control configuration must validate the signed
long token and enforce the same revocation list that the API updates.
Ensure the full token `session_id` is the API identifier; a channel/content ID
or an arbitrary user ID is not a substitute.

For analytics, log the session ID through DataStream into TrafficPeak. A
`user-sessionSuffix` convention allows account grouping; the user prefix must
be opaque and contain no hyphen. The issuer, not the player, must choose and sign
these values. Concurrent players sharing one full ID count as only one session.

Property routing, token issuance, logging configuration, Grafana queries/rules,
and revocation-list creation are environment-specific and are not provisioned
by this sample.

## Deploy to Akamai Functions -- operator execution only

These commands change external systems. Review account, application, and
environment selection yourself; use non-production first.

1. Install the plugin and authenticate if needed:

   ```sh
   spin plugin install aka
   spin aka login
   ```

2. From the sample root, validate and build:

   ```sh
   cargo test --locked
   spin build
   ```

3. For an existing application, explicitly link and verify the target:

   ```sh
   spin aka app link --app-name <NONPROD_APP_NAME>
   spin aka info
   ```

   For a new application, review the name and account prompts during deployment.
   Do not copy deployment-link directories from another application.

4. Populate the following environment variables using an approved secret
   manager/process, then deploy:

   ```sh
   spin aka deploy \
     --variable "client_secret=${AKAMAI_CLIENT_SECRET:?required}" \
     --variable "client_token=${AKAMAI_CLIENT_TOKEN:?required}" \
     --variable "access_token=${AKAMAI_ACCESS_TOKEN:?required}" \
     --variable "host=${AKAMAI_API_HOST:?required}" \
     --variable "revocation_list_id=${REVOCATION_LIST_ID:?required}" \
     --variable "webhook_secret=${WEBHOOK_SECRET:?required}" \
     --variable 'revocation_duration_seconds=1800'
   ```

   Add `--variable "account_switch_key=${ACCOUNT_SWITCH_KEY:?required}"` only
   when account switching is needed. CLI values may be visible through process
   inspection; never paste real secrets into shared terminals or documents.

5. Record the deployed URL privately and test a resolved notification first:

   ```sh
   curl -i "${APP_URL:?required}/webhook" \
     -H 'Content-Type: application/json' \
     -H "X-Webhook-Secret: ${WEBHOOK_SECRET:?required}" \
     --data-binary @examples/webhook_resolved.json
   ```

6. Only after verifying the non-production list, credentials, and enforcement,
   send a firing notification using disposable session IDs. It changes the
   revocation list; validate actual media rejection and an unaffected session.
   Repeated notifications refresh revocation; resolved alerts do not clear it.

## Official documentation

- [Akamai Functions quickstart](https://techdocs.akamai.com/akamai-functions/docs/quickstart)
- [Application variables and encryption](https://techdocs.akamai.com/akamai-functions/docs/deploy-app-variables)
- [Spin aka command reference](https://techdocs.akamai.com/akamai-functions/docs/aka-command-reference)
- [Spin application variables](https://spinframework.dev/variables)
