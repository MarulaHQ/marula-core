# Runbook: reverse proxy and TLS

> **Audience:** site operator / MSP admin — you host Meridian yourself and
> own its deployment.

The compose stack targets local development deliberately ("no TLS, no
secrets management, no volume backup jobs", per its own header). Production
deployment puts a reverse proxy in front of the web console and core-api.
This runbook covers what the proxy must serve and the pitfalls specific to
Meridian's seams.

## What to terminate

| Browser-facing path | Upstream | Notes |
|---|---|---|
| `/` web console | `web:3000` | |
| `/api/…` | `core-api:8080` | REST + instant-first envelope |
| the realtime WebSocket path | `realtime:8081` | must support upgrade |

`NEXT_PUBLIC_API_URL` is baked into the web bundle at build time — after
adding TLS, rebuild with the public `https://` URL so the browser reaches
the API through the proxy, not the internal service name.

TLS termination is your proxy's job (nginx, Caddy, Traefik all work); the
compose file ships none. Example stubs are intentionally not published
here because there is no blessed single-file recipe in the repo — any
standard TLS-terminating proxy config that forwards the paths above is
supported.

## Checks a proxy must survive

- **WebSocket passthrough.** The live surfaces (boards, presence, realtime
  envelopes) upgrade from HTTP; a proxy that buffers or times out upgrades
  breaks them silently.
- **Request body size.** Document attachments are capped by
  `MERIDIAN_MAX_UPLOAD_MB` (default 25); align the proxy's client-body limit
  or uploads die at the proxy, not the API.
- **Secrets.** Override `JWT_SECRET` and `MERIDIAN_SECRET_KEY` — the
  compose defaults are dev-only, and core-api/worker must share the same
  `MERIDIAN_SECRET_KEY` or sealed webhook secrets become undecryptable.
  Set `EMAIL_INBOUND_HMAC_SECRET` before enabling the signed inbound mail
  webhook. Run the [backup runbook](backup-restore.md) rehearsal before
  cutover, not after.

## What not to terminate here

The internal service ports (Postgres 15432, Redis 16379, the worker health
port, the observability profile's collectors) stay on the private network.
Grafana in the observability profile grants anonymous admin on the demo
baseline — re-authenticate it before exposing port 3001 anywhere.
