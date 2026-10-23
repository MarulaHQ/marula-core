//! `http_request(profile_ptr, profile_len, req_ptr, req_len) -> i64` — the
//! only way a guest reaches the network, gated by a capability and by a
//! consented egress profile.
//!
//! ## What this file no longer does
//!
//! It used to build the request, resolve a name, pin the answer, apply the
//! timeout, cap the body and read the response — in the process that runs
//! untrusted wasm. Phase 2 moved all of that parent-side (W53-12, ADR 0023's
//! `secret_get` deferral), because the process that runs a guest must not hold
//! a credential, a key, or the code path that would ever receive one. This
//! process no longer links a network client at all.
//!
//! What remains is the ABI edge: read the two guest buffers, reject what the ABI
//! would never have accepted, and forward the call over the parent bridge
//! ([`crate::egress::BridgeHttp`]). The answer is status, headers and body.
//!
//! ## The gate still runs here, and again there
//!
//! [`super::HostGate::Egress`] parses the request, resolves the capability
//! (`automation.reads` for `GET`/`HEAD`/`OPTIONS`, `automation.actions` for
//! everything else), checks that the named profile is in the install's consent,
//! and screens the URL with [`core_domain::egress`]. An unconsented profile
//! therefore costs no round trip. The parent re-runs the same functions against
//! its own copy of the install's scopes, because the worker is the process it
//! isolates — and it re-derives the capability from the install's grant set for
//! the same reason.

use core_domain::egress;

use super::{
    AbiParam, AbiReturn, GuestMemory, HostEntry, HostGate, HostReturn, ImplStatus, Registry,
};
use crate::abi::{self, Refusal};
use crate::egress::current as current_transport;

const PARAMS: &[AbiParam] = &[AbiParam::I32, AbiParam::I32, AbiParam::I32, AbiParam::I32];

/// Longest request URL the ABI accepts (matches the connector-endpoint cap and
/// [`core_domain::egress::MAX_URL_BYTES`]).
pub const MAX_URL_BYTES: usize = egress::MAX_URL_BYTES;

pub(super) fn register(registry: &mut Registry) {
    registry.register(HostEntry::new(
        "http_request",
        PARAMS,
        AbiReturn::Data,
        HostGate::Egress,
        ImplStatus::Shipped,
        call,
    ));
}

fn call(
    _state: &mut crate::host_fns::InvocationState,
    memory: &dyn GuestMemory,
    args: &[i32],
) -> Result<HostReturn, Refusal> {
    let profile = memory.read_str(args[0] as u32, args[1] as u32)?;
    let raw = memory.read(args[2] as u32, args[3] as u32)?;
    let request: abi::HttpRequest =
        serde_json::from_slice(&raw).map_err(|_| Refusal::FrameInvalid)?;
    if request.url.len() > MAX_URL_BYTES || request.method.trim().is_empty() {
        return Err(Refusal::FrameInvalid);
    }
    if let Some(body) = &request.body {
        if body.len() > abi::MAX_INPUT_BYTES {
            return Err(Refusal::InputTooLarge);
        }
    }
    // A guest with no transport is a process with nowhere to send; that is
    // `plugin_not_implemented`, not a refusal the guest caused.
    let transport = current_transport().ok_or(Refusal::NotImplemented)?;
    let response = transport.send(&profile, &request)?;
    let body = serde_json::to_vec(&response).map_err(|_| Refusal::FrameInvalid)?;
    Ok(HostReturn::Data(abi::ok_frame(&body)))
}
