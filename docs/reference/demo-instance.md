# The demo instance

A customer-facing demo has to look pristine for every prospect and still be a
deliberate deployment, not a laptop stack that escaped. This page records the
posture Wave 47 lane `p47-demo-deploy-profile` chose and the reset ritual lane
`p47-demo-reset-cron` automates.

## The posture

The API and worker run with `MARULA_DEV_TOOLS=1`, because the demo reset needs
`/api/dev/reset-org` and `/api/dev/time-travel`. That surface has no bearer auth.
It is safe here for three reasons, and all three are required:

1. `deploy/docker-compose.demo.yml` pins core-api to `127.0.0.1:8080`
(`DEMO_API_PORT` moves the host side only). The dev
   surface never listens on a public interface.
2. `deploy/edge/marula-routes.caddy` answers `404` for `/api/dev/*`. The public
   edge cannot reach the reset routes even if someone later publishes 8080.
3. `MARULA_SECRET_KEY` is explicit and required. The at-rest key is never derived
   from `JWT_SECRET` on this profile.

The overlay also runs a one-shot `demo-seed` service after migrations and before
core-api. It runs `seed-demo` without `--force`, so a restart never wipes a demo
an operator has prepared. `seed-demo` exits 0 when the org already exists. A
`web` service serves the standalone build on `127.0.0.1:3000`; it is built with
`DEMO_PUBLIC_ORIGIN` (default `http://localhost:8080`) because the browser calls
core-api directly.

A one-shot seed with the flag off was the alternative. It was rejected because
the reset path needs the API up, and a second hidden dev container is more
machinery than loopback plus one edge deny.

## First bring-up

Generate the eight values the base compose file requires. Use a new at-rest key,
not the repository's published dev values.

```bash
export JWT_SECRET=$(openssl rand -hex 32)
export MARULA_SECRET_KEY=$(openssl rand -hex 32)
export MARULA_INTERNAL_TOKEN=$(openssl rand -hex 32)
export EMAIL_INTERNAL_TOKEN=$(openssl rand -hex 32)
export POSTGRES_PASSWORD=$(openssl rand -hex 32)
export MARULA_APP_PASSWORD=marula_app
export GARAGE_RPC_SECRET=$(openssl rand -hex 32)
export GLITCHTIP_SECRET_KEY=$(openssl rand -hex 32)

export DEMO_PUBLIC_ORIGIN=https://demo.example.com   # or http://localhost:8080 on a laptop
export DEMO_API_PORT=8080                            # host port; keep 8080 unless it is taken
export DEMO_WEB_PORT=3000                            # host port for the web console
docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.demo.yml \
  --profile demo up -d --build
```

The boot prints `DEMO POSTURE` in the `dev-posture` service: core-api is on
loopback, the edge denies `/api/dev/*`, and resets go over a tunnel. The demo
logins are printed by the seeder: `owner@demo.local` and the other three, with
`demo-password-123`.

On a VPS, reach the reset routes through SSH, never through the public hostname:

```bash
ssh -L 8080:127.0.0.1:8080 demo@vps
# then, on the host, the reset script sees http://127.0.0.1:8080
```

## The reset ritual

Before a demo:

```bash
set -a; . /secure/path/demo.env; set +a   # the same secrets the stack uses
scripts/demo-reset.sh --apply --hours 24 --yes
```

The script:

1. Refuses if core-api is not bound to `127.0.0.1`, if `/api/dev` is not
   mounted, or if the org slug is not `marula-demo`.
2. Records a SHA-256 fingerprint of `MARULA_SECRET_KEY`. It never prints the key
   and never rotates it.
3. Copies the demo org's BYOK configuration to `.demo-reset/<timestamp>/`:
   `byok_secrets`, `ai_model_credentials`, `ai_surface_grants`, `ai_caps` and
   `ai_model_picks`.
4. Calls `POST /api/dev/reset-org`.
5. Restores the five tables, then calls `POST /api/dev/time-travel` (24 hours by
   default) so the SLA ladders are close enough to breach on stage.
6. Verifies the key fingerprint and every restored row count, keeps the five most
   recent snapshots, and prints the logins.

`scripts/demo-reset.sh` without `--apply` is a dry run that prints the plan and
the current counts. `--snapshot-only` backs up BYOK and changes nothing.

The reset never touches the at-rest key and never loses BYOK. If the reset call
succeeds but a restore fails, the snapshot stays on disk and the script stops
before time-travel; restore it before running another reset.

## Cron or a systemd timer

Run the reset on a schedule only if the demo box is not mid-presentation. A
timer that fires at 06:00 local time is usually enough.

```ini
# /etc/systemd/system/marula-demo-reset.service
[Unit]
Description=Reset the Marula demo org
After=docker.service

[Service]
Type=oneshot
WorkingDirectory=/opt/marula
EnvironmentFile=/secure/path/demo.env
ExecStart=/opt/marula/scripts/demo-reset.sh --apply --hours 24 --yes
```

```ini
# /etc/systemd/system/marula-demo-reset.timer
[Unit]
Description=Daily Marula demo reset

[Timer]
OnCalendar=*-*-* 06:00:00
Persistent=true

[Install]
WantedBy=timers.target
```

The timer needs the same environment the stack used. Keep that file at `0600`
and outside the repository. A cron entry is the same command with `--apply
--yes`; cron has no TTY, which is why `--yes` exists.

## What not to do

- Do not publish `8080` on `0.0.0.0`. The overlay pins loopback; leave it.
- Do not remove the `/api/dev/*` deny from the edge.
- Do not rotate `MARULA_SECRET_KEY` during a reset. The restored BYOK rows are
  sealed with it.
- Do not run `seed-demo --force` directly against a demo with BYOK. It deletes
  the org and the credentials with it; use `scripts/demo-reset.sh`.
- Do not reuse `deploy/dev.env` on a box a prospect can reach. It carries the
  repository's published values.
