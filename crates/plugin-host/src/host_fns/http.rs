//! `http_request(profile_ptr, profile_len, req_ptr, req_len) -> i64` — the
//! only way a guest reaches the network, gated by a capability and by a
//! consented egress profile.
//!
//! The gate runs before this handler and is not this file's to add:
//! [`super::HostGate::Egress`] parses the request, resolves the capability
//! (`automation.reads` for `GET`/`HEAD`/`OPTIONS`, `automation.actions` for
//! everything else), checks that the named profile is in the install's consent,
//! and runs [`core_domain::egress`] against the URL. A handler therefore cannot
//! forget a gate, and an ungranted call never reaches the socket.
//!
//! What is left is the part that needs a socket, in [`crate::http`]: resolution
//! (once, checked, pinned), the 5,000 ms budget, the 2 MiB cap, the refusal of
//! every redirect, and the host-only credential injection point.
//!
//! The handler re-runs the allowlist check even though the gate just did. It
//! needs the [`core_domain::egress::EgressTarget`] anyway — the transport
//! fetches the screened URL, never the guest's string — and re-checking costs a
//! few parses against a closed list. The alternative is a handler that is only
//! safe because of who called it.

use core_domain::egress;

use super::{
    AbiParam, AbiReturn, GuestMemory, HostEntry, HostGate, HostReturn, ImplStatus, InvocationState,
    Registry,
};
use crate::abi::{self, Refusal};
use crate::http::refusal_for;

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
    state: &mut InvocationState,
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

    // An absent or empty profile is the same answer to a request: there is no
    // allowlist to match, so nothing is consented.
    let allowlist = state
        .egress()
        .get(&profile)
        .filter(|scopes| !scopes.is_empty())
        .ok_or(Refusal::EgressProfileMissing)?;
    let target = egress::authorize_strings(&request.url, allowlist).map_err(refusal_for)?;

    // No credential today: the install-scoped vault handle this needs is not in
    // the handler's signature. See `crate::http` for the gap and the built
    // injection path.
    let response = crate::http::send(&target, &request, None)?;
    let body = serde_json::to_vec(&response).map_err(|_| Refusal::FrameInvalid)?;
    Ok(HostReturn::Data(abi::ok_frame(&body)))
}
