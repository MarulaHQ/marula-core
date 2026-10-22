//! Scoped outbound HTTP for `http_request` (W53-7).
//!
//! Empty on purpose. `p53-runtime` pre-declares this module so the three lanes of
//! R3 each own one file and none of them edits `lib.rs`. W53-7 fills this in; the
//! guest-facing entry stays in `host_fns/http.rs`, and the outbound-URL guard
//! itself belongs in `core-domain` so Wave 54 can consume it.
