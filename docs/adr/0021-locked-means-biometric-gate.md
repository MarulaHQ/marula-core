# ADR 0021 — `locked` means the biometric gate; device approval is a different, unbuilt product

**Status:** Accepted · 2026-10-12
**Scope:** the native auth status vocabulary, the `/locked` screen, and what the pack's `09-locked` state is allowed to claim
**Origin:** Wave 43 track 2 lane 8 (`p43-api-payload-gaps`), from the gap the p41 mobile lane recorded: *"a real device-approval status is an API question, not a UI one"*

## Context

One word carried two meanings in shipped code, and neither owner knew about the other.

**Reading 1 — the pack.** The mobile pack state `09-locked` is "phone not yet approved by the service desk". It ships as: the state id in `native/scripts/contact-sheets.mjs:35`, the screen composition in `native/app/locked.tsx` ("This phone is not approved yet", "Your service desk needs to approve this phone before you can sign in. We have asked them, and you will get a notification the moment they do.", facts Phone / Asked / Waiting on), and the pre-written admin mail in `native/src/auth/admin-contact.ts` ("Approve my phone for Marula").

**Reading 2 — the app.** `AuthStatus` carries `locked` (`native/src/auth/auth-context.tsx:42`), and it is set in exactly one place: the boot effect, when the operator has biometrics enabled and a stored session exists (`auth-context.tsx:174-176`). The only way out is `unlock()` (`:251`), which runs `LocalAuthentication` and applies the stored session. So the shipped status is the Face ID gate on resume.

**What does not exist.** An exhaustive sweep of the workspace finds no device-approval concept anywhere: no table, no column, no route, no domain type, no event type, no contract-registry entry. Nothing asks the service desk for anything, and nothing will ever send the notification the screen promises. The "Check again" button runs a 1200 ms timer and asks nothing. So the approval reading has copy and a state id, and no mechanism at all.

Two further facts shaped the ruling. The word `locked` is already overloaded in
the domain for unrelated things: the timesheet-approval outcome `locked`
(`crates/core-api/src/routes/approvals.rs:336`), the `time_entry_locked` reason
(`crates/core-api/src/error.rs:705`), stock-ledger locks. None of them is this
screen. And the mobile pack itself (`design/mockups/marula-app-mockup-images/manifest.json`)
has no `09-locked` state at all: that id comes from the original meridian mobile
pack the handoff kit's redlines still index, so the pack reference for it is a
redline (`design/marula-handoff-kit/redlines/mobile-scan/`), not a shipped
Marula capture.

## Decision

**1. The biometric gate is the product truth of that screen, and the status is
renamed to say so.** The native auth status `locked` becomes `biometric`, the route
`/locked` becomes `/unlock`, and the screen tells the truth about the gate it
enforces: the operator turned Face ID on, and Face ID opens the stored session.
The rename covers every consumer found by the sweep, which is
`native/src/auth/auth-context.tsx` (the type and the single set site),
`native/src/ui/require-auth.tsx`, `native/app/login.tsx`,
`native/app/(tabs)/_layout.tsx`, `native/app/_layout.tsx` (route registration),
the screen itself, the capture script (`native/scripts/capture-pack.mjs`), and
the contact-sheet pack mapping (`native/scripts/contact-sheets.mjs`).

**2. Device approval is a separate, unbuilt product concept, and this lane does not build it.** Modelling it honestly means a device registry (a row per phone, with a status), an approval state machine, a service-desk surface to approve and revoke, a decision about what an unapproved device is still allowed to do (does a read work? does an offline write sync?), an expiry or re-approval policy, and a real notification on approval. Those are product and security decisions with a real blast radius on the auth path, not a payload fix. Inventing that API inside a payload lane would be a speculative build with no ruling behind it, so the pack's `09-locked` semantics stop being claimed by shipped code and become a named, owned follow-up instead.

**3. The composition is kept, the claims are not.** The pack's layout for that
state, which is a grey lock disc, a neutral (never red) sentence, a facts table,
a primary action and a secondary way out, is a good lock screen and is kept. What
changes is that the facts and actions now describe the gate that exists. The
admin-mail leg is deleted, because there is nothing to approve and a mailto that
asks for an approval that no system performs is a lie the product tells a
technician.

**4. What is explicitly NOT decided here.** The device-approval policy: who approves (any admin? the client's own contact?), whether approval is per-device or per-device-per-user, whether it expires, whether revoking it kills a live session, and how it behaves on desktop and web. A lane that builds device approval owns those questions and answers them in its own ADR.

## Consequences

- The auth status vocabulary reads as the mechanism it is: `loading | biometric | signedOut | mfa | signedIn`. A future `pendingApproval` (or whatever the approval lane names it) can join it without colliding, which is the point of splitting the word now rather than after a third meaning appears.
- The pack's `09-locked` is no longer presented as an implemented state.
  Captures of that screen are labelled as a derived state, so the fidelity ledger
  can say plainly that device approval is not drawn rather than quietly implying
  it is.
- The mobile capture set gains one honest state and loses one false one; the contact sheet's `03-auth` group keeps its slot with the derived label.
- Anything that later reads the old status string from a stored session or a deep link must handle `biometric`; the sweep found no persisted consumer, and the scan payload's `kind` strings are untouched.
- The superseeded copy (approval sentences, the fake "Check again", the approval mail) is removed in the same change, so no half-state exists where the status is renamed but the screen still claims an approval was requested.

## Alternatives considered

- **Model device approval now, as its own status.** Rejected for this lane. It is the right product direction and the wrong place to build it: it needs a device registry, an approval policy, a service-desk surface and a notification path, none of which exists, and none of which a payload lane can rule on. Recorded as a lane instead.
- **Keep `locked` and add a second status for approval.** Rejected. Two statuses, one of which no code can reach, leaves the false claims in place and makes the ambiguity permanent. The rename removes the ambiguity at its root.
- **Rename only the screen, keep the status.** Rejected. The status is the shared vocabulary every guard reads; renaming the screen and not the status leaves three guards redirecting to a route that no longer matches what they check.
