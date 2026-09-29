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
https://updates.marula.app/desktop/{{target}}/{{arch}}/{{current_version}}
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
- The `updates.marula.app` host is the deployment-layer fact — point the
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
migrations and seeds the demo org (ADR 0005).
