# React Native groundwork — SEAM-ONLY (Wave 22 lane 1 `p22-ecosystem-stretch`)

Status: **seam-only deliverable** — this lane documents the mobile seam;
NO mobile app, no Expo/RN package, no `native` workspace member ships in
this lane (the Wave 22 row's scope: groundwork = the boundary + the
contract, not an app).

## What the seam IS

Everything a future RN client consumes ALREADY exists and is pinned:

| Seam | Contract | Wave 22 delta consumed by RN |
| --- | --- | --- |
| Wire API | `fixtures/openapi/v1/openapi.json` pins (byte-identical pair) | connectors install/test, voip ingest, pricing feeds, marketplace ratings/publishers, `attachments/{id}/transcribe`, roles editor, stripe webhook shape |
| Typed SDK | `packages/sdk` (generated) | the RN client imports the SAME regenerated types — no second source of truth |
| Auth | existing bearer sessions / API scopes (no RN-specific grant changes) | none — same matrix |
| Envelopes | IF2/IF3 (`payment_declined` typed codes, `ai_*` refusals) | mobile clients get IDENTICAL typed decline/refusal reasons — the earliest candidate for offline-first handling |

## What the seam IS NOT

* no app target / storefront package in this lane;
* no push-notification plumbing (FCM/APNs) — transport decisions stay
  deferred;
* no marketplace v1 restructure: the RN client consumes the v1 pack
  formats exactly as the theming lane's consumers do (contract with the
  p24 theming lane holds).

## Non-goals (binding for whoever phones this home later)

1. The RN groundwork does NOT fork the SDK. Mobile reuses
   `packages/sdk` — regen flows stay the single pin + typed-source path.
2. No mobile-specific migration endpoints: every screen ships from the
   same IF contract; any new warp-down call must land here FIRST.
3. The typed payment_declined codes + AI typed refusals are THE reason
   the native shell can render honest states — a future RN lane must
   treat every typed reason as rendered copy, never swallowed.
