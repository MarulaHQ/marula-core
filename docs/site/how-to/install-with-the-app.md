---
description: "Walk the installer app's five screens: measure the machine, pick a shape, read the seven decisions, then review the plan before anything is written."
---

# Install with the app

> **Audience:** owner or site operator — you are installing on the machine you
> sit in front of, and you would rather click than type.

The app is a window, not a second product. It loads the same wizard the terminal
can serve and calls the same engine, so a plan made here and a plan made with
`marula install` are the same plan. What it adds is a way to compare options
before committing to them.

Download the build for your platform, open it, and the first screen says what it
is: **This wizard will guide you through the setup. You can choose between
recommended settings to get you up and running quickly, or developer mode which
will give you all the options.** The artwork behind that screen is one of the
brand's four textures, picked at launch.

The app sends nothing off the machine and has no telemetry. It reads the
descriptor that ships inside it and measures the host.

## The five screens

1. **How.** Recommended or developer mode. This decides how many questions you
   are asked, not what the product does.
2. **This machine.** Cores, memory, free disk at the install path, whether
   Docker answers, and a GPU if one is visible. The recommendation names the
   numbers it was chosen on, so you can disagree with it.
3. **Shape.** Every shape the build ships, each floor marked against this host.
   What each one runs is one disclosure away rather than in the reading path.
   The demo tier is here too, and is never marked recommended: its sign-in is
   published and its reset surface stays on loopback. See
   [deployment shapes](../reference/deployment-shapes.md).
4. **Settings.** Seven decisions in recommended mode, the whole catalogue in
   developer mode.
5. **Review.** The plan as steps, then the config it writes, then what stays
   yours to do. **Apply and start** runs it; **Check the result** runs the
   doctor against what came up.

The rail at the top shows where you are and lets you go back. The theme control
sits at the right: system, light or dark.

## Recommended mode

Recommended asks seven questions and keeps the default answer everywhere else.
Each default carries the sentence that explains why it is the default, so a
kept default is a decision you made rather than one made for you.

The shapes offered in this mode are only the ones this machine can carry. If
that leaves nothing, the screen says which floor each shape misses instead of
offering a shape that will not work.

## Developer mode

Developer mode shows all 180 settings, grouped the way the descriptor groups them
and searchable by name or by what it changes. `⌘K` opens the catalogue from any
screen, in either mode. Each field shows its default, the consequence of moving
it, and what it needs or cannot be combined with.

Developer mode also lets you plan a shape this machine cannot carry. The card
repeats the floors it misses and explains what the shape runs as separate
containers; **Open split anyway** takes you into its settings. The review then
carries the same numbers at the top, because an install that cleared the floors
quietly is an install that lies about the box. The stack can still run out of
memory or disk once it carries real work.

From a terminal the same override is `--allow-unfit`, on
[the CLI path](install-on-a-server.md).

## The settings screen

Each row is one decision: a plain name, the descriptor's lever name underneath,
and the value. Changing a field shows what changes because of it, which is the
sentence the descriptor carries rather than one the wizard invented.

Fields with a closed set are selects, so you choose between the values the
process actually parses. Anything else is free text, and a value outside the set
the process accepts is refused before the plan is built.

A lever that depends on another says so on the row. One that conflicts with
another says that too, so you find out here rather than at boot.

## Review, then apply

The review is the plan, not a summary of it. Each step names what it touches,
why it exists, and the descriptor field that justifies it. Below that is the
config it writes, grouped by what each value configures, with the exact setting
name under the label.

**Apply and start** writes the files, starts the stack and waits for the health
checks. The progress bar counts steps, so a step that fails is visible where it
happened rather than as a stack that never came up.

Afterwards **Check the result** runs the doctor: host floors, the Docker daemon,
the boot-critical values, the ports, the embedding model. A failed check names
what it looked at.

## Where the files go

The app installs into `~/marula`. Set `MARULA_INSTALL_DIR` before launching it to
install somewhere else; the review screen always names the directory it will
write to.

It writes `marula.env` and a restart file. A registry install also gets an images
file that pins each service by digest. Nothing else on the machine is touched.

## Installing on a machine with no desktop

`marula install --ui` serves the same wizard over loopback and opens a browser:

```sh
marula install --ui --no-open --port 8899
```

The printed URL carries a one-time token. Opening it sets an `HttpOnly` session
cookie and the token stops working. The server binds `127.0.0.1` only, refuses a
`Host` header that is not loopback, and requires an installer header on every API
call, so another page in the same browser cannot drive it.

The screens, the plan and the apply are the same. Only the transport differs:
the app talks to the engine over IPC, the fallback over HTTP.

Once the install lands, the workstations want the desktop client pointed at
this instance. Its page is [install the desktop app](install-the-desktop-app.md),
and [which installer do I want?](choose-your-installer.md) sorts every download
the release pages carry.
