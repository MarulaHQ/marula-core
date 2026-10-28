# Updating a Marula install

Six verbs, one rule: nothing is applied that was not signed, and nothing is
called done until the doctor agrees.

```sh
marula update --check --manifest marula.json --signature marula.json.sig --key release.pub
marula update --apply --manifest marula.json --signature marula.json.sig --key release.pub --dir /srv/marula
marula artifact verify --sums SHA256SUMS --signature SHA256SUMS.sig --key release.pub --root .
marula artifact checksum marula
```

## `--check` says what kind of update it is

The manifest carries releases and advisories in the same channel, so the check
can be honest about urgency:

```text
a security update is available: 0.10.0 (from 0.9.0). M-1: token leak in the worker
```

An ordinary release says "an update is available". A version below the
manifest's `revoked_below` is told it is revoked and to update now, and a
platform the manifest does not publish for says that instead of offering an
artifact that does not exist.

A manifest read without `--signature` and `--key` is printed anyway, with a
warning, because reading costs nothing. `--apply` refuses that path: an unsigned
manifest is a suggestion, not an instruction.

## The manifest

```json
{
  "schema_version": 1,
  "channel": "stable",
  "releases": [
    {
      "version": "0.10.0",
      "os": "macos",
      "arch": "arm64",
      "url": "https://…/marula-0.10.0-macos-arm64.tar.zst",
      "sha256": "…",
      "published": "2026-10-28T09:00:00Z"
    }
  ],
  "advisories": [
    {
      "id": "M-1",
      "severity": "security",
      "summary": "token leak in the worker",
      "affects": ["0.9.0"],
      "fixed_in": "0.10.0"
    }
  ],
  "revoked_below": "0.9.0"
}
```

`severity` is `normal`, `high` or `security`, and the check prints the most
urgent one that applies. `affects` empty means every release before `fixed_in`.
A `schema_version` this binary does not understand is refused rather than
half-read.

The signature is detached ed25519 over the manifest's exact bytes, hex encoded,
verified against the public keys you pass. It is the same verifier that checks
plugin artifacts, so the project has one answer to "is this signature good?".

## What `--apply` does, in order

1. Verifies the manifest's signature. Refuses without one.
2. Backs up the files the install owns into `<dir>/backups/<version>-<stamp>/`:
   `marula.env`, the compose files beside it, and the `VERSION` marker.
3. Builds the plan through the same deployment target the installer used, so an
   update and a first install cannot disagree about what a working install is.
4. Applies it.
5. Runs the doctor against the result.

If the apply fails, or the doctor disagrees, the files are restored from the
backup and the command exits non-zero with the reason:

```text
update rolled back: the doctor reports the install is not healthy after the update (1 checks failed)
the files were restored from /srv/marula/backups/0.9.0-20261028T091500Z: /srv/marula/marula.env
```

**Rollback restores files, not data.** It puts the install's configuration and
compose files back. It does not restore the database or the object store, and it
says so rather than implying otherwise. A release that changes the schema needs a
database backup taken before the update, and the plan's "still yours to do" line
keeps naming the backup destination for exactly that reason.

## Verifying a bundle before anything runs

`marula artifact verify` is the command a release bootstrap runs first. It
verifies the detached signature over `SHA256SUMS`, then every file the sums
name, and refuses a file that does not match by name. A sums file that lists an
absolute path or one that leaves the bundle directory is refused, and a line it
cannot parse refuses the whole file rather than being skipped: a skipped line is
an unverified file.

Without `--signature` it verifies checksums only and says so. A checksum proves
the bytes arrived intact. It does not prove who published them.

## What this does not do

It does not fetch. The manifest and the artifact come from the release pipeline
in `docs/release.md`, and the bootstrap there downloads and verifies them before
executing anything. It does not schedule itself, and it does not install a
security update without being asked: the sentence is the prompt.
