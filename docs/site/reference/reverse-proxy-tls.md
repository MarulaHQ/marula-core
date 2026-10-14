# Runbook: reverse proxy and TLS

> **Audience:** site operator / MSP admin — you host Marula yourself and
> own its deployment.

The compose stack targets local development deliberately ("no TLS, no
secrets management, no volume backup jobs", per its own header). Production
deployment puts a reverse proxy in front of the web console and core-api.
This runbook covers what the proxy must serve and the pitfalls specific to
Marula's seams.

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
  `MARULA_MAX_UPLOAD_MB` (default 25); align the proxy's client-body limit
  or uploads die at the proxy, not the API.
- **Secrets.** Override `JWT_SECRET` and `MARULA_SECRET_KEY` — the
  compose defaults are dev-only, and core-api/worker must share the same
  `MARULA_SECRET_KEY` or sealed webhook secrets become undecryptable.
  Set `EMAIL_INBOUND_HMAC_SECRET` before enabling the signed inbound mail
  webhook. Run the [backup runbook](backup-restore.md) rehearsal before
  cutover, not after.

## Telling the API which proxy to believe

Once you terminate TLS at a proxy, core-api no longer sees the caller's
address — it sees the proxy. Real-client-IP resolution therefore starts
from the **direct peer** (the one address the socket proves) and only then
trusts a proxy header, and only for proxies you declared:

| Variable | Values | Meaning |
|---|---|---|
| `TRUSTED_PROXIES` | comma-separated CIDRs | Peers allowed to speak for someone else. **Unset ⇒ loopback + RFC1918 + IPv6 ULA.** An explicit list REPLACES the default. |
| `CF_CONNECTING_IP` | `auto` (default) \| `true` \| `false` | Whether `CF-Connecting-IP` may be believed. `auto` believes it only from a peer that is both trusted and inside a known Cloudflare range. |
| `CF_IP_RANGES` | comma-separated CIDRs | EXTENDS the built-in Cloudflare range constant (Cloudflare publishes new ranges between releases). |

How the client IP is picked:

1. The peer is **not** in the trusted set ⇒ the peer address wins and every
   proxy header is ignored. A caller reaching the origin directly chose its
   own headers.
2. The peer **is** trusted ⇒ `CF-Connecting-IP` when believed, else walk
   `X-Forwarded-For` right-to-left, skipping trusted hops. The first
   untrusted hop from the right is the client. When every hop is trusted,
   the peer itself is the client.
3. Malformed entries in any of the three variables are logged and dropped.
   They never block boot.

An internet-reachable bind (`0.0.0.0`, `::`) with `TRUSTED_PROXIES` unset
logs a warning at boot naming this section — the default set does not
include Cloudflare, so a CDN-fronted deployment would otherwise collapse
every real client onto one edge IP.

### Cloudflare-fronted deployment

Cloudflare appends the client IP to `X-Forwarded-For`, so the header's
first hop is attacker-controllable whenever CF is not your direct peer.
Name Cloudflare's edges and let the API prefer its canonical header:

```sh
TRUSTED_PROXIES=173.245.48.0/20,103.21.244.0/22,103.22.200.0/22,103.31.4.0/22,141.101.64.0/18,108.162.192.0/18,190.93.240.0/20,188.114.96.0/20,197.234.240.0/22,198.41.128.0/17,162.158.0.0/15,104.16.0.0/13,104.24.0.0/14,172.64.0.0/13,131.0.72.0/22,2400:cb00::/32,2606:4700::/32,2803:f800::/32,2405:b500::/32,2405:8100::/32,2a06:98c0::/29,2c0f:f248::/32
CF_CONNECTING_IP=auto
```

The list above is the versioned constant in
`crates/core-api/src/client_ip.rs` (source: <https://www.cloudflare.com/ips/>).
Refresh it with `CF_IP_RANGES` if Cloudflare publishes new ranges before
the next Marula release. If your own load balancer sits in front of
Cloudflare, put BOTH ranges in `TRUSTED_PROXIES` — the walk skips every
trusted hop.

For an nginx/Caddy/Traefik edge on your own infrastructure, list the
proxy's own address (or its private subnet) in `TRUSTED_PROXIES` and make
sure it sets `X-Forwarded-For` by appending, not replacing.

### What the resolved IP is and is not

The per-IP throttle keys use the resolved IP. They stay **noise**: the
per-actor keys (email, user id) are what lock an account, so IP rotation
cannot bypass a lock no matter what the header claims. Honest resolution
improves the quality of throttle and audit records. It is not a new
security boundary, and no per-route variant of the resolver exists.


## What not to terminate here

The internal service ports (Postgres 15432, Redis 16379, the worker health
port, the observability profile's collectors) stay on the private network.
Grafana in the observability profile grants anonymous admin on the demo
baseline — re-authenticate it before exposing port 3001 anywhere.
