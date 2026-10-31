# ADR 0028: The desktop client is a shell, and its trust roots are two

Date: 2026-10-31 · Status: Accepted · Lane: p61-desktop-wrapper-adr (W61-1)

## Context

W61-1 asks four questions: the wrapper, the updater and signing story, what a
window is, and what the client may hold offline. Three of the four were answered
by code that already ships, and the fourth was answered by accident when the
updater's public key in `desktop/tauri.conf.json` turned out to be a sentence
beginning `REPLACE_AT_RELEASE_TIME`. That is worth recording, because a
placeholder key looks like a decision until someone tries to verify against it.

ADR 0005 chose Tauri 2 for a desktop build that embeds Postgres and core-api in
one application. ADR 0027 chose one front-end with two hosts for the installer.
Neither covers the client that connects to a running instance, which is what
W61-2 builds, and neither says how that client updates itself.

The forces:

- The web application is the product. A second front-end written for the shell
  would drift, and the drift would be invisible until a screen behaved
  differently in one of the two.
- Two kinds of artifact leave this repository: installer bundles and release
  manifests, signed with an SSH key; and desktop update payloads, which Tauri
  verifies with a minisign key. They are not interchangeable, and one key cannot
  stand in for the other.
- A desktop client is expected to work when the network is not there, which
  invites it to cache more than it should.
- macOS and Windows builds ship unsigned for now (recorded in
  `docs/release.md`), and an unsigned build cannot produce a trusted update
  payload.

## Decision

### 1. One front-end, hosted. The shell adds a webview and native capability.

The client loads the same Next.js export the web deployment serves. It does not
contain a second implementation of any screen, and a feature that cannot be
expressed in that front-end is not a desktop feature. What the shell adds is
what a browser cannot do: registered windows, OS integration, and the two
capabilities documented below — printers and raw ZPL delivery.

That is the whole division. The shell is a host, and every screen in it is the
product's own.

### 2. Two trust roots, and neither may substitute for the other.

| Artifact | Signature | Public half | Private half |
|---|---|---|---|
| Release bundles, `SHA256SUMS`, `release.json` | OpenSSH `sshsig` | `release-keys/allowed_signers` | release secret store, `/release` |
| Desktop update payloads | minisign | `desktop/tauri.conf.json` → `plugins.updater.pubkey` | release secret store, `/release` |

Both private halves live in the same store and both are the maintainer's keys,
but the formats are fixed by the verifiers, not by us: `ssh-keygen -Y verify`
reads one, Tauri's updater reads the other. `scripts/check-desktop-trust.mjs`
refuses a placeholder, base64 that is not a minisign document, and a plaintext
update endpoint.

Rotation is not symmetric, and the asymmetry is the cost of this decision.
Rotating the SSH key invalidates old release verifications; rotating the
minisign key means clients older than the rotation stop accepting updates until
they are reinstalled, because the key they trust is the one compiled into the
build they already have. A rotation therefore ships a release first, then takes
effect.

### 3. A window is a role plus an index, and the platform owns its geometry.

`window_manager` keys windows by role and position and persists their geometry
per key, so reopening a ticket after a restart lands where it was. Deep links use
`marula://`, handled by `os_integrations`, and open or focus the matching window
rather than opening a second copy of it. This replaces the tab model: a tab is a
view, and the client is expected to hold several views of one workspace at once.

### 4. Offline holds the shell, never the data.

The client may keep the application shell and its static assets so it starts
without a network. It may not keep ticket content, attachments, or an offline
queue in a store of its own: the product's store is Postgres on the server, and
a second store in the client is a second truth. The embedded-Postgres build from
ADR 0005 is the exception that proves the rule, because there the database is
local and the client is the server.

Secrets the shell mints stay in files the operator can see and delete, as
`docs/desktop-release.md` documents: `desktop-secret` at 0600, and
`printers.json`, which is the allowlist the raw-ZPL command checks before it
opens a socket.

### 5. The update channel is a deployment fact.

The endpoint is a URL in configuration, not a promise compiled into the binary,
and the host that serves it is chosen when a release is cut. Until macOS and
Windows signing exists, those platforms ship without update payloads and the
updater reports that it cannot update them, rather than offering an artifact it
cannot verify.

## Consequences

- A desktop story that needs a screen the web application does not have becomes
  a web feature first. That is a constraint on the roadmap, accepted on purpose.
- Two keys, two formats, one rotation asymmetry. Any release procedure has to
  name which key it is using.
- The client starts offline and shows the shell, then the first request decides
  what it can do. There is no offline read path, and no partial-sync state to
  reconcile.
- The unsigned-platform decision puts a ceiling on the updater until signing
  exists. It is visible in `docs/release.md` rather than discovered at a release.

## Alternatives considered

**Electron.** A heavier runtime and a second Chromium to update, with no benefit
this product needs. ADR 0005 already ruled it out for the embedded build and the
reasoning holds here.

**A thin wrapper around a hosted browser view.** Cheapest to build, and it
cannot host a window manager, deep links into a native surface, or the printer
allowlist. It also puts the app's trust entirely in whatever the OS browser
does.

**One key for both artifacts.** Attractive for exactly as long as it takes to
look at the verifier: Tauri reads minisign, the release lane reads sshsig. A
single key would mean writing our own verifier for one of the two, which is more
code in the trust path than a second key is.
