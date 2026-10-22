//! `emit_event(type_ptr, type_len, payload_ptr, payload_len) -> u32` — publish
//! an event envelope of a type the manifest declares.
//!
//! The gate is the whole of this lane's share, and it is in
//! [`super::HostGate::Event`]: `events.emit` must be granted
//! (`plugin_capability_not_granted`) and the type must appear in the
//! manifest's declared event types (`plugin_event_type_not_declared`, ADR 0023
//! Decision 3). Both run before this handler.
//!
//! **The publish itself is deferred.** It needs the `event_types` registry
//! (migration 0124) and the core-events producer, which are request-path
//! resources: that is the execution lane's work, not W53-2's, W53-6's or
//! W53-7's. Until then this refuses with `plugin_not_implemented` rather than
//! pretending a row was written.

use super::{
    AbiParam, AbiReturn, GuestMemory, HostEntry, HostGate, HostReturn, ImplStatus, InvocationState,
    Registry,
};
use crate::abi::{self, Refusal};

const PARAMS: &[AbiParam] = &[AbiParam::I32, AbiParam::I32, AbiParam::I32, AbiParam::I32];

pub(super) fn register(registry: &mut Registry) {
    registry.register(HostEntry::new(
        "emit_event",
        PARAMS,
        AbiReturn::Status,
        HostGate::Event,
        // The event-type REGISTRY check (the second half of the ADR's gate) and
        // the publish need the request path; see the module docs.
        ImplStatus::Deferred { owner: "W53-5" },
        call,
    ));
}

fn call(
    _state: &mut InvocationState,
    memory: &dyn GuestMemory,
    args: &[i32],
) -> Result<HostReturn, Refusal> {
    let payload = memory.read(args[2] as u32, args[3] as u32)?;
    if payload.len() > abi::MAX_INPUT_BYTES {
        return Err(Refusal::InputTooLarge);
    }
    Err(Refusal::NotImplemented)
}
