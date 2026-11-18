---
bump: minor
---

### Added

- A tagged release publishes the four application images to GHCR and records their digests in the manifest, so an install from a release bundle can pin the images that were published instead of refusing for a missing digest.
