---
bump: minor
---

### Added

- The release archive now installs itself. Unpack it, run ./install.sh, and the installer checks the archive against its own checksums before it starts.
- On a server, pass --dir to install into a service directory and read the plan before anything changes, then add --apply.

### Changed

- The split and demo install shapes now dial Postgres directly, and the connection pooler is a profile you turn on when instances multiply rather than a service every install starts.
