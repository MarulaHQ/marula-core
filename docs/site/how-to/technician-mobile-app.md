---
description: "The mobile app is the same product as the web app against the same API. A note you write on the phone lands on the ticket everyone else is looking at, and a…"
---

# How to use the Marula mobile app

> **Audience:** both — technicians carry the app, and owners and admins install
> builds and turn on biometric sign-in.

The mobile app is the same product as the web app against the same API. A note
you write on the phone lands on the ticket everyone else is looking at, and a
timer you start is the org's timer, not a local one. There is no mobile-only
data.

## Install a build

Builds come from EAS. The repository ships three profiles in `native/eas.json`:

| Profile | Use it for |
| --- | --- |
| `development` | a dev client with the dev menu and the scan injector |
| `preview` | internal testers and TestFlight |
| `production` | store builds |

```bash
pnpm --filter @marula-core/native exec eas build --profile preview --platform ios
```

An operator registers the EAS project once and stores the Expo token as a
repository secret. `EXPO_PUBLIC_EAS_PROJECT_ID` stays empty until that
happens, and push registration shows an honest "not configured" state
meanwhile.

## Sign in

1. Enter your work email and password. Accounts come from the same directory
   as the web app.
2. If your account has a second factor, the app asks for the 6-digit code from
   your authenticator app. Recovery codes work in the same field.
3. Optional: open **More → Security** and turn on biometric sign-in. The app
   keeps your session in the device keychain and asks for Face ID (or Touch
   ID) before it loads. "Sign out instead" on the lock screen clears the saved
   session.

## Work offline

The app keeps a cache of the read models you have opened and queues the writes
you make. The banner at the top of the shell says what is happening:

- **Offline — 3 changes queued for sync** means your writes are on the device
  and will replay when the connection returns.
- **Syncing 1 change…** means the queue is draining.
- A red banner means a change was rejected. Tap it. The queue inspector shows
  the reason and gives you **Retry** and **Discard**.

Queued writes carry an idempotency key, so a replay after a dropped connection
lands once. If someone edited the same record while you were offline, the app
adopts the server's current version, re-applies your change, and records that
in the operation's trail. Nothing merges behind your back.

To prove the path without touching system settings, open **More → Sync →
Simulate offline**, write a note, then switch it back.

## Work a ticket

1. **Board** lists your work. Pick a ticket.
2. **Start timer** begins a server-backed time entry. **Stop** closes it. The
   running timer stays visible above the tab bar on every tab.
3. **Add a note** writes a comment. Choose **Internal**, **Client**, or
   **Update** before sending — the kind decides who can see it.
4. On a tablet, the board is a two-pane layout: the list on the left, the open
   ticket on the right. Rotate or resize the window and the layout follows.

## Scan a code

Printed labels carry `marula://` links and short codes. The Scan tab reads
them both ways:

1. Open **Scan**.
2. Tap **Open the camera** on a phone, or type the code into the field under
   it. The simulator has no lens, so it points you at the field; the resolver
   behind both paths is the same one.
3. The card shows what the code means. If several records match, pick one —
   the app never guesses. If nothing matches, it says so and keeps your text.

Deep links work too: opening `marula://stock-item/<uuid>` from anywhere
resolves the same way a camera scan does.

## Check tools and stock

- **Stock item** shows on-hand by location and the movement ledger. **Consume**
  records a consumption against the chosen location.
- **Serialized unit** shows where a tool is right now, with **Check out to me**
  and **Return**.
- **Tools** (from Today or More) lists open checkouts. Scan a tool's serial to
  return it, or a toolbox stencil to open the whole-kit transaction.
- **Move stock** (from Today) runs Use, Transfer, and Receive with the same
  scan-then-confirm grammar. **Stock** looks up availability from the device
  cache when you have no signal.

All of these queue offline and replay once, the same as notes and timers. The
field flows in depth: [How to move stock and check out tools in the
field](technician-field-ops.md).

## When something looks wrong

- **A ticket says it no longer exists.** It was closed, deleted, or moved out
  of your visibility. Pull the list again.
- **A change shows as rejected.** The reason is in the queue inspector. Retry
  after fixing the cause, or discard it.
- **The timer is on another ticket.** Stop it there first. The app allows one
  running session per person, same as the web app.
