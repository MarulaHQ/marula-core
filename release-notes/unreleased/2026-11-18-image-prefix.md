---
bump: patch
---

### Fixed

- The image prefix the CLI and the installer suggest is now `ghcr.io/marulahq/marula`; the previous one named a registry namespace that does not exist, so copying it failed on the first pull.
