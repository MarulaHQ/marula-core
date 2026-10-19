# Marula native app (`@marula-core/native`)

The Expo / React Native client. Wave 33 lane 1 ships the foundation: auth
with MFA and biometric sign-in, the app shell, an offline-first read cache, a
replay-safe write queue, live sync over the event stream, push registration,
deep links, and the adaptive two-pane layout. Field screens (scanning,
receiving, the technician's day) build on top of these seams.

Everything talks to the public API through `@marula-core/sdk`. There are no
mobile-only endpoints and no hand-declared entity types. The SDK's generated
types are the contract (`docs/rn-groundwork.md`).

## Dev loop

Metro and the iOS Simulator are the primary loop. Expo web is the
fast-iteration surface for layout work, but it never stands in for a
native-behavior check (camera, haptics, offline queue, push).

| Service | Port | Notes |
|---|---|---|
| Metro / Expo dev server | 8181 | Per-lane convention |
| Expo web | 8182 | `pnpm --filter @marula-core/native dev:web` |
| core-api | per-run | From `scripts/e2e-stack.sh up` (random high port); serves the live stream |

```bash
# 1. Boot the stack (Postgres + Valkey + core-api + demo seed)
bash scripts/e2e-stack.sh up
. e2e/.e2e-stack.env

# 2. Point the app at it and start Metro (live sync rides the API's stream)
EXPO_PUBLIC_API_URL="http://localhost:${API_PORT}" \
pnpm --filter @marula-core/native dev

# iOS Simulator (Xcode required)
pnpm --filter @marula-core/native ios

# Android emulator (secondary, host is 10.0.2.2)
pnpm --filter @marula-core/native android
```

Demo logins come from the seed: `owner@demo.local`, `tech@demo.local` (and
`dispatcher@`, `bookkeeper@`), password `demo-password-123`.

## Checks

```bash
pnpm --filter @marula-core/native typecheck   # tsc --noEmit
pnpm --filter @marula-core/native lint        # eslint (token + raw-hex guard)
pnpm --filter @marula-core/native test        # vitest: queue, cache, realtime, sync
pnpm --filter @marula-core/native export:web  # bundle check for the web surface
```

Against a live stack:

```bash
pnpm tsx native/scripts/live-sync-proof.ts
```

That harness runs the real `SyncEngine`, `WriteQueue`, and `RealtimeClient`
against the API: an offline note replays exactly once, a teammate's stream
receives the comment live, a second stream receives ticket updates, timer
start/stop materialize a time entry, and consumption and tool checkout
replays each land one ledger/status change.

Maestro flows live in `native/e2e/flows` and drive the simulator build:

```bash
pnpm --filter @marula-core/native e2e
```

The day surface is captured at phone and tablet widths, light and dark, with
`node native/scripts/capture-web-tech-day.mjs` once Expo web runs against the
stack (the header of the script carries the full boot sequence). The script
signs in through the API, seeds the session into browser storage, and writes
to `design/qa-p33-rn-tech-day/captures`.

## Code map

```
app/                  expo-router routes (tabs, ticket, stock, scan, dev)
src/core/             pure TypeScript: cache, queue, realtime, sync engine,
                      typed read models and write operations, error copy
src/data/             React bindings: storage adapters, connectivity,
                      auth-aware sync provider, useModel/useMutation/useQueue
src/auth/             login, MFA bridge, refresh rotation, biometric sign-in
src/ui/               tokens (light + dark), primitives, adaptive layout,
                      offline banner, timer bar
src/day/              today's schedule rail, on-call strip, travel drafts
src/tickets/          ticket list + detail (clock, time, notes, photos)
src/scan/             scan resolver flow and result card
src/push/             push registration + notification tap routing
tests/                vitest: replay invariants, parity with the web copy
```

## Sync model

- **Reads** hydrate from a durable cache and re-render from live events. A
  screen shows cached data immediately, marks it stale when an event says the
  server moved, and reconciles when the link allows.
- **Writes** are queued per entity with a client-minted idempotency key, so a
  replay (reconnect, relaunch) lands exactly once. A version conflict adopts
  the server's current version once and records the resolution in the op's
  audit trail. A second conflict is a typed rejection surfaced to the user.
  Nothing is merged silently.
- **Timers and notes are server entities.** The app never keeps a private
  timer; `GET /timers/active` is the source of truth, and every timer mutation
  (start, pause, resume, stop) is queued.
- **Live sync** holds one authed stream to `/api/events/stream`, which is
  org-scoped: every contract event the org commits arrives there, including the
  comment, stock and tool events the deleted WebSocket hub routed per user or
  not at all. Those events land on cache invalidation. See
  `docs/rn-groundwork.md` for the wire shapes.

## Adaptive layout

Width decides the layout class, not the device. At 768pt and wider the board
is a two-pane master–detail. Below that it pushes. iPad Stage Manager
resizing hits the same breakpoint, and pointer affordances are gated on
capability rather than device model.

## Operator gaps

These need an operator, an account, or a server change, and none of them block
the foundation's checks above.

1. **EAS project.** `eas.json` defines three build profiles (development, preview,
   production), but `EXPO_PUBLIC_EAS_PROJECT_ID` is empty until an operator
   registers the project with Expo. Store EAS tokens as repository secrets. Never commit them.
2. **Push registration endpoint.** This app requests permission, sets the
   Android channel, and acquires a device token, but the server has no device
   token registration route and the worker's fan-out has no APNs/FCM
   transport. Until a contract lane adds both, the in-app notification center
   (live over the event stream) is the only push surface.
3. **Simulator and TestFlight legs.** The foundation host runs no Xcode, so
   simulator captures, Maestro runs, and TestFlight checklist items (real
   camera scan, push delivery, App Store review) are pending hardware or
   runner access. The CI workflow in `.github/workflows/native.yml.disabled`
   defines the macOS runner path.
