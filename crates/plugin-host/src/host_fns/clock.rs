//! `now() -> i64` — milliseconds since the Unix epoch.
//!
//! A guest has no clock and no WASI, so without this a plugin cannot timestamp
//! a cache entry or a note. It carries no capability: the clock is not an org's
//! data, and every invocation already knows the time.
//!
//! Shipped in W53-2. The value is the HOST's clock, taken once per call; a
//! guest cannot advance or rewind it, and two calls in one invocation observe
//! real elapsed time (bounded by the invocation's 250 ms wall clock).

use std::time::{SystemTime, UNIX_EPOCH};

use super::{GuestMemory, HostEntry, HostGate, HostReturn, ImplStatus, InvocationState, Registry};
use crate::abi::Refusal;

const PARAMS: &[super::AbiParam] = &[];

pub(super) fn register(registry: &mut Registry) {
    registry.register(HostEntry::new(
        "now",
        PARAMS,
        super::AbiReturn::Int,
        HostGate::InstallScope,
        ImplStatus::Shipped,
        call,
    ));
}

fn call(
    _state: &mut InvocationState,
    _memory: &dyn GuestMemory,
    _args: &[i32],
) -> Result<HostReturn, Refusal> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0);
    Ok(HostReturn::Int(millis))
}
