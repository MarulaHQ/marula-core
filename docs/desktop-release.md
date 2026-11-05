# Desktop release workflow — packaging, connecting, updating, removing

Status: GA packaging (Wave 20 lane 1 `p20-ga-packaging`) · Shell: Tauri 2 (ADR 0005, ADR 0028)

This document is the operator workflow for the desktop client: point it at an
instance, cut a signed release, publish an update channel, and take it off a
machine. **No private key, password, or notarization credential is ever
committed** — everything below runs from local secrets or CI secrets injected as
environment variables.

## What's wired in the repo

| Piece | Where |
|---|---|
| Icons (multi-size .ico, .icns, .png) regenerated from `marula-ci/source/*.svg` | `scripts/generate-desktop-icons.sh` → `desktop/icons/` |
| Bundle config (targets `app`+`dmg`, category, copyright, min macOS 12, updater artifacts) | `desktop/tauri.conf.json` |
| Platform bundle targets (`deb`+`appimage`; `nsis`+`msi`) | `desktop/tauri.linux.conf.json`, `desktop/tauri.windows.conf.json` |
| Desktop release pipeline (macOS embedded, Linux/Windows connected) | `.github/workflows/release-desktop.yml` (unrun while F89 blocks CI) |
| Updater plugin (channel endpoint + pubkey) | `desktop/tauri.conf.json` `plugins.updater` + `desktop/src/main.rs` registration |
| Dev/test build (no bundle, `desktop` profile, shared target dir) | `scripts/build-desktop.sh` |
| First-run connect screen (the shell's own page, embedded) | `desktop/connect/` → `desktop/tauri.conf.json` `build.frontendDist` |
| Connection record and its validation | `desktop/src/connection.rs` → `connection.json` in the app data dir |
| Uninstall (app bundle, optionally the data dir) | `scripts/uninstall-desktop.sh` |

## 0. Platform matrix

| Platform | Mode | Bundle targets | Signing |
|---|---|---|---|
| macOS | embedded and connected | `app`, `dmg` | the Apple identity and notarization secrets when present; ad-hoc otherwise |
| Linux | connected | `deb`, `appimage` | unsigned |
| Windows | connected | `nsis`, `msi` | Authenticode when the certificate secret is present; unsigned otherwise, and SmartScreen will warn |

The Linux and Windows builds are connected clients: they ask for an instance and
refuse `--local`. Embedded Linux and Windows is a named follow-up (ADR 0031),
because it needs a per-OS Postgres sidecar and a decision on whether the
installer carries the database or downloads it.

## 1. Point the client at an instance

A desktop client either runs the app on this machine or connects to an instance
that already runs it. The choice is made once and stored, and the shell reads it
before it starts anything:

| Mode | What starts | Where the data is |
|---|---|---|
| **Connected** (ADR 0028) | The window, the menu, the tray, deep links, the update channel. No database, no API, no bundled server. | On the instance. The webview loads that instance's own URL, so the page and the API share an origin and the session cookie is first-party. |
| **Embedded** (ADR 0005; macOS only) | Everything above plus embedded Postgres, core-api in-process, and the bundled Next.js server on loopback ports. | `~/Library/Application Support/Marula/pgdata`. |

On the first run the shell asks, in one window: an instance address, or "Run
Marula on this machine". The second choice exists only in the macOS build; the
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

## 2. The signing keypair

Tauri's updater uses minisign signatures (`TAURI_SIGNING_PRIVATE_KEY`), a
different key from the SSH key that signs release artifacts. The pair already
exists:

| Half | Where |
|---|---|
| Public key | committed in `desktop/tauri.conf.json` → `plugins.updater.pubkey` |
| Private key | `~/.tauri/marula-desktop-updater.key` (0600), to be held in the release secret store |

Store the private key before the first desktop release, and never in the repo:

```sh
infisical secrets set TAURI_SIGNING_PRIVATE_KEY --env prod --path /release
# paste the file's contents at the prompt, not `$(cat …)`, so the key
# does not land in your shell history
```

Then delete the local copy once the secret store holds it. To rotate, generate a
new pair (`pnpm tauri signer generate -w ~/.tauri/marula-desktop-updater.key`),
commit the new public key, store the new private key, and say in the release
notes that clients older than the rotation will not accept updates until they
are reinstalled. `scripts/check-desktop-trust.mjs` refuses a placeholder, a key
that is not a minisign document, and a plaintext endpoint.

## 3. Build a signed bundle

```sh
export TAURI_SIGNING_PRIVATE_KEY=$(cat ~/.tauri/marula.key)
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
  shipping update artifacts nothing can verify: that is deliberate.

## 4. Publish the update channel

The updater endpoint in `tauri.conf.json` is:

```
https://updates.getmarula.com/desktop/{{target}}/{{arch}}/{{current_version}}
```

Serve a static JSON manifest there (any static host / object store works;
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

- Bump `version` in `desktop/tauri.conf.json` every release; the updater
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
the client should live; take "Run Marula on this machine" to exercise the
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

The desktop client is a **macOS** artifact. ADR 0005 chose the `.app` and the
bundler config asks for `app` + `dmg`; nothing in this repository builds a
Windows or Linux desktop bundle, and CI excludes `marula-desktop` from the Linux
clippy leg because the Tauri shell needs GTK headers that leg does not install
(`.github/workflows/ci.yml`). Adding Windows or Linux desktop bundles is a
change to ADR 0005, not a configuration file.

Reading a per-OS artifacts request as "desktop bundles for every platform" is
therefore a conflict with that ADR. This is the honest state of it:

| Deliverable | State |
|---|---|
| macOS install (`.app`, `.dmg`) | Configured in `desktop/tauri.conf.json`. Assembled locally by `scripts/build-desktop.sh` (no bundle, ad-hoc signed) or by `pnpm tauri build` (full bundle). |
| Windows / Linux desktop bundles | Not built, by ADR 0005. The per-OS bundle files that exist (`installer/tauri.macos.conf.json`, `installer/tauri.windows.conf.json`) belong to the **installer** app for a server, not to the client. |
| Installer bundles for a server (the `deb`/`dmg`/NSIS legs) | `release-installer.yml` builds five legs. macOS and Windows ship unsigned until the trust chain exists (`docs/release.md`). |
| Signed desktop artifacts (updater payloads, notarization) | **Blocked on the signing trust chain.** The key, the endpoint and the checker exist (`scripts/check-desktop-trust.mjs`, §2 and §4 above). No signed desktop release has been cut, and this document does not claim one has. |

Until that trust chain exists, the desktop client is installed from a build you
produce locally, and its updater verifies nothing that does not exist yet. A full
`pnpm tauri build` fails closed without `TAURI_SIGNING_PRIVATE_KEY`, which is the
loud outcome and the one to keep.

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
