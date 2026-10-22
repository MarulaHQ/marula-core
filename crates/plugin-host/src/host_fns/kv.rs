//! `kv_get`, `kv_put`, `kv_delete`, `kv_list` — the install-scoped byte
//! store of ADR 0023 Decision 5.
//!
//! **This file is W53-6's.** W53-2 (this lane) ships the ABI half: the argument
//! shapes, the namespace/key/value validation the ADR fixes, the typed
//! refusals, and a `plugin_not_implemented` answer from every verb. The store,
//! the quota counter, the migrations and the TTL sweep land in W53-6, in this
//! file only — the registry line already exists in
//! [`super::register_all`] and no shared file needs editing.
//!
//! Why validation lives here rather than in W53-6: a namespace label, a key
//! length and a value length are ABI facts. If the store validated them, the
//! refusal would arrive from a SQL transaction instead of from the contract,
//! and every future store would have to repeat it.
//!
//! The namespace the HOST composes is `(org_id, install_id, label)`. The guest
//! supplies only `label`; it can never name an org or an install, which is why
//! the store carries no cross-tenant refusal of its own.

use super::{
    AbiParam, AbiReturn, GuestMemory, HostEntry, HostGate, HostReturn, ImplStatus, InvocationState,
    Registry,
};
use crate::abi::{self, Refusal};

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

fn get(
    _state: &mut InvocationState,
    memory: &dyn GuestMemory,
    args: &[i32],
) -> Result<HostReturn, Refusal> {
    let _namespace = read_namespace(memory, args[0] as u32, args[1] as u32)?;
    let _key = read_key(memory, args[2] as u32, args[3] as u32)?;
    Err(Refusal::NotImplemented)
}

fn put(
    _state: &mut InvocationState,
    memory: &dyn GuestMemory,
    args: &[i32],
) -> Result<HostReturn, Refusal> {
    let _namespace = read_namespace(memory, args[0] as u32, args[1] as u32)?;
    let _key = read_key(memory, args[2] as u32, args[3] as u32)?;
    let _value = read_value(memory, args[4] as u32, args[5] as u32)?;
    Err(Refusal::NotImplemented)
}

fn delete(
    _state: &mut InvocationState,
    memory: &dyn GuestMemory,
    args: &[i32],
) -> Result<HostReturn, Refusal> {
    let _namespace = read_namespace(memory, args[0] as u32, args[1] as u32)?;
    let _key = read_key(memory, args[2] as u32, args[3] as u32)?;
    Err(Refusal::NotImplemented)
}

fn list(
    _state: &mut InvocationState,
    memory: &dyn GuestMemory,
    args: &[i32],
) -> Result<HostReturn, Refusal> {
    let _namespace = read_namespace(memory, args[0] as u32, args[1] as u32)?;
    let prefix = memory.read_str(args[2] as u32, args[3] as u32)?;
    let cursor = memory.read_str(args[4] as u32, args[5] as u32)?;
    if prefix.len() > abi::KV_KEY_MAX_BYTES || cursor.len() > abi::KV_KEY_MAX_BYTES {
        return Err(Refusal::FrameInvalid);
    }
    Err(Refusal::NotImplemented)
}
