# core-mail

The email edge for Meridian: trait boundaries plus adapters, no database.

## Purpose

- `MailTransport` (§4.7) — every outbound provider sits behind it:
  - **`smtp`** — minimal RFC 5321 client for local relays and Mailpit
    (`deploy/docker-compose.yml` stage2 profile). No TLS in v1: point it at
    private-network smarthosts only.
  - **`postmark`** — hosted API adapter, the production default.
  - **`recording`** / **flaky** — test doubles backing the CI mail harness.
- `ScanProvider` — malware scanning between inbound webhook and persistence:
  - **`clamd`** — ClamAV clamd TCP INSTREAM adapter.
  - **`none`** — the documented none mode: reports clean but records scan
    status `skipped`; deployments without a scanner stay fully functional.

## None modes (standalone-first doctrine)

| Capability | None mode | Behavior |
|---|---|---|
| Outbound transport | `EMAIL_TRANSPORT=none` | Mail stays queued in `email_outbound`, retried harmlessly and logged; nothing is silently dropped. |
| Malware scan | `EMAIL_SCAN=none` | Inbound mail persists with `scan_status='skipped'`. |
| Inbound webhook | unset `EMAIL_INBOUND_HMAC_SECRET` | The webhook endpoint is disabled (503) rather than open. |

## Non-purpose

No MIME parsing of raw messages (v1 consumes provider-extracted fields), no
database access, no send/retry policy — orchestration lives in core-api, the
ledger in core-db, the retry loop in worker.

## Manual verification against Mailpit

```sh
docker run -d --name mailpit -p 1025:1025 -p 8025:8025 axllent/mailpit
EMAIL_TRANSPORT=smtp SMTP_HOST=127.0.0.1 SMTP_PORT=1025 \
  cargo run -p worker   # enqueue a message via the API first
# open http://localhost:8025 — the rendered message appears with headers intact
```

The deterministic CI harness (`crates/core-mail/tests/two_weeks.rs`) covers
the zero-loss acceptance criterion; this manual pass verifies real SMTP wire
behavior once per release.
