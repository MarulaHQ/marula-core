# The edge — TLS termination for a public deployment

The base stack (`deploy/docker-compose.yml`) publishes raw ports and speaks
plaintext http. That is the right posture for a laptop and the wrong one
for a host on the internet. This directory adds the front door: one Caddy
proxy that terminates TLS, routes the three Marula seams to their services,
and takes the raw ports back off the network.

Everything here is opt-in. `docker compose up` without `--profile edge`
behaves exactly as it did before.

## Pick your mode first

| Mode | `MARULA_TLS_MODE` | When | Certificate |
|---|---|---|---|
| Direct | `direct` | Your DNS points at this host, nothing in front | Let's Encrypt, ACME HTTP-01 (TLS-ALPN-01 as fallback) |
| Edge (Origin CA) | `cf-origin` | DNS at Cloudflare, orange cloud on | Cloudflare Origin CA, SSL/TLS mode Full (strict) |
| Edge (DNS-01) | `cf-dns01` | DNS at Cloudflare and the origin needs a public CA | Let's Encrypt, ACME DNS-01 through an API token |

The mode selects the Caddyfile that gets mounted, so all three run the
same service definition in `compose.edge.yml`. Two of the three need a
Caddy binary with the Cloudflare DNS provider compiled in, so the edge
image is built in-lane from `Dockerfile.caddy-dns` rather than pulled.
Direct and Origin-CA mode use none of that module; if you never use DNS-01,
drop the `build:` block from the `edge` service and set the image to the
pinned stock one (`caddy:2.9.1-alpine` with its digest, which is written in
the Dockerfile header) to skip the build entirely.

A mode needs an email address, a domain, and, for DNS-01, a scoped token.
None of those can be defaulted. Compose refuses to start without
`MARULA_DOMAIN`, and Caddy refuses to start without the certificate it can
obtain.

## Direct mode

DNS for `MARULA_DOMAIN` resolves to this host's address. Nothing sits in
front.

```sh
export MARULA_DOMAIN=marula.example.com
export ACME_EMAIL=ops@example.com
export MARULA_MAX_UPLOAD_MB=25

docker compose -f deploy/docker-compose.yml \
               -f deploy/edge/compose.edge.yml \
               --profile edge up --build -d
```

Ports 80 and 443 have to reach the container: 80 for the HTTP-01 challenge
and the https redirect, 443 for everything else. If port 80 is unavailable,
Caddy falls back to TLS-ALPN-01, which only needs 443.

Point `ACME_CA` at Let's Encrypt's staging endpoint while you are proving
the wiring out. A hostname that cannot be validated fails enough times to
hit the production rate limit, and staging is where you find out for free.

## Edge mode — Origin CA certificate

Cloudflare's orange cloud means the edge answers for your hostname, so
ACME can never reach you. The simplest origin is a Cloudflare Origin CA
certificate paired with SSL/TLS mode **Full (strict)**.

1. Cloudflare dashboard → SSL/TLS → Overview → set the mode to **Full
   (strict)**. Not Full, and never Flexible: Flexible has Cloudflare fetch
   your origin over plaintext http, which is a downgrade any observer on
   that path can replay.
2. SSL/TLS → Origin Server → **Create Certificate**. Let Cloudflare
   generate the key and CSR, list the hostnames, RSA 2048 or ECDSA P-256.
3. Save the certificate as `certs/origin.crt` and the key as
   `certs/origin.key` under this directory.
4. Start the edge with `MARULA_TLS_MODE=cf-origin`.

Full (strict) is what makes the Origin CA certificate matter: Cloudflare
validates the certificate your origin presents instead of accepting
anything that answers. With the mode left at Full, the certificate proves
nothing.

## Edge mode — DNS-01 through a Cloudflare token

When the origin needs a publicly trusted certificate, the reason is
usually a second proxy in front, a monitoring agent that refuses a private
CA, or a compliance rule about chains. That is what this mode is for: it
uses the one challenge channel the edge cannot intercept, your DNS zone.

Create the token at Cloudflare → My Profile → API Tokens, scoped to the
zone containing `MARULA_DOMAIN`, with **Zone → DNS → Edit** and nothing
else. A token with more scope is a token that can rewrite your zone.

```sh
export MARULA_TLS_MODE=cf-dns01
export CF_API_TOKEN=...   # Zone:DNS:Edit for the one zone
```

Caddy creates the `_acme-challenge` TXT record, waits for Cloudflare's
authoritative nameservers to hold it, and answers the challenge. Cloudflare
passes the query through because it is a TXT record it just wrote.

This is the only mode that needs the extra module, which is why the edge
image is built rather than pulled. Nothing about a public CA is required at
the origin otherwise.

## What the proxy routes

| Path | Upstream | Notes |
|---|---|---|
| `/` and everything else | `web:3000` | console, `/_next/*`, 404s |
| `/api`, `/api/*` | `core-api:8080` | canonical `/api/v1` and legacy `/api` |
| `/openapi.json` | `core-api:8080` | SDK contract |
| `/healthz` | `core-api:8080` | liveness, rewritten to `/api/health` (core-api's real route) |
| `/ws` | `realtime:8081` | WebSocket upgrade |

`/internal/*` is deliberately not routed. That is the worker-to-core effect
seam, guarded by its own token; the console answering 404 for it is the
correct behaviour. Adding a route there is the one change that turns an
internal boundary into a public endpoint.

## What the proxy refuses

- Bodies over `MARULA_MAX_UPLOAD_MB` (default 25), the same number core-api
  enforces per route. The unit matters: the proxy says `25MiB`, which is
  core-api's `mb * 1024 * 1024`, while Caddy's `MB` is decimal and would cap
  1.2 MB lower. Raise both together.
- The ceiling trips while the body is read, so an unauthenticated oversized
  request is answered by the app's auth gate first (401, not 413).
- Requests from anyone but you. In Edge mode, run
  `./origin-lockdown.sh --plan` and read the output before `--apply`.

## Origin lockdown

The orange cloud is a boundary only if the origin does not serve anyone
else. `origin-lockdown.sh` builds the allowlist from Cloudflare's published
ranges — 80 and 443 from their edges, plus the SSH address you are
connecting from, and everything else dropped.

The script never locks you out by itself. `--plan` prints the rules,
`--apply` writes them, `--revert` takes them back. It protects port 22 from
the SSH connection it can see, or from `--ssh-from` when it cannot. It also
never writes the final `policy drop` line: which ports this host serves for
other reasons is not something the script can know.

After applying, confirm the edge is the only way in: from a machine outside
Cloudflare, `curl https://$MARULA_DOMAIN/healthz` should still work, and a
direct request to the origin's IP should time out.

## Verifying a bring-up

```sh
# the certificate chain, end to end
openssl s_client -connect "$MARULA_DOMAIN:443" -servername "$MARULA_DOMAIN" </dev/null \
  | openssl x509 -noout -subject -issuer -dates

# the three seams, through the proxy
curl -fsS "https://$MARULA_DOMAIN/healthz"          # core-api liveness
curl -fsS "https://$MARULA_DOMAIN/openapi.json" | head -c 40
curl -fsS "https://$MARULA_DOMAIN/api/health"

# the WebSocket path reaches the hub, not the console: a plain GET with no
# upgrade header is 400 from axum's upgrade extractor, and a 404 would mean
# the request went to web instead
curl -isS "https://$MARULA_DOMAIN/ws" | head -1

# the proxy forwards the client identity core-api resolves
curl -fsS -H 'X-Forwarded-For: 198.51.100.7' "https://$MARULA_DOMAIN/healthz" >/dev/null

# a body over the ceiling is refused here, not read and rejected by the API.
# Send it authenticated: the cap trips when the body is read, and without a
# token the auth gate answers first (401, not 413).
head -c $((26 * 1024 * 1024)) /dev/zero > /tmp/oversize.bin
curl -isS -X POST --data-binary @/tmp/oversize.bin -H "authorization: Bearer $TOKEN" \
  "https://$MARULA_DOMAIN/api/v1/tickets/$TICKET/attachments" | head -1   # expect 413

# health of the proxy itself
docker compose -f deploy/docker-compose.yml -f deploy/edge/compose.edge.yml \
  --profile edge ps edge
```

## Files

| File | What it is |
|---|---|
| `compose.edge.yml` | The overlay: the proxy plus the port lockdown |
| `Dockerfile.caddy-dns` | Pinned Caddy build carrying the Cloudflare DNS module |
| `Caddyfile.direct` | ACME HTTP-01 / TLS-ALPN-01 |
| `Caddyfile.cf-origin` | Cloudflare Origin CA certificate |
| `Caddyfile.cf-dns01` | ACME DNS-01 through the Cloudflare API |
| `marula-routes.caddy` | The routing table, imported by all three modes |
| `origin-lockdown.sh` | Host firewall allowlist for Cloudflare's ranges |
| `certs/` | Where `origin.crt` / `origin.key` land (gitignored) |

The operator runbook, with the core-api settings each mode implies, is
[`docs/site/reference/reverse-proxy-tls.md`](../../docs/site/reference/reverse-proxy-tls.md).
