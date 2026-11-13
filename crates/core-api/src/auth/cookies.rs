//! Cookie session transport (Wave 19 lane 5, threat model §A2).
//!
//! The refresh token rides an httpOnly cookie set by core-api; the access
//! token stays in memory-only client state. The Authorization bearer
//! path is UNCHANGED for MCP/desktop/connectors/e2e helpers — cookies are
//! additive, never a replacement.
//!
//! CSRF choice (documented, docs/reference/auth-threat-model.md §A2;
//! decision of record ADR 0022):
//! * `SameSite=Lax` is the baseline (cross-site fetches never carry the
//!   cookie) — ports do not split a site, so same-host different-port
//!   dev stacks (web :3000 / api :8080) still deliver the cookie.
//! * PLUS double-submit for the mutating cookie-only surface: core-api
//!   also sets a NON-httpOnly `marula_csrf` value that browsers echo
//!   back on `x-marula-csrf` (double-submit works BECAUSE JavaScript
//!   can read a non-httpOnly cookie; a foreign origin cannot read it, so
//!   it cannot forge the header). Bearer-authenticated calls carry no
//!   ambient credential and skip the header check entirely.
//! * Secure is auto-derived from the API base URL (https ⇒ Secure); the
//!   dev http loops keep the cookie local-only but still httpOnly + Lax.
//!
//! Cookie naming: `marula_session` for the STAFF refresh chain and
//! `marula_portal` for the portal family — two jars so a browser
//! profile can never bleed staff refresh into the portal surface or vice
//! versa (parity with the localStorage families they replace). Both
//! scoped `Path=/api` because the API mounts every generation under it.

use axum::http::{header, HeaderMap};

pub const SESSION_COOKIE: &str = "marula_session";
pub const PORTAL_COOKIE: &str = "marula_portal";
pub const CSRF_COOKIE: &str = "marula_csrf";
pub const CSRF_HEADER: &str = "x-marula-csrf";

const COOKIE_PATH: &str = "/api";
/// The double-submit value is read by the APPLICATION's own JavaScript
/// (that is the mechanism), so it must sit where the app's documents live —
/// the web app serves /portal, /tickets and friends, never /api. Scoping it
/// to /api (as the session jars are) hides it from every page that needs to
/// echo it on x-marula-csrf, which breaks the cookie-driven refresh on any
/// split deployment. The value is a fresh random secret that authenticates
/// nothing by itself, so a host-wide path weakens nothing: a foreign origin
/// still cannot read it (ADR 0022).
const CSRF_COOKIE_PATH: &str = "/";

/// Builds a `Set-Cookie` value honoring the config-derived flags.
///
/// `http_only` is per-cookie and deliberate (ADR 0022): the session secrets
/// are unreadable to JavaScript; the double-submit csrf value MUST NOT be,
/// because the browser's own script echoing it on `x-marula-csrf` is the
/// entire mechanism.
pub fn session_cookie(secure: bool, value: &str, max_age: i64) -> String {
    cookie_string(SESSION_COOKIE, value, max_age, secure, true)
}

/// Portal twin.
pub fn portal_cookie(secure: bool, value: &str, max_age: i64) -> String {
    cookie_string(PORTAL_COOKIE, value, max_age, secure, true)
}

/// The double-submit CSRF value handed out with any cookie-bearing auth
/// response (non-httpOnly by design — double-submit REQUIRES it).
pub fn csrf_cookie(secure: bool, value: &str) -> String {
    cookie_string_at(
        CSRF_COOKIE,
        value,
        86_400 * 7,
        secure,
        false,
        CSRF_COOKIE_PATH,
    )
}

fn cookie_string(name: &str, value: &str, max_age: i64, secure: bool, http_only: bool) -> String {
    cookie_string_at(name, value, max_age, secure, http_only, COOKIE_PATH)
}

fn cookie_string_at(
    name: &str,
    value: &str,
    max_age: i64,
    secure: bool,
    http_only: bool,
    path: &str,
) -> String {
    let tail = if secure { " Secure" } else { "" };
    let http_only = if http_only { "; HttpOnly" } else { "" };
    format!("{name}={value}; Path={path}; Max-Age={max_age}{http_only}; SameSite=Lax{tail}")
}

/// Clears a cookie (Max-Age=0).
pub fn clear_session_cookie(secure: bool) -> String {
    cookie_string(SESSION_COOKIE, "", 0, secure, true)
}

pub fn clear_portal_cookie(secure: bool) -> String {
    cookie_string(PORTAL_COOKIE, "", 0, secure, true)
}

/// The csrf clear matches the set flags exactly (no HttpOnly, host-wide
/// path) so the browser expires the same cookie the setter wrote — a clear
/// scoped differently silently leaves the old value in the jar.
pub fn clear_csrf_cookie(secure: bool) -> String {
    cookie_string_at(CSRF_COOKIE, "", 0, secure, false, CSRF_COOKIE_PATH)
}

/// Same entropy class as refresh tokens.
pub fn new_csrf_token() -> String {
    use rand::TryRng;
    let mut bytes = [0u8; 32];
    rand::rngs::SysRng
        .try_fill_bytes(&mut bytes)
        .expect("OS CSPRNG unavailable");
    hex::encode(bytes)
}

fn cookie_from(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| {
            raw.split(';').find_map(|pair| {
                let (key, val) = pair.trim().split_once('=')?;
                (key == name && !val.is_empty()).then(|| val.to_string())
            })
        })
}

/// The staff cookie carries `{session_id}.{refresh}` — split here so the
/// HTTP layer never parses cookie payloads itself.
pub fn session_refresh_pair_from(headers: &HeaderMap) -> Option<(uuid::Uuid, String)> {
    let raw = cookie_from(headers, SESSION_COOKIE)?;
    let (sid, secret) = raw.split_once('.')?;
    if secret.len() != 64 {
        return None;
    }
    Some((uuid::Uuid::parse_str(sid).ok()?, secret.to_string()))
}

/// The portal twin: `{session_id}.{refresh}` inside `marula_portal`.
pub fn portal_refresh_pair_from(headers: &HeaderMap) -> Option<(uuid::Uuid, String)> {
    let raw = cookie_from(headers, PORTAL_COOKIE)?;
    let (sid, secret) = raw.split_once('.')?;
    if secret.len() != 64 {
        return None;
    }
    Some((uuid::Uuid::parse_str(sid).ok()?, secret.to_string()))
}

/// True when an ambient session cookie exists on the request (staff or
/// portal) — the trigger for enforcing the CSRF contract.
///
/// The detection goes through the PAIR parsers because the cookies carry
/// `{session_id}.{refresh_secret}` (~101 chars), not a bare 64-char secret.
/// Before ADR 0022 this filtered on `len() == 64`, which the shipped cookie
/// shape never satisfies, so `cookie_authed` answered false for every real
/// session and `double_submit_ok` silently degraded to "bearer mode, no
/// check" — the documented CSRF contract was never enforced at all.
pub fn cookie_authed(headers: &HeaderMap) -> bool {
    session_refresh_pair_from(headers).is_some() || portal_refresh_pair_from(headers).is_some()
}

/// Double-submit check for a cookie-authenticated call: BOTH the
/// non-httpOnly csrf cookie AND the header must be present and equal.
/// Bearer-mode requests (no ambient session cookie) always pass — there
/// is no ambient credential for a forged request to ride.
pub fn double_submit_ok(headers: &HeaderMap) -> bool {
    if !cookie_authed(headers) {
        return true;
    }
    let (Some(cookie), Some(presented)) = (
        cookie_from(headers, CSRF_COOKIE),
        headers.get(CSRF_HEADER).and_then(|v| v.to_str().ok()),
    ) else {
        return false;
    };
    constant_time_eq(cookie.as_bytes(), presented.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
