---
description: "Match what you want to the artifact: a server install, the desktop client for workstations, or Marula running on one machine with no server."
---

# Which installer do I want?

> **Audience:** site operator — you are on a release page with several
> downloads and need to know which one is yours.

Marula installs in two directions. A **server install** puts the product on one
machine, and a team reaches it over the network from browsers. The **desktop
client** puts Marula in a window on a workstation, connected to a server. The
macOS client can also run the product by itself on one machine, with no server
anywhere. Everything on the release page is one of those.

## The decision table

| You want | On | Get |
|---|---|---|
| A server a team signs into | a host with Docker | the `marula` CLI or the installer app, from a `v*.*.*` release |
| Marula on staff workstations, connected to your server | macOS, Linux, Windows | the desktop client, from a `desktop-v*` release |
| Marula on one machine, with no server | macOS | the same desktop client. At first run choose "Run Marula on this machine" |
| A look before you commit anything | your own machine | [the local demo stack](local-demo-stack.md), or the embedded desktop app in the row above |

## The server: three ways in

One engine, three entrances. All three measure the machine, print the same
plan, and write the same `marula.env`. The difference is only how you drive them.

| Way in | What it suits | The page |
|---|---|---|
| The Marula Installer app | the machine you sit in front of | [install with the app](install-with-the-app.md) |
| `marula install --ui` | the same wizard served over loopback, for a host you reach over SSH | [install with the app](install-with-the-app.md#installing-on-a-machine-with-no-desktop) |
| `marula install` | a terminal, a script, or a runbook | [install on a server](install-on-a-server.md) |

A `v*.*.*` release carries the CLI archives and the installer app builds, each
for the platforms it installs on. Check what you downloaded before you run it:
[verify a release](verify-a-release.md).

## The desktop client

One client, published for three platforms in the `desktop-v*` releases. It is
a window on your workstation that loads your server's own page, so the data
stays on the server and the workstation holds little more than the address.

| Platform | Formats | First run asks |
|---|---|---|
| macOS | `.app` in a zip | an instance address, or "Run Marula on this machine" |
| Linux | `deb`, `AppImage` | an instance address; `--local` is refused |
| Windows | NSIS setup, `msi` | an instance address; `--local` is refused |

Install steps, the first-run choice and the uninstall path are on
[install the desktop app](install-the-desktop-app.md).

## "Run Marula on this machine" means the whole product

The macOS build can be the whole product. Choose that sentence at first run and
the app starts an embedded Postgres, runs core-api, and serves the bundled web
console. Nothing else is needed: no Docker, no server, no second machine. The
data lives in `~/Library/Application Support/Marula`.

The first-run screen offers the two choices as one decision. An instance
address connects this window to a server. "Run Marula on this machine" runs
everything here, and stores that answer so the next launch does not ask. The
Linux and Windows builds offer the address only, because their embedded app
needs a database bundled per platform, which is a named follow-up
([ADR 0031](../../adr/0031-desktop-platform-matrix.md)).

## What the OS says about an unsigned build

Bundles ship unsigned while the signing lane is in flight. A warning does not
mean the download failed. It means the bundle carries no certificate yet. Each
platform says so its own way:

| Platform | What you see | What it means |
|---|---|---|
| macOS | Gatekeeper says it cannot check the app for malicious software | the app is signed ad-hoc, with no notarization. Open it from System Settings, Privacy & Security, via Open Anyway |
| Windows | SmartScreen: "Windows protected your PC" | the bundle has no Authenticode certificate. More info, then Run anyway |
| Linux | nothing | the `deb` and the AppImage are unsigned; the checksum list is the check that remains |

That checksum list, and the signature over it, is what a careful installer
reads on any platform: [verify a release](verify-a-release.md). Platform
signing is being wired into the release pipeline now, and when a signed
release ships, its verification instructions come with it. Nothing on this
page promises a date.
