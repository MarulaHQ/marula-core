#!/usr/bin/env bash
# install-e2e.sh — install Marula from a release bundle, the way a person does.
#
# The install path had no end-to-end test: the unit tests cover the plan, and
# nothing ran a published bundle and started a stack. Both ways of installing
# from an archive are now checked here, and they share every check but the entry
# point:
#
#   scripts/install-e2e.sh            marula-0.6.1-linux-x64 /tmp/marula-e2e
#   scripts/install-e2e.sh --offline  marula-0.6.1-offline-linux-x64 /tmp/marula-offline
#
# The online leg is the registry install. It takes an unpacked release directory,
# runs the bundle's own `install.sh` against the published digests, asks `marula
# doctor` whether the result works, waits for `/readyz`, and then proves the
# compose front door from the same bundle's files.
#
# The offline leg is the air-gapped install. It takes an unpacked *offline*
# bundle, runs the bundle's `install-offline.sh` — which checks the bundle's own
# sums, loads `images.tar` and applies — and makes the same doctor and `/readyz`
# checks against the result. It also asserts the property the bundle exists for:
# nothing in the install asked a registry for an image. The assertion is spelled
# out where it runs, including what it cannot show on a runner that still has a
# network.
#
# Non-purpose: it downloads nothing (the caller fetches the release), it does not
# verify the release signature (that is the caller's trust decision, see issue
# #101), and it leaves the Docker volumes behind on failure on purpose — the
# stack you want to inspect is the one that failed.
set -euo pipefail

say() { printf '==> %s\n' "$*"; }
die() { printf 'install-e2e: %s\n' "$*" >&2; exit 1; }

USAGE='usage: install-e2e.sh [--offline] <unpacked-bundle-dir> <empty-install-dir>'
MODE=online
if [ "${1:-}" = "--offline" ]; then
  MODE=offline
  shift
fi

BUNDLE=${1:-}
INSTALL_DIR=${2:-}
[ -n "$BUNDLE" ] || die "$USAGE"
[ -n "$INSTALL_DIR" ] || die "$USAGE"

BUNDLE=$(cd "$BUNDLE" && pwd)
mkdir -p "$INSTALL_DIR"
INSTALL_DIR=$(cd "$INSTALL_DIR" && pwd)

# ROOT is the release directory, the one holding `marula`, `deploy/` and
# `release.json`. An online bundle is that directory. An offline bundle wraps one
# inside itself and puts the images beside it, which is the only structural
# difference between the two legs: from here on the checks are the same.
if [ "$MODE" = offline ]; then
  [ -f "$BUNDLE/images.tar" ] || die "$BUNDLE/images.tar is missing: this is not an offline bundle"
  [ -f "$BUNDLE/images.list" ] || die "$BUNDLE/images.list is missing: the bundle does not name its images"
  shopt -s nullglob
  NESTED=("$BUNDLE"/marula-[0-9]*-*)
  shopt -u nullglob
  [ "${#NESTED[@]}" -eq 1 ] ||
    die "$BUNDLE must hold exactly one release directory, found ${#NESTED[@]}"
  ROOT=${NESTED[0]}
  INSTALLER="$BUNDLE/install-offline.sh"
else
  ROOT=$BUNDLE
  INSTALLER="$BUNDLE/install.sh"
fi

[ -x "$INSTALLER" ] || die "$INSTALLER is missing: this is not a release bundle"
[ -x "$ROOT/marula" ] || die "$ROOT/marula is missing: the bundle does not carry the CLI"
[ -f "$ROOT/release.json" ] || die "$ROOT/release.json is missing: the bundle does not describe its images"
[ -f "$ROOT/deploy/docker-compose.yml" ] || die "$ROOT/deploy/docker-compose.yml is missing"

# On success the stacks are torn down. On failure they stay up: the workflow's
# next step dumps `docker ps` and the logs, and a teardown here would remove the
# evidence of the failure it exists to explain.
SUCCESS=0
cleanup() {
  if [ "$SUCCESS" -ne 1 ]; then
    say "leaving every stack this leg started up so its logs can be read"
    return
  fi
  for env_file in "$INSTALL_DIR/marula.env" "$INSTALL_DIR/compose.env"; do
    [ -f "$env_file" ] || continue
    say "stopping the stack that used $env_file"
    (cd "$ROOT" && docker compose --env-file "$env_file" \
      -f deploy/docker-compose.yml \
      ${OVERLAYS[@]+"${OVERLAYS[@]}"} down -v) >/dev/null 2>&1 || true
  done
}
trap cleanup EXIT

# Waits for `/readyz` on the port the compose file publishes. $1 is appended to
# the success line and $2 is the sentence a timeout ends with: a leg that never
# came up and a leg that came up wrong are different diagnoses, and the caller
# knows which one it is running.
wait_for_readyz() {
  local success_suffix=$1
  local failure=$2
  for _ in $(seq 1 30); do
    if curl -fsS --max-time 2 http://127.0.0.1:8080/readyz >/dev/null 2>&1; then
      echo "  /readyz ok${success_suffix}"
      return 0
    fi
    sleep 2
  done
  die "$failure"
}

# The shape is named, not recommended. Left alone, the installer and the doctor
# each ask the host what fits, and on a box near a floor they can answer
# differently: the test would then install one shape and check another. `split`
# is the shape this leg checks, and the doctor is told the same, so its verdict
# is about the stack that was installed.
#
# `--allow-unfit` is the capacity waiver and nothing else: the job clears space
# before it pulls or loads images, but a runner's free disk is not guaranteed past
# split's 40 GB floor, so the doctor is told to report the floors as `waived`
# instead of failing on them. The floors themselves are covered by the doctor's
# unit tests.
# What this leg exists to prove is the install path, and nothing else checks it.
SHAPE_NAME='split'

if [ "$MODE" = offline ]; then
  # Offline, the install must not be told to use the registry: the images are
  # already here under the names compose derives for this project, and the plan's
  # default — `build` — is what resolves to them. Compose looks the derived name
  # up before it considers building, so `up` starts the loaded images and never
  # opens a build context the bundle does not carry. `install-offline.sh` pins
  # this same project name for the same reason, and every compose call here runs
  # under it so the teardown finds the containers the install made.
  export COMPOSE_PROJECT_NAME=marula
  say "installing from the images this bundle carries into $INSTALL_DIR"
  # The daemon's image event log is the only place a registry fetch is visible
  # afterwards, and it is what the assertion below reads. It is queried by
  # timestamp rather than streamed, so an event cannot slip past while the
  # stream is being set up.
  EVENTS_SINCE=$(date +%s)
  # `install-offline.sh` applies by itself — it loads the images and then hands
  # the rest to `marula install --apply` — and the CLI refuses a flag it is given
  # twice, so `--apply` and `--images` are the online leg's to name.
  INSTALL_ARGS=(--dir "$INSTALL_DIR" --profile "$SHAPE_NAME" --allow-unfit)
else
  say "installing from the published digests into $INSTALL_DIR"
  # `--images registry` is the whole point of the online leg: it builds nothing
  # and pins every service to the digest the release names, which is what a
  # self-hoster runs.
  INSTALL_ARGS=(--dir "$INSTALL_DIR" --images registry --profile "$SHAPE_NAME" --apply \
    --allow-unfit)
fi
"$INSTALLER" "${INSTALL_ARGS[@]}"

# The installer writes its overrides beside `marula.env`, and which ones exist
# depends on the mode: a registry install pins the release's digests in
# `marula.images.yml`, while the offline install uses the images the bundle
# loaded and writes no such file. Every compose call below passes the ones that
# are here, so a teardown cannot name a file that was never written.
OVERLAYS=()
for overlay in "$INSTALL_DIR/marula.images.yml" "$INSTALL_DIR/marula.restart.yml"; do
  if [ -f "$overlay" ]; then
    OVERLAYS+=(-f "$overlay")
  fi
done

# What "the bundle installs with the network off" can be shown to mean on a
# runner that still has a network. Three facts, all read from the daemon:
#
#   1. every reference in `images.list` is in the local store, so nothing the
#      stack names has to be fetched;
#   2. the images the installed stack resolves to are all in that list, so the
#      install did not need an image the bundle left out;
#   3. the daemon recorded no `pull` between the start of the install and the
#      moment `/readyz` answered, which is when this runs.
#
# What it does not show: that a host with no route at all behaves identically.
# Cutting a runner's network kills the job, and the daemon does the fetching
# rather than the CLI, so this reads what the install asked for, not what a
# network would have refused. The archive's own sums are checked by
# `install-offline.sh`, which is also why this function does not re-check them.
assert_no_registry_fetch() {
  local image count=0
  local -a missing=()
  while IFS= read -r image; do
    [ -n "$image" ] || continue
    count=$((count + 1))
    docker image inspect "$image" >/dev/null 2>&1 || missing+=("$image")
  done <"$BUNDLE/images.list"
  [ "$count" -gt 0 ] || die "$BUNDLE/images.list is empty: the bundle names no images"
  if [ "${#missing[@]}" -gt 0 ]; then
    printf 'install-e2e: images.list names these and the local store does not have them:\n' >&2
    printf '  %s\n' "${missing[@]}" >&2
    die "the bundle did not load every image it names"
  fi
  echo "  $count images named by images.list are in the local store"

  local -a stack_images=()
  while IFS= read -r image; do
    [ -n "$image" ] || continue
    stack_images+=("$image")
  done < <(cd "$ROOT" && docker compose --env-file "$INSTALL_DIR/marula.env" \
    -f deploy/docker-compose.yml \
    ${OVERLAYS[@]+"${OVERLAYS[@]}"} \
    config --images | LC_ALL=C sort -u)
  [ "${#stack_images[@]}" -gt 0 ] || die "the installed stack resolves to no images"

  local -a unlisted=()
  for image in "${stack_images[@]}"; do
    grep -Fxq -- "$image" "$BUNDLE/images.list" || unlisted+=("$image")
  done
  if [ "${#unlisted[@]}" -gt 0 ]; then
    printf 'install-e2e: the stack names these and images.list does not carry them:\n' >&2
    printf '  %s\n' "${unlisted[@]}" >&2
    die "the install names images the bundle does not carry, so it would have to fetch them"
  fi
  echo "  the stack resolves to ${#stack_images[@]} images, every one of them in the bundle"

  local pulls
  pulls=$(docker events --since "$EVENTS_SINCE" --until "$(date +%s)" \
    --filter type=image --format '{{.Action}} {{.Actor.Attributes.name}}' |
    awk '$1 == "pull"')
  if [ -n "$pulls" ]; then
    printf 'install-e2e: the daemon recorded these fetches during the install:\n' >&2
    printf '  %s\n' "$pulls" >&2
    die "the install reached a registry; this bundle is not installable with the network off"
  fi
  echo "  the daemon recorded no pull during the install"
}

say "checking the result"
"$ROOT/marula" doctor \
  --dir "$INSTALL_DIR" \
  --profile "$SHAPE_NAME" \
  --compose-file "$ROOT/deploy/docker-compose.yml" \
  --allow-unfit

say "the API answers on its published port"
wait_for_readyz '' 'the API did not answer /readyz on 127.0.0.1:8080; the install reported success'

if [ "$MODE" = offline ]; then
  # After `/readyz`, so the window the assertion reports is the whole install,
  # which is what docs/install.md promises a reader.
  assert_no_registry_fetch
  SUCCESS=1
  say "the air-gapped bundle installed a working stack and asked no registry for an image"
  exit 0
fi

say "tearing the first stack down before the second one takes its ports"
(cd "$ROOT" && docker compose --env-file "$INSTALL_DIR/marula.env" \
  -f deploy/docker-compose.yml \
  ${OVERLAYS[@]+"${OVERLAYS[@]}"} down -v) ||
  say "teardown of the first stack reported a problem; continuing to the compose path"

# The compose path is a different front door: no CLI plan, one generated env
# file, and compose itself. It has to work from the bundle's own files or the
# archive is only half a distribution.
"$ROOT/gen-env.sh" "$INSTALL_DIR/compose.env"
# The base file's four application services are `build:`-only and a bundle
# carries no sources, so this leg needs the overrides the installer wrote:
# `marula.images.yml` pins each service to the published digest and resets
# `build:`. Without them compose tries to compile and the leg cannot pass.
# The shape decides which services exist: all-in-one runs the loops inside
# core-api and has no worker. Starting the base file's worker there would build a
# service the shape never starts, from a directory with no sources.
# The shape is recorded in the installer's env file, not in the one `gen-env.sh`
# writes: that one carries the eight secrets and nothing about the topology.
SHAPE_SERVICES=(core-api worker web)
SHAPE_FILES=()
if grep -q '^MARULA_ALL_IN_ONE=1' "$INSTALL_DIR/marula.env" 2>/dev/null; then
  # all-in-one runs the loops inside core-api: no worker, and the overlay that
  # removes it and adds the in-process wiring the shape needs.
  SHAPE_SERVICES=(core-api web)
  SHAPE_FILES=(-f deploy/docker-compose.all-in-one.yml)
fi
(cd "$ROOT" && docker compose \
  --env-file "$INSTALL_DIR/compose.env" \
  -f deploy/docker-compose.yml \
  ${SHAPE_FILES[@]+"${SHAPE_FILES[@]}"} \
  -f "$INSTALL_DIR/marula.images.yml" \
  -f "$INSTALL_DIR/marula.restart.yml" \
  up -d --wait "${SHAPE_SERVICES[@]}")

say "the compose path answers on its published port"
wait_for_readyz ' from the compose path' 'the compose path did not bring the API up'

SUCCESS=1
say "both paths installed a working stack"
