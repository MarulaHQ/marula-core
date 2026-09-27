//! First-party Meridian reference plugin (Wave 34 lane 4, §4.6).
//!
//! The declarative surface this plugin contributes lives in
//! `manifest.json` (`surface_profile`) — the host registers widgets /
//! actions / validators from that DATA and executes everything through
//! its own trusted components. This module carries the WASM entry the
//! Extism runtime (when it ships) invokes; building it produces the
//! `plugin.wasm` artifact the PDK bundles.

#![no_std]

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn meridian_plugin_entry() -> i32 {
    0
}
