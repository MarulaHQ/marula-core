---
description: "Install the Marula desktop client on macOS, Linux or Windows: where the download lives, what first run asks, and how to remove it again."
---

# Install the desktop app

> **Audience:** both — the app is for any staff member's workstation, and the
> connected-or-embedded choice at first run is the site operator's.

The desktop client is Marula in a desktop window: the menu bar, the tray, deep
links, and the product's own page loaded from your instance. Which instance,
and whether the app runs Marula itself, is decided once at first run and then
stored. If you are not sure this is the download you want,
[which installer do I want?](choose-your-installer.md) sorts the release page
out in one table.

## Where the download lives

GitHub Releases, published for the `desktop-v*` tags. Each release lists one
set per platform:

| Platform | Artifact | Architectures |
|---|---|---|
| macOS | a zip of `Marula.app` | arm64, x64 |
| Linux | `deb` and `AppImage` | x64, arm64 |
| Windows | NSIS setup `.exe` and `msi` | x64 |

## Install on macOS

1. Download the zip for your Mac. Apple silicon is arm64. Unzip it and drag
   `Marula.app` into Applications.
2. First launch is stopped by Gatekeeper, because the app is not notarized
   yet. Open it from System Settings, Privacy & Security, via Open Anyway.
   [Why](#unsigned-builds-and-what-the-os-says).

## Install on Linux

- The `deb` installs with your package manager, from the file you downloaded:
  `sudo apt install ./<file>.deb`.
- The AppImage needs no install. Make it executable and run it:
  `chmod +x <file>.AppImage && ./<file>.AppImage`.

## Install on Windows

- The NSIS setup is a normal installer: run the `.exe` and follow it.
- The `msi` installs by double-click, or from a terminal for a scripted
  install: `msiexec /i <file>.msi`.

## First run: connected or embedded

Launch the app. One window asks where this client should live.

- **An instance address** connects the window to your server, for example
  `https://psa.example.com`. The page then loads from that server, the data
  stays on it, and signing in is the same as signing in through a browser.
- **"Run Marula on this machine"** runs the product here: an embedded
  Postgres, core-api, and the bundled web console. No Docker, no server. The
  first embedded launch applies the migrations and creates no demo accounts,
  so you create your own.

The embedded choice exists only in the macOS build. The Linux and Windows
builds ask for an address and refuse `--local`, with a sentence that says why:
their embedded app needs a database bundled per platform, which is a named
follow-up ([ADR 0031](../../adr/0031-desktop-platform-matrix.md)).

You can settle the choice from a terminal instead of the dialog, and both
flags write it down so the next launch does not ask:

```sh
"/Applications/Marula.app/Contents/MacOS/marula-desktop" --server https://psa.example.com
"/Applications/Marula.app/Contents/MacOS/marula-desktop" --local     # macOS only
```

`MARULA_DESKTOP_SERVER` (a URL, or `local`) overrides the stored choice for
one launch. The connection record, what the shell refuses, and what a
connected page may and may not do on your machine are in
[the desktop release workflow document](../../desktop-release.md).

## Unsigned builds, and what the OS says

The bundles ship without certificates while the signing lane is in flight, so
expect one of these at install or first launch:

| Platform | What you see | What you do |
|---|---|---|
| macOS | Gatekeeper says it cannot check the app for malicious software | open it from System Settings, Privacy & Security, via Open Anyway |
| Windows | SmartScreen: "Windows protected your PC" | More info, then Run anyway |
| Linux | nothing | no signature is attached, and no check is offered beyond the download itself |

Platform signing (Developer ID and notarization on macOS, Authenticode on
Windows, and a signature on the Linux packages) is being wired into the
release pipeline. When it lands, each release carries its verification
instructions in the same shape as [verify a release](verify-a-release.md), and
this page links them. Nothing here promises a date.

## Uninstall

- **macOS.** `scripts/uninstall-desktop.sh` prints its plan first and removes
  nothing until you say `--go`. `--purge --go` also removes the data dir,
  which holds the embedded database, the install secret, the printer list and
  the saved connection. Removing the bundle alone keeps that data. The script
  refuses any bundle whose identifier is not `org.marula.desktop`, so a
  mistyped path cannot delete a different app.
- **Linux.** Remove the `deb` with your package manager, or delete the
  AppImage file. The same script runs on Linux, and `--purge` clears
  `~/.local/share/Marula`.
- **Windows.** Uninstall from Settings, Apps, or with `msiexec /x <file>.msi`.
  The Windows build is connected-only, so it holds no local database.
