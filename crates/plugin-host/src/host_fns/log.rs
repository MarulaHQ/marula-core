//! `log(level: u32, ptr: i32, len: i32)` — the one diagnostic channel a guest
//! has. Shipped in W53-2.
//!
//! No capability gate: writing a line about your own invocation is inside the
//! install's own scope, and the budget is the limit that matters. The budget is
//! ADR 0023 Decision 4: 4 KiB per line and 100 lines per invocation, and the
//! refusal is `plugin_log_budget_exceeded`.
//!
//! The signature has no return value, so the guest cannot see that refusal. The
//! host still records it: a line over budget is not captured, and the executor
//! (W53-3) decides whether the invocation continues. That is why the refusal
//! lives in [`crate::abi::Refusal::LogBudgetExceeded`] rather than in a return
//! word the ABI does not have.

use super::{
    AbiParam, AbiReturn, GuestMemory, HostEntry, HostGate, HostReturn, ImplStatus, InvocationState,
    Registry,
};
use crate::abi::Refusal;

/// `error`
pub const LEVEL_ERROR: u32 = 1;
/// `warn`
pub const LEVEL_WARN: u32 = 2;
/// `info`
pub const LEVEL_INFO: u32 = 3;
/// `debug`
pub const LEVEL_DEBUG: u32 = 4;

const PARAMS: &[AbiParam] = &[AbiParam::U32, AbiParam::I32, AbiParam::I32];

pub(super) fn register(registry: &mut Registry) {
    registry.register(HostEntry::new(
        "log",
        PARAMS,
        AbiReturn::None,
        HostGate::InstallScope,
        ImplStatus::Shipped,
        call,
    ));
}

fn call(
    state: &mut InvocationState,
    memory: &dyn GuestMemory,
    args: &[i32],
) -> Result<HostReturn, Refusal> {
    let level = args[0] as u32;
    if !(LEVEL_ERROR..=LEVEL_DEBUG).contains(&level) {
        return Err(Refusal::FrameInvalid);
    }
    let message = memory.read_str(args[1] as u32, args[2] as u32)?;
    state.take_log_line(message, level)?;
    Ok(HostReturn::None)
}
