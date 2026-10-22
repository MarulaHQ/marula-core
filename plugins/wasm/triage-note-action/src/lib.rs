//! First-party Marula reference plugin (Wave 34 lane 4, §4.6).
//!
//! The action kind this plugin contributes lives in `manifest.json`
//! (`surface_profile.actions`): a typed wrapper around the built-in
//! `add_internal_comment` effect the rule engine executes. This module is the
//! WASM entry (ADR 0023) — it accepts the action's params and answers whether
//! the call is well-formed, which is the part the plugin owns.
//!
//! `emit_event` is deliberately not called: the action's result flows back as
//! the handler's reply, and publishing an event belongs to the data plane
//! (W53-5).

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use marula_pdk::host;

/// True when the params carry a non-empty `"body"` key, which is the one
/// property the action's `params_schema` requires. A full JSON parse is not
/// worth a dependency for one key; the schema check on the core side is the
/// real validator.
fn has_body(params: &[u8]) -> bool {
    const KEY: &[u8] = b"\"body\"";
    params
        .windows(KEY.len())
        .position(|window| window == KEY)
        .is_some_and(|at| {
            params[at + KEY.len()..]
                .iter()
                .any(|byte| !byte.is_ascii_whitespace() && *byte != b':')
        })
}

fn handle(input: &[u8]) -> Vec<u8> {
    let accepted = has_body(input);
    host::log(
        if accepted {
            host::Level::Info
        } else {
            host::Level::Warn
        },
        if accepted {
            "triage-note-action: accepted"
        } else {
            "triage-note-action: params carry no body"
        },
    );
    if accepted {
        b"{\"effect\":\"add_internal_comment\",\"accepted\":true}".to_vec()
    } else {
        b"{\"effect\":\"add_internal_comment\",\"accepted\":false,\"reason\":\"body_required\"}".to_vec()
    }
}

marula_pdk::plugin!(handle);
