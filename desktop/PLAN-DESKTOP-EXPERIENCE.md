# Desktop experience — where the shell is, and what is left

Status: plan · Row W61-3 (`p61-desktop-experience`) · Written 2026-11-13 · Trackers: `desktop/`, `docs/adr/0028`, `docs/adr/0031`

The row asks for "the multi-screen experience: real windows rather than tabs,
deep links (`marula://ticket/...`), native notifications, per-window session, and
the same density and keyboard model as the web app."

Much of that is already in the tree. This file separates what is built from what
is not, so a lane can take one slice instead of reading the whole shell. Every
claim below cites the code that holds it.

## What ships today

| Piece | Where it lives | What it does |
|---|---|---|
| Real OS windows, no tabs | `desktop/src/window_manager.rs:226-360`, `desktop/src/windows.rs:29-36` | Six roles (`ticket-workspace`, `ticket-detail`, `dispatch-board`, `stock-lookup`, `decisions`, `merge-workbench`) each open a `WebviewWindow` labelled `subwindow::<role>::<n>`. Native tabbing is deliberately off (`desktop/src/main.rs:1124-1127`, `desktop/src/os_integrations.rs:253-264`), so macOS lists windows instead of merging them. |
| Per-window geometry | `desktop/src/window_manager.rs:49-98`, `desktop/src/windows.rs` | Move/resize events debounce 400 ms, then write the frame per role and instance to `window-state.json`. Restores clamp to monitors that still exist, and convert physical pixels back to logical points (`window_manager.rs:278-300`). |
| Per-window identity in the renderer | `desktop/src/window_manager.rs:142-150` → `apps/web/lib/desktop/windows.ts:17-21` | The window URL carries `?popout=<role>&wid=<label>`, so a page can scope state to one window. |
| Deep-link grammar | `desktop/src/os_integrations.rs:46-80` | `ticket`, `board`, the five stock/asset/tool kinds and `notifications`; shape-checked ids, query dropped, foreign schemes refused. |
| Deep-link delivery | `desktop/src/main.rs:1145-1159` (argv first launch), `main.rs:1065-1084` (single-instance re-launch), `main.rs:1166-1176` (`RunEvent::Opened`, macOS warm launch) | Three doors, one router. |
| Deep-link routing | `desktop/src/os_integrations.rs:136-181` | Open or focus the role window the link names, then move it to the record; the renderer can ask for the same thing through `route_marula_link` (`os_integrations.rs:189-191`), granted at `desktop/capabilities/desktop-web.json:15-24`. |
| Menu bar and tray | `desktop/src/os_integrations.rs:201-388` | Window menu with tabs off, tray with the two quick actions plus stock lookup and quit. |

Density and the keyboard model are not shell features: the shell draws no
screens (ADR 0028 §1), so both come from the page it hosts, and they stay
correct for free as the web app changes.

## What is missing

Five gaps, in the order I would take them. Slices 1 and 2 are both about deep
links and belong together; the rest are independent.

### Slice 1 — a deep link has to land on its target (delivered in this lane)

**What was wrong.** `route_deep_link` focused a window that matched the role and
stopped. The comment there said the window was "told via the widened wid param —
future: navigate it". So `marula://ticket/B` on a machine already showing
`marula://ticket/A` raised the `A` window and showed the wrong ticket, and a link
whose role window failed to open reported an error with nothing opened.
`DeepLinkTarget::main_path` already built the route for every kind and carried
`#[allow(dead_code, reason = "routing helper with a test but no call site yet")]`:
the helper existed, nothing called it.

**What changed.** `navigate_role_window` focuses the window and then sends it to
the URL the link names (`desktop/src/window_manager.rs:187-205`), skipping the
reload when the window already shows that record — a link that names what is
already on screen must not throw away unsaved work (`navigate_window`,
`window_manager.rs:169-182`). The URL comes from the same builder the window was
created with (`role_window_url`, `window_manager.rs:142-150`), so a window keeps
its `popout` role and `wid` identity across the move. When the role window cannot
be opened, the link falls back to the main window at `main_path`
(`os_integrations.rs:136-181`, `navigate_main_window`,
`window_manager.rs:209-224`).

Id shape is checked harder as a result: the id is pasted into a path, so
`parse_deep_link` now accepts only hex and dashes (`os_integrations.rs:55-64`).

**Ownership.** `desktop/src/os_integrations.rs`, `desktop/src/window_manager.rs`.
Nothing outside `desktop/`.

### Slice 2 — a shipped bundle registers the scheme

**Gap.** `desktop/` parses and routes `marula://` correctly, but no shipped
bundle claims the scheme. The only registration in the tree is the hand-written
`CFBundleURLTypes` in `scripts/build-desktop.sh:114-123`, which is the
developer's `.app` in `dist/`. `desktop/tauri.conf.json:1-36` carries no scheme
and no deep-link plugin, so a `tauri build` artifact registers nothing on any
platform, and the release legs (`release-desktop.yml`) build exactly that.

**Fix.** `plugins.deep-link.desktop.schemes: ["marula"]` in
`desktop/tauri.conf.json`, `tauri-plugin-deep-link` in `desktop/Cargo.toml`
registered in `base_builder` (`desktop/src/main.rs:1047-1101`), and
`register_all()` in `setup` for Linux/Windows, where the scheme is otherwise only
claimed by an installed package. The plugin generates the macOS `CFBundleURLTypes`
entry at bundle time, which retires the hand-written plist in the dev script as
the only registration.

**Ownership.** `desktop/tauri.conf.json`, `desktop/Cargo.toml`,
`desktop/src/main.rs`, and `desktop/gen/schemas/*` (regenerated by
`desktop/build.rs`). It also wants a line in `docs/desktop-release.md` §0.

**Why it is its own slice.** It adds a dependency and touches the release path,
so it needs a build this lane could not run. Slice 1 is the half that can be
proved without one.

### Slice 3 — native notifications

**Gap.** There are none. `desktop/Cargo.toml:11-33` has no
`tauri-plugin-notification`, and the word appears in `desktop/src` only as the
name of a deep-link target (`os_integrations.rs:35`).

**Decision first.** A PSA that raises OS banners needs a rule for what may
interrupt a person and what waits: an SLA breach and an assigned ticket are not
the same event. Write that rule down (an ADR) before the plugin is wired, then
add a `desktop/src/notifications.rs` that maps the web app's existing notification
items onto native banners, with the renderer asking through a command the way it
asks for windows.

**Ownership.** `desktop/src/notifications.rs` (new), `desktop/src/main.rs`,
`desktop/build.rs` (command manifest), `desktop/capabilities/*.json`,
`desktop/Cargo.toml`.

### Slice 4 — per-window session

**Gap.** Every window shares one cookie jar: neither `open_main_window`
(`desktop/src/main.rs:1119-1127`) nor `create_role_window`
(`window_manager.rs:243-254`) sets a data directory or a partition. That is also
what makes the connected mode work: the page is first-party because it loads the
instance's own origin (ADR 0028 amendment, 2026-11-05). So "per-window session"
is a decision about whether a second window may ever belong to a different
person, and no setting can settle that.

**What I would do.** Answer that question first. If the answer is no (the likely
one), the slice is documentation: say in the ADR that the shell holds one
session, and let the `wid` param handle the per-window state the row actually
means. If the answer is yes, it is a `data_directory` per window plus a second
login surface, which is a much larger piece of work.

**Ownership.** `docs/adr/0028-desktop-shell-and-trust-roots.md` (amendment), then
`desktop/src/window_manager.rs` if it proceeds.

### Slice 5 — keyboard model across windows

**Gap.** The shortcut set itself is the web app's (`e2e/flows/14-keyboard-shortcuts.spec.ts`),
so it already works in every window. What a native shell adds is a menu
accelerator for the actions that are not page actions: new ticket window
(`⌘N`), dispatch board (`⌘⇧D`), the stock lookup, and window cycling. Today the
Window menu carries only the platform defaults (`os_integrations.rs:266-281`).

**Ownership.** `desktop/src/os_integrations.rs`. Small, and it needs a key map
agreed with the web shortcuts first so the two do not collide.

## Non-goals

- No second front end. ADR 0028 §1 makes a screen the web app does not have a
  web feature first.
- No offline store. ADR 0028 §4.
- Nothing here changes the two trust roots (ADR 0028 §2) or the platform matrix
  (ADR 0031).

## Order, and why

Slice 1 is in the tree; slice 2 is the half of deep links that is left, and the
two belong together because a link that the OS never delivers is not routed at
all. Slice 3 needs a decision before code. Slice 4 needs a decision and may need
no code. Slice 5 is small and can ride any slice that touches
`os_integrations.rs`.
