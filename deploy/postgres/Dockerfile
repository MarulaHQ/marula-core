# Postgres with the two extensions the base image does not carry.
#
# Why a separate image: `pg_cron` and `hypopg` are not part of the PostgreSQL
# contrib set, so neither is in `pgvector/pgvector:pg18` nor in the Docker
# Hardened Image the compose stack runs by default. The hardened image has no
# package manager, so the only way to add them is a different base. This one
# starts from the same official pgvector build the E2E stack uses and installs
# the two packages from the PGDG apt repository that image already configures.
#
# Build:  docker build -t marula/postgres-extensions:18 deploy/postgres
# Use:    docker compose -f docker-compose.yml \
#                        -f postgres/compose.extensions.yml up -d
#
# Trade-off, stated plainly: this base is not the DHI-hardened one, and the
# overlay that uses it is opt-in for that reason. A deployment that does not
# need pg_cron or hypopg should keep the default image — migration 0145 works
# on a database that has neither.
#
# Package versions resolved at build time from bookworm-pgdg on 2026-10-21:
# pg_cron 1.6.8, hypopg 1.4.3. They are unpinned deliberately: PGDG retires
# old builds, and a pinned apt version is a build that stops working without a
# change on our side. The base image digest is what pins the PostgreSQL major.
ARG PG_IMAGE=pgvector/pgvector:pg18@sha256:2ba9ca5f2e7daa0f0e7723cba1ee9167bab54efd3640516a44ac1a928dd67e7a
FROM ${PG_IMAGE}

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      postgresql-18-cron \
      postgresql-18-hypopg \
 && rm -rf /var/lib/apt/lists/*
