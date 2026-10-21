#!/usr/bin/env bash
# vector-ann-bench.sh — measure filtered ANN against the exact KNN Marula runs.
#
#   scripts/vector-ann-bench.sh                     both engines, full fixture
#   scripts/vector-ann-bench.sh --engines a         pgvector image only
#   scripts/vector-ann-bench.sh --small 200 --large 2000 --fillers 2 --filler-rows 100
#                                                   a smoke run that proves the harness
#   scripts/vector-ann-bench.sh --iters 10 --out /tmp/ann
#   scripts/vector-ann-bench.sh --corpus real --vectors /tmp/p52-corpus-real
#                                                   real EmbeddingGemma 2 vectors
#   scripts/vector-ann-bench.sh --keep              leave the containers up
#
# WHAT IT MEASURES. The production vector leg
# (crates/core-db/src/search.rs:650-673) run as a role that is subject to RLS,
# over a synthetic corpus with two tenant sizes. Every arm reports p50/p95 and
# recall@10 against that tenant's exact result set. The exact pass runs before
# any index exists and again on each engine, so a cross-engine reading has a
# baseline from the same build.
#
# WHY IT IS SHAPED THIS WAY.
#   * One container per engine, run one engine at a time. Two engines sharing
#     CPU measure contention.
#   * One index exists at a time. Two ANN indexes on one column leave the
#     planner free to pick whichever it likes, and an arm that measured the
#     wrong index is worse than no measurement.
#   * The benchmark role owns no table and is not a superuser. Owner bypasses
#     RLS, and a measurement that bypasses the policy tests the opposite of
#     what migration 0062 was worried about.
#   * enable_seqscan is off for ANN arms, because with a tenant predicate the
#     planner often prefers the exact scan it already trusted. That choice is
#     recorded separately in PLANS.txt, unforced, through the exact arm.
#
# Exit 1 on any failed stage. The numbers are the output; a partial run is a
# failed run.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 2

SQL_DIR="$ROOT/scripts/vector-ann-bench"
ITERS=40
SMALL=2000
LARGE=100000
FILLERS=20
FILLER_ROWS=10000
ENGINES="both"
KEEP=0
OUT=""
CORPUS="synthetic"
VECTORS=""

while [ $# -gt 0 ]; do
  case "$1" in
    --engines) ENGINES="${2:-}"; shift 2 ;;
    --iters) ITERS="${2:-}"; shift 2 ;;
    --small) SMALL="${2:-}"; shift 2 ;;
    --large) LARGE="${2:-}"; shift 2 ;;
    --fillers) FILLERS="${2:-}"; shift 2 ;;
    --filler-rows) FILLER_ROWS="${2:-}"; shift 2 ;;
    --out) OUT="${2:-}"; shift 2 ;;
    --corpus) CORPUS="${2:-}"; shift 2 ;;
    --vectors) VECTORS="${2:-}"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    -h|--help) sed -n '2,30p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

RUN_ID="p52ann-$$-$(date +%s)"
OUT="${OUT:-$ROOT/.kilo/vector-ann-bench/$RUN_ID}"
mkdir -p "$OUT" || exit 2

log() { printf '%s\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }

CURRENT=""
cleanup() {
  if [ "$KEEP" -eq 0 ] && [ -n "$CURRENT" ]; then
    docker rm -f "$CURRENT" >/dev/null 2>&1
  fi
}
trap cleanup EXIT

psql_in() {  # psql_in <container> <args...>
  local c="$1"; shift
  docker exec -i "$c" psql -U postgres -q -v ON_ERROR_STOP=1 "$@"
}

wait_ready() {  # wait_ready <container>
  # The timescale image runs initdb, stops the server for a moment, then starts
  # it again. A single successful probe lands in that gap and the next command
  # dies with "the database system is shutting down". Two good probes three
  # seconds apart mean the server is settled, not passing through.
  local c="$1" i=0 got=0
  while [ "$i" -lt 120 ]; do
    if docker exec "$c" psql -U postgres -tAc 'select 1' >/dev/null 2>&1; then
      got=$((got + 1))
      [ "$got" -ge 2 ] && return 0
      sleep 3
    else
      got=0
      sleep 2
    fi
    i=$((i + 1))
  done
  docker logs --tail 20 "$c" >&2
  return 1
}

engine_name() {
  case "$1" in
    a) printf 'pgvector' ;;
    b) printf 'extensions' ;;
  esac
}

engine_image() {
  case "$1" in
    a) printf '%s' "${ENGINE_A_IMAGE:-pgvector/pgvector:pg18}" ;;
    b) printf '%s' "${ENGINE_B_IMAGE:-timescale/timescaledb-ha:pg18}" ;;
  esac
}

start_engine() {  # start_engine <a|b> -> sets CURRENT
  local e="$1" name image
  name="$(engine_name "$e")"
  image="$(engine_image "$e")"
  CURRENT="p52-ann-$RUN_ID-$name"
  docker rm -f "$CURRENT" >/dev/null 2>&1

  if [ "$e" = "b" ]; then
    # vchord refuses to load without the preload; timescaledb needs its own
    # preload or the image's init step fails.
    docker run -d --name "$CURRENT" \
      --shm-size=4g \
      -e POSTGRES_PASSWORD=bench \
      "$image" -c shared_preload_libraries=timescaledb,vchord >/dev/null || return 1
  else
    # A parallel index build allocates its shared memory from /dev/shm, which
    # docker defaults to 64MB. Without this the HNSW build dies at 2GB of
    # maintenance_work_mem with "could not resize shared memory segment".
    docker run -d --name "$CURRENT" --shm-size=4g -e POSTGRES_PASSWORD=bench "$image" >/dev/null || return 1
  fi

  wait_ready "$CURRENT" || fail "engine $name never became ready"

  psql_in "$CURRENT" -c 'CREATE EXTENSION IF NOT EXISTS vector' >/dev/null \
    || fail "engine $name: pgvector would not install"
  if [ "$e" = "b" ]; then
    psql_in "$CURRENT" -c 'CREATE EXTENSION IF NOT EXISTS vectorscale' >/dev/null \
      || fail "engine $name: pgvectorscale would not install"
    psql_in "$CURRENT" -c 'CREATE EXTENSION IF NOT EXISTS vchord CASCADE' >/dev/null \
      || fail "engine $name: vchord would not install"
  fi
  return 0
}

versions() {  # versions <container>
  docker exec "$1" psql -U postgres -tAc \
    "select extname || ' ' || extversion from pg_extension
      where extname in ('vector','vchord','vectorscale') order by 1"
}

fixture_args=(-v "small=$SMALL" -v "large=$LARGE" -v "fillers=$FILLERS" -v "filler_rows=$FILLER_ROWS")
arm_args=(-v "iters=$ITERS")

case "$CORPUS" in
  synthetic) FIXTURE_SQL="01-fixture.sql" ;;
  real)
    FIXTURE_SQL="01r-fixture-real.sql"
    VECTORS="${VECTORS:-/tmp/p52-corpus-real}"
    for f in "$VECTORS/vectors.tsv" "$VECTORS/queries.tsv"; do
      [ -r "$f" ] || fail "real corpus file missing: $f"
    done
    arm_args+=(-v "real_corpus=1")
    fixture_args+=(-v "real_corpus=1")
    ;;
  *) fail "unknown --corpus value: $CORPUS" ;;
esac

{
  printf 'run_id %s\n' "$RUN_ID"
  printf 'started %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf 'host %s %s\n' "$(uname -m)" "$(sw_vers -productVersion 2>/dev/null || uname -r)"
  printf 'cpus %s\n' "$(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo unknown)"
  printf 'memory_bytes %s\n' "$(sysctl -n hw.memsize 2>/dev/null || echo unknown)"
  printf 'corpus %s\n' "$CORPUS"
  if [ "$CORPUS" = "real" ]; then
    printf 'vectors %s (%s docs, %s queries)\n' "$VECTORS" \
      "$(wc -l < "$VECTORS/vectors.tsv" | tr -d ' ')" \
      "$(wc -l < "$VECTORS/queries.tsv" | tr -d ' ')"
  else
    printf 'fixture small=%s large=%s fillers=%s filler_rows=%s iters=%s\n' \
      "$SMALL" "$LARGE" "$FILLERS" "$FILLER_ROWS" "$ITERS"
  fi
} > "$OUT/ENV.txt"

run_engine() {  # run_engine <a|b>
  local e="$1" name
  name="$(engine_name "$e")"
  log "== engine $name"
  start_engine "$e" || exit 1
  versions "$CURRENT" | sed "s/^/   $name: /" | tee -a "$OUT/ENV.txt"

  log "   schema"
  psql_in "$CURRENT" "${fixture_args[@]}" -f - < "$SQL_DIR/00-schema.sql" > "$OUT/$name-schema.log" 2>&1 \
    || { tail -20 "$OUT/$name-schema.log" >&2; fail "$name: schema"; }

  log "   fixture ($FIXTURE_SQL)"
  if [ "$CORPUS" = "real" ]; then
    docker cp "$VECTORS/vectors.tsv" "$CURRENT:/tmp/p52-vectors.tsv" >/dev/null || fail "copy vectors"
    docker cp "$VECTORS/queries.tsv" "$CURRENT:/tmp/p52-queries.tsv" >/dev/null || fail "copy queries"
  fi
  psql_in "$CURRENT" "${fixture_args[@]}" -f - < "$SQL_DIR/$FIXTURE_SQL" > "$OUT/$name-fixture.log" 2>&1 \
    || { tail -20 "$OUT/$name-fixture.log" >&2; fail "$name: fixture"; }
  grep -E '^(S|L)\|' "$OUT/$name-fixture.log" | sed "s/^/   /"

  log "   harness"
  psql_in "$CURRENT" "${fixture_args[@]}" -f - < "$SQL_DIR/02-core.sql" > "$OUT/$name-core.log" 2>&1 \
    || { tail -20 "$OUT/$name-core.log" >&2; fail "$name: harness"; }

  if [ "$CORPUS" = "real" ]; then
    log "   real query vectors"
    psql_in "$CURRENT" "${fixture_args[@]}" -f - < "$SQL_DIR/03-queries-real.sql" > "$OUT/$name-queries.log" 2>&1 \
      || { tail -20 "$OUT/$name-queries.log" >&2; fail "$name: real queries"; }
  fi

  case "$e" in
    a) local arm_file="10-pgvector.sql" ;;
    b) local arm_file="20-extensions.sql" ;;
  esac

  log "   arms ($arm_file)"
  psql_in "$CURRENT" "${arm_args[@]}" -f - < "$SQL_DIR/$arm_file" > "$OUT/$name-arms.log" 2>&1 \
    || { tail -30 "$OUT/$name-arms.log" >&2; fail "$name: arms"; }

  # pgvectorscale's build can take the postmaster down on a small VM. Run it
  # best-effort and report it as skipped rather than losing the arms that
  # already committed.
  if [ -f "$SQL_DIR/21-diskann.sql" ] && [ "$e" = "b" ]; then
    log "   diskann arm (best effort)"
    if psql_in "$CURRENT" "${arm_args[@]}" -f - < "$SQL_DIR/21-diskann.sql" > "$OUT/$name-diskann.log" 2>&1; then
      log "   diskann arm completed"
    else
      log "   diskann arm FAILED, continuing without it (see $name-diskann.log)"
      tail -6 "$OUT/$name-diskann.log" | sed 's/^/      /'
      # Put the fixture back if the crash left it isolated.
      psql_in "$CURRENT" -c 'SELECT bench.restore_indexes()' >/dev/null 2>&1 || true
    fi
  fi

  psql_in "$CURRENT" -f - < "$SQL_DIR/90-report.sql" >> "$OUT/RESULTS.tsv" 2> "$OUT/$name-report.err" \
    || { tail -20 "$OUT/$name-report.err" >&2; fail "$name: report"; }
  psql_in "$CURRENT" -f - < "$SQL_DIR/91-plans.sql" > "$OUT/PLANS-$name.txt" 2>&1 \
    || fail "$name: plans"

  if [ "$KEEP" -eq 0 ]; then
    docker rm -f "$CURRENT" >/dev/null 2>&1
    CURRENT=""
  fi
}

case "$ENGINES" in
  both) run_engine a; run_engine b ;;
  a) run_engine a ;;
  b) run_engine b ;;
  *) fail "unknown --engines value: $ENGINES" ;;
esac

log ""
log "results: $OUT/RESULTS.tsv"
column -t -s "$(printf '\t')" "$OUT/RESULTS.tsv" 2>/dev/null || cat "$OUT/RESULTS.tsv"
