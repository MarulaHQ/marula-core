//! The host-function registry — the seam every later lane plugs into.
//!
//! ADR 0023 Decision 3 fixes nine ABI entries across six functions. This
//! module owns the MACHINERY: the entry shape, the gate each entry must pass,
//! the per-invocation state the handlers read, and the dispatch that makes the
//! gate unskippable. The implementations live one per file, and each file
//! registers itself in [`register_all`]:
//!
//! ```text
//! host_fns/log.rs     log                          shipped (this lane)
//! host_fns/kv.rs      kv_get/put/delete/list       W53-6
//! host_fns/http.rs    http_request                 W53-7
//! host_fns/events.rs  emit_event                   data plane (see the file)
//! host_fns/model.rs   read_model_query             data plane (see the file)
//! host_fns/clock.rs   now                          shipped (this lane)
//! ```
//!
//! **Adding a host function is a registry line plus a file.** A later lane
//! fills its own file's `register`; it never edits `lib.rs` and never edits
//! this module. That is the shape three parallel lanes depend on.
//!
//! ## The gate is not optional
//!
//! [`Registry::invoke`] resolves [`HostEntry::gate`] before it calls the
//! handler, and [`HostEntry`] can only be built inside this crate
//! (`HostEntry::new` is `pub(crate)`), so an executor cannot register an entry
//! that skips its gate and a handler cannot run without one. A host function
//! with no gate is [`HostGate::InstallScope`], which is a decision on the
//! record, not an omission — `log`, `kv_*` and `now` carry no capability
//! because the install's own scope is the grant.
//!
//! ## Refusals before work
//!
//! Order inside a dispatch is: unknown entry, arity, gate, handler. A
//! not-yet-implemented handler therefore still refuses an ungranted call with
//! `plugin_capability_not_granted` rather than `plugin_not_implemented`, so a
//! stub cannot hide a missing grant.

pub mod clock;
pub mod events;
pub mod http;
pub mod kv;
pub mod log;
pub mod model;

use core_domain::entities::PluginCapability;
use core_domain::policy::{self, Permission};
use uuid::Uuid;

use crate::abi::{self, Refusal};

/// Guest linear memory, as the host functions see it. The executor implements
/// this over a wasmtime `Memory`; [`Harness`] implements it over a plain byte
/// vector so a host function can be driven with no sandbox at all.
pub trait GuestMemory {
    /// Current size in bytes.
    fn len(&self) -> usize;

    /// No bytes addressable at all. Provided rather than required: every
    /// implementation answers it the same way, and the executor should not
    /// have to write it out to satisfy a lint.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Copies `len` bytes at `ptr`. Out-of-bounds reads are a typed refusal,
    /// never a panic and never a short read.
    ///
    /// `len == 0` is the empty range whatever `ptr` holds, because an empty
    /// guest slice has no address to be valid: the PDK hands back `(0, 0)` for
    /// one, and a label or namespace that is allowed to be empty must not be
    /// refused for pointing at null.
    fn read(&self, ptr: u32, len: u32) -> Result<Vec<u8>, Refusal>;

    /// UTF-8 view of the same range (labels, event types, profile names).
    fn read_str(&self, ptr: u32, len: u32) -> Result<String, Refusal> {
        let bytes = self.read(ptr, len)?;
        String::from_utf8(bytes).map_err(|_| Refusal::FrameInvalid)
    }
}

/// One declared parameter of a host entry. All ABI words are 32-bit; the
/// distinction is signedness for a reader and for the executor's signature
/// check against the module's type section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiParam {
    I32,
    U32,
}

/// What a host entry returns, which decides how the executor hands the value
/// back to the guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiReturn {
    /// No value at all (`log`).
    None,
    /// A `u32` status: `0` = success, otherwise a [`Refusal`] code.
    Status,
    /// A packed pointer/length ([`abi::pack`]) to a tagged reply frame.
    Data,
    /// A bare `i64` value (`now`, milliseconds since the Unix epoch).
    Int,
}

/// The authorization a host entry runs before its handler. See the ADR's
/// Decision 3 table; every variant resolves onto the existing permission
/// matrix through [`InvocationState::require`], so no variant invents
/// authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostGate {
    /// No capability: the install's own scope is the grant (`log`, `kv_*`,
    /// `now`).
    InstallScope,
    /// A fixed capability (`emit_event` -> `events.emit`).
    Capability(PluginCapability),
    /// Resolved per query id (`read_model_query`): the id names a query and
    /// the query names its capability. An unknown id refuses before any
    /// capability is considered.
    Query,
    /// `http_request`: a read needs `automation.reads`, a write needs
    /// `automation.actions`, and both need a consented egress profile. The
    /// method is read from the PARSED request, never trusted from a claim.
    Egress,
    /// `emit_event`: the capability plus the manifest's declared event type.
    Event,
}

impl HostGate {
    /// Resolves this gate. Returns the capability the call ran under, or the
    /// typed refusal.
    pub(crate) fn resolve(
        self,
        state: &mut InvocationState,
        memory: &dyn GuestMemory,
        args: &[i32],
    ) -> Result<Option<PluginCapability>, Refusal> {
        match self {
            HostGate::InstallScope => Ok(None),
            HostGate::Capability(capability) => {
                state.require(capability)?;
                Ok(Some(capability))
            }
            HostGate::Query => {
                let id = args.first().copied().unwrap_or(0) as u32;
                let spec = model::query(id).ok_or(Refusal::QueryUnknown)?;
                state.require(spec.capability)?;
                Ok(Some(spec.capability))
            }
            HostGate::Egress => {
                let profile = memory.read_str(args[0] as u32, args[1] as u32)?;
                let raw = memory.read(args[2] as u32, args[3] as u32)?;
                let request: abi::HttpRequest =
                    serde_json::from_slice(&raw).map_err(|_| Refusal::FrameInvalid)?;
                let capability = if request.is_write() {
                    PluginCapability::AutomationActions
                } else {
                    PluginCapability::AutomationReads
                };
                state.require(capability)?;
                abi::authorize_egress(state.egress(), &profile, &request.url)?;
                Ok(Some(capability))
            }
            HostGate::Event => {
                let event_type = memory.read_str(args[0] as u32, args[1] as u32)?;
                state.require(PluginCapability::EventsEmit)?;
                if !state.declares_event(&event_type) {
                    return Err(Refusal::EventTypeNotDeclared);
                }
                Ok(Some(PluginCapability::EventsEmit))
            }
        }
    }
}

/// Whether the entry's implementation ships or is a typed stub for a later
/// lane. The catalogue is honest about this: a deferred entry refuses with
/// [`Refusal::NotImplemented`], never with a fake value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplStatus {
    Shipped,
    /// `owner` names the lane that fills the body.
    Deferred {
        owner: &'static str,
    },
}

/// What a handler returns. The executor maps it onto the guest word: `None` ->
/// no return value, `Status` -> the word itself, `Data` -> the bytes written to
/// guest memory and packed with [`abi::pack`], `Int` -> the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostReturn {
    None,
    Status(u32),
    Data(Vec<u8>),
    Int(i64),
}

impl HostReturn {
    /// Success for a `Status` entry.
    pub const fn ok_status() -> Self {
        HostReturn::Status(0)
    }
}

type HostCall = fn(&mut InvocationState, &dyn GuestMemory, &[i32]) -> Result<HostReturn, Refusal>;

/// One row of the catalogue: the metadata a reader (and the executor's
/// signature check) needs, plus the handler.
#[derive(Clone, Copy)]
pub struct HostEntry {
    /// The import name inside the `marula` namespace.
    pub name: &'static str,
    /// The declared parameters, in order.
    pub params: &'static [AbiParam],
    /// The declared return shape.
    pub returns: AbiReturn,
    /// The gate that always runs first.
    pub gate: HostGate,
    /// Shipped or deferred, with the owner of a deferred body.
    pub status: ImplStatus,
    call: HostCall,
}

impl HostEntry {
    /// Only this crate can build an entry, so no external code can register a
    /// handler without a gate.
    pub(crate) const fn new(
        name: &'static str,
        params: &'static [AbiParam],
        returns: AbiReturn,
        gate: HostGate,
        status: ImplStatus,
        call: HostCall,
    ) -> Self {
        HostEntry {
            name,
            params,
            returns,
            gate,
            status,
            call,
        }
    }

    /// Arity in 32-bit words.
    pub fn arity(&self) -> usize {
        self.params.len()
    }
}

impl std::fmt::Debug for HostEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostEntry")
            .field("name", &self.name)
            .field("arity", &self.arity())
            .field("returns", &self.returns)
            .field("gate", &self.gate)
            .field("status", &self.status)
            .finish()
    }
}

/// The `marula` namespace. One instance per worker; built once by
/// [`register_all`] and shared read-only across invocations.
#[derive(Default)]
pub struct Registry {
    entries: Vec<HostEntry>,
}

impl Registry {
    pub fn new() -> Self {
        Registry {
            entries: Vec::new(),
        }
    }

    /// Registers one entry. A duplicate name is a programming error and panics
    /// at boot rather than shadowing a gate at runtime.
    pub fn register(&mut self, entry: HostEntry) {
        assert!(
            !self.entries.iter().any(|e| e.name == entry.name),
            "host function {:?} registered twice",
            entry.name
        );
        self.entries.push(entry);
    }

    /// The entry for `name`, if the namespace declares it.
    pub fn get(&self, name: &str) -> Option<&HostEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Every entry, in registration order — the catalogue a PDK generator and
    /// the docs are checked against.
    pub fn catalogue(&self) -> &[HostEntry] {
        &self.entries
    }

    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.entries.iter().map(|e| e.name)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Dispatches one host call. Unknown entry, wrong arity, a failed gate and
    /// a handler refusal are all typed; nothing here panics on guest input.
    pub fn invoke(
        &self,
        name: &str,
        state: &mut InvocationState,
        memory: &dyn GuestMemory,
        args: &[i32],
    ) -> Result<HostReturn, Refusal> {
        let entry = self.get(name).ok_or(Refusal::UnknownHostFn)?;
        if args.len() != entry.arity() {
            return Err(Refusal::FrameInvalid);
        }
        entry.gate.resolve(state, memory, args)?;
        (entry.call)(state, memory, args)
    }
}

/// Builds the namespace. The single place the six modules are wired in; a
/// later lane changes its own file and nothing here.
pub fn register_all() -> Registry {
    let mut registry = Registry::new();
    log::register(&mut registry);
    kv::register(&mut registry);
    http::register(&mut registry);
    events::register(&mut registry);
    model::register(&mut registry);
    clock::register(&mut registry);
    registry
}

/// One captured `log` line. The executor drains these after the call and
/// forwards them wherever the deployment sends logs; the ABI itself does not
/// decide that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    /// `1` error, `2` warn, `3` info, `4` debug (see [`log`]).
    pub level: u32,
    pub message: String,
}

/// Everything one host call may read, and the only state it may change:
/// the resolved identity, the grants, the declared event types, the consented
/// egress profile, and the log budget.
///
/// Built from an [`abi::InvocationRequest`] on the worker, or by hand in a
/// test. It never holds a database handle: a host function that needs the
/// database takes it from the executor's own context in its own lane.
pub struct InvocationState {
    pub org_id: Uuid,
    pub install_id: Uuid,
    pub invocation_id: Uuid,
    granted: Vec<PluginCapability>,
    declared_events: Vec<String>,
    egress: abi::EgressScopes,
    log_lines: u32,
    log_bytes: usize,
    logs: Vec<LogLine>,
}

impl InvocationState {
    /// A state with no grants, no declared events and no egress consent — the
    /// deny-by-default starting point.
    pub fn new(org_id: Uuid, install_id: Uuid, invocation_id: Uuid) -> Self {
        InvocationState {
            org_id,
            install_id,
            invocation_id,
            granted: Vec::new(),
            declared_events: Vec::new(),
            egress: abi::EgressScopes::new(),
            log_lines: 0,
            log_bytes: 0,
            logs: Vec::new(),
        }
    }

    /// The state a worker derives from a validated request frame. A frame that
    /// does not validate is refused whole.
    pub fn from_request(request: &abi::InvocationRequest) -> Result<Self, Refusal> {
        request.validate()?;
        let mut state =
            InvocationState::new(request.org_id, request.install_id, request.invocation_id);
        state.granted = request
            .granted
            .iter()
            .filter_map(|key| PluginCapability::parse(key))
            .collect();
        state.declared_events = request.declared_events.clone();
        state.egress = request.egress.clone();
        Ok(state)
    }

    /// Adds one grant.
    pub fn grant(&mut self, capability: PluginCapability) -> &mut Self {
        if !self.granted.contains(&capability) {
            self.granted.push(capability);
        }
        self
    }

    /// Declares one event type the manifest emits.
    pub fn declare_event(&mut self, event_type: impl Into<String>) -> &mut Self {
        let event_type = event_type.into();
        if !self.declared_events.contains(&event_type) {
            self.declared_events.push(event_type);
        }
        self
    }

    /// Consents one egress profile.
    pub fn consent_egress(&mut self, profile: impl Into<String>, scopes: Vec<String>) -> &mut Self {
        self.egress.insert(profile.into(), scopes);
        self
    }

    pub fn granted(&self) -> &[PluginCapability] {
        &self.granted
    }

    pub fn declared_events(&self) -> &[String] {
        &self.declared_events
    }

    pub fn egress(&self) -> &abi::EgressScopes {
        &self.egress
    }

    /// The install's granted permissions in the standard matrix vocabulary —
    /// what [`policy::check`] runs against, exactly as for a human actor.
    pub fn permissions(&self) -> Vec<Permission> {
        self.granted.iter().map(|c| c.permission()).collect()
    }

    /// The one authorization call a host function makes. Resolves the
    /// capability onto the existing matrix and runs the same `policy::check` a
    /// human actor's action would.
    pub fn require(&self, capability: PluginCapability) -> Result<(), Refusal> {
        if policy::check(&self.permissions(), capability.permission()).is_allowed() {
            Ok(())
        } else {
            Err(Refusal::CapabilityNotGranted)
        }
    }

    pub fn allows(&self, capability: PluginCapability) -> bool {
        self.require(capability).is_ok()
    }

    pub fn declares_event(&self, event_type: &str) -> bool {
        self.declared_events.iter().any(|t| t == event_type)
    }

    /// Charges one log line against the invocation budget. Over budget is a
    /// typed refusal; the line is not written.
    pub fn take_log_line(&mut self, message: String, level: u32) -> Result<(), Refusal> {
        if message.len() > abi::MAX_LOG_LINE_BYTES || self.log_lines >= abi::MAX_LOG_LINES {
            return Err(Refusal::LogBudgetExceeded);
        }
        self.log_lines += 1;
        self.log_bytes += message.len();
        self.logs.push(LogLine { level, message });
        Ok(())
    }

    /// The lines captured so far, oldest first.
    pub fn logs(&self) -> &[LogLine] {
        &self.logs
    }

    /// Takes the captured lines (the executor forwards them after the call).
    pub fn take_logs(&mut self) -> Vec<LogLine> {
        std::mem::take(&mut self.logs)
    }

    pub fn log_bytes(&self) -> usize {
        self.log_bytes
    }
}

/// A test harness: the registry plus a simulated linear memory. Drives a host
/// function with no wasmtime, no worker and no database, so a typed refusal can
/// be asserted directly.
///
/// ```ignore
/// let mut harness = Harness::new(InvocationState::new(org, install, invocation));
/// let (ptr, len) = harness.put_str("telemetry");
/// let (req_ptr, req_len) = harness.put_str(r#"{"method":"GET","url":"https://api.example.com/v1"}"#);
/// let err = harness
///     .call("http_request", &[ptr as i32, len as i32, req_ptr as i32, req_len as i32])
///     .expect_err("no consent");
/// assert_eq!(err, Refusal::EgressProfileMissing);
/// ```
pub struct Harness {
    registry: Registry,
    state: InvocationState,
    memory: Vec<u8>,
    next: usize,
}

struct HarnessMemory<'a> {
    bytes: &'a [u8],
}

impl GuestMemory for HarnessMemory<'_> {
    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn read(&self, ptr: u32, len: u32) -> Result<Vec<u8>, Refusal> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let start = ptr as usize;
        let end = start
            .checked_add(len as usize)
            .ok_or(Refusal::FrameInvalid)?;
        if end > self.bytes.len() {
            return Err(Refusal::FrameInvalid);
        }
        Ok(self.bytes[start..end].to_vec())
    }
}

impl Harness {
    pub fn new(state: InvocationState) -> Self {
        Harness {
            registry: register_all(),
            state,
            memory: vec![0; 4096],
            next: 1024,
        }
    }

    pub fn state(&self) -> &InvocationState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut InvocationState {
        &mut self.state
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Copies `bytes` into simulated guest memory and returns its address.
    /// Addresses start at 1024 so `0` is always a null pointer in a test.
    pub fn put(&mut self, bytes: &[u8]) -> u32 {
        let start = self.next;
        let end = start + bytes.len().max(1) + 8;
        if end > self.memory.len() {
            self.memory.resize(end.next_power_of_two(), 0);
        }
        self.memory[start..start + bytes.len()].copy_from_slice(bytes);
        self.next = end;
        start as u32
    }

    /// [`Harness::put`] for a string: `(ptr, len)` ready to pass as two words.
    pub fn put_str(&mut self, value: &str) -> (u32, u32) {
        let ptr = self.put(value.as_bytes());
        (ptr, value.len() as u32)
    }

    /// Dispatches one call through the real registry (gate included).
    pub fn call(&mut self, name: &str, args: &[i32]) -> Result<HostReturn, Refusal> {
        self.registry.invoke(
            name,
            &mut self.state,
            &HarnessMemory {
                bytes: &self.memory,
            },
            args,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_domain::policy::{Action, Module};

    fn state() -> InvocationState {
        InvocationState::new(Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7())
    }

    #[test]
    fn the_namespace_has_exactly_the_nine_abi_entries() {
        let registry = register_all();
        let names: Vec<&str> = registry.names().collect();
        assert_eq!(
            names,
            vec![
                "log",
                "kv_get",
                "kv_put",
                "kv_delete",
                "kv_list",
                "http_request",
                "emit_event",
                "read_model_query",
                "now",
            ]
        );
        assert_eq!(registry.len(), 9);
    }

    #[test]
    fn every_entry_declares_the_arity_and_return_the_guest_sees() {
        let registry = register_all();
        let expected: [(&str, usize, AbiReturn); 9] = [
            ("log", 3, AbiReturn::None),
            ("kv_get", 4, AbiReturn::Data),
            ("kv_put", 6, AbiReturn::Status),
            ("kv_delete", 4, AbiReturn::Status),
            ("kv_list", 6, AbiReturn::Data),
            ("http_request", 4, AbiReturn::Data),
            ("emit_event", 4, AbiReturn::Status),
            ("read_model_query", 3, AbiReturn::Data),
            ("now", 0, AbiReturn::Int),
        ];
        for (name, arity, returns) in expected {
            let entry = registry.get(name).expect(name);
            assert_eq!(entry.arity(), arity, "{name} arity");
            assert_eq!(entry.returns, returns, "{name} return");
        }
    }

    #[test]
    fn deferred_entries_name_their_owner_and_shipped_ones_do_not() {
        let registry = register_all();
        assert_eq!(registry.get("log").unwrap().status, ImplStatus::Shipped);
        assert_eq!(registry.get("now").unwrap().status, ImplStatus::Shipped);
        // W53-11 shipped the parent bridge; the four verbs work in the default
        // (pool) profile, so the catalogue stops calling them deferred.
        for name in ["kv_get", "kv_put", "kv_delete", "kv_list"] {
            assert_eq!(
                registry.get(name).unwrap().status,
                ImplStatus::Shipped,
                "{name} works in the pool profile"
            );
        }
        // W53-7 shipped the transport; the entry no longer defers.
        assert_eq!(
            registry.get("http_request").unwrap().status,
            ImplStatus::Shipped
        );
        for entry in registry.catalogue() {
            assert!(
                !matches!(entry.status, ImplStatus::Deferred { owner } if owner.is_empty()),
                "{} defers without an owner",
                entry.name
            );
        }
    }

    #[test]
    fn every_gate_resolves_onto_the_real_permission_matrix() {
        // The mapping the ADR's Decision 3 table states, checked against
        // `PluginCapability::permission()` rather than restated by hand.
        let cases = [
            (PluginCapability::TicketsRead, Module::Tickets, Action::View),
            (PluginCapability::ClientsRead, Module::Clients, Action::View),
            (
                PluginCapability::AutomationReads,
                Module::Plugins,
                Action::View,
            ),
            (
                PluginCapability::AutomationActions,
                Module::Tickets,
                Action::Edit,
            ),
            (PluginCapability::EventsEmit, Module::Plugins, Action::View),
        ];
        for (capability, module, action) in cases {
            assert_eq!(
                capability.permission(),
                Permission::Matrix { module, action },
                "{} maps onto the matrix",
                capability.as_str()
            );
        }
    }

    #[test]
    fn an_ungranted_capability_refuses_even_when_the_body_is_deferred() {
        let mut harness = Harness::new(state());
        let (ns, ns_len) = harness.put_str("cache");
        let (key, key_len) = harness.put_str("k");
        // kv_* carries no capability, so the deferred body is what refuses.
        assert_eq!(
            harness.call(
                "kv_get",
                &[ns as i32, ns_len as i32, key as i32, key_len as i32]
            ),
            Err(Refusal::NotImplemented)
        );

        // emit_event without the grant refuses with the capability, not the
        // stub's not_implemented.
        let (event, event_len) = harness.put_str("ticket.escalated");
        let (payload, payload_len) = harness.put_str("{}");
        assert_eq!(
            harness.call(
                "emit_event",
                &[
                    event as i32,
                    event_len as i32,
                    payload as i32,
                    payload_len as i32
                ]
            ),
            Err(Refusal::CapabilityNotGranted)
        );
    }

    #[test]
    fn read_model_query_resolves_a_capability_per_query_id() {
        let mut harness = Harness::new(state());
        let (params, params_len) = harness.put_str("{\"id\":\"01\"}");

        // Unknown id refuses before any capability question.
        assert_eq!(
            harness.call("read_model_query", &[999, params as i32, params_len as i32]),
            Err(Refusal::QueryUnknown)
        );
        // ticket.by_id needs tickets.read.
        assert_eq!(
            harness.call("read_model_query", &[1, params as i32, params_len as i32]),
            Err(Refusal::CapabilityNotGranted)
        );
        harness.state_mut().grant(PluginCapability::TicketsWrite);
        assert_eq!(
            harness.call("read_model_query", &[1, params as i32, params_len as i32]),
            Err(Refusal::CapabilityNotGranted),
            "tickets.write must not stand in for tickets.read"
        );
        harness.state_mut().grant(PluginCapability::TicketsRead);
        assert_eq!(
            harness.call("read_model_query", &[1, params as i32, params_len as i32]),
            Err(Refusal::NotImplemented),
            "the gate passes and the deferred body refuses; it never fakes a row"
        );
        // client.by_id needs clients.read, a different grant.
        assert_eq!(
            harness.call("read_model_query", &[3, params as i32, params_len as i32]),
            Err(Refusal::CapabilityNotGranted)
        );
    }

    #[test]
    fn http_reads_and_writes_need_their_own_capability() {
        let mut harness = Harness::new(state());
        let (profile, profile_len) = harness.put_str("telemetry");
        let (request, request_len) =
            harness.put_str(r#"{"method":"GET","url":"https://api.example.com/v1/tickets"}"#);
        let args = [
            profile as i32,
            profile_len as i32,
            request as i32,
            request_len as i32,
        ];

        // No capability at all.
        assert_eq!(
            harness.call("http_request", &args),
            Err(Refusal::CapabilityNotGranted)
        );

        // A read grant, still no consent for the profile.
        harness.state_mut().grant(PluginCapability::AutomationReads);
        assert_eq!(
            harness.call("http_request", &args),
            Err(Refusal::EgressProfileMissing)
        );
        // W53-7: the consent names a host that never resolves, so the gate
        // passes and the transport refuses at resolution. No network, and the
        // refusal proves the handler ran rather than the stub.
        harness
            .state_mut()
            .consent_egress("telemetry", vec!["https://no-such-host.invalid/v1".into()]);
        let (unresolvable, unresolvable_len) =
            harness.put_str(r#"{"method":"GET","url":"https://no-such-host.invalid/v1/tickets"}"#);
        assert_eq!(
            harness.call(
                "http_request",
                &[
                    profile as i32,
                    profile_len as i32,
                    unresolvable as i32,
                    unresolvable_len as i32
                ]
            ),
            Err(Refusal::EgressHostDenied),
            "a consented read passes the gate and reaches the transport"
        );

        // A write under only the read grant is refused by the capability, not
        // by the egress profile.
        let (write, write_len) =
            harness.put_str(r#"{"method":"POST","url":"https://api.example.com/v1/tickets"}"#);
        assert_eq!(
            harness.call(
                "http_request",
                &[
                    profile as i32,
                    profile_len as i32,
                    write as i32,
                    write_len as i32
                ]
            ),
            Err(Refusal::CapabilityNotGranted)
        );
        harness
            .state_mut()
            .grant(PluginCapability::AutomationActions);
        let (write, write_len) =
            harness.put_str(r#"{"method":"POST","url":"https://no-such-host.invalid/v1/tickets"}"#);
        assert_eq!(
            harness.call(
                "http_request",
                &[
                    profile as i32,
                    profile_len as i32,
                    write as i32,
                    write_len as i32
                ]
            ),
            Err(Refusal::EgressHostDenied)
        );
    }

    #[test]
    fn http_egress_refuses_an_unconsented_host_and_scheme() {
        let mut harness = Harness::new(state());
        harness
            .state_mut()
            .grant(PluginCapability::AutomationReads)
            .consent_egress("telemetry", vec!["https://api.example.com/v1".into()]);
        let (profile, profile_len) = harness.put_str("telemetry");

        let (bad_host, bad_host_len) =
            harness.put_str(r#"{"method":"GET","url":"https://evil.example.net/v1"}"#);
        assert_eq!(
            harness.call(
                "http_request",
                &[
                    profile as i32,
                    profile_len as i32,
                    bad_host as i32,
                    bad_host_len as i32
                ]
            ),
            Err(Refusal::EgressHostDenied)
        );

        let (bad_scheme, bad_scheme_len) =
            harness.put_str(r#"{"method":"GET","url":"http://api.example.com/v1"}"#);
        assert_eq!(
            harness.call(
                "http_request",
                &[
                    profile as i32,
                    profile_len as i32,
                    bad_scheme as i32,
                    bad_scheme_len as i32
                ]
            ),
            Err(Refusal::EgressSchemeDenied)
        );
    }

    #[test]
    fn emit_event_requires_the_grant_and_the_declaration() {
        let mut harness = Harness::new(state());
        harness
            .state_mut()
            .grant(PluginCapability::EventsEmit)
            .declare_event("ticket.escalated");
        let (event, event_len) = harness.put_str("ticket.escalated");
        let (payload, payload_len) = harness.put_str("{}");
        assert_eq!(
            harness.call(
                "emit_event",
                &[
                    event as i32,
                    event_len as i32,
                    payload as i32,
                    payload_len as i32
                ]
            ),
            Err(Refusal::NotImplemented),
            "granted and declared: the gate passes, the data-plane body is deferred"
        );

        let (undeclared, undeclared_len) = harness.put_str("ticket.deleted");
        assert_eq!(
            harness.call(
                "emit_event",
                &[
                    undeclared as i32,
                    undeclared_len as i32,
                    payload as i32,
                    payload_len as i32
                ]
            ),
            Err(Refusal::EventTypeNotDeclared)
        );
    }

    #[test]
    fn log_charges_the_invocation_budget() {
        let mut harness = Harness::new(state());
        let (line, line_len) = harness.put_str("a triage line");
        for _ in 0..abi::MAX_LOG_LINES {
            assert_eq!(
                harness.call("log", &[3, line as i32, line_len as i32]),
                Ok(HostReturn::None)
            );
        }
        assert_eq!(harness.state().logs().len(), abi::MAX_LOG_LINES as usize);
        assert_eq!(
            harness.call("log", &[3, line as i32, line_len as i32]),
            Err(Refusal::LogBudgetExceeded)
        );
        // One line over 4 KiB is refused even with budget left, and an unknown
        // level is a frame error rather than a silent default.
        let mut fresh = Harness::new(state());
        let (big, big_len) = fresh.put_str(&"x".repeat(abi::MAX_LOG_LINE_BYTES + 1));
        assert_eq!(
            fresh.call("log", &[3, big as i32, big_len as i32]),
            Err(Refusal::LogBudgetExceeded)
        );
        let (small, small_len) = fresh.put_str("ok");
        assert_eq!(
            fresh.call("log", &[9, small as i32, small_len as i32]),
            Err(Refusal::FrameInvalid)
        );
    }

    #[test]
    fn now_returns_milliseconds_since_the_epoch() {
        let mut harness = Harness::new(state());
        match harness.call("now", &[]) {
            Ok(HostReturn::Int(millis)) => {
                // 2020-01-01 in milliseconds; anything earlier is a broken clock.
                assert!(millis > 1_577_836_800_000, "clock returned {millis}");
            }
            other => panic!("now must return Int, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_entry_and_a_wrong_arity_are_typed_refusals() {
        let mut harness = Harness::new(state());
        assert_eq!(harness.call("secret_get", &[]), Err(Refusal::UnknownHostFn));
        assert_eq!(harness.call("now", &[0]), Err(Refusal::FrameInvalid));
    }

    #[test]
    fn a_read_outside_guest_memory_is_a_frame_refusal() {
        let mut harness = Harness::new(state());
        assert_eq!(
            harness.call("log", &[3, 0, 0]),
            Ok(HostReturn::None),
            "an empty range at 0 is legal"
        );
        assert_eq!(
            harness.call("log", &[3, 0, 1_000_000]),
            Err(Refusal::FrameInvalid)
        );
        assert_eq!(
            harness.call("log", &[3, u32::MAX as i32, 2]),
            Err(Refusal::FrameInvalid)
        );
    }

    #[test]
    fn a_request_frame_builds_the_state_and_a_bad_one_is_refused() {
        let request = abi::InvocationRequest {
            invocation_id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            install_id: Uuid::now_v7(),
            entry: "plugin.triage-note-action.triage_note".into(),
            payload: b"{}".to_vec(),
            granted: vec!["automation.actions".into(), "events.emit".into()],
            declared_events: vec!["ticket.escalated".into()],
            egress: abi::EgressScopes::new(),
        };
        let state = InvocationState::from_request(&request).expect("valid frame");
        assert!(state.allows(PluginCapability::AutomationActions));
        assert!(state.allows(PluginCapability::EventsEmit));
        assert!(!state.allows(PluginCapability::ClientsRead));
        assert_eq!(state.org_id, request.org_id);
        assert_eq!(state.install_id, request.install_id);
        assert_eq!(state.invocation_id, request.invocation_id);

        let mut corrupt = request.clone();
        corrupt.granted = vec!["not.a.capability".into()];
        assert_eq!(
            InvocationState::from_request(&corrupt).err(),
            Some(Refusal::FrameInvalid)
        );
    }
}
