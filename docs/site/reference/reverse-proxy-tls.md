# Runbook: reverse proxy and TLS

> **Audience:** site operator / MSP admin — you host Marula yourself and
> own its deployment.

This runbook is the front door. It covers what the proxy must serve, the
two TLS modes a real deployment runs, the two ways to run the edge one, the
Cloudflare steps that go with both, and the core-api settings that make the
resolved client IP mean something.

The local compose stack targets development deliberately ("no TLS, no
secrets management, no volume backup jobs", per its own header). The
production front door ships as an overlay, not a fork:
`deploy/edge/compose.edge.yml` adds a Caddy proxy and takes the raw service
ports back off the network. The Helm chart's `ingress` block is the same
thing for a cluster. Both are described here.

## What to terminate

| Browser-facing path | Upstream | Notes |
|---|---|---|
| `/` web console | `web:3000` | also `/_next/*` |
| `/api/…` | `core-api:8080` | REST + instant-first envelope, incl. `/api/health` |
| `/openapi.json` | `core-api:8080` | SDK contract |
| `/healthz` | `core-api:8080` | rewritten to `/api/health` — the app's own liveness route |
| `/ws` | `realtime:8081` | must support upgrade |

`NEXT_PUBLIC_API_URL` is baked into the web bundle at build time — after
adding TLS, rebuild with the public `https://` URL so the browser reaches
the API through the proxy, not the internal service name. The realtime
socket comes from `NEXT_PUBLIC_WS_URL`, baked the same way, and it must be
the proxied path (`wss://<domain>/ws`). Leave it empty and the client
derives the address from the API base and port 8081, which is the origin's
raw port — exactly what the proxy exists to remove from the internet.

## Two TLS modes, and how to pick

| Mode | `MARULA_TLS_MODE` | Certificate | Use when |
|---|---|---|---|
| Direct | `direct` | Let's Encrypt, ACME HTTP-01 (TLS-ALPN-01 as fallback) | DNS points at this host |
| Edge (Origin CA) | `cf-origin` | Cloudflare Origin CA, SSL/TLS mode Full (strict) | DNS at Cloudflare, orange cloud on |
| Edge (DNS-01) | `cf-dns01` | Let's Encrypt, ACME DNS-01 through an API token | Cloudflare in front and the origin needs a public CA |

For compose, `deploy/edge/README.md` walks each mode. For the Helm chart,
`deploy/helm/README.md` maps the same decision onto `ingress.tlsMode`. The
mode is a deployment decision, not a feature: pick it once and it does not
change again until your DNS does.

### Why Edge mode breaks the obvious path

With Cloudflare's proxy on, the edge answers for your hostname. An ACME
HTTP-01 challenge lands on Cloudflare, not on you, and TLS-ALPN is captured
the same way — so `MARULA_TLS_MODE=direct` cannot issue a certificate
through an orange-cloud record and the failure surfaces as an ACME timeout
rather than as something readable. That is the whole reason the second mode
exists.

### Edge mode, choice (i): Cloudflare Origin CA certificate

The simplest origin. Cloudflare signs a certificate as its own CA; it is
valid only between the edge and your origin, needs no ACME and no inbound
port 80, and pairs with Full (strict):

1. Cloudflare dashboard → SSL/TLS → Overview: set the mode to **Full
   (strict)**.
2. SSL/TLS → Origin Server → Create Certificate. Let Cloudflare generate
   the key and CSR, list the hostnames, RSA 2048 or ECDSA P-256.
3. Save the certificate as `deploy/edge/certs/origin.crt` and the key as
   `deploy/edge/certs/origin.key`.
4. `MARULA_TLS_MODE=cf-origin` and start the edge profile.

Full (strict) is what makes the Origin CA certificate mean anything: it is
the mode under which Cloudflare validates what your origin presents. **Full**
accepts any certificate, and **Flexible** has Cloudflare fetch your origin
over plaintext http, which is a downgrade any observer on that path can
replay. If you see Flexible anywhere in this runbook, it is as the thing not
to do.

### Edge mode, choice (ii): DNS-01 through a Cloudflare token

When the origin needs a publicly trusted certificate, the reason is
usually a second proxy in front, a monitoring agent that refuses a private
CA, or a compliance rule about chains. That is what the DNS-01 choice is
for: it uses the one challenge channel the edge cannot intercept, your DNS
zone. Caddy creates the `_acme-challenge` TXT record through the Cloudflare
API and Cloudflare's authoritative nameservers hold the record it just
wrote.

The token is created at My Profile → API Tokens, scoped to the zone
containing your domain, with **Zone → DNS → Edit** and nothing else. A
token with more scope is a token that can rewrite your zone. Pass it as
`CF_API_TOKEN` in the proxy's environment; no Caddyfile or image ever
contains it.

Choice (ii) is the only mode that needs a Caddy build carrying the
Cloudflare DNS module, which is why the edge image is built in-lane
(`deploy/edge/Dockerfile.caddy-dns`) instead of pulled. If you never use
it, drop the build and pin the stock image — the Dockerfile header names
it.

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

With the compose edge, all of that is wired for you:
`deploy/edge/compose.edge.yml` sets `TRUSTED_PROXIES` to Cloudflare's
ranges plus the private ranges the compose network lives on, and Caddy
appends to `X-Forwarded-For` and sets `X-Forwarded-Proto` itself. Behind
the Helm chart the ingress controller is the proxy: add its ranges to
`coreApi.trustedProxies` alongside Cloudflare's.

One Docker detail worth knowing before you debug a solved IP. Publishing a
host port through Docker's userland proxy means the origin sees the bridge
gateway as its peer, not the caller. That is why the overlay trusts the
private ranges: without that hop the walk would stop at the gateway and
every request would resolve to one address. In Direct mode you can get the
real client address as far as the proxy by setting `"userland-proxy": false`
in the Docker daemon, which lets the container see the connection's actual
source. Nothing in either mode is broken without it. The per-actor keys are
what lock an account. What goes quiet is the per-IP signal and the audit
records.

### What the resolved IP is and is not

The per-IP throttle keys use the resolved IP. They stay **noise**: the
per-actor keys (email, user id) are what lock an account, so IP rotation
cannot bypass a lock no matter what the header claims. Honest resolution
improves the quality of throttle and audit records. It is not a new
security boundary, and no per-route variant of the resolver exists.

## Checks a proxy must survive

- **WebSocket passthrough.** The live surfaces (boards, presence, realtime
  envelopes) upgrade from HTTP; a proxy that buffers or times out upgrades
  breaks them silently. The shipped configs set `flush_interval -1`, no
  stream timeout, and a 60s close delay on reload so a config change does
  not drop live sockets.
- **Request body size.** Document attachments are capped by
  `MARULA_MAX_UPLOAD_MB` (default 25), and the proxy has to carry the same
  number or uploads die at the wrong layer. Watch the unit: core-api
  computes `mb * 1024 * 1024`, so its 25 means 26,214,400 bytes, while
  Caddy's `MB` is decimal and its `MiB` is binary. The shipped configs say
  `25MiB` (and the ingress annotation `25Mi`) to match the API exactly;
  written as `MB`, the proxy caps 1.2 MB lower and refuses uploads the app
  would have accepted. Raise the ceiling in both places together.
- **Secrets.** Override `JWT_SECRET` and `MARULA_SECRET_KEY` — the
  compose defaults are dev-only, and core-api/worker must share the same
  `MARULA_SECRET_KEY` or sealed webhook secrets become undecryptable.
  Set `EMAIL_INBOUND_HMAC_SECRET` before enabling the signed inbound mail
  webhook. Run the [backup runbook](backup-restore.md) rehearsal before
  cutover, not after.

## Origin lockdown (Edge mode)

The orange cloud is a boundary only if the origin does not serve anyone
else. Leave 80 and 443 open to the world and an attacker who learns your
origin's IP reaches the real server directly, bypassing every rule, WAF
policy and rate limit you configured at the edge. The IP leaks through a
historical DNS record, a certificate transparency log, or a subdomain that
was never proxied.

`deploy/edge/origin-lockdown.sh` builds the allowlist from Cloudflare's
published ranges (80 and 443 from their edges, plus your SSH address) and
prints it before it writes anything. It never writes the final
`policy drop` line itself: which other ports this host serves is not
something the script can know. From a machine outside Cloudflare, confirm
the edge is the only way in afterwards — `https://<domain>/healthz` still
answers, a request to the origin IP times out.

In Direct mode there is nothing to allowlist: the host is its own edge, so
80 and 443 have to be open. The script says so and exits rather than
pretending to help.

Do not publish the raw service ports on a public host, in either mode.
The compose edge overlay pulls `8080`, `8081`, `3000`, `8082`, `15432` and
`16379` back to the compose network. The preview overlay's `0.0.0.0`
publishes are the exception, and only until the edge overlay is layered on
top of it.

## Secure cookies

Core-api derives the session cookies' `Secure` flag from
`PUBLIC_BASE_URL` starting with `https` (`crates/core-api/src/auth/cookies.rs`).
On a public deployment that variable has to be the browser-reachable
origin. Set it to the internal service name and every cookie on the site
drops its `Secure` flag, which is the kind of mistake that only shows up
when someone looks at a Set-Cookie header in a browser a year later.

So, on a public deployment, all of these are the same `https://<domain>`:
`PUBLIC_BASE_URL`, `WEB_BASE_URL`, `CORS_ORIGIN`, `WEBAUTHN_RP_ID` and
`WEBAUTHN_ORIGIN`, plus the two baked-in client URLs. The compose edge
overlay sets all of them from `MARULA_DOMAIN` for exactly that reason.

## Verifying a bring-up

```sh
DOMAIN=marula.example.com

# the proxy is serving the certificate you asked for
openssl s_client -connect "$DOMAIN:443" -servername "$DOMAIN" </dev/null \
  | openssl x509 -noout -subject -issuer -dates

# the three seams, through the proxy. core-api's liveness route is
# {base}/health, so /api/health answers and a bare /healthz does not: the
# proxy rewrites the public /healthz onto it.
curl -fsS "https://$DOMAIN/healthz"
curl -fsS "https://$DOMAIN/api/health"
curl -fsS "https://$DOMAIN/openapi.json" | head -c 40
curl -isS "https://$DOMAIN/ws" | head -1     # 400 from the hub, not 404
                                             # from the web console

# the upload ceiling bites at the proxy (413), not the API
# 26 MiB > 25MiB, so this is refused at the proxy. Send it authenticated:
# the ceiling trips when the body is read, and an unauthenticated request is
# answered by the auth gate without ever reading it.
head -c $((26 * 1024 * 1024)) /dev/zero > /tmp/oversize.bin
curl -isS -X POST --data-binary @/tmp/oversize.bin \
  -H "authorization: Bearer $TOKEN" \
  "https://$DOMAIN/api/v1/tickets/$TICKET/attachments" | head -1

# the proxy forwards the client identity core-api resolves
curl -fsS -H 'X-Forwarded-For: 198.51.100.7' "https://$DOMAIN/healthz" >/dev/null
```

The last two are the ones that separate a proxy that looks correct from one
that is. The body ceiling is the normal first failure after a bring-up —
the app works, then uploads die, and the error names the wrong layer. The
forwarded-identity check is quieter: it works when the walk resolves, and it
fails plainly when it does not, which is the `TRUSTED_PROXIES` warning at
boot.

## What not to terminate here

The internal service ports (Postgres 15432, Redis 16379, the worker health
port, the observability profile's collectors) stay on the private network.
Grafana in the observability profile grants anonymous admin on the demo
baseline — re-authenticate it before exposing port 3001 anywhere.

`/internal/*` is not a public route in any mode. It is the worker-to-core
effect seam, guarded by its own token
(`crates/core-api/src/routes/mod.rs` mounts it outside the `/api`
namespace on purpose). The console answering 404 for it is the correct
behaviour; adding a route there is the one change that turns an internal
boundary into a public endpoint.
