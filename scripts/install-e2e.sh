#!/usr/bin/env bash
# install-e2e.sh — install Marula from a release bundle, the way a person does.
#
# The installer's registry mode had no end-to-end test: the unit tests cover the
# plan, and nothing ever pulled the published images and started a stack. This
# script is that test. It takes an unpacked bundle and an empty directory, runs
# the bundle's own `install.sh` against the published digests, asks `marula
# doctor` whether the result works, and then proves the compose path from the
# same bundle's files.
#
#   scripts/install-e2e.sh marula-0.6.0-linux-x64 /tmp/marula-e2e
#
# Non-purpose: it downloads nothing (the caller fetches the release), it does not
# verify the release signature (that is the caller's trust decision, see issue
# #101), and it leaves the Docker volumes behind on failure on purpose — the
# stack you want to inspect is the one that failed.
set -euo pipefail

say() { printf '==> %s\n' "$*"; }
die() { printf 'install-e2e: %s\n' "$*" >&2; exit 1; }

BUNDLE=${1:?usage: install-e2e.sh <unpacked-bundle-dir> <empty-install-dir>}
INSTALL_DIR=${2:?usage: install-e2e.sh <unpacked-bundle-dir> <empty-install-dir>}

BUNDLE=$(cd "$BUNDLE" && pwd)
mkdir -p "$INSTALL_DIR"
INSTALL_DIR=$(cd "$INSTALL_DIR" && pwd)

[ -x "$BUNDLE/install.sh" ] || die "$BUNDLE/install.sh is missing: this is not a release bundle"
[ -f "$BUNDLE/release.json" ] || die "$BUNDLE/release.json is missing: the bundle does not describe its images"
[ -f "$BUNDLE/deploy/docker-compose.yml" ] || die "$BUNDLE/deploy/docker-compose.yml is missing"

# On success the stacks are torn down. On failure they stay up: the workflow's
# next step dumps `docker ps` and the logs, and a teardown here would remove the
# evidence of the failure it exists to explain.
SUCCESS=0
cleanup() {
  if [ "$SUCCESS" -ne 1 ]; then
    say "leaving both stacks running so their logs can be read"
    return
  fi
  for env_file in "$INSTALL_DIR/marula.env" "$INSTALL_DIR/compose.env"; do
    [ -f "$env_file" ] || continue
    say "stopping the stack that used $env_file"
    (cd "$BUNDLE" && docker compose --env-file "$env_file" \
      -f deploy/docker-compose.yml \
      -f "$INSTALL_DIR/marula.images.yml" \
      -f "$INSTALL_DIR/marula.restart.yml" down -v) >/dev/null 2>&1 || true
  done
}
trap cleanup EXIT

say "installing from the published digests into $INSTALL_DIR"
# --images registry is the whole point: it builds nothing and pins every service
# to the digest the bundle names, which is what a self-hoster runs.
# The GitHub runner's free disk is below the split shape's 40 GB floor, so the
# plan is told to proceed on this box. The test is about the install path, not
# about capacity; the floor itself is exercised by the doctor's own checks.
"$BUNDLE/install.sh" --dir "$INSTALL_DIR" --images registry --apply --allow-unfit

say "checking the result"
"$BUNDLE/marula" doctor \
  --dir "$INSTALL_DIR" \
  --compose-file "$BUNDLE/deploy/docker-compose.yml"

say "the API answers on its published port"
for _ in $(seq 1 30); do
  if curl -fsS --max-time 2 http://127.0.0.1:8080/healthz >/dev/null 2>&1; then
    echo "  /healthz ok"
    break
  fi
  sleep 2
done
curl -fsS --max-time 2 http://127.0.0.1:8080/healthz >/dev/null ||
  die "the API did not answer /healthz on 127.0.0.1:8080; the install reported success"

say "tearing the first stack down before the second one takes its ports"
(cd "$BUNDLE" && docker compose --env-file "$INSTALL_DIR/marula.env" \
  -f deploy/docker-compose.yml \
  -f "$INSTALL_DIR/marula.images.yml" \
  -f "$INSTALL_DIR/marula.restart.yml" down -v) ||
  say "teardown of the first stack reported a problem; continuing to the compose path"

# The compose path is a different front door: no CLI plan, one generated env
# file, and compose itself. It has to work from the bundle's own files or the
# archive is only half a distribution.
"$BUNDLE/gen-env.sh" "$INSTALL_DIR/compose.env"
# The base file's four application services are `build:`-only and a bundle
# carries no sources, so this leg needs the overrides the installer wrote:
# `marula.images.yml` pins each service to the published digest and resets
# `build:`. Without them compose tries to compile and the leg cannot pass.
(cd "$BUNDLE" && docker compose \
  --env-file "$INSTALL_DIR/compose.env" \
  -f deploy/docker-compose.yml \
  -f "$INSTALL_DIR/marula.images.yml" \
  -f "$INSTALL_DIR/marula.restart.yml" \
  up -d --wait)

for _ in $(seq 1 30); do
  if curl -fsS --max-time 2 http://127.0.0.1:8080/healthz >/dev/null 2>&1; then
    echo "  /healthz ok from the compose path"
    break
  fi
  sleep 2
done
curl -fsS --max-time 2 http://127.0.0.1:8080/healthz >/dev/null ||
  die "the compose path did not bring the API up"

SUCCESS=1
say "both paths installed a working stack"
