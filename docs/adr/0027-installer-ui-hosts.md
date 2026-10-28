# ADR 0027: The installer ships as an app, and keeps a loopback fallback

Date: 2026-10-28 · Status: Accepted · Lane: p60-install-gui (W60-3)

## Context

W60-3's row described the wizard as a page on `127.0.0.1` behind a one-time
token. That is a good fallback and a poor primary: the first thing a person
meets when they install a self-hosted product should not be a browser tab and a
token URL, and some hosts — a minimal server, a locked-down workstation, a box
whose operator works over SSH — have no browser to open it in.

The wave also has constraints that outlive any one screen. The installer runs
before the product exists on the machine, so it cannot be a mode of the product.
The engine already exists in the core CLI (W60-2): host facts, a plan, an apply
and a doctor, with every explanation taken from the descriptor. The design system
is one token vocabulary, and the palette has exactly one file it may come from.
The wizard may not phone home, and it has to work with the network down once the
images are pulled. W60-4 then has to build, sign and ship whatever this lane
decides.

On 2026-10-28 the maintainer directed that the primary form is a proper
application for Linux, Windows and macOS, with the browser version kept as a
fallback.

## Decision

### 1. One front-end, two hosts, no third copy of a decision

The screens live once, in `apps/installer` (Vite, React, TypeScript). They are
loaded by two hosts:

- `installer/` — a Tauri v2 app, the primary. It loads the same bundle into a
  native window and reaches the engine over IPC.
- `marula install --ui` — the fallback. It serves the same bundle from
  `127.0.0.1` and reaches the engine over loopback HTTP.

Every payload either host sends comes from `install::view::Session` in the core
CLI. The front-end holds presentation rules only: which floors a host misses,
which source a value came from, how a parameter search ranks. A host that needed
a rule the other did not would be the beginning of two wizards, and the transport
adapter in `apps/installer/src/api.ts` is the only code that knows which host is
running.

### 2. Tauri, on the precedent the repository already has

`desktop/` is a Tauri v2 shell with its own capabilities file, and the product's
tokens are plain CSS custom properties, so a webview renders them unchanged.

Not Electron: a second Chromium, a larger download, and update machinery that
duplicates W60-5. Not egui or iced: a second design system, no reuse of the token
pipeline, and a keyboard-and-screen-reader story that would be written from
scratch for a four-screen flow. Not three native UIs: three codebases for the
same four decisions, with three chances to disagree about a floor.

### 3. The boundary between the window and the engine is an API

The front-end renders text the descriptor and the machine supply. So the IPC
surface is typed, not a script bridge: each command takes a struct, the engine
refuses a lever the descriptor does not publish, and secrets are masked in the
view layer — a value marked secret is replaced before it can be serialised, not
when it is drawn. `installer_open` accepts `http` and `https` only, because
handing the operating system an arbitrary URL scheme from a page is how a local
document opens a local handler.

### 4. The fallback keeps the security posture its row asked for

`marula install --ui` binds `127.0.0.1` on a port the OS chooses, prints one URL
with a 32-byte token, and redeems that token exactly once for an `HttpOnly`,
`SameSite=Strict` cookie. Every `/api` call needs that cookie and an
`x-marula-installer: 1` header, so a cross-site form post cannot drive an
install. A `Host` header that is not loopback is refused, so a DNS name pointing
at the address does not reach it. Static files are served from one directory with
traversal refused, behind a CSP that permits nothing off the machine. The process
makes no outbound call.

The cookie is not `Secure`: the origin is `http://127.0.0.1`, and a `Secure`
cookie would never be sent to it. The token, not transport encryption, is what
guards the session, and the token exists only in the terminal and the URL.

### 5. A headless box is a supported place to run this

If no browser can be opened, the fallback prints the URL and carries on. A
server with no `xdg-open` is not a failed install. The app defaults to
`~/marula` for its files and looks for `deploy/docker-compose.yml` beside its
own executable, so a double-clicked bundle has a sensible target without
arguments; both are overridable through the environment for a packager or a
test.

### 6. W60-4 owns the artifact, and this ADR says what it must carry

Per-OS bundles, signing, notarisation and the offline bundle belong to the
release lane. This decision constrains it: the release ships
`apps/installer/dist` and the compose files beside the binary, and the fallback's
`--ui-dir` points at that same directory. Signing identities stay unset in the
committed configuration, as they are in `desktop/tauri.conf.json`, because a
committed identity is a committed guess.

## Consequences

- Two hosts to keep in step. The mitigation is structural: both call `Session`,
  and the shared `PlanView`/`StateView` types mean a renamed field is a compile
  error on the Rust side and a typecheck error on the TypeScript side.
- The app adds a webview dependency per platform, which is slower to build
  first time and needs CI caching. That cost is paid once per machine.
- Every future endpoint is written twice (a command and a route) but implemented
  once. If a third host appears, the view layer is already the seam.
- No remote asset can ever be added to the wizard without changing its CSP and
  breaking its offline promise, which is the point.

## Alternatives rejected

| Option | Why not |
|---|---|
| Browser wizard only (the row's original wording) | A browser tab and a token URL is a weak first run, and headless hosts have no browser at all. Kept as the fallback. |
| The wizard as a first-run mode of the product | The product is what is being installed; it cannot host the installer. |
| A terminal UI | Fast for experts, but the row's job is consequences and recommendations for operators who are not experts, and a 180-lever catalogue is not a TUI surface. |
| The engine compiled to WebAssembly so the page owns it | The engine runs Docker and reads the host; a browser cannot. |
