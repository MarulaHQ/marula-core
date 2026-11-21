#!/usr/bin/env bash
# upgrade-e2e.sh — install one release, then install the next one over it.
#
# The installer is also the upgrade path: give it the same `--dir` and it
# replaces what is there. Nothing in CI exercised that, so a release could be
# installable from scratch and refuse to upgrade, which is the shape the previous
# tag's users would hit first.
#
# What it proves, in order: the older release installs and answers; the newer
# release installs over it without a wipe; the running stack is the image the
# newer release pins, not merely a different one; and the result answers
# `/readyz`.
#
# Non-purpose: it does not verify the archives. The caller fetches each release
# and checks its signed checksum list, the same way install-e2e.sh's caller does,
# because the two legs share that trust decision rather than duplicating it here.
#
# Usage: upgrade-e2e.sh <from-release-dir> <to-release-dir> <install-dir>

set -euo pipefail

say() { printf '==> %s\n' "$*"; }
die() { printf 'upgrade-e2e: %s\n' "$*" >&2; exit 1; }

USAGE='usage: upgrade-e2e.sh <from-release-dir> <to-release-dir> <install-dir>'
FROM=${1:-}
TO=${2:-}
INSTALL_DIR=${3:-}
[ -n "$FROM" ] && [ -n "$TO" ] && [ -n "$INSTALL_DIR" ] || die "$USAGE"

[ -d "$FROM" ] || die "$FROM is not a directory"
[ -d "$TO" ] || die "$TO is not a directory"
FROM=$(cd "$FROM" && pwd)
TO=$(cd "$TO" && pwd)
[ -x "$FROM/install.sh" ] || die "$FROM/install.sh is missing: that is not a release directory"
[ -x "$TO/install.sh" ] || die "$TO/install.sh is missing: that is not a release directory"
mkdir -p "$INSTALL_DIR"
INSTALL_DIR=$(cd "$INSTALL_DIR" && pwd)

# `--allow-unfit` is the capacity waiver install-e2e.sh also uses: a runner's
# free disk is not guaranteed past split's floor, and the floors are the doctor's
# unit tests' business, not this leg's.
INSTALL_ARGS=(--dir "$INSTALL_DIR" --images registry --profile split --apply --allow-unfit)

core_api_image() {
  # The container the stack is running right now, addressed by compose's own
  # label rather than by a project name this script would have to guess.
  local id
  id=$(docker ps -q --filter label=com.docker.compose.service=core-api | head -1)
  [ -n "$id" ] || return 1
  docker inspect -f '{{.Image}}' "$id"
}

# The image the newer release's installer pinned for core-api. The installer
# writes `marula.images.yml` from the release's signed manifest, so this is the
# target's own statement of which image should be running — not a version string
# guessed from a tag.
core_api_pin() {
  awk '
    /^  core-api:$/ { wanted = 1; next }
    wanted && /^    image:/ {
      sub(/^    image:[[:space:]]*/, "")
      sub(/[[:space:]]+$/, "")
      print
      exit
    }
    wanted && /^  [a-z]/ { wanted = 0 }
  ' "$INSTALL_DIR/marula.images.yml"
}

wait_for_readyz() {
  local why=$1
  for _ in $(seq 1 30); do
    if curl -fsS --max-time 2 http://127.0.0.1:8080/readyz >/dev/null 2>&1; then
      echo "  /readyz ok"
      return 0
    fi
    sleep 2
  done
  die "$why"
}

teardown() {
  local root=$1
  local -a overlays=()
  local overlay
  for overlay in "$INSTALL_DIR/marula.images.yml" "$INSTALL_DIR/marula.restart.yml"; do
    [ -f "$overlay" ] && overlays+=(-f "$overlay")
  done
  for env_file in "$INSTALL_DIR/marula.env" "$INSTALL_DIR/compose.env"; do
    [ -f "$env_file" ] || continue
    docker compose --env-file "$env_file" -f "$root/deploy/docker-compose.yml" \
      ${overlays[@]+"${overlays[@]}"} down -v >/dev/null 2>&1 || true
  done
}

# The workflow has its own "what was running" step, but it runs after this
# script exits and this script's trap removes the containers. Capture the state
# here, before teardown, and only when the run failed: a passing leg should leave
# nothing behind but a teardown.
on_exit() {
  local status=$?
  trap - EXIT
  set +e
  if [ "$status" -ne 0 ]; then
    printf '==> the upgrade failed; what was running before teardown\n'
    docker ps -a --format 'table {{.Names}}\t{{.Image}}\t{{.Status}}' || true
    local id
    for id in $(docker ps -aq); do
      printf -- '--- %s\n' "$(docker inspect -f '{{.Name}}' "$id" 2>/dev/null || echo "$id")"
      docker logs --tail 60 "$id" 2>&1 || true
    done
  fi
  teardown "$TO" || true
  exit "$status"
}
trap on_exit EXIT

say "installing $(basename "$FROM") first"
"$FROM/install.sh" "${INSTALL_ARGS[@]}"
wait_for_readyz "the first install reported success but /readyz never answered"

BEFORE=$(core_api_image) || die "no core-api container is running after the first install"
say "core-api is running image ${BEFORE:0:19}…"

say "installing $(basename "$TO") over it"
"$TO/install.sh" "${INSTALL_ARGS[@]}"
wait_for_readyz "the upgrade reported success but /readyz never answered"

AFTER=$(core_api_image) || die "no core-api container is running after the upgrade"
# The identity check below is the pass condition. An image id that did not change
# is not a failure on its own: image ids identify content, so two releases whose
# core-api content is identical share one, and the pipeline never promised
# otherwise. What must hold is that the running image is the one the newer
# release pins, which also catches an installer that left the old container
# running while the target's image was already in the local store.
if [ "$BEFORE" = "$AFTER" ]; then
  say "core-api's image id is unchanged: the two releases carry identical core-api content"
else
  say "core-api moved to image ${AFTER:0:19}…"
fi

# The newer release names the image it expects in the override it writes from its
# signed manifest, and the running container has to be that image.
PIN=$(core_api_pin) || die "could not read the core-api pin from $INSTALL_DIR/marula.images.yml"
[ -n "$PIN" ] || die "the upgrade did not pin core-api in $INSTALL_DIR/marula.images.yml"
PIN_ID=$(docker image inspect -f '{{.Id}}' "$PIN") \
  || die "the image the newer release pins ($PIN) is not in the local store"
if [ "$AFTER" != "$PIN_ID" ]; then
  die "core-api is running ${AFTER:0:19}…, but the newer release pins $PIN (${PIN_ID:0:19}…)"
fi
say "it is the image the newer release pins: $PIN"

say "checking the result with the newer release's doctor"
"$TO/marula" doctor \
  --dir "$INSTALL_DIR" \
  --profile split \
  --compose-file "$TO/deploy/docker-compose.yml" \
  --allow-unfit

say "$(basename "$FROM") upgraded to $(basename "$TO") and the stack answers"
