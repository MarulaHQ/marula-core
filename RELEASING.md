# Releasing

## Versioning

Semantic versioning, `0.x` until GA. Pre-1.0 a **minor** bump may include breaking
changes, but only with an upgrade note in `CHANGELOG.md` and, when a migration is
involved, upgrade documentation. A breaking database migration additionally requires
the `--breaking` flag and fails CI without it (see `CONTRIBUTING.md`).

## Tag format

`vX.Y.Z` — annotated **and signed**. Not a lightweight tag.

- **Tagger identity:** `Marula Core <releases@getmarula.com>`, not a personal key.
- **Signing:** an SSH signing key, or a GPG key, belonging to that identity. The release
  key and its fingerprint are below, and `release-keys/allowed_signers` carries the same
  binding so a clean clone can verify a tag without local setup.
- **Immutability:** the `v*` ruleset blocks tag updates and deletions. A published tag
  is a promise; if a release is wrong, publish the next one.

### The release key

An SSH signing key created for this purpose, used for nothing else, and registered on the
maintainer's account. **The repository does not carry the key, and it carries no placeholder
for one.** A public key committed as a stand-in is worse than no key at all: it tells an
operator which key to trust while being a key nobody holds.

Before the first release, the maintainer creates the key and commits its public half:

```sh
ssh-keygen -t ed25519 -C releases@getmarula.com -f ~/.ssh/marula-release
printf 'releases@getmarula.com %s\n' "$(cat ~/.ssh/marula-release.pub)" \
  >> release-keys/allowed_signers
ssh-keygen -l -f ~/.ssh/marula-release.pub    # the fingerprint to publish
```

`release-keys/allowed_signers` ships with no key line, and the signing job in
`.github/workflows/release-installer.yml` refuses to publish until one is added. That is the
intended state: a release nobody can verify is not a release.

The same public line is what a clean clone uses to verify a tag, which is why it is committed
rather than only published on the release page.

Sign a tag, with the key loaded in the agent or a passphrase prompt to answer:

```
GIT_COMMITTER_NAME="Marula Core" GIT_COMMITTER_EMAIL="releases@getmarula.com" \
  git -c user.signingkey=~/.ssh/marula-release.pub tag -s vX.Y.Z -m "Marula Core vX.Y.Z"
```

Verify one in any clone:

```
git -c gpg.ssh.allowedSignersFile=release-keys/allowed_signers verify-tag vX.Y.Z
```

An SSH key is not cryptographically bound to an address, so the identity above is asserted
twice: by the tagger fields on the tag, and by the binding in `release-keys/allowed_signers`.

Treat it as a release credential: it signs tags and nothing else, it is registered on GitHub
as a signing key rather than an authentication key, and its private half never enters the
repository. A signature proves which key signed, not who held it.

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
9. **The binary and the installers.** The `v*.*.*` tag runs the
   `release-installer` workflow, which builds and signs them. `docs/release.md`
   covers what each file is, how to check a download by hand, and what to do with
   the artifacts once the run is green.

## Sign after a history rewrite, never before

A rewrite (identity change, file exclusion) changes every commit SHA and invalidates any
tag created earlier. If a rewrite is planned, tag afterwards.
