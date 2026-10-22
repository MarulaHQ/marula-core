//! `kv_get`, `kv_put`, `kv_delete`, `kv_list` — the install-scoped byte
//! store of ADR 0023 Decision 5.
//!
//! **This file is W53-6's.** It owns two things and nothing else: the ABI facts
//! (the argument shapes, the namespace/key/value rules the ADR fixes, the typed
//! refusals) and the hand-off to the store. The store itself, the quota
//! arithmetic and the list-page codec live in [`crate::kv`]; the table, the
//! counter and the receipts live in `core_db::plugin_kv`.
//!
//! Why validation lives here rather than in the store: a namespace label, a key
//! length and a value length are ABI facts. If the store validated them, the
//! refusal would arrive from a SQL transaction instead of from the contract, and
//! every future store would have to repeat it.
//!
//! The namespace the HOST composes is `(org_id, install_id, label)`. The guest
//! supplies only `label`; it can never name an org or an install, which is why
//! the store carries no cross-tenant refusal of its own. Both identities are
//! read from [`InvocationState`], never from the frame's payload.
//!
//! ## Why every verb still answers `plugin_not_implemented` today
//!
//! Not because the body is missing — it is written — but because no store is
//! installed in the process that runs these entries. [`crate::kv::current`]
//! returns `None` and each verb refuses with [`Refusal::NotImplemented`], which
//! is the honest answer for "this process cannot serve plugin state".
//!
//! `ImplStatus::Deferred { owner: "W53-6" }` below says the same thing, and it
//! stays that way on purpose: the entry is deferred until something installs a
//! store at process start, and the two files that pin that status
//! (`host_fns::tests` and `packages/plugin-kit/abi/host-abi.v1.json`) belong to
//! other lanes. Flipping it is a one-line change in each, to be made together
//! with the wiring, not before it.
//!
//! ## TTL
//!
//! `kv_put` takes six words and none of them is an expiry (Decision 3), so v1's
//! guest cannot set a TTL. The `expires_at` column, the read-path filter and
//! `core_db::plugin_kv::sweep_expired` exist anyway: the export carries an
//! expiry and the restore re-applies it, and an operator or a future amendment
//! can set one. Nothing here invents a seventh argument to reach it.

use super::{
    AbiParam, AbiReturn, GuestMemory, HostEntry, HostGate, HostReturn, ImplStatus, InvocationState,
    Registry,
};
use crate::abi::{self, Refusal};
use crate::kv;

const GET_PARAMS: &[AbiParam] = &[AbiParam::I32, AbiParam::I32, AbiParam::I32, AbiParam::I32];
const PUT_PARAMS: &[AbiParam] = &[
    AbiParam::I32,
    AbiParam::I32,
    AbiParam::I32,
    AbiParam::I32,
    AbiParam::I32,
    AbiParam::I32,
];
const LIST_PARAMS: &[AbiParam] = &[
    AbiParam::I32,
    AbiParam::I32,
    AbiParam::I32,
    AbiParam::I32,
    AbiParam::I32,
    AbiParam::I32,
];

/// The store, plus the label/key/value rules the ABI fixes.
pub(super) fn register(registry: &mut Registry) {
    // See the module header: the body ships with this lane, the process wiring
    // does not, so the status stays deferred until something calls
    // `kv::install`. The two files that pin this string are not this lane's.
    let owner = ImplStatus::Deferred { owner: "W53-6" };
    registry.register(HostEntry::new(
        "kv_get",
        GET_PARAMS,
        AbiReturn::Data,
        HostGate::InstallScope,
        owner,
        get,
    ));
    registry.register(HostEntry::new(
        "kv_put",
        PUT_PARAMS,
        AbiReturn::Status,
        HostGate::InstallScope,
        owner,
        put,
    ));
    registry.register(HostEntry::new(
        "kv_delete",
        GET_PARAMS,
        AbiReturn::Status,
        HostGate::InstallScope,
        owner,
        delete,
    ));
    registry.register(HostEntry::new(
        "kv_list",
        LIST_PARAMS,
        AbiReturn::Data,
        HostGate::InstallScope,
        owner,
        list,
    ));
}

/// A namespace label: `[a-z0-9_.-]{1,64}`. Anything else is
/// `plugin_kv_namespace_denied` — the ADR's refusal for a namespace the host
/// will not compose.
pub fn read_namespace(memory: &dyn GuestMemory, ptr: u32, len: u32) -> Result<String, Refusal> {
    let namespace = memory
        .read_str(ptr, len)
        .map_err(|_| Refusal::KvNamespaceDenied)?;
    let legal = !namespace.is_empty()
        && namespace.len() <= abi::KV_NAMESPACE_MAX_BYTES
        && namespace.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'-')
        });
    if legal {
        Ok(namespace)
    } else {
        Err(Refusal::KvNamespaceDenied)
    }
}

/// A key: 1..=512 bytes of UTF-8 without NUL (the column is `text`). A key that
/// breaks this is a malformed frame, not a store refusal — the ADR has no
/// `plugin_kv_key_*` code, and inventing a second namespace refusal for keys
/// would make the audit trail say the wrong thing.
pub fn read_key(memory: &dyn GuestMemory, ptr: u32, len: u32) -> Result<String, Refusal> {
    let key = memory.read_str(ptr, len)?;
    if key.is_empty() || key.len() > abi::KV_KEY_MAX_BYTES || key.contains('\0') {
        return Err(Refusal::FrameInvalid);
    }
    Ok(key)
}

/// A value: at most 1 MiB (`plugin_kv_value_too_large`).
pub fn read_value(memory: &dyn GuestMemory, ptr: u32, len: u32) -> Result<Vec<u8>, Refusal> {
    let value = memory.read(ptr, len)?;
    if value.len() > abi::KV_VALUE_MAX_BYTES {
        return Err(Refusal::KvValueTooLarge);
    }
    Ok(value)
}

/// The store this process installed, or the refusal that says it has none.
fn store_or_refuse() -> Result<std::sync::Arc<dyn kv::KvStore>, Refusal> {
    kv::current().ok_or(Refusal::NotImplemented)
}

fn get(
    state: &mut InvocationState,
    memory: &dyn GuestMemory,
    args: &[i32],
) -> Result<HostReturn, Refusal> {
    let namespace = read_namespace(memory, args[0] as u32, args[1] as u32)?;
    let key = read_key(memory, args[2] as u32, args[3] as u32)?;
    let store = store_or_refuse()?;
    let value = store
        .get(state.org_id, state.install_id, &namespace, &key)
        .map_err(|error| error.refusal())?;
    // An absent key and an empty value are the same answer: Decision 3's table
    // gives `kv_get` exactly one refusal code, so absence is not a refusal.
    Ok(HostReturn::Data(abi::ok_frame(&value.unwrap_or_default())))
}

fn put(
    state: &mut InvocationState,
    memory: &dyn GuestMemory,
    args: &[i32],
) -> Result<HostReturn, Refusal> {
    let namespace = read_namespace(memory, args[0] as u32, args[1] as u32)?;
    let key = read_key(memory, args[2] as u32, args[3] as u32)?;
    let value = read_value(memory, args[4] as u32, args[5] as u32)?;
    let store = store_or_refuse()?;
    // No expiry: the ABI's `kv_put` has six words and no TTL (see the module
    // header).
    store
        .put(
            state.org_id,
            state.install_id,
            &namespace,
            &key,
            &value,
            None,
        )
        .map_err(|error| error.refusal())?;
    Ok(HostReturn::ok_status())
}

fn delete(
    state: &mut InvocationState,
    memory: &dyn GuestMemory,
    args: &[i32],
) -> Result<HostReturn, Refusal> {
    let namespace = read_namespace(memory, args[0] as u32, args[1] as u32)?;
    let key = read_key(memory, args[2] as u32, args[3] as u32)?;
    let store = store_or_refuse()?;
    // The ABI's answer is only "did it work", and deleting an absent key is
    // success in every store a plugin author has met.
    store
        .delete(state.org_id, state.install_id, &namespace, &key)
        .map_err(|error| error.refusal())?;
    Ok(HostReturn::ok_status())
}

fn list(
    state: &mut InvocationState,
    memory: &dyn GuestMemory,
    args: &[i32],
) -> Result<HostReturn, Refusal> {
    let namespace = read_namespace(memory, args[0] as u32, args[1] as u32)?;
    let prefix = memory.read_str(args[2] as u32, args[3] as u32)?;
    let cursor = memory.read_str(args[4] as u32, args[5] as u32)?;
    if prefix.len() > abi::KV_KEY_MAX_BYTES || cursor.len() > abi::KV_KEY_MAX_BYTES {
        return Err(Refusal::FrameInvalid);
    }
    let store = store_or_refuse()?;
    let page = store
        .list(
            state.org_id,
            state.install_id,
            &namespace,
            &prefix,
            (!cursor.is_empty()).then_some(cursor.as_str()),
        )
        .map_err(|error| error.refusal())?;
    // The payload is the JSON page the PDK documents:
    // `{"keys": [...], "cursor": "..."}`, with an empty cursor on the last page.
    Ok(HostReturn::Data(abi::ok_frame(&kv::encode_page(&page))))
}
