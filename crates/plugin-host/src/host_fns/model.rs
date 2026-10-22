//! `read_model_query(query_id: u32, params_ptr: i32, params_len: i32) -> i64` —
//! the only way a guest reads a Marula entity, and the reason it takes a query
//! ID rather than SQL.
//!
//! The table below is the whole vocabulary (ADR 0023 Decision 3): four named
//! queries, each with the capability it needs. An ID outside the table is
//! `plugin_query_unknown` BEFORE any capability question is asked, so a guest
//! cannot probe the grants by varying the ID. The four IDs are append-only, in
//! the order the ADR lists them.
//!
//! **The fetch is deferred.** Returning rows needs core-db and the caller's
//! transaction, which the request-path lane owns; this handler parses the
//! frame and refuses with `plugin_not_implemented`. The gate — the half that
//! decides whether the guest may ask at all — is already enforced by
//! [`super::HostGate::Query`] before this runs.

use core_domain::entities::PluginCapability;

use super::{
    AbiParam, AbiReturn, GuestMemory, HostEntry, HostGate, HostReturn, ImplStatus, InvocationState,
    Registry,
};
use crate::abi::{self, Refusal};

/// One query the ABI exposes: its stable id, its name for the docs and the
/// audit trail, and the capability that gates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuerySpec {
    pub id: u32,
    pub name: &'static str,
    pub capability: PluginCapability,
}

/// The closed query table. Append only: an id is never reused.
pub const QUERIES: [QuerySpec; 4] = [
    QuerySpec {
        id: 1,
        name: "ticket.by_id",
        capability: PluginCapability::TicketsRead,
    },
    QuerySpec {
        id: 2,
        name: "board.list",
        capability: PluginCapability::TicketsRead,
    },
    QuerySpec {
        id: 3,
        name: "client.by_id",
        capability: PluginCapability::ClientsRead,
    },
    QuerySpec {
        id: 4,
        name: "contact.by_client",
        capability: PluginCapability::ClientsRead,
    },
];

/// The query `id` names, or `None` for an id the ABI does not define.
pub fn query(id: u32) -> Option<QuerySpec> {
    QUERIES.iter().copied().find(|q| q.id == id)
}

const PARAMS: &[AbiParam] = &[AbiParam::U32, AbiParam::I32, AbiParam::I32];

pub(super) fn register(registry: &mut Registry) {
    registry.register(HostEntry::new(
        "read_model_query",
        PARAMS,
        AbiReturn::Data,
        HostGate::Query,
        ImplStatus::Deferred { owner: "W53-5" },
        call,
    ));
}

fn call(
    _state: &mut InvocationState,
    memory: &dyn GuestMemory,
    args: &[i32],
) -> Result<HostReturn, Refusal> {
    let params = memory.read(args[1] as u32, args[2] as u32)?;
    if params.len() > abi::MAX_INPUT_BYTES {
        return Err(Refusal::InputTooLarge);
    }
    serde_json::from_slice::<serde_json::Value>(&params).map_err(|_| Refusal::FrameInvalid)?;
    Err(Refusal::NotImplemented)
}
