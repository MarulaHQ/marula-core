# Releasing

## Versioning

Semantic versioning, `0.x` until GA. Pre-1.0 a **minor** bump may include breaking
changes, but only with an upgrade note in `CHANGELOG.md` and, when a migration is
involved, upgrade documentation. A breaking database migration additionally requires
the `--breaking` flag and fails CI without it (see `CONTRIBUTING.md`).

## Tag format

`vX.Y.Z` — annotated **and signed**. Not a lightweight tag.

- **Tagger identity:** `Marula Core <releases@getmarula.com>`, not a personal key.
- **Signing:** GPG or SSH key belonging to that identity. The public key and its
  fingerprint are listed below so a downloader can verify.
- **Immutability:** the `v*` ruleset blocks tag updates and deletions. A published tag
  is a promise; if a release is wrong, publish the next one.

```
# Publish the fingerprint, once, when the key is created:
# signing key fingerprint: <FILL IN>
```

## Steps

1. **Land everything.** `main` green, no open release blockers.
2. **Changelog.** Move `[Unreleased]` bullets under the new version and date. Bullets
   say what changed for the user, never which wave or lane did it.
3. **Version bump.** `Cargo.toml`, `package.json` and any other manifest that carries
   the version, in one commit: `chore(release): vX.Y.Z`.
4. **Tag, signed, from the project identity.**

   ```
   git tag -s vX.Y.Z -m "Marula Core vX.Y.Z" -u <releases@getmarula.com key>
   git verify-tag vX.Y.Z
   ```

5. **Push** `main` and the tag.
6. **Release notes** on GitHub: the changelog section verbatim, plus a "Breaking"
   heading when there is one, and the database-migration note.
7. **Images** (when published): `ghcr.io/marulahq/<component>:vX.Y.Z`. GHCR owner names
   are lowercase, so the registry path is `marulahq` even though the organisation is
   `MarulaHQ`.
8. **Verify the artefacts** a user would get: pull the image, run the documented
   self-host path, confirm the version endpoint reports `vX.Y.Z`.

## Sign after a history rewrite, never before

A rewrite (identity change, file exclusion) changes every commit SHA and invalidates any
tag created earlier. If a rewrite is planned, tag afterwards.
