# Preview environments (per PR)

A preview environment is a running copy of the branch, seeded with demo
data, that a reviewer can open in a browser. It is the cheapest possible
answer to "did my change actually work?" — the alternative is a reviewer
reading a diff and guessing.

This directory holds the parts that belong in the repository: the compose
overlay that sizes and exposes a preview stack, and the CI definition that
boots one. It does **not** hold the part that needs your account — the
host, the domain, and the decision about who may see an unreleased
branch. Those are yours to provide, and the definition is written to fail
loudly until you do rather than to improvise them.

## What is here

| File | What it is |
| --- | --- |
| `compose.preview.yml` | Compose overlay: sizing, published interfaces, secrets posture |
| `../edge/compose.edge.yml` | The optional TLS front door, layered last for a public preview |
| `../.github/workflows/preview.yml.disabled` | The CI definition (user-gated, like every other disabled workflow) |

## Why an overlay and not a second compose file

`deploy/docker-compose.yml` stays the only description of how Marula
runs. The overlay changes resource ceilings, published interfaces, and
the mandatory secret set, and inherits everything else. A preview
environment that silently lost the digest pins, the dropped capabilities,
or the read-only root filesystems would be a worse environment than a
laptop, and the most likely way for that to happen is a forked compose
file nobody remembers to update.

## The two ways to publish one

Both need a host you control. Which one to pick is a question about your
network, not about Marula.

### 1. SSH tunnel (no infrastructure)

The overlay binds the web and API ports on `0.0.0.0`, so a tunnel on
your laptop reaches them:

```sh
ssh -N -L 3000:localhost:3000 -L 8080:localhost:8080 preview-host
```

Then the preview is at `http://localhost:3000`. Nothing is published to
the internet, no DNS, no certificates, no wildcard domain, and the
reviewer is whoever holds the tunnel.

What it costs: one reviewer at a time, unless each of them opens their
own tunnel. That is fine for a design review and awkward for "here, look
at this" in a channel with six people.

### 2. Wildcard DNS + TLS (real per-PR URLs)

A domain like `*.preview.example.com` pointing at the preview host, with
the TLS-terminating proxy from `deploy/edge/` in front, gives every PR a
stable URL: `https://pr-482.preview.example.com`. This is the option the
CI definition is written to wire, and the reason
`PREVIEW_PUBLIC_URL` / `PREVIEW_API_URL` / `PREVIEW_CORS_ORIGIN` exist as
variables rather than defaults.

The edge overlay is layered **last**, after this directory's overlay:

```sh
export JWT_SECRET=$(openssl rand -hex 32)
export MARULA_SECRET_KEY=$(openssl rand -hex 32)
export MARULA_INTERNAL_TOKEN=$(openssl rand -hex 32)
export EMAIL_INTERNAL_TOKEN=$(openssl rand -hex 32)
export MARULA_DOMAIN=pr-482.preview.example.com   # per-PR host
export MARULA_TLS_MODE=direct                      # or cf-origin / cf-dns01
export ACME_EMAIL=ops@example.com                  # direct and cf-dns01 only
export PREVIEW_PUBLIC_URL=https://pr-482.preview.example.com
export PREVIEW_API_URL=https://pr-482.preview.example.com
export PREVIEW_CORS_ORIGIN=https://pr-482.preview.example.com

docker compose -f deploy/docker-compose.yml \
               -f deploy/preview/compose.preview.yml \
               -f deploy/edge/compose.edge.yml \
               --profile edge up --build -d
```

Three things about that command that are easy to get wrong:

- **Order.** The edge overlay clears the raw ports with
  `ports: !override []`, and compose applies that replacement only if the
  edge file comes last. Layered first, the proxy is a decoration and
  8080/3000 stay published.
- **One URL for both.** When TLS is on, the browser reaches the API
  through the same origin as the console. That is what makes
  `PREVIEW_API_URL` equal `PREVIEW_PUBLIC_URL` here.
- **The build arg is baked.** `NEXT_PUBLIC_API_URL` is compiled into the
  client bundle, so changing it after `up --build` changes nothing until
  you rebuild.

Which TLS mode fits a preview host depends on how the wildcard domain is
managed, exactly as it does for a real instance — the full decision and the
Cloudflare dashboard steps are in `deploy/edge/README.md`. A wildcard
`*.preview.example.com` works with all three: Direct mode minting a
per-hostname ACME certificate, an Origin CA certificate issued for the
wildcard, or DNS-01 minting per-hostname certificates through your zone.

What it costs: the wildcard DNS record, a certificate that renews itself,
and a decision about access. A wildcard certificate on a per-PR subdomain
means every branch a developer opens is potentially reachable by anyone
who guesses the URL — so put authentication in front of it, or accept that
preview data is public. Preview data is demo data, so that trade is often
fine; it stops being fine the moment someone connects a real customer's
data, which is the rule to write on the wall.

## Booting one by hand

Useful before wiring CI — you want to know the overlay works before you
trust it to a workflow.

```sh
export JWT_SECRET=$(openssl rand -hex 32)
export MARULA_SECRET_KEY=$(openssl rand -hex 32)
export MARULA_INTERNAL_TOKEN=$(openssl rand -hex 32)
export EMAIL_INTERNAL_TOKEN=$(openssl rand -hex 32)
export PREVIEW_PUBLIC_URL=http://localhost:3000
export PREVIEW_API_URL=http://localhost:8080
export PREVIEW_CORS_ORIGIN=http://localhost:3000

docker compose -f deploy/docker-compose.yml \
               -f deploy/preview/compose.preview.yml \
               up --build -d

# Demo data, so you open a populated app rather than a sign-up form.
docker compose -f deploy/docker-compose.yml \
               -f deploy/preview/compose.preview.yml \
               run --rm -e MARULA_DEV_TOOLS=1 migrate seed-demo --force
```

The three URLs have to agree with each other and with what the browser
uses. `NEXT_PUBLIC_API_URL` is baked into the client bundle at build time,
so changing it after `up --build` changes nothing until you rebuild — the
most common reason a preview "points at the wrong place".

## What the overlay deliberately changes

**Published interfaces.** Exactly two: web and API. Postgres,
Valkey and the worker's health port are pulled *back* to unpublished.
Compose merges a `ports:` list from an overlay rather than replacing it,
so the overlay uses `!override` to clear them — omitting the key would
have quietly left the local stack's database port open to the network.
The database is still reachable by service name inside the compose
network, which is where every client of it lives.

**Secrets.** `JWT_SECRET`, `MARULA_SECRET_KEY`, `MARULA_INTERNAL_TOKEN` and
`EMAIL_INTERNAL_TOKEN` are required by the base file, with no default
anywhere. On a network-visible interface a development key is not a
convenience, it is a published token that anyone who has read this
repository can use to mint a session. Compose refuses to start without
them, which is the behaviour you want: a preview that will not boot until
you gave it a real key cannot be forgotten, and one that boots on a known
key will be.

This overlay used to create that requirement by renaming the two keys to
`PREVIEW_*`. The base file carries it now, so the preview inherits it and
there is one name per secret across every deployment shape.

**Mail.** `EMAIL_TRANSPORT=none` on core-api and the worker. A preview
stack holds demo data and is reachable by more people than production is,
so it has no business sending mail to anyone. Messages stay queued and
unsent, which is what `none` means.

**Sizing.** Every ceiling is below the local baseline so several previews
fit on one host without starving each other (~3.7 GiB total). This is
deliberately not a performance claim — the `perf-smoke` CI job owns
performance, against its own runner and its own 50k-ticket seed. The one
non-obvious setting is Postgres `shm_size: 256m`: parallel query takes
shared memory, and Docker's 64m default surfaces as a
"could not resize shared memory segment" error under any plan that goes
parallel.

**Timestamps.** It runs the branch's merge commit
(`refs/pull/N/merge`), not the head of the base branch. A preview
serving `main` makes every reviewer believe their change broke nothing.

## Enabling the CI leg

User-gated, like every workflow in `.github/workflows/*.disabled`:

1. Rename `preview.yml.disabled` to `preview.yml`.
2. Add the repository **variables** (not secrets — none is a credential):
   `PREVIEW_HOST`, `PREVIEW_PUBLIC_URL`, `PREVIEW_API_URL`,
   `PREVIEW_CORS_ORIGIN`. For a TLS front door, add `PREVIEW_TLS_MODE`
   (`direct`, `cf-origin` or `cf-dns01`), `PREVIEW_DOMAIN` (the wildcard
   zone) and, for the two ACME modes, `PREVIEW_ACME_EMAIL`. Leave
   `PREVIEW_TLS_MODE` empty and the job boots the raw-port stack above.
   The DNS-01 token is the only credential, so it goes in secrets as
   `CF_API_TOKEN`, scoped to Zone:DNS:Edit on the one zone.
3. Fill in the two placeholder steps: host provisioning and teardown.
   They are deliberately `exit 1` with a message rather than a plausible
   guess, because a teardown that is not wired is how previews outlive
   their PRs.
4. Give the job `pull-requests: write` (already in the definition) and
   make sure your `GITHUB_TOKEN` can reach your host.

The teardown is a separate job gated on the PR's `closed` action, and it
deliberately does not depend on the deploy job succeeding — a failed
deploy must never be the reason a preview stays up.

## Teardown, when you do it by hand

```sh
docker compose -f deploy/docker-compose.yml \
               -f deploy/preview/compose.preview.yml \
               -f deploy/edge/compose.edge.yml down -v
```

`-v` takes the volumes with it, which is what you want for a preview
whose whole value is being disposable. A preview that survives its PR is
a stale branch on the network at a URL people remember, so tear it down
even when nothing is wrong with the branch. The edge overlay is optional
in the teardown — including it also removes the `edgedata` volume, which
holds the proxy's certificates. Leaving it off tears down the stack and
keeps the certificates; including it is the honest "nothing survives" form.

## Related

- `deploy/docker-compose.yml` is the base stack; its header explains the
  hardening posture this overlay inherits.
- `docs/site/reference/self-host-local-stack.md` is the operator's view of
  the same file.
