---
bump: patch
---

### Fixed

- Air-gapped bundles install again: the archive references three images by tag instead of by digest, because a digest does not survive `docker save`/`docker load` on the store most hosts use and the images came back with no name at all.
