# ADR 0031: Desktop platform matrix — connected everywhere, embedded on macOS

Date: 2026-11-05 · Status: Accepted · Ruling: user directive, W61-2

## Context

ADR 0005 built the desktop client as a macOS app that runs the whole product on
one machine. ADR 0028 then gave the shell two modes: embedded (that app) and
connected (the same window pointed at an instance). The connected mode made the
shell independent of the platform's ability to host Postgres, but the bundle
config still named only `app` and `dmg`, and the code chose the mode without
asking whether the build could run it.

The user asked for Linux and Windows desktop installers. That is a product
decision, and it forces one question: does each platform ship the embedded app
too, or only the connected client?

## Decision

1. **The desktop client ships on macOS, Linux and Windows.**
2. **Linux and Windows ship the connected client.** The first run asks for an
   instance address. The embedded app is not offered, `--local` is refused with
   a sentence that says why, and a stored `local` choice from another machine is
   answered by the connect screen rather than by a half-started boot.
3. **macOS ships both modes**, unchanged from ADR 0005 and ADR 0028.
4. **Embedded parity on Linux and Windows is a named follow-up, not a hidden
   gap.** It needs a per-OS Postgres sidecar, the Next standalone tree staged
   into the bundle, and a decision on whether an installer downloads the database
   or carries it. That is a lane, not a config flag.
5. **Bundle targets are per platform.** macOS keeps `app` and `dmg`. Linux
   produces `deb` and `appimage`. Windows produces `nsis` and `msi`, through
   `desktop/tauri.linux.conf.json` and `desktop/tauri.windows.conf.json`.
6. **Connected windows still hold no app commands.** The origin a person types
   is not in `desktop-web.json`, so raw-ZPL printing and the native window
   commands stay with the loopback origins the shell serves (ADR 0028). The menu,
   tray, deep links, geometry and the update channel work on every platform.

## Consequences

- A Linux or Windows user gets the product, not a second backend. The client
  adds what a browser cannot: a window, a tray, deep links and updates.
- The Linux and Windows binaries still compile the embedded dependencies because
  they share the crate. They never boot them. Splitting the crate by feature is
  a clean-up, not a requirement, and is left for the embedded-parity lane.
- Signing is per platform and is not solved here. macOS uses the existing
  identity and notarization secrets when present. Windows needs an Authenticode
  certificate for a quiet install, and ships unsigned with the SmartScreen cost
  stated when the secret is absent. Linux bundles are unsigned.
- The release workflow `release-desktop.yml` is written and has not run: CI is
  blocked on billing (F89). Its first run must be a manual dispatch on a branch,
  not a tag.
- ADR 0005's title still says "local desktop test app". Its decision now
  describes the macOS mode of a cross-platform client.
