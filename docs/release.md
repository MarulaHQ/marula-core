# Release artifacts

This page is for the person cutting a Marula Core release, and for the person
who has downloaded one and wants to check it before running anything.

Two things ship in a release: the `marula` command line binary, and the
installer app. A third artifact exists for hosts with no network: an offline
bundle carrying the container images the install needs.

## What a release produces

The `release-installer` workflow builds five legs, one per platform, and each
leg writes a directory plus one archive:

| File | What it is |
| --- | --- |
| `marula-<version>-<os>-<arch>.tar.gz` | One platform's release, archived. This is what the bootstrap downloads and what a person unpacks. |
| `SHA256SUMS-<os>-<arch>` | One line: the sha256 of that archive. |
| `SHA256SUMS-<os>-<arch>.sig` | An SSH signature over that one line, made with the release key. |
| `marula-<version>-offline-<os>-<arch>.tar.zst` | The air-gapped bundle for a Linux host, built for the Linux legs only. |
| `release.json` | The manifest: the version, and which files belong to which platform. |
| `SHA256SUMS` and `SHA256SUMS.sig` | One combined list over every file above, and its signature. Useful for a mirror or a release page. |

Unpacked, a release directory holds:

```
marula                       the CLI (marula.exe on Windows)
installers/<kind>/           the Tauri bundle: .app and .dmg, .exe setup, .deb
apps/installer/dist/         the wizard front end, where `marula install --ui` looks
deploy/                      the compose files and the files they mount
LICENSE, README.md           what a redistribution carries
release.json                 this directory's own manifest
SHA256SUMS                   every file above, for `sha256sum -c`
```

The directory is the unit a release is reviewed as. The archive is the unit a
person downloads, and its checksum is what a signature covers.

## Where the signature comes from

Checksums are signed with the same SSH key that signs release tags. Its public
half is in `release-keys/allowed_signers`, and `RELEASING.md` carries the
fingerprint and the tag-signing commands. The private half is held as the
`MARULA_CHECKSUM_SIGNING_KEY` Actions secret and nowhere else.

The signature is an SSH signature in the `file` namespace, which is why
`ssh-keygen -Y verify` and `marula artifact verify` agree about it. The
workflow signs each list, then verifies its own signature against
`release-keys/allowed_signers` before the job can pass. A key that does not
match the allowed signers fails the job rather than producing an unusable
signature.

Code signing is a separate matter and uses the platform's own credentials:
`APPLE_*` for macOS signing and notarisation, `WINDOWS_*` for Authenticode.
`tauri bundle` reads those and fails loudly rather than shipping something
unsigned. None of them live in this repository.

## Cutting a release

Follow `RELEASING.md` for the version bump and the signed tag. The workflow
starts from the tag.

```sh
git tag -s v0.3.1 -m "Marula Core v0.3.1"
git push origin v0.3.1        # the maintainer's call, not an agent's
```

On a `v*.*.*` tag each leg requires the signing secrets for its own platform. If
one is missing the leg fails with the secret's name in the message, because an
unsigned release is not a release. On a pull request or a branch push the same
legs build unsigned and print a notice saying so.

The final `publish` job fetches every leg, writes `release.json` and the
combined `SHA256SUMS`, signs the lists, re-verifies them, and uploads the whole
set as one workflow artifact. It does not create a GitHub Release or push
anything to a host. Attaching the files to a release page, and choosing the
base URL the docs print, stay with the maintainer.

A manual run can rehearse the pipeline:

```sh
gh workflow run release-installer \
  --ref main \
  -f offline=false -f sign=false
```

The `offline` input adds the air-gapped bundles. They cost a full container
build of the workspace per architecture, which is why a pull request skips them
and a release does not.

## Verify a download by hand

Four layers, each one cheap. Run them in a directory holding the files you
downloaded.

Check the signature over the checksum list first. Nothing below it means
anything until this passes:

```sh
ssh-keygen -Y verify \
  -f release-keys/allowed_signers \
  -I releases@getmarula.com \
  -n file \
  -s SHA256SUMS-linux-x64.sig < SHA256SUMS-linux-x64
```

A `Good "file" signature for releases@getmarula.com` line is the pass. The
same check through the CLI, which also checks every file the list names when
the file is present:

```sh
marula artifact verify \
  --sums SHA256SUMS-linux-x64 \
  --signature SHA256SUMS-linux-x64.sig \
  --key release-keys/allowed_signers
```

Check the archive itself against the signed line:

```sh
marula artifact checksum marula-0.3.1-linux-x64.tar.gz
grep 'marula-0.3.1-linux-x64.tar.gz' SHA256SUMS-linux-x64
```

The two hashes must match. Then unpack and check the tree:

```sh
tar -xzf marula-0.3.1-linux-x64.tar.gz
cd marula-0.3.1-linux-x64
sha256sum -c SHA256SUMS        # shasum -a 256 -c on a Mac without coreutils
```

The tree list is not signed on its own. It exists so an extracted release can
report a corrupted file by name. The signed line over the archive is the
security boundary, and the two together cover both.

## Install with the bootstrap

The bootstrap is one script, run from a file:

```sh
curl -fsSLO https://<release host>/bootstrap.sh
bash bootstrap.sh --base-url https://<release host>/v0.3.1/
```

`<release host>` is supplied by whoever publishes the release. Nothing in this
repository names a host, and the script refuses any base URL that is not
`https`.

What it does, stopping at the first failure: fetches `release.json` for the
version, the platform's checksum list and its signature, and the archive
itself. It downloads the archive before it checks the signature, so a verifier
that re-hashes every file its list names finds that file in place. It then
verifies the signature with an already trusted `marula`, compares the archive's
hash against the signed line, unpacks it, checks the tree's own list, and
installs the binary into `~/.local/bin` (`--install-dir` moves it).

It cannot verify with the binary it just downloaded, and it does not pretend
to. It uses the `marula` already on `PATH` (`--verifier` names another). With
none present it stops and prints the by-hand command above. It also refuses to
run when its own source is a pipe, so `curl … | sh` fails rather than working.

## The offline bundle

`marula-<version>-offline-<os>-<arch>.tar.zst` is built for a Linux host that
cannot reach a registry. Inside:

```
marula-<version>-<os>-<arch>/   the release directory above
images.tar                      every image the compose file references
images.list                     their names
install-offline.sh              load the images, then `marula install --apply`
SHA256SUMS                      every file above
```

On the air-gapped host:

```sh
tar --zstd -xf marula-0.3.1-offline-linux-x64.tar.zst
cd marula-0.3.1-offline-linux-x64
sha256sum -c SHA256SUMS
./install-offline.sh --dir /srv/marula
```

The bundle pins `COMPOSE_PROJECT_NAME=marula` around both the save and the
install. Compose derives the image name of a service it builds from the project
name, so the pinned value is what makes the install find the loaded images
instead of deciding to build images it cannot build. Before it starts anything,
`install-offline.sh` checks that every name in `images.list` is loaded and
stops with the missing names if one is not.

Build one yourself with:

```sh
scripts/release/offline-bundle.sh --version 0.3.1 --os linux --arch x64
```

It reads the image list from `docker compose config --images` using
`deploy/dev.env` for interpolation only, so no resolved secret enters the
bundle. Pass `--skip-build` when the images are already in the local store.

## Where the secrets live

Every value below is an Actions secret on the repository. None is in the
working tree, and a pull request from a fork cannot read them.

| Secret | What it is |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64 of the Developer ID Application certificate and key, as `.p12`. |
| `APPLE_CERTIFICATE_PASSWORD` | The `.p12` password. |
| `APPLE_SIGNING_IDENTITY` | The identity string `codesign` should use. |
| `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | The Apple account that submits the notarisation, its app-specific password, and the team id. |
| `WINDOWS_CERTIFICATE` | Base64 of the Authenticode `.pfx`. |
| `WINDOWS_CERTIFICATE_PASSWORD` | The `.pfx` password. |
| `WINDOWS_TIMESTAMP_URL` | The timestamp endpoint your certificate authority publishes. Empty means no timestamp. |
| `MARULA_CHECKSUM_SIGNING_KEY` | The private half of the release SSH key. |
| `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The updater channel key. Not required today, because this release config does not emit updater artifacts. |

## Decisions worth knowing

**Bundle targets are chosen per platform.** The base `installer/tauri.conf.json`
asks for `deb`, and `tauri.macos.conf.json` asks for `app` and `dmg`, and
`tauri.windows.conf.json` asks for the NSIS setup. `"targets": "all"` was the
wrong answer: on Linux it asks for an AppImage that needs tooling an arm64
runner does not have, and on Windows it asks for an MSI that needs WiX. Adding
a target is a one-line edit in the platform file, and `scripts/release/assemble.mjs`
already copies any bundle directory it finds.

**There is no updater endpoint.** The installer ships no `plugins.updater`
block, so nothing claims to update itself. A release channel needs an endpoint
and a key, and inventing an endpoint that no host serves would be a promise
nothing keeps.

**`deploy/dev.env` is not in the bundle.** It marks a dev-posture stack and
holds the repository's published dev values. An install writes its own
`marula.env` from the plan, so shipping the dev marker would only invite a
deployment that trusts a public secret.

**The bundle carries all of `deploy/`.** The compose files mount
`pooler/pgbouncer.ini`, `garage.toml` and the observability configs by relative
path. Shipping the two compose files alone would produce a directory that
renders but cannot start.

## Rehearse it locally

The assembler needs a built binary and the built wizard, and it refuses by name
when either is missing:

```sh
pnpm --dir apps/installer build
cargo build --release -p marula-core-cli
node scripts/release/assemble.mjs --version 0.3.1 --os linux --arch x64
```

Its own checks run without any of that:

```sh
node scripts/release/assemble.mjs --self-test
bash -n scripts/release/*.sh
shellcheck -S warning scripts/release/*.sh
```
