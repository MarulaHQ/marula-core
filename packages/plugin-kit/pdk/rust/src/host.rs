//! Typed bindings for the nine host entries of ADR 0023 Decision 3. wasm32
//! only: this module is the guest side of the boundary.
//!
//! Every call is `Result`-shaped. A refusal the host decided arrives as the same
//! [`Refusal`] the audit row carries, so a plugin can log `refusal.as_str()` and
//! an operator can grep for it. The data-returning calls (`kv_get`, `kv_list`,
//! `http_request`, `read_model_query`) return the raw bytes the host wrote;
//! JSON is the convention, and parsing it is the plugin's business.

use alloc::vec::Vec;

use crate::abi::{unpack, Refusal, REPLY_TAG_ERR, REPLY_TAG_OK};

#[link(wasm_import_module = "marula")]
extern "C" {
    #[link_name = "log"]
    fn raw_log(level: u32, ptr: i32, len: i32);
    #[link_name = "kv_get"]
    fn raw_kv_get(ns_ptr: i32, ns_len: i32, key_ptr: i32, key_len: i32) -> i64;
    #[link_name = "kv_put"]
    fn raw_kv_put(
        ns_ptr: i32,
        ns_len: i32,
        key_ptr: i32,
        key_len: i32,
        value_ptr: i32,
        value_len: i32,
    ) -> u32;
    #[link_name = "kv_delete"]
    fn raw_kv_delete(ns_ptr: i32, ns_len: i32, key_ptr: i32, key_len: i32) -> u32;
    #[link_name = "kv_list"]
    fn raw_kv_list(
        ns_ptr: i32,
        ns_len: i32,
        prefix_ptr: i32,
        prefix_len: i32,
        cursor_ptr: i32,
        cursor_len: i32,
    ) -> i64;
    #[link_name = "http_request"]
    fn raw_http_request(profile_ptr: i32, profile_len: i32, req_ptr: i32, req_len: i32) -> i64;
    #[link_name = "emit_event"]
    fn raw_emit_event(type_ptr: i32, type_len: i32, payload_ptr: i32, payload_len: i32) -> u32;
    #[link_name = "read_model_query"]
    fn raw_read_model_query(query_id: u32, params_ptr: i32, params_len: i32) -> i64;
    #[link_name = "now"]
    fn raw_now() -> i64;
}

/// `log` levels. The host accepts 1..=4 and refuses anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
}

/// The four `read_model_query` ids (append-only).
pub mod query {
    /// `ticket.by_id` — needs `tickets.read`.
    pub const TICKET_BY_ID: u32 = 1;
    /// `board.list` — needs `tickets.read`.
    pub const BOARD_LIST: u32 = 2;
    /// `client.by_id` — needs `clients.read`.
    pub const CLIENT_BY_ID: u32 = 3;
    /// `contact.by_client` — needs `clients.read`.
    pub const CONTACT_BY_CLIENT: u32 = 4;
}

/// `(ptr, len)` for a byte string. An empty slice is `(0, 0)`: the host treats
/// a zero-length range as empty without touching the address, so the dangling
/// pointer Rust gives an empty `&[]` never reaches a bounds check.
fn parts(bytes: &[u8]) -> (i32, i32) {
    if bytes.is_empty() {
        (0, 0)
    } else {
        (bytes.as_ptr() as usize as i32, bytes.len() as i32)
    }
}

fn status(code: u32) -> Result<(), Refusal> {
    if code == 0 {
        Ok(())
    } else {
        Err(Refusal::from_code(code).unwrap_or(Refusal::FrameInvalid))
    }
}

fn reply(packed: i64) -> Result<Vec<u8>, Refusal> {
    let (ptr, len) = unpack(packed);
    if len == 0 {
        return Err(Refusal::FrameInvalid);
    }
    // SAFETY: the host allocated this block in guest memory through
    // `marula_alloc` and wrote exactly `len` bytes before returning.
    let frame = unsafe { core::slice::from_raw_parts(ptr as usize as *const u8, len as usize) };
    match frame.split_first() {
        Some((&REPLY_TAG_OK, payload)) => Ok(payload.to_vec()),
        Some((&REPLY_TAG_ERR, code)) if code.len() == 4 => {
            let code = u32::from_le_bytes([code[0], code[1], code[2], code[3]]);
            Err(Refusal::from_code(code).unwrap_or(Refusal::FrameInvalid))
        }
        _ => Err(Refusal::FrameInvalid),
    }
}

/// Writes one line to the invocation log. Carries no capability: the install's
/// own scope is the grant. Over the 4 KiB/100-line budget the host refuses the
/// line and the signature has no return value to say so — the refusal lands in
/// the invocation's audit trail, not here.
pub fn log(level: Level, message: &str) {
    let (ptr, len) = parts(message.as_bytes());
    unsafe { raw_log(level as u32, ptr, len) }
}

/// Reads one value. `Err(plugin_kv_namespace_denied)` for a label outside
/// `[a-z0-9_.-]{1,64}`; the host composes the physical namespace from the
/// invocation's org and install, so the label alone cannot reach another
/// tenant.
pub fn kv_get(namespace: &str, key: &str) -> Result<Vec<u8>, Refusal> {
    let (ns_ptr, ns_len) = parts(namespace.as_bytes());
    let (key_ptr, key_len) = parts(key.as_bytes());
    reply(unsafe { raw_kv_get(ns_ptr, ns_len, key_ptr, key_len) })
}

/// Writes one value, up to 1 MiB (`plugin_kv_value_too_large`), inside the
/// install's quota (`plugin_kv_quota_exceeded`).
pub fn kv_put(namespace: &str, key: &str, value: &[u8]) -> Result<(), Refusal> {
    let (ns_ptr, ns_len) = parts(namespace.as_bytes());
    let (key_ptr, key_len) = parts(key.as_bytes());
    let (value_ptr, value_len) = parts(value);
    status(unsafe { raw_kv_put(ns_ptr, ns_len, key_ptr, key_len, value_ptr, value_len) })
}

/// Deletes one key. Deleting an absent key is success, as in every other store.
pub fn kv_delete(namespace: &str, key: &str) -> Result<(), Refusal> {
    let (ns_ptr, ns_len) = parts(namespace.as_bytes());
    let (key_ptr, key_len) = parts(key.as_bytes());
    status(unsafe { raw_kv_delete(ns_ptr, ns_len, key_ptr, key_len) })
}

/// Lists up to 500 keys under `prefix`. Returns the bytes of the page — JSON
/// `{"keys": [...], "cursor": "..."}` — and `cursor` is the opaque value from
/// the previous page (empty for the first).
pub fn kv_list(namespace: &str, prefix: &str, cursor: &str) -> Result<Vec<u8>, Refusal> {
    let (ns_ptr, ns_len) = parts(namespace.as_bytes());
    let (prefix_ptr, prefix_len) = parts(prefix.as_bytes());
    let (cursor_ptr, cursor_len) = parts(cursor.as_bytes());
    reply(unsafe {
        raw_kv_list(
            ns_ptr, ns_len, prefix_ptr, prefix_len, cursor_ptr, cursor_len,
        )
    })
}

/// One outbound request, authorized by `automation.reads` (a read) or
/// `automation.actions` (a write) plus the install's consented `profile`.
/// `request` is JSON: `{"method","url","headers":{},"body":null}`. Returns the
/// JSON response. W53-7 fills the transport; until then the host refuses with
/// `plugin_not_implemented`.
pub fn http_request(profile: &str, request: &str) -> Result<Vec<u8>, Refusal> {
    let (profile_ptr, profile_len) = parts(profile.as_bytes());
    let (req_ptr, req_len) = parts(request.as_bytes());
    reply(unsafe { raw_http_request(profile_ptr, profile_len, req_ptr, req_len) })
}

/// Publishes an event envelope whose type the manifest declares. Needs
/// `events.emit`; an undeclared type is `plugin_event_type_not_declared`.
pub fn emit_event(event_type: &str, payload: &[u8]) -> Result<(), Refusal> {
    let (type_ptr, type_len) = parts(event_type.as_bytes());
    let (payload_ptr, payload_len) = parts(payload);
    status(unsafe { raw_emit_event(type_ptr, type_len, payload_ptr, payload_len) })
}

/// Runs one of the four named model queries ([`query`]). `params` is JSON; the
/// reply is JSON. Unknown id: `plugin_query_unknown`.
pub fn read_model_query(query_id: u32, params: &str) -> Result<Vec<u8>, Refusal> {
    let (params_ptr, params_len) = parts(params.as_bytes());
    reply(unsafe { raw_read_model_query(query_id, params_ptr, params_len) })
}

/// Milliseconds since the Unix epoch, with no capability and no WASI.
pub fn now() -> i64 {
    unsafe { raw_now() }
}
