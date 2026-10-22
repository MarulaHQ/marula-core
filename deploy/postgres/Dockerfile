# Postgres with the extensions the base image does not carry.
#
# Why a separate image: `pg_cron`, `hypopg` and VectorChord's `vchord` are not
# part of the PostgreSQL contrib set, so none of them is in
# `pgvector/pgvector:pg18` nor in the Docker Hardened Image the compose stack
# runs by default. The hardened image has no package manager, so the only way to
# add them is a different base. This one starts from the same official pgvector
# build the E2E stack uses and installs the two PGDG packages from the apt
# repository that image already configures, plus the VectorChord deb.
#
# Build:  docker build -t marula/postgres-extensions:18 deploy/postgres
# Use:    docker compose -f docker-compose.yml \
#                        -f postgres/compose.extensions.yml up -d
#
# Trade-off, stated plainly: this base is not the DHI-hardened one, and the
# overlay that uses it is opt-in for that reason. A deployment that does not
# need pg_cron, hypopg or `MARULA_RETRIEVAL_MODE=index` should keep the default
# image — migration 0145 works on a database that has neither extension, and
# migration 0153 reports `skipped: ...` for a mode this image cannot run.
#
# Package versions resolved at build time from bookworm-pgdg on 2026-10-21:
# pg_cron 1.6.8, hypopg 1.4.3. They are unpinned deliberately: PGDG retires
# old builds, and a pinned apt version is a build that stops working without a
# change on our side. The base image digest is what pins the PostgreSQL major.
#
# VectorChord is pinned the other way: it is not in apt, so its release asset is
# fetched by URL and checked against a per-architecture sha256. `vchord` needs
# the deb for the exact PostgreSQL major (18) and the machine's architecture,
# which is why the filename is built rather than hardcoded. The release is
# published under the `supervc-stack` org; the `tensorchord/VectorChord` URL
# redirects there. Bumping the version means updating both sums — a wrong sum
# fails the build loudly instead of installing something unexpected.
ARG PG_IMAGE=pgvector/pgvector:pg18@sha256:2ba9ca5f2e7daa0f0e7723cba1ee9167bab54efd3640516a44ac1a928dd67e7a
FROM ${PG_IMAGE}

ARG VCHORD_VERSION=0.5.3
ARG TARGETARCH

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      postgresql-18-cron \
      postgresql-18-hypopg \
      curl \
      ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# VectorChord 0.5.3 for pg18. `TARGETARCH` is set by buildx; a plain
# `docker build` leaves it empty, so the container's own dpkg architecture is
# the fallback.
RUN set -eu; \
    arch="${TARGETARCH:-$(dpkg --print-architecture)}"; \
    case "$arch" in \
      amd64) sum=5323f58f22fc1882039aeb936cf0631d8a922bd1ee34042debfc9e865a2283d6 ;; \
      arm64) sum=1c88669550578c9b4a196cf7193c8648f6fcacc89155a934f10515f277e95dc9 ;; \
      *) echo "no VectorChord deb for architecture $arch" >&2; exit 1 ;; \
    esac; \
    deb="postgresql-18-vchord_${VCHORD_VERSION}-1_${arch}.deb"; \
    curl -fsSL -o "/tmp/${deb}" \
      "https://github.com/supervc-stack/VectorChord/releases/download/${VCHORD_VERSION}/${deb}"; \
    echo "${sum}  /tmp/${deb}" | sha256sum -c -; \
    apt-get update; \
    apt-get install -y --no-install-recommends "/tmp/${deb}"; \
    rm -f "/tmp/${deb}"; \
    rm -rf /var/lib/apt/lists/*

# Fail the build, not the first query, when the library is not where Postgres
# looks for it: a `shared_preload_libraries=vchord` that cannot load leaves the
# server refusing to start, which is a worse place to find out.
RUN test -f "/usr/lib/postgresql/18/lib/vchord.so"
