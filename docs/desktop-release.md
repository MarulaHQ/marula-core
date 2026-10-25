# Desktop release workflow — packaging, signing, auto-update

Status: GA packaging (Wave 20 lane 1 `p20-ga-packaging`) · Shell: Tauri 2 (ADR 0005)

This document is the operator workflow for cutting a signed, auto-updating
Marula desktop release. **No private key, password, or notarization
credential is ever committed** — everything below runs from local secrets or
CI secrets injected as environment variables.

## What's wired in the repo

| Piece | Where |
|---|---|
| Icons (multi-size .ico, .icns, .png) regenerated from `marula-ci/source/*.svg` | `scripts/generate-desktop-icons.sh` → `desktop/icons/` |
| Bundle config (targets `app`+`dmg`, category, copyright, min macOS 12, updater artifacts) | `desktop/tauri.conf.json` |
| Updater plugin (channel endpoint + pubkey) | `desktop/tauri.conf.json` `plugins.updater` + `desktop/src/main.rs` registration |
| Dev/test build (no bundle, `desktop` profile, shared target dir) | `scripts/build-desktop.sh` |

## 1. One-time: generate the signing keypair

Tauri's updater uses minisign signatures (`TAURI_SIGNING_PRIVATE_KEY`).

```sh
pnpm tauri signer generate ~/.tauri/marula.key
```

- Keep the **private key** + its password in your password manager / CI
  secrets (`TAURI_SIGNING_PRIVATE_KEY` = file contents or path,
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`).
- Paste the printed **public key** into `desktop/tauri.conf.json`
  → `plugins.updater.pubkey`, replacing the `REPLACE_AT_RELEASE_TIME`
  placeholder. This is the ONLY key material that is committed.

## 2. Build a signed bundle

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
- With `createUpdaterArtifacts: true` and a placeholder/missing key, a full
  `pnpm tauri build` fails loudly instead of shipping unsigned update
  artifacts — that is deliberate.

## 3. Publish the update channel

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

## 4. Verify before shipping

```sh
codesign --verify --deep --strict dist/Marula.app
spctl -a -vv dist/Marula.app
# updater artifact signature:
minisign -Vm Marula.app.tar.gz -P "$(jq -r .pubkey desktop/tauri.conf.json 2>/dev/null)" # or verify via tauri tooling
```

Launch smoke: open the built `.app` once on a clean machine profile
(`~/Library/Application Support/Marula` untouched) — first run applies all
migrations, mints `desktop-secret`, and seeds **no** demo org: a release build
mounts no `/api/dev/*` route and creates no demo account (ADR 0005, amendment
2026-10-25). Create the account you want to sign in with.

## 5. Local state the shell mints

Two files under `~/Library/Application Support/Marula` are part of the
security posture, not scratch:

| File | Written by | What it is |
|---|---|---|
| `desktop-secret` (0600) | first launch of a **release** build | The per-install loopback secret. The in-process API signs tokens with it, derives its at-rest seal key from it, and derives the three embedded-Postgres passwords from it. Delete it and the next launch mints a new one, which cannot open sealed rows written under the old key. A **debug** build never creates it: it keeps the repository's published dev secret and Postgres defaults so demo data and compose parity hold. |
| `printers.json` | first launch, as an empty template | The printers the raw-ZPL command may send to. The web app may only name an entry in this file. The socket goes to the configured address, so a compromised page cannot use the shell as a TCP write primitive. An empty list means raw ZPL is unavailable and the browser print dialog is the only target. |

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
