# How to use the keyboard shortcuts

> **Audience:** both — every signed-in staff member (owners, dispatchers,
> technicians, bookkeepers) can drive the app from the keyboard; admins
> additionally control nothing here, the profile is per account.

## Prerequisites

- A signed-in session on the web app. New? The tutorial
  [your first ticket](../tutorials/first-ticket.md) walks the loop on the
  seeded demo org (`owner@demo.local` / `demo-password-123`).
- A physical keyboard. The shortcuts are desktop-oriented; on touch-only
  devices the same actions stay reachable by clicking.

## The activation key: Alt

**Alt is the activation key.** Alt+letter and Alt+digit chords carry
commands; chords that OSes and browsers reserve (Ctrl+T/W/L, Cmd+Q,
Alt+Tab/F4, browser history with Alt+arrows) are deliberately absent, and
a consistency check in the repository fails the tests if a future command
collides with them. One documented exception: the search palette keeps
⌘K / Ctrl+K, the chord it shipped with before this registry existed.

## The two views

1. **Hold-Alt overlay** — press and HOLD Alt anywhere in the app (not in
   a text field) and a small layer appears listing every shortcut valid on
   the surface you are on: typed command labels, the key chord, and a
   scope badge (Global, or the surface family). Release Alt and it
   dismisses. Rows execute nothing by themselves — they document you.
2. **? cheat sheet** — press `?` to open the FULL registry dialog: every
   command across every surface, grouped (Everywhere / Navigation /
   On specific surfaces) with descriptions. Escape or the Close button
   dismisses it. It stays readable even when shortcuts are switched off,
   because it is reference, not an action.

## The chords that ship today

| Chord | Command | Where |
|---|---|---|
| Alt+N | New record (client / ticket on a board) | the surfaces that have a create control |
| Alt+1…7 | Jump to a rail section | everywhere |
| ⌘K / Ctrl+K | Command palette (unified search) | everywhere |
| Alt+J | Next page within the current rail section | everywhere |
| Alt+C | Copy the record's link | ticket / client / device detail |
| Alt+X | Close the topmost overlay | everywhere |
| ? | Open the cheat sheet | everywhere (even all-off) |

The same chord means the same command on every surface — Alt+N is "new
record" whether you are on clients or on a ticket board, and a chord is
never split into per-surface different meanings. Similar actions share one
command by design; that is the standard this registry enforces at
authoring time.

## Click parity — what a chord can and cannot do

Commands are built to behave exactly like the click they mirror: Alt+N
opens the same create dialog the New client / New ticket button opens,
Alt+1..7 land on the same pages the rail icons navigate to. A chord never
does anything the corresponding click cannot: disabled surfaces, RBAC
walls, and guarded routes stay exactly as guarded for the keyboard. The
Alt-N command is absent on surfaces without a create control — the overlay
simply does not list it there.

## Turning shortcuts off

Settings → Shortcuts → Keyboard shortcuts profile:

- **Default** — chords + the hold-Alt overlay are active.
- **All off** — chords and the hold-Alt overlay are off for your account
  (useful for presentations and readers). Pressing ? still opens the
  cheat sheet, which is read-only.

Choosing a profile is a per-user setting; other members are unaffected.
Per-command remapping is planned to arrive later in the same place.

## Where to go from here

- Admins configuring boards and workflows that the chords mirror: see
  [How to set workflow categories](admin-workflow-categories.md).
- The ticket loop itself: [How to work a ticket](staff-work-a-ticket.md).
