# Desktop release workflow — packaging, connecting, updating, removing

Status: GA packaging (Wave 20 lane 1 `p20-ga-packaging`) · Shell: Tauri 2 (ADR 0005, ADR 0028) · Conditional signing: W61-4 (ADR 0031 decision 7)

This document is the operator workflow for the desktop client: point it at an
instance, cut a signed release, publish an update channel, and take it off a
machine. **No private key, password, or notarization credential is ever
committed** — everything below runs from local secrets or CI secrets injected as
environment variables.

## What a person installs

This runbook is the maintainer's. If you are the person installing the app, the
docs answer the shorter questions:

- Which installer do I want — <https://getmarula.com/docs/psa/v0.3/how-to/choose-your-installer>
- Install the desktop app — <https://getmarula.com/docs/psa/v0.3/how-to/install-the-desktop-app>

## What's wired in the repo

| Piece | Where |
|---|---|
| Icons (multi-size .ico, .icns, .png) regenerated from `marula-ci/source/*.svg` | `scripts/generate-desktop-icons.sh` → `desktop/icons/` |
| Bundle config (targets `app`+`dmg`, category, copyright, min macOS 12, updater artifacts) | `desktop/tauri.conf.json` |
| Platform bundle targets (`deb`+`appimage`, `nsis`+`msi`) | `desktop/tauri.linux.conf.json`, `desktop/tauri.windows.conf.json` |
| Desktop release pipeline (macOS embedded, Linux/Windows connected, conditional signing) | `.github/workflows/release-desktop.yml` (unrun; the first run is a manual dispatch on a branch) |
| Updater plugin (channel endpoint + pubkey) | `desktop/tauri.conf.json` `plugins.updater` + `desktop/src/main.rs` registration |
| Dev/test build (no bundle, `desktop` profile, shared target dir) | `scripts/build-desktop.sh` |
| First-run connect screen (the shell's own page, embedded) | `desktop/connect/` → `desktop/tauri.conf.json` `build.frontendDist` |
| Connection record and its validation | `desktop/src/connection.rs` → `connection.json` in the app data dir |
| Uninstall (app bundle, optionally the data dir) | `scripts/uninstall-desktop.sh` |

## 0. Platform matrix

| Platform | Mode | Bundle targets | Signing |
|---|---|---|---|
| macOS | embedded and connected | `app` (CI ships the `.app` zipped), `dmg` locally | Developer ID signature + notarization when the Apple secrets are set (`§2`); ad-hoc otherwise |
| Linux | connected | `deb`, `appimage` | the `.AppImage` carries the updater's minisign signature when `TAURI_SIGNING_PRIVATE_KEY` is set; the `deb` is checksum-only |
| Windows | connected | `nsis`, `msi` | Authenticode when `WINDOWS_CERTIFICATE` is set; unsigned otherwise, and SmartScreen will warn |

The Linux and Windows builds are connected clients: they ask for an instance and
refuse `--local`. Embedded Linux and Windows is a named follow-up (ADR 0031),
because it needs a per-OS Postgres sidecar and a decision on whether the
installer carries the database or downloads it.

**Signing is conditional on the secrets, and the posture is never silent.**
`release-desktop.yml` checks the secrets for the leg it is about to run. A
`workflow_dispatch` without them still builds: the bundler's updater artifacts
are switched off for that run (with the flag on and no key, `tauri build`
refuses to emit ones it cannot sign), and the job Summary states unsigned. A
`desktop-v*` tag is the opposite gate — a missing secret fails the leg and
names it, because a tagged release is not cut unsigned
(`release-installer.yml` runs the same rule). A bundle without a certificate
installs and runs; the OS adds a warning, and the warning is named here instead
of being discovered. What each posture produces and how a user verifies it is
§7.

## 1. Point the client at an instance

A desktop client either runs the app on this machine or connects to an instance
that already runs it. The choice is made once and stored, and the shell reads it
before it starts anything:

| Mode | What starts | Where the data is |
|---|---|---|
| **Connected** (ADR 0028) | The window, the menu, the tray, deep links, the update channel. No database, no API, no bundled server. | On the instance. The webview loads that instance's own URL, so the page and the API share an origin and the session cookie is first-party. |
| **Embedded** (ADR 0005, macOS only) | Everything above plus embedded Postgres, core-api in-process, and the bundled Next.js server on loopback ports. | `~/Library/Application Support/Marula/pgdata`. |

On the first run the shell asks, in one window: an instance address, or "Run
Marula on this machine". The second choice exists only in the macOS build. The
Linux and Windows builds ask for an address and refuse `--local` with a sentence
that says why (ADR 0031). Settle the same decision without a dialog when you
install by script:

```sh
"/Applications/Marula.app/Contents/MacOS/marula-desktop" --server https://psa.example.com
"/Applications/Marula.app/Contents/MacOS/marula-desktop" --local
```

Both flags write `connection.json`, so the next launch does not ask. The launch
also reads `MARULA_DESKTOP_SERVER` (a URL, or `local`) when you want one run to
override the stored choice without changing it.

`connection.json` holds an address and nothing else:

```json
{ "version": 1, "mode": "remote", "server_url": "https://psa.example.com" }
```

```json
{ "version": 1, "mode": "local" }
```

Rules the shell enforces, in `desktop/src/connection.rs`:

- http and https only. `file:`, `data:`, `javascript:` and a bare host are
  refused before anything loads.
- No username or password in the address, and no query or fragment.
- Trailing slashes are trimmed, so the address is stored once and compared
  once.
- A file it cannot read is reported on the connect screen and can be replaced
  there. It never falls back to a mode nobody chose.

**A connected page gets no app commands.** `desktop/capabilities/desktop-web.json`
grants raw-ZPL printing, the update pair and the native window commands to the
loopback origins this shell serves, as exact origins. An address a person typed
is not one of those, so the remote page can reach none of them. That is the
intended posture: a page served by an instance you do not control does not get
to open windows on your machine or send bytes to a printer. What still works is
everything that starts in Rust rather than in the page: the window, the menu,
the tray, a `marula://` deep link, window geometry and the update channel. The
tray's quick actions still open a role window at the instance, because they call
the window factory directly instead of asking the page's permission.

A hosted instance serves its own HTTPS certificate, so nothing extra is needed
there. A self-hosted instance behind a private CA needs that CA trusted by the
system, because the webview uses the platform trust store.

## 2. The signing keys and certificates

Four credentials sign a desktop release, and they are deliberately not one key
(ADR 0028): the updater channel and the OS install paths are separate trust
roots with separate lifetimes. Everything below lives in the repository's
Actions secrets on MarulaHQ/marula-core, never in the tree.

### The updater keypair (minisign)

Tauri's updater uses minisign signatures, a different key from the SSH key that
signs release artifacts:

| Half | Where |
|---|---|
| Public key | committed in `desktop/tauri.conf.json` → `plugins.updater.pubkey` |
| Private key | `~/.tauri/marula-desktop-updater.key` (0600), uploaded as `TAURI_SIGNING_PRIVATE_KEY` |
| Password | set at generation time, uploaded as `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (omit the secret if the key was generated with none) |

When `TAURI_SIGNING_PRIVATE_KEY` is set, the bundler signs the updater
artifacts and drops a `.sig` beside each bundle. When it is not, the workflow
switches `createUpdaterArtifacts` off for that run — with the flag left on and
no key, `tauri build` refuses to build at all, which is the right failure for a
local release build and an unusable one for a rehearsal. Rotation is not
symmetric (ADR 0028): clients older than the rotation stop accepting updates
until they are reinstalled, because the key they trust is the one compiled into
the build they already have. A rotation therefore ships a release first, then
takes effect: generate a new pair
(`pnpm tauri signer generate -w ~/.tauri/marula-desktop-updater.key`), commit
the new public key, store the new private half, and say in the release notes
which build is the last one the old key reaches.

### The certificates (per OS)

| Secret | What it holds | What it signs |
|---|---|---|
| `APPLE_CERTIFICATE` | base64 of the Developer ID Application `.p12` | the macOS `.app` (codesign) |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` password | — |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: <Org> (<TEAMID>)` | the identity codesign uses |
| `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | the Apple account that submits the notarisation, its app-specific password, and the team id | notarytool |
| `WINDOWS_CERTIFICATE` | base64 of the Authenticode `.pfx` | the NSIS and MSI bundles (signtool, via Tauri's bundler) |
| `WINDOWS_CERTIFICATE_PASSWORD` | the `.pfx` password | — |
| `WINDOWS_TIMESTAMP_URL` | the RFC 3161 endpoint your CA publishes; empty means no timestamp | — |

On macOS the workflow imports the `.p12` into a throwaway keychain (its
password is generated on the runner and never stored), re-signs the assembled
app with the Developer ID identity, submits the zip to `notarytool`, and
staples the ticket onto the `.app`. On Windows the `.pfx` is imported into the
runner's user store, and the thumbprint the import produces is what the bundler
is handed — never the certificate blob itself, which is a certificate, not a
hash of one. Local builds need none of these: `scripts/build-desktop.sh` stays
ad-hoc signed by design.

`scripts/check-desktop-trust.mjs` refuses a placeholder pubkey, a key that is
not a minisign document, a plaintext endpoint, and a workflow whose secrets or
whose unsigned posture have drifted from the list above.

## 3. Build a signed bundle

```sh
export TAURI_SIGNING_PRIVATE_KEY=$(cat ~/.tauri/marula-desktop-updater.key)
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD='…'
pnpm tauri build            # full bundler: .app + .dmg + updater artifacts (.tar.gz + .sig)
```

macOS distribution signing + notarization (App Store–outside distribution):

```sh
export APPLE_SIGNING_IDENTITY="Developer ID Application: <Your Org> (<TEAMID>)"
export APPLE_ID="you@example.com" APPLE_PASSWORD="app-specific-password" APPLE_TEAM_ID="<TEAMID>"
pnpm tauri build            # Tauri codesigns (--options runtime --timestamp) + submits for notarization
```

Notes:
- `scripts/build-desktop.sh` (the dev/test path) uses `tauri build --no-bundle`
  and stays ad-hoc signed — it never needs any of the secrets above.
- With `createUpdaterArtifacts: true`, a full `pnpm tauri build` needs
  `TAURI_SIGNING_PRIVATE_KEY`. Without it the bundle step fails rather than
  shipping update artifacts nothing can verify: that is deliberate. CI's
  unsigned rehearsal opts out of the flag explicitly (`release-desktop.yml`)
  instead of failing, and says so in the Summary.

## 4. Publish the update channel

The updater endpoint in `tauri.conf.json` is:

```
https://updates.getmarula.com/desktop/{{target}}/{{arch}}/{{current_version}}
```

Serve a static JSON manifest there (any static host or object store works, and
self-hosting is the doctrine). The Tauri CLI emits the artifact JSON for you
next to the bundle (`latest.json` shape). Minimum fields:

```json
{
  "version": "0.1.0",
  "platforms": {
    "darwin-aarch64": { "signature": "<contents of .sig>", "url": "https://…/Marula.app.tar.gz" }
  }
}
```

- Bump `version` in `desktop/tauri.conf.json` every release. The updater
  refuses downgrades and compares semver.
- Rotate keys by adding a second pubkey context in the release notes and
  re-issuing the manifest (minisign trust is config-side).
- The `updates.getmarula.com` host is the deployment-layer fact — point the
  endpoints array at whatever host actually serves the channel and commit
  that change with the release.

## 5. Verify before shipping

```sh
codesign --verify --deep --strict dist/Marula.app
spctl -a -vv dist/Marula.app
# updater artifact signature:
minisign -Vm Marula.app.tar.gz -P "$(jq -r .pubkey desktop/tauri.conf.json 2>/dev/null)" # or verify via tauri tooling
```

Launch smoke: open the built `.app` once on a clean machine profile
(`~/Library/Application Support/Marula` untouched). The first window asks where
the client should live. Take "Run Marula on this machine" to exercise the
embedded path. That run applies all migrations, mints `desktop-secret`, and
seeds **no** demo org: a release build mounts no `/api/dev/*` route and creates
no demo account (ADR 0005, amendment 2026-10-25). Create the account you want to
sign in with.

## 6. Local state the shell mints

Three files under `~/Library/Application Support/Marula` are part of the
security posture, not scratch:

| File | Written by | What it is |
|---|---|---|
| `connection.json` | the connect screen, `--server`/`--local`, or `MARULA_DESKTOP_SERVER` | Which mode this client runs in, and the instance address when it is connected. Holds no credential. Delete it and the next launch asks again. |
| `desktop-secret` (0600) | first launch of a **release** build | The per-install loopback secret. The in-process API signs tokens with it, derives its at-rest seal key from it, and derives the three embedded-Postgres passwords from it. Delete it and the next launch mints a new one, which cannot open sealed rows written under the old key. A **debug** build never creates it: it keeps the repository's published dev secret and Postgres defaults so demo data and compose parity hold. |
| `printers.json` | first launch of the **embedded** app, as an empty template | The printers the raw-ZPL command may send to. The web app may only name an entry in this file. The socket goes to the configured address, so a compromised page cannot use the shell as a TCP write primitive. An empty list means raw ZPL is unavailable and the browser print dialog is the only target. It is inert in connected mode, because no granted origin can call the printing command. |

Which build exposes what, after Wave 55:

- **Debug** (`cargo run`, `cargo test`, the plain `dev` profile): the API trusts
  the published dev JWT secret and the embedded Postgres keeps
  `postgres`/`marula`/`marula_app`. Any local process can forge an owner token
  or connect as superuser. It is the whole local test app: `/api/dev/*` is
  mounted and the demo org is seeded, so `owner@demo.local` and the rest of the
  `demo-password-123` logins work. This is deliberate and logged at boot.
- **Release** (`desktop` profile, `scripts/build-desktop.sh`, the bundler):
  no credential from this repository is in force, no `/api/dev/*` route is
  mounted, and no demo account is seeded. A local process cannot mint a token
  or connect to Postgres with a known password, and the app carries no dev
  surface for anyone who installs it. `Config::from_local` derives the dev-tools
  flag from the build profile, so the routes and the seed follow the same
  decision (ADR 0005, amendment 2026-10-25).

## 7. Per-OS artifacts, and what ships signed

The client ships on three platforms (ADR 0031). macOS carries both modes. Linux
and Windows carry the connected client, and their bundle targets live in
`desktop/tauri.linux.conf.json` (`deb`, `appimage`) and
`desktop/tauri.windows.conf.json` (`nsis`, `msi`). CI still excludes
`marula-desktop` from the Linux clippy leg because the shell needs GTK headers
that leg does not install (`.github/workflows/ci.yml`).

| Deliverable | State |
|---|---|
| macOS install (`.app`, zipped) | Configured in `desktop/tauri.conf.json` and assembled by hand in CI, because the Tauri bundler does not carry the embedded resource tree. Developer ID-signed and notarized when the Apple secrets exist; ad-hoc signed otherwise. Ships no updater payload: updater artifacts come from the bundler, and the macOS leg builds without it — that arrives with the bundler migration, not with this signing wiring. |
| Linux desktop bundles (`deb`, `appimage`) | Configured in `desktop/tauri.linux.conf.json`. The `.AppImage` carries the updater's minisign signature when the updater key exists; the `deb` is checksum-only until an apt repository publishes (the reason is below). |
| Windows desktop bundles (`nsis`, `msi`) | Configured in `desktop/tauri.windows.conf.json`. Authenticode-signed when `WINDOWS_CERTIFICATE` exists; unsigned otherwise, and SmartScreen will warn. |
| Updater artifacts (`.sig` files) | Signed with the minisign updater key when `TAURI_SIGNING_PRIVATE_KEY` exists, and collected beside the bundles. Without the key the run emits none and says so. |
| Installer bundles for a server (the `deb`/`dmg`/NSIS legs) | `release-installer.yml` builds five legs. These belong to the installer app, not to the client (`docs/release.md`). |

**What a dispatch produces.** With every secret set in place: macOS signed and
notarized, the Linux AppImage minisign-signed, Windows Authenticode-signed, and
`.sig` files beside every bundler-emitted artifact. With none: the same
bundles, an ad-hoc macOS app, no `.sig` files, and a job Summary whose lines
say exactly that. The Summary is the posture statement — read it before
reading a bundle to learn what was signed.

**Why the AppImage carries the signature, not the deb.** CI produces a
standalone deb and an AppImage attached to a workflow run; it publishes no apt
repository. Repository signing (`debsign`, `reprepro`) signs repository
metadata: the `Release` files a person's apt checks with `Signed-By`. Signing
metadata for a repository nobody serves is theater. The AppImage is
the artifact the updater itself verifies, its `.sig` is minisign-format that
`minisign` verifies (the tool is packaged in the Debian, Ubuntu and Fedora
archives), and the public half is already committed in this repository. The
deb's honest story is checksum plus dpkg's install-time integrity check until
a repository exists, and when one does, its `Release` signing belongs to the
deployment that serves it, not to this build.

### Verifying a downloaded bundle

macOS:

```sh
codesign --verify --deep --strict Marula.app
spctl -a -vv Marula.app          # a notarized build answers "source=Notarized Developer ID"
```

Windows (PowerShell):

```powershell
Get-AuthenticodeSignature .\Marula-setup.exe | Format-List
# Status: Valid, and the signer is the certificate owner you expect. An
# unsigned installer shows NotSigned here and a SmartScreen warning at install.
```

Linux, the AppImage — the public half is the `plugins.updater.pubkey` field of
`desktop/tauri.conf.json`, base64 of a minisign key document:

```sh
jq -r '.plugins.updater.pubkey' desktop/tauri.conf.json | base64 -d > marula-updater.pub
minisign -Vm Marula_*.AppImage -p marula-updater.pub
```

Linux, the deb: compare its sha256 against the checksums on the workflow run.
`dpkg` checks archive integrity at install time, not origin; the signed origin
arrives with the apt repository, not before.

Unsigned bundles are the rehearsal posture, and the job Summary names them.
The updater key in `desktop/tauri.conf.json` is independent of the OS signing
certificate: a bundle can verify updates while the OS still warns about the
installer. `pnpm tauri build` fails closed without
`TAURI_SIGNING_PRIVATE_KEY`, which is the loud outcome and the one to keep.

## 8. Uninstall

An Applications folder is not the whole install: the app data dir outlives a
bundle dragged to the trash, and in embedded mode it holds a database.

```sh
scripts/uninstall-desktop.sh                 # print the plan, remove nothing
scripts/uninstall-desktop.sh --go            # remove the app bundle
scripts/uninstall-desktop.sh --purge --go    # and the data dir
scripts/uninstall-desktop.sh --app /path/to/Marula.app --go
```

The script removes a bundle only when its `CFBundleIdentifier` is
`org.marula.desktop`, so a typo in a path cannot delete a different app. The data
dir goes only with `--purge`, after the script names what is inside it. On an
embedded install that removes `pgdata` and `desktop-secret`, and sealed rows
written under that secret cannot be opened by a later install. On a connected
client the only local state is `connection.json`, which is usually worth keeping
if you are about to reinstall.

## 9. Go-live checklist

The signing is wired and unproven: no leg of this workflow has ever run, with
or without secrets. The order below is the one the workflow's gates expect.
None of it is an agent's to execute — uploading secrets and firing dispatches
are the maintainer's calls.

### 9.1 The free interim (until the certificates are bought)

Every certificate below costs money; the updater key does not. The interim
posture is therefore: sign the updater payloads now, leave macOS ad-hoc, and
optionally self-sign Windows. Steps 1 and 4 below are the whole interim.

1. Upload `TAURI_SIGNING_PRIVATE_KEY` (+ password) — step 1 below. With it in
   place, every leg's updater payload carries a verifiable minisign signature
   and the AppImage ships with a `.sig` a user checks with `minisign -V`
   against the public half committed in `desktop/tauri.conf.json`. That is
   tamper-evidence, not a trusted-install promise, and it is the part that
   does not expire when a subscription lapses.
2. Windows, optionally: a self-signed Authenticode PFX costs nothing
   (`New-SelfSignedCertificate -Type CodeSigningCert` in PowerShell, exported
   as PFX) and passes through the same import step as a paid one — the
   workflow reads `WINDOWS_CERTIFICATE` as base64 PFX and does not care who
   issued it. Set only `WINDOWS_CERTIFICATE` and
   `WINDOWS_CERTIFICATE_PASSWORD`; leave `WINDOWS_TIMESTAMP_URL` unset, since
   a timestamp server would be a third party and a self-signed chain has none.
   Be honest about what it buys: the bundles stop being *unsigned*, but a
   machine that has not imported the certificate still shows "Unknown
   publisher". An org that wants a quiet install imports the exported `.cer`
   into its Trusted People store, which is a real chain you control; a
   stranger's machine sees the same warning an unsigned bundle gets.
3. macOS: no interim. A self-signed Apple-style certificate imports and
   codesigns, but it cannot be notarized, so Gatekeeper shows a user the same
   warning the ad-hoc fallback already produces. The ad-hoc path the workflow
   already takes when the Apple secrets are absent is the interim.
4. State the posture in the release notes per §7, per platform, every time:
   AppImage signed (minisign), Windows self-signed or unsigned, macOS ad-hoc.

### 9.2 The full run

1. Upload the updater key to the repository's Actions secrets:
   `TAURI_SIGNING_PRIVATE_KEY` (the contents of
   `~/.tauri/marula-desktop-updater.key`, pasted at the prompt, not `$(cat …)`,
   so the key stays out of your shell history) and
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if the key was generated with one.
   Keep the local copy until step 5 succeeds, then delete it.
2. Obtain and upload the Apple identity: `APPLE_CERTIFICATE`,
   `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`,
   `APPLE_PASSWORD`, `APPLE_TEAM_ID` (§2).
3. Obtain and upload the Windows certificate: `WINDOWS_CERTIFICATE`,
   `WINDOWS_CERTIFICATE_PASSWORD`, and `WINDOWS_TIMESTAMP_URL` if the CA
   publishes one (§2).
4. Fire one manual `workflow_dispatch` on a branch — the workflow's first run
   ever, and the rehearsal. A missing secret makes a leg go unsigned with a
   notice naming the gap; fix the secret and dispatch again.
5. Verify the artifacts that run produced, per §7: `spctl` on macOS,
   `Get-AuthenticodeSignature` on Windows, `minisign -V` on the AppImage, and
   the Summary's posture lines against what you actually downloaded.
6. Only then tag `desktop-v*` — its gate fails closed if a secret has gone
   missing since the rehearsal.
