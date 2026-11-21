---
bump: patch
---

### Fixed

- Air-gapped bundles install again. The archive named three of its images by digest, and a digest does not survive `docker save`/`docker load`: on the overlay2 driver the image comes back with no name at all, so `install-offline.sh` reported `dhi.io/pgvector`, `dhi.io/valkey` and `dhi.io/pgbouncer` missing and refused to continue. The bundle now carries every image under its tag, and the compose file beside it names them the same way.
- Air-gapped bundles now carry an executable CLI. The release tree reached the air-gapped job through a workflow artifact, which does not carry file modes, so `marula`, `install.sh` and `gen-env.sh` arrived without their executable bit and `install-offline.sh` refused the bundle it was unpacked from.
- An air-gapped install on a host below the chosen shape's floors no longer reports itself broken: the bundle's own `install-offline.sh` now passes the shape and the capacity waiver to the health check it runs at the end, so the check judges the stack that was installed.
