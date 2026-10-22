//! `http_request(profile_ptr, profile_len, req_ptr, req_len) -> i64` — the
//! only way a guest reaches the network, gated by a capability and by a
//! consented egress profile.
//!
//! **This file is W53-7's.** W53-2 (this lane) ships the authorization half
//! and a `plugin_not_implemented` transport. W53-7 adds the request itself —
//! the HTTP client, the core-domain egress guard against SSRF, DNS pinning,
//! the redirect refusal — in this file only. The gate is not theirs to add:
//! [`super::HostGate::Egress`] already runs before this handler is reached, so
//! an implementation cannot forget it.
//!
//! What the gate already decided, before any transport code runs:
//!
//! - the request JSON parsed (a malformed request is `plugin_abi_frame_invalid`);
//! - `GET`/`HEAD`/`OPTIONS` needed `automation.reads`, everything else needed
//!   `automation.actions` (`plugin_capability_not_granted`);
//! - the named profile is in the install's consent
//!   (`plugin_egress_profile_missing`);
//! - the scheme is `https` (`plugin_egress_scheme_denied`);
//! - the authority is in the profile's allowlist and the path is under its
//!   prefix (`plugin_egress_host_denied`).
//!
//! So W53-7's remaining job is the part that needs a socket, and the refusal
//! left for it is `plugin_egress_response_too_large` (2 MiB).

use super::{
    AbiParam, AbiReturn, GuestMemory, HostEntry, HostGate, HostReturn, ImplStatus, InvocationState,
    Registry,
};
use crate::abi::{self, Refusal};

const PARAMS: &[AbiParam] = &[AbiParam::I32, AbiParam::I32, AbiParam::I32, AbiParam::I32];

/// Longest request URL the ABI accepts (matches the connector-endpoint cap).
pub const MAX_URL_BYTES: usize = 2048;

pub(super) fn register(registry: &mut Registry) {
    registry.register(HostEntry::new(
        "http_request",
        PARAMS,
        AbiReturn::Data,
        HostGate::Egress,
        ImplStatus::Deferred { owner: "W53-7" },
        call,
    ));
}

fn call(
    _state: &mut InvocationState,
    memory: &dyn GuestMemory,
    args: &[i32],
) -> Result<HostReturn, Refusal> {
    let raw = memory.read(args[2] as u32, args[3] as u32)?;
    let request: abi::HttpRequest =
        serde_json::from_slice(&raw).map_err(|_| Refusal::FrameInvalid)?;
    if request.url.len() > MAX_URL_BYTES || request.method.is_empty() {
        return Err(Refusal::FrameInvalid);
    }
    if let Some(body) = &request.body {
        if body.len() > abi::MAX_INPUT_BYTES {
            return Err(Refusal::InputTooLarge);
        }
    }
    Err(Refusal::NotImplemented)
}
