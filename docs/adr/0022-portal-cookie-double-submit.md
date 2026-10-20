# ADR 0022 — the csrf cookie is readable by design, and the detector that enforces it had never fired

**Status:** Accepted · 2026-10-13
**Scope:** the cookie session CSRF contract in `crates/core-api/src/auth/cookies.rs`, the four gated POST surfaces (staff refresh, staff logout, magic link confirm, portal refresh), and the web and SDK clients that echo the header
**Origin:** Wave 47 lane A1 (`p47-portal-session-hardening`), closing BUILD-PROMPT follow-up 11b (`docs/BUILD-PROMPT-VERDICTS.md:270`, evidence `internal/design/qa-p34-spec-verdict-sweep/REVIEW.md` batch 5)

## Context

The portal and staff cookie sessions were built on a written design.
`docs/reference/auth-threat-model.md` §A2 and the module comment in
`crates/core-api/src/auth/cookies.rs` say the same thing. `SameSite=Lax` is
the baseline. On top of it, the POST surfaces that consume cookies run
double-submit: the server sets a `marula_csrf` value without the `HttpOnly`
flag beside the session jar, and the browser's own JavaScript echoes it on
`x-marula-csrf`. A foreign origin cannot read the cookie, so it cannot
forge the header. Bearer-authenticated calls carry no ambient credential
and skip the check. The doc comment on `csrf_cookie` itself said
"non-httpOnly by design — double-submit REQUIRES it".

Two things were wrong in the shipped code, and the second one hid the
first.

**1. Every cookie shipped HttpOnly.** The shared builder `cookie_string`
appended `HttpOnly` unconditionally, and `csrf_cookie` built through it. So
the one value the design requires the browser to read was the one value the
browser could not read. `document.cookie` never reported `marula_csrf`, the
`x-marula-csrf` header was never sent, and a cookie-driven
`POST /api/v1/portal/refresh` could not pass `double_submit_ok`.

**2. The detector never fired, on anyone.** `cookie_authed` detected the
ambient session by filtering the session cookie on a bare 64-character
value. The shipped cookies carry `{session_id}.{refresh}`, which is a UUID,
a dot, then 64 hex characters, about 101 characters in all. That filter
matched nothing. `cookie_authed` answered false on every real request, and
`double_submit_ok` took its "bearer mode, no check" branch for every
caller, cookie-bearing or not. The documented CSRF contract was never
enforced for a single request.

**3. The readable value was also invisible.** The lane proved the fix in a
real browser and the gate still rendered. The csrf cookie ships
`Path=/api`, because the session jars do — but the web application serves
`/portal`, `/tickets` and friends, never anything under `/api`.
`document.cookie` returns only cookies whose path is a prefix of the
document's path, so a `/api`-scoped value is unreadable from every page
that needs to echo it. On the split web/api topology (dev :3000/:8080, the
e2e stack, the desktop app's in-process API) the header could never be
built even with the first two defects fixed. The scoping that is right for
a cookie that is only ever SENT is wrong for one that must be READ.

Fact 2 is why the portal defect was survivable for so long, and why fixing
only fact 1 would have made things worse. With the check vacuous, the
HttpOnly cookie cost nothing: cookie-driven refresh sailed through the
"bearer mode" branch. Fact 3 then waits on the other side: the lane's first
fix attempt removed HttpOnly, made the detector live, and the browser still
showed the sign-in gate, because a `/api`-scoped cookie is invisible to
`document.cookie` on `/portal`. The three defects had to be fixed together.
The p34 walk's live observation (`me()` returning 401 and cookie-only reads
returning 401 after login) is the combination of the first two, and the
reload proof the lane's capture harness now runs is the combination of all
three.

## Decision

**1. The cookie ships as documented: the double-submit value is readable,
nothing sensitive is.** `csrf_cookie` and `clear_csrf_cookie` carry neither
`HttpOnly` nor the `/api` path. The builder takes both flags per cookie
instead of forcing them. `marula_portal` and `marula_session` keep both —
the refresh secrets stay unreadable to JavaScript and `/api`-scoped, because
they only ever need to be SENT. The readable value is 32 bytes from the OS
RNG (`new_csrf_token`) and it authenticates nothing by itself. It proves the
request came from same-origin script, which is the whole mechanism. A
cross-site attacker cannot read the cookie, so it cannot produce the header,
and the session it would ride never becomes usable.

**2. The detector now detects the cookies that exist.** `cookie_authed`
goes through the pair parsers (`session_refresh_pair_from` and
`portal_refresh_pair_from`), which already handle the `{session_id}.{secret}`
shape. The bare-secret readers (`session_refresh_from` and
`portal_refresh_from`) are deleted. Their only caller was the broken
detector, and a bare 64-character secret is not a cookie this server ever
set.

**3. The check itself is untouched.** `double_submit_ok` still requires both
the cookie and a constant-time-equal header on every cookie-bearing call,
and it still exempts calls with no ambient cookie. No branch was removed
and no status code was changed. What changed is that the check now runs.

**4. Pinned from both directions.** `crates/core-api/tests/portal_flow.rs`
gains `portal_cookie_session_double_submit_round_trip`. The login
response's `marula_csrf` must be free of `HttpOnly` and host-wide
(`Path=/`), while `marula_portal` keeps the flag and the `/api` scope. A
cookie-only rotation with the echoed header must answer 200. The same
rotation without the header, or with a wrong header, must answer 403. A
body-credential refresh with no ambient cookie must still answer 200. The
staff twin (`auth_hardening_flow.rs`'s
`login_sets_the_hardened_cookie_trio`) gains the matching readability
assertion. Each of those assertions fails on the pre-change tree, either on
the flags, the scope, or the enforcement.

## Consequences

- A full page load of a portal surface the client is already signed into now
  restores the session through the cookie jar. The `me()` probe goes out
  without a bearer token and gets a 401. `portalFetch` then rotates using
  only the cookie jar and the echoed header, the rotation answers 200, and
  the retry lands signed in. This is the recovery path the 2026-10-02 walk
  could not reach, and the p47 capture harness proves it with a mid-session
  reload.
- Every cookie-bearing staff refresh, staff logout, magic link confirm and
  portal refresh now actually enforces double-submit. A non-JS client
  holding the cookie but not echoing the header gets a typed 403
  `csrf check failed`, which is the contract the threat model already
  documents. The bearer path is unchanged and remains the escape hatch for
  cross-registrable-domain deployments, a limitation §A2 already records.
- The in-repo browser clients needed no change. `apps/web/lib/portal-api.tsx`
  (`csrfHeader` and `portalFetch`) and `packages/sdk/src/client.ts`
  (`authCall`) already read `document.cookie` and echo the header when the
  value is there. The web-side provider fix for the same follow-up, an
  in-page login observed by `PortalAuthProvider`, is separate and lands
  beside this one.
- One behaviour change for a caller that never existed in the repo. A
  cookie-carrying request without the header that used to pass vacuously now
  fails closed. That is the check working. The e2e suite does not call the
  gated endpoints directly, because its browser flows ride the SDK and
  portal clients, which send the header.

## Alternatives considered

- **Amend the design: keep the cookie HttpOnly and drop double-submit,
  resting on `SameSite=Lax` alone.** Rejected. Lax is a browser-enforced
  attribute, and the repo committed in writing to the stronger posture. The
  lane brief also ruled out deleting the check. This would trade a
  deliberate second factor for one cookie attribute and leave the threat
  model describing a shape nobody ships.
- **Fix only the cookie flag and leave the detector broken.** Rejected. The
  documented check would stay a green-looking no-op, the negative assertions
  (missing header means 403) could never pass, and the CSRF surface would
  remain exactly as unenforced as before.
- **Move the double-submit value out of a cookie, into a response header the
  client stashes.** Rejected. Same trust model, more machinery, and both
  shipped clients already read the cookie. The documented shape was right.
  The implementation had drifted from it.
