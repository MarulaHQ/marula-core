# React Native app — foundation shipped (Wave 33 lane 1 `p33-rn-foundation`)

Status: the app exists. `native/` is a pnpm workspace member (`@meridian/native`)
built on Expo SDK 57 with expo-router, and it consumes `packages/sdk` unchanged.
The seam Wave 22 documented is now load-bearing: same pins, same typed
envelopes, no mobile fork of anything.

## What ships

| Leg | What landed | Where |
| --- | --- | --- |
| Auth | email + password login, MFA bridge token with TOTP entry, rotating refresh pair in SecureStore, platform biometric sign-in | `native/src/auth/auth-context.tsx` |
| Read cache | durable cache over the public read models. A screen renders cached data, marks it stale on contract events, reconciles when the link allows | `native/src/core/cache.ts` |
| Write queue | per-entity FIFO, client-minted idempotency keys, persisted audit trail, one adopted version on a 409, typed rejection surfaced instead of a silent merge | `native/src/core/queue.ts` |
| Live sync | one authed socket to `/ws`, org and board subscriptions, event → cache invalidation | `native/src/core/realtime.ts`, `native/src/core/sync.ts` |
| Timers and notes | server-backed entities: `GET /timers/active` plus queued start/pause/resume/stop. Notes are comments with optimistic render and server reconcile | `native/src/tickets/ticket-detail.tsx` |
| Adaptive layout | width class decides two-pane master–detail at 768pt and up. Compact pushes. Stage Manager resizes into the same breakpoint | `native/src/ui/adaptive.ts` |
| Scan injector | paste a scan or open `meridian://scan-result?text=…`. The typed resolver drives routing, ambiguity renders a picker, unknown stays unknown | `native/app/(tabs)/scan.tsx`, `native/src/core/scan.ts` |
| Push seam | permission, Android channel, device/expo token acquisition, notification tap routing | `native/src/push/push.ts` |

Verification lives with the lane: 43 unit tests (`native/tests`), a live-stack
harness (`native/scripts/live-sync-proof.ts`), and Maestro flows
(`native/e2e/flows`). `native/README.md` carries the dev loop and the
operator gaps.

## What the seam still IS NOT

* No second source of truth for types. Mobile imports the regenerated SDK, and
  a contract drift fails `tsc`.
* No mobile-specific endpoints. Every screen ships from the same IF2/IF3
  contract.
* No server contract changes landed with this lane. Three gaps surfaced while
  wiring live sync. They are listed below and need a contract lane, not a
  mobile workaround.

## Findings a follow-up contract lane owns

1. `ticket_comment` events route only to mentioned users
   (`crates/realtime/src/channel.rs`, `routes_for`). The author's other clients
   do not receive their own note over the socket, so the phone-to-desktop note
   leg reconciles on refocus instead of live. Routing comments to the ticket's
   board channel closes it.
2. Stock movements and tool checkouts emit events with no route in the same
   match arm, so those read models reconcile on refocus and reconnect.
3. No device-token registration route exists, and the worker's fan-out has no
   APNs/FCM transport. The client acquires a token. Nothing can receive it yet.

## Non-goals (binding, unchanged)

1. Mobile does not fork the SDK. Regen stays the single pin and typed-source
   path.
2. No mobile-specific migration endpoints. Any new warp-down call lands in the
   contract first.
3. Typed reasons are rendered copy, never swallowed. The mobile reason map is
   byte-checked against `apps/web/lib/error-copy.ts` by a test
   (`native/tests/error-copy-parity.test.ts`), and the scan routing vocabulary
   against `apps/web/lib/scan/routing.ts`.
