//! First-party Marula reference plugin (Wave 34 lane 4, §4.6).
//!
//! The declarative surface this plugin contributes lives in `manifest.json`
//! (`surface_profile`) — the host registers widgets from that DATA and renders
//! them through its own trusted components. This module is the WASM entry the
//! runtime (ADR 0023) invokes: the v1 ABI, via `marula-pdk`.
//!
//! The handler is deliberately small. It logs one line, stamps the reply with
//! the host clock, and returns the props the widget renders, which is the whole
//! job of a widget plugin that reads its data from its own manifest. Calls that
//! need the data plane (`read_model_query`, `emit_event`) arrive with W53-5 and
//! are not used here.

#![no_std]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use marula_pdk::host;

fn handle(input: &[u8]) -> Vec<u8> {
    host::log(
        host::Level::Info,
        "escalation-widget: rendering escalation_policy",
    );
    // `now` needs no capability: the install's own scope is the grant, and the
    // clock is the host's, never the guest's idea of it.
    format!(
        "{{\"widget\":\"escalation_policy\",\"rendered_at_ms\":{},\"input_bytes\":{}}}",
        host::now(),
        input.len()
    )
    .into_bytes()
}

marula_pdk::plugin!(handle);
