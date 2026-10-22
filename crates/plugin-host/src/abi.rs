//! The v1 guest ABI of ADR 0023 — the types both sides of the boundary agree
//! on, the packing convention, the limits as numbers, and the closed refusal
//! vocabulary.
//!
//! This module is the CONTRACT. Nothing here executes plugin code, talks to a
//! database or opens a socket: it is the data every executor, every host
//! function and every guest SDK has to speak. Three later lanes (the sandbox,
//! the KV store, egress) build on these types, so a change here is a breaking
//! ABI change and [`PLUGIN_ABI_VERSION`] moves with it.
//!
//! ## The boundary, in one paragraph
//!
//! A guest is a core WASM module with no WASI. It exports three functions —
//! `marula_alloc(i32) -> i32`, `marula_entry(i32, i32) -> i64` and
//! `marula_free(i32, i32)` — and imports one namespace, `marula`, listing the
//! nine entries in [`crate::host_fns::register_all`]. Buffers cross as
//! `(pointer, length)` pairs into guest linear memory. A function returning
//! `i64` returns a **packed** pointer/length ([`pack`]); the payload it points
//! at is a tagged reply frame ([`ok_frame`] / [`err_frame`]). A function
//! returning `u32` returns `0` for success or a [`Refusal`] code outright,
//! which is the cheap path for calls whose answer is only "did it work".
//!
//! ## What this module deliberately does not decide
//!
//! The parent↔worker wire codec. [`InvocationRequest`] and [`InvocationReply`]
//! are serde structs, and the pool lane picks JSON, bincode or anything else
//! that round-trips them. The ABI only fixes the fields the audit row needs
//! (ADR 0023 Decision 7) and the byte caps that bound them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Version of the guest ABI. Bump on any breaking change to the export
/// convention, the import namespace, a frame shape or a refusal string. It is
/// deliberately NOT the manifest interface version
/// (`PLUGIN_RUNTIME_INTERFACE_VERSION`): a manifest can stay valid while the
/// ABI behind it changes, and vice versa.
pub const PLUGIN_ABI_VERSION: u32 = 1;

/// The one import namespace a guest may use. Anything else — WASI included —
/// is a module the host refuses to instantiate.
pub const HOST_NAMESPACE: &str = "marula";

// ---------------------------------------------------------------------------
// Limits (ADR 0023 Decision 4). Numbers live here so the executor, the host
// functions and the guest SDK cannot drift apart.
// ---------------------------------------------------------------------------

/// Input per invocation: the payload handed to `marula_entry`.
pub const MAX_INPUT_BYTES: usize = 1024 * 1024;
/// Output per invocation: the payload `marula_entry` may return.
pub const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
/// One `log` line.
pub const MAX_LOG_LINE_BYTES: usize = 4096;
/// `log` lines per invocation.
pub const MAX_LOG_LINES: u32 = 100;
/// Namespace label a guest may choose: `[a-z0-9_.-]{1,64}`.
pub const KV_NAMESPACE_MAX_BYTES: usize = 64;
/// One KV key.
pub const KV_KEY_MAX_BYTES: usize = 512;
/// One KV value.
pub const KV_VALUE_MAX_BYTES: usize = 1024 * 1024;
/// Keys returned by one `kv_list` page.
pub const KV_LIST_PAGE: u32 = 500;
/// Bytes of an HTTP response the host will hand back.
pub const HTTP_RESPONSE_MAX_BYTES: usize = 2 * 1024 * 1024;
/// Connect + read timeout for one `http_request`, in milliseconds.
pub const HTTP_TIMEOUT_MS: u64 = 5000;
/// Redirects followed. Zero: a redirect is an unconsented host.
pub const HTTP_MAX_REDIRECTS: u32 = 0;

/// Fuel per invocation (W53-3 enforces).
pub const FUEL_PER_INVOCATION: u64 = 200_000_000;
/// Wall clock per invocation (W53-3 enforces with an epoch tick).
pub const WALL_CLOCK_MS: u64 = 250;
/// Epoch tick interval (W53-3).
pub const EPOCH_TICK_MS: u64 = 50;
/// Linear memory per instance (W53-3, `StoreLimits`).
pub const INSTANCE_MEMORY_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Worker resident ceiling (W53-3 restarts above it).
pub const WORKER_RESIDENT_MAX_BYTES: u64 = 256 * 1024 * 1024;

// The three numbers below are the rest of ADR 0023 Decision 4 and live beside the
// ones above so nothing retypes them. They are deliberately absent from
// `packages/plugin-kit/abi/host-abi.v1.json`: that catalogue is the GUEST
// contract, and a guest can neither read nor act on a restart threshold or a
// rate budget. The executor is their only consumer.

/// Worker restart threshold for one invocation. A worker that has not answered
/// within this window is killed and replaced, whatever it is doing.
pub const WORKER_RESTART_THRESHOLD_MS: u64 = 500;
/// Invocations one install may make per minute (the refill of its token bucket).
pub const INVOCATIONS_PER_MINUTE_PER_INSTALL: u32 = 300;
/// Burst allowance of the same bucket — peaks are absorbed, sustained load is
/// not.
pub const INVOCATION_BURST_PER_INSTALL: u32 = 60;

// ---------------------------------------------------------------------------
// The refusal vocabulary. One enum over ADR 0023 Decision 3's static strings,
// plus the ABI-level codes this lane owns. Every value has a stable wire
// string (audit rows, API envelopes) and a stable numeric code (the `u32`
// return path). Codes are append-only: never reuse one.
// ---------------------------------------------------------------------------

/// A typed refusal. The string is what the audit row and the API envelope
/// carry; the code is what a `u32`-returning host function hands the guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Refusal {
    // -- ADR 0023 Decision 3: per-function refusals -----------------------
    /// `log` went over 4 KiB per line or 100 lines this invocation.
    LogBudgetExceeded,
    /// The KV namespace label is not a legal `[a-z0-9_.-]{1,64}` label.
    KvNamespaceDenied,
    /// The install's KV byte or key quota would be exceeded (W53-6 raises it).
    KvQuotaExceeded,
    /// A KV value above [`KV_VALUE_MAX_BYTES`].
    KvValueTooLarge,
    /// The named egress profile is not in the install's consented profile.
    EgressProfileMissing,
    /// The request host (or host+path) is not in the profile's allowlist.
    EgressHostDenied,
    /// The request scheme is not `https`.
    EgressSchemeDenied,
    /// The response is above [`HTTP_RESPONSE_MAX_BYTES`].
    EgressResponseTooLarge,
    /// `emit_event` named a type the manifest does not declare.
    EventTypeNotDeclared,
    /// `read_model_query` named a query id the ABI does not define.
    QueryUnknown,
    /// The capability behind the gate is not in the install's grants.
    CapabilityNotGranted,
    /// The worker died mid-call; the parent writes the row (Decision 7).
    WorkerFailed,

    // -- ABI-level refusals (W53-2) ---------------------------------------
    /// The entry exists but its implementation belongs to a later lane. Never
    /// faked, never a silent `Ok`.
    NotImplemented,
    /// The guest called an entry that is not in the `marula` namespace.
    UnknownHostFn,
    /// A frame, an argument word or a memory range did not parse.
    FrameInvalid,
    /// The invocation payload is above [`MAX_INPUT_BYTES`].
    InputTooLarge,
    /// The guest's reply is above [`MAX_OUTPUT_BYTES`].
    OutputTooLarge,

    // -- Executor refusals (W53-3). None of these can arrive in-band: they are
    // decided by the sandbox or the pool around the guest, never by a host
    // function, so a guest SDK sees them only through the audit row. ---------
    /// The invocation consumed its [`FUEL_PER_INVOCATION`] budget.
    FuelExhausted,
    /// The invocation reached the wall-clock deadline ([`WALL_CLOCK_MS`])
    /// before it ran out of fuel. The worker survived; the invocation did not.
    WallClockExceeded,
    /// The guest trapped: a panic, `unreachable`, an out-of-bounds access, a
    /// stack overflow, or a memory growth past [`INSTANCE_MEMORY_MAX_BYTES`].
    /// The worker survived.
    GuestTrap,
    /// The install is quarantined after repeated worker failures and an
    /// operator has not cleared it.
    Quarantined,
    /// The install is over its invocation rate ([`INVOCATIONS_PER_MINUTE_PER_INSTALL`],
    /// burst [`INVOCATION_BURST_PER_INSTALL`]).
    RateLimited,
    /// No worker, org or install slot came free inside the wait budget.
    ConcurrencyLimited,
    /// Plugin execution is admin-disabled on this deployment. `enabled()` on
    /// the runtime is the outermost gate and this is what it refuses with.
    RuntimeDisabled,
}

impl Refusal {
    /// Every refusal, in code order.
    pub const ALL: [Refusal; 24] = [
        Refusal::CapabilityNotGranted,
        Refusal::LogBudgetExceeded,
        Refusal::KvNamespaceDenied,
        Refusal::KvQuotaExceeded,
        Refusal::KvValueTooLarge,
        Refusal::EgressProfileMissing,
        Refusal::EgressHostDenied,
        Refusal::EgressSchemeDenied,
        Refusal::EgressResponseTooLarge,
        Refusal::EventTypeNotDeclared,
        Refusal::QueryUnknown,
        Refusal::WorkerFailed,
        Refusal::NotImplemented,
        Refusal::UnknownHostFn,
        Refusal::FrameInvalid,
        Refusal::InputTooLarge,
        Refusal::OutputTooLarge,
        Refusal::FuelExhausted,
        Refusal::WallClockExceeded,
        Refusal::GuestTrap,
        Refusal::Quarantined,
        Refusal::RateLimited,
        Refusal::ConcurrencyLimited,
        Refusal::RuntimeDisabled,
    ];

    /// The static string an audit row and an API envelope carry. Never prose
    /// to parse.
    pub const fn as_str(self) -> &'static str {
        match self {
            Refusal::CapabilityNotGranted => "plugin_capability_not_granted",
            Refusal::LogBudgetExceeded => "plugin_log_budget_exceeded",
            Refusal::KvNamespaceDenied => "plugin_kv_namespace_denied",
            Refusal::KvQuotaExceeded => "plugin_kv_quota_exceeded",
            Refusal::KvValueTooLarge => "plugin_kv_value_too_large",
            Refusal::EgressProfileMissing => "plugin_egress_profile_missing",
            Refusal::EgressHostDenied => "plugin_egress_host_denied",
            Refusal::EgressSchemeDenied => "plugin_egress_scheme_denied",
            Refusal::EgressResponseTooLarge => "plugin_egress_response_too_large",
            Refusal::EventTypeNotDeclared => "plugin_event_type_not_declared",
            Refusal::QueryUnknown => "plugin_query_unknown",
            Refusal::WorkerFailed => "plugin_worker_failed",
            Refusal::NotImplemented => "plugin_not_implemented",
            Refusal::UnknownHostFn => "plugin_unknown_host_fn",
            Refusal::FrameInvalid => "plugin_abi_frame_invalid",
            Refusal::InputTooLarge => "plugin_input_too_large",
            Refusal::OutputTooLarge => "plugin_output_too_large",
            Refusal::FuelExhausted => "plugin_fuel_exhausted",
            Refusal::WallClockExceeded => "plugin_wall_clock_exceeded",
            Refusal::GuestTrap => "plugin_guest_trap",
            Refusal::Quarantined => "plugin_install_quarantined",
            Refusal::RateLimited => "plugin_rate_limited",
            Refusal::ConcurrencyLimited => "plugin_concurrency_limited",
            Refusal::RuntimeDisabled => "plugin_runtime_disabled",
        }
    }

    /// The in-band numeric code. `0` is reserved for success and no refusal
    /// may ever take it. Stable across releases; append only.
    pub const fn code(self) -> u32 {
        match self {
            Refusal::CapabilityNotGranted => 1,
            Refusal::LogBudgetExceeded => 2,
            Refusal::KvNamespaceDenied => 3,
            Refusal::KvQuotaExceeded => 4,
            Refusal::KvValueTooLarge => 5,
            Refusal::EgressProfileMissing => 6,
            Refusal::EgressHostDenied => 7,
            Refusal::EgressSchemeDenied => 8,
            Refusal::EgressResponseTooLarge => 9,
            Refusal::EventTypeNotDeclared => 10,
            Refusal::QueryUnknown => 11,
            Refusal::WorkerFailed => 12,
            Refusal::NotImplemented => 13,
            Refusal::UnknownHostFn => 14,
            Refusal::FrameInvalid => 15,
            Refusal::InputTooLarge => 16,
            Refusal::OutputTooLarge => 17,
            Refusal::FuelExhausted => 18,
            Refusal::WallClockExceeded => 19,
            Refusal::GuestTrap => 20,
            Refusal::Quarantined => 21,
            Refusal::RateLimited => 22,
            Refusal::ConcurrencyLimited => 23,
            Refusal::RuntimeDisabled => 24,
        }
    }

    /// Inverse of [`Refusal::as_str`].
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|r| r.as_str() == value)
    }

    /// Inverse of [`Refusal::code`].
    pub fn from_code(code: u32) -> Option<Self> {
        Self::ALL.iter().copied().find(|r| r.code() == code)
    }
}

// A refusal is its static string on the wire — the audit row's `reason`, the
// API envelope's code, the worker frame's field. Never the variant name: an
// audit row has to be readable by a tool that does not link this crate.
impl Serialize for Refusal {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Refusal {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Refusal::parse(&value)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown refusal {value:?}")))
    }
}

// ---------------------------------------------------------------------------
// The packing convention: two 32-bit words in one 64-bit return value.
// ---------------------------------------------------------------------------

/// Pack `(pointer, length)` into the `i64` a data-returning host function
/// hands back. The high half is the pointer, the low half the length.
pub const fn pack(ptr: u32, len: u32) -> i64 {
    (((ptr as u64) << 32) | (len as u64)) as i64
}

/// Inverse of [`pack`].
pub const fn unpack(packed: i64) -> (u32, u32) {
    let bits = packed as u64;
    ((bits >> 32) as u32, bits as u32)
}

// ---------------------------------------------------------------------------
// The guest-side reply frame: one tag byte, then either the payload or the
// refusal code. Binary-safe (no base64, no JSON escaping) and one byte of
// overhead on the hot path.
// ---------------------------------------------------------------------------

/// Tag byte: the frame carries a payload.
pub const REPLY_TAG_OK: u8 = 0;
/// Tag byte: the frame carries a refusal code.
pub const REPLY_TAG_ERR: u8 = 1;

/// An empty success frame.
pub fn empty_frame() -> Vec<u8> {
    vec![REPLY_TAG_OK]
}

/// A success frame wrapping `data`.
pub fn ok_frame(data: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(data.len() + 1);
    frame.push(REPLY_TAG_OK);
    frame.extend_from_slice(data);
    frame
}

/// A refusal frame for a `i64`-returning call: `[1, code_le_u32]`.
pub fn err_frame(refusal: Refusal) -> Vec<u8> {
    let mut frame = Vec::with_capacity(5);
    frame.push(REPLY_TAG_ERR);
    frame.extend_from_slice(&refusal.code().to_le_bytes());
    frame
}

/// Decode a reply frame: `Ok(payload)` or the typed refusal it carries.
pub fn parse_frame(frame: &[u8]) -> Result<&[u8], Refusal> {
    match frame.split_first() {
        Some((&REPLY_TAG_OK, payload)) => Ok(payload),
        Some((&REPLY_TAG_ERR, code)) => {
            if code.len() != 4 {
                return Err(Refusal::FrameInvalid);
            }
            let code = u32::from_le_bytes([code[0], code[1], code[2], code[3]]);
            Err(Refusal::from_code(code).unwrap_or(Refusal::FrameInvalid))
        }
        _ => Err(Refusal::FrameInvalid),
    }
}

// ---------------------------------------------------------------------------
// Invocation frames (parent -> worker -> parent). ADR 0023 Decision 7: the
// PARENT writes the audit row, so the reply must carry the invocation id, the
// fuel figure and the duration even when the call is a refusal.
// ---------------------------------------------------------------------------

/// What a worker was asked to run. The parent has already resolved every
/// authorization gate; the worker re-checks nothing it cannot prove, it
/// enforces limits and runs the module. Every field is therefore a value the
/// parent decided, not a value the guest supplied — except `payload`, which is
/// the guest's input and is bounded here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationRequest {
    /// The audit correlation id. The reply must echo it.
    pub invocation_id: Uuid,
    pub org_id: Uuid,
    pub install_id: Uuid,
    /// The surface entry being invoked (`plugin.<slug>.<id>`).
    pub entry: String,
    /// The task input, JSON by convention. Capped at [`MAX_INPUT_BYTES`].
    pub payload: Vec<u8>,
    /// Capability keys the parent resolved and granted for this call. The
    /// worker re-parses them so a corrupt frame cannot widen a grant.
    pub granted: Vec<String>,
    /// Event types the manifest declares (the `emit_event` declaration gate).
    #[serde(default)]
    pub declared_events: Vec<String>,
    /// The install's consented egress profile: profile name -> allowlist.
    #[serde(default)]
    pub egress: EgressScopes,
}

impl InvocationRequest {
    /// Validates the frame a worker received. A frame that fails this is
    /// refused as a whole; no field is repaired, defaulted or ignored.
    pub fn validate(&self) -> Result<(), Refusal> {
        if self.entry.trim().is_empty() {
            return Err(Refusal::FrameInvalid);
        }
        if self.payload.len() > MAX_INPUT_BYTES {
            return Err(Refusal::InputTooLarge);
        }
        if self
            .granted
            .iter()
            .any(|key| core_domain::entities::PluginCapability::parse(key).is_none())
        {
            return Err(Refusal::FrameInvalid);
        }
        Ok(())
    }
}

/// The outcome word of an invocation. The three values ADR 0023 Decision 7
/// writes into `audit_events.detail.outcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationOutcome {
    Ok,
    Refused,
    WorkerFailed,
}

impl InvocationOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            InvocationOutcome::Ok => "ok",
            InvocationOutcome::Refused => "refused",
            InvocationOutcome::WorkerFailed => "worker_failed",
        }
    }
}

/// What the worker hands back. `fuel_used` is `None` exactly when the worker
/// died (there is no fuel figure to report), and `duration_us` is always
/// present so the audit row is complete for a failure as well as a success.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationReply {
    pub invocation_id: Uuid,
    pub outcome: InvocationOutcome,
    /// Present for `refused` and `worker_failed`; absent for `ok`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<Refusal>,
    /// Present for `ok` when the entry returns a payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Vec<u8>>,
    /// `None` when the worker died mid-call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuel_used: Option<u64>,
    pub duration_us: u64,
}

impl InvocationReply {
    pub fn ok(
        invocation_id: Uuid,
        output: Option<Vec<u8>>,
        fuel_used: u64,
        duration_us: u64,
    ) -> Self {
        InvocationReply {
            invocation_id,
            outcome: InvocationOutcome::Ok,
            reason: None,
            output,
            fuel_used: Some(fuel_used),
            duration_us,
        }
    }

    pub fn refused(invocation_id: Uuid, reason: Refusal, fuel_used: u64, duration_us: u64) -> Self {
        InvocationReply {
            invocation_id,
            outcome: InvocationOutcome::Refused,
            reason: Some(reason),
            output: None,
            fuel_used: Some(fuel_used),
            duration_us,
        }
    }

    /// The row a dead worker leaves: the parent fills the duration it measured
    /// and there is no fuel figure to report.
    pub fn worker_failed(invocation_id: Uuid, duration_us: u64) -> Self {
        InvocationReply {
            invocation_id,
            outcome: InvocationOutcome::WorkerFailed,
            reason: Some(Refusal::WorkerFailed),
            output: None,
            fuel_used: None,
            duration_us,
        }
    }

    /// Checks the invariants the parent relies on before it writes the row.
    pub fn validate(&self) -> Result<(), Refusal> {
        match self.outcome {
            InvocationOutcome::Ok => {
                if self.reason.is_some() {
                    return Err(Refusal::FrameInvalid);
                }
                if self.fuel_used.is_none() {
                    return Err(Refusal::FrameInvalid);
                }
            }
            InvocationOutcome::Refused => {
                if self.reason.is_none() || self.fuel_used.is_none() {
                    return Err(Refusal::FrameInvalid);
                }
            }
            InvocationOutcome::WorkerFailed => {
                if self.reason != Some(Refusal::WorkerFailed) || self.fuel_used.is_some() {
                    return Err(Refusal::FrameInvalid);
                }
            }
        }
        if let Some(output) = &self.output {
            if output.len() > MAX_OUTPUT_BYTES {
                return Err(Refusal::OutputTooLarge);
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Egress (ADR 0023 Decision 3, `http_request`). The consent shape is the
// manifest's relaxed `connector_profile.outbound_scopes`: a map of profile
// name -> allowlist. Authorization against it lives here so the `Egress` gate
// can run before any transport code does.
// ---------------------------------------------------------------------------

/// Profile name -> the scopes consented for it. Each scope is
/// `https://host[:port][/path-prefix]`. The map shape is what makes
/// `plugin_egress_profile_missing` meaningful: a guest names a profile, and a
/// name the install did not consent is a refusal rather than an empty list.
pub type EgressScopes = BTreeMap<String, Vec<String>>;

/// One `http_request` payload, as JSON in guest memory. Kept to text here:
/// v1 bodies are UTF-8 and the shape can gain a `body_b64` additively.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

impl HttpRequest {
    /// A write is anything that is not `GET`/`HEAD`/`OPTIONS`. The gate reads
    /// this from the PARSED request, never from a default or a claim.
    pub fn is_write(&self) -> bool {
        !matches!(
            self.method.to_ascii_uppercase().as_str(),
            "GET" | "HEAD" | "OPTIONS"
        )
    }
}

/// One `http_request` reply payload, as JSON. `docs` carry the decision in the
/// ABI catalogue; W53-7 fills it from a real response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: String,
}

/// Authorizes one outbound request against a consented profile. The
/// authorization half of egress; the transport half (DNS, SSRF, redirects) is
/// W53-7's and runs after this returns `Ok`.
///
/// Refusals, in order: the profile is not consented, the scheme is not
/// `https`, the authority is not in the allowlist, the path is not under the
/// scope's prefix.
pub fn authorize_egress(scopes: &EgressScopes, profile: &str, url: &str) -> Result<(), Refusal> {
    let allowlist = scopes
        .get(profile)
        .filter(|list| !list.is_empty())
        .ok_or(Refusal::EgressProfileMissing)?;

    let rest = url
        .strip_prefix("https://")
        .ok_or(Refusal::EgressSchemeDenied)?;
    let (authority, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };
    if authority.is_empty() || authority.contains('@') {
        return Err(Refusal::EgressHostDenied);
    }
    // A query or fragment on the request URL never reaches the host match.
    let path = path.split(['?', '#']).next().unwrap_or("/");

    for scope in allowlist {
        let Some(scope_rest) = scope.strip_prefix("https://") else {
            continue;
        };
        let (scope_authority, scope_path) = match scope_rest.find('/') {
            Some(idx) => (&scope_rest[..idx], &scope_rest[idx..]),
            None => (scope_rest, "/"),
        };
        if !scope_authority.eq_ignore_ascii_case(authority) {
            continue;
        }
        if scope_path == "/" || path == scope_path || path.starts_with(&format!("{scope_path}/")) {
            return Ok(());
        }
    }
    Err(Refusal::EgressHostDenied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_and_unpack_round_trip_at_the_edges() {
        for (ptr, len) in [
            (0u32, 0u32),
            (1, 1),
            (0xffff_ffff, 0xffff_ffff),
            (65_536, 1024),
        ] {
            assert_eq!(unpack(pack(ptr, len)), (ptr, len));
        }
        assert_eq!(pack(2, 3), (2i64 << 32) | 3);
    }

    #[test]
    fn frames_carry_a_payload_or_a_typed_refusal() {
        assert_eq!(parse_frame(&ok_frame(b"hello")), Ok(&b"hello"[..]));
        assert_eq!(parse_frame(&empty_frame()), Ok(&b""[..]));

        let err = err_frame(Refusal::KvNamespaceDenied);
        assert_eq!(parse_frame(&err), Err(Refusal::KvNamespaceDenied));
        assert_eq!(err.len(), 5);
        assert_eq!(err[0], REPLY_TAG_ERR);
        assert_eq!(parse_frame(&[]), Err(Refusal::FrameInvalid));
        assert_eq!(parse_frame(&[9, 0]), Err(Refusal::FrameInvalid));
        assert_eq!(
            parse_frame(&[REPLY_TAG_ERR, 1, 2]),
            Err(Refusal::FrameInvalid)
        );
    }

    #[test]
    fn every_refusal_has_a_unique_string_and_code_and_round_trips() {
        let mut strings = std::collections::HashSet::new();
        let mut codes = std::collections::HashSet::new();
        for refusal in Refusal::ALL {
            assert!(
                strings.insert(refusal.as_str()),
                "{:?} reuses a string",
                refusal
            );
            assert!(codes.insert(refusal.code()), "{:?} reuses a code", refusal);
            assert_ne!(refusal.code(), 0, "{:?} took the success code", refusal);
            assert!(refusal.as_str().starts_with("plugin_"));
            assert_eq!(Refusal::parse(refusal.as_str()), Some(refusal));
            assert_eq!(Refusal::from_code(refusal.code()), Some(refusal));
        }
        assert_eq!(Refusal::ALL.len(), 24);
        assert_eq!(Refusal::from_code(0), None);
        assert_eq!(Refusal::from_code(99), None);
    }

    #[test]
    fn the_shared_denial_string_matches_the_authorization_module() {
        assert_eq!(
            Refusal::CapabilityNotGranted.as_str(),
            crate::denial_reasons::CAPABILITY_NOT_GRANTED
        );
    }

    #[test]
    fn a_request_frame_carries_the_bounded_payload_and_validates() {
        let request = InvocationRequest {
            invocation_id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            install_id: Uuid::now_v7(),
            entry: "plugin.escalation-widget.escalation_policy".into(),
            payload: b"{}".to_vec(),
            granted: vec!["ui.widgets".into()],
            declared_events: vec![],
            egress: EgressScopes::new(),
        };
        assert_eq!(request.validate(), Ok(()));

        let mut oversized = request.clone();
        oversized.payload = vec![0; MAX_INPUT_BYTES + 1];
        assert_eq!(oversized.validate(), Err(Refusal::InputTooLarge));

        let mut unknown_grant = request.clone();
        unknown_grant.granted = vec!["sudo.everything".into()];
        assert_eq!(unknown_grant.validate(), Err(Refusal::FrameInvalid));

        let mut blank = request.clone();
        blank.entry = "   ".into();
        assert_eq!(blank.validate(), Err(Refusal::FrameInvalid));
    }

    #[test]
    fn a_worker_failure_reply_still_identifies_the_invocation() {
        let id = Uuid::now_v7();
        let reply = InvocationReply::worker_failed(id, 700);
        assert_eq!(reply.invocation_id, id);
        assert_eq!(reply.outcome, InvocationOutcome::WorkerFailed);
        assert_eq!(reply.outcome.as_str(), "worker_failed");
        assert_eq!(reply.reason, Some(Refusal::WorkerFailed));
        assert_eq!(reply.fuel_used, None);
        assert_eq!(reply.validate(), Ok(()));

        assert_eq!(
            InvocationReply::ok(id, Some(b"x".to_vec()), 38_909, 5).validate(),
            Ok(())
        );
        assert_eq!(
            InvocationReply::refused(id, Refusal::EventTypeNotDeclared, 12, 3).validate(),
            Ok(())
        );
    }

    #[test]
    fn a_reply_that_contradicts_its_outcome_is_rejected() {
        let id = Uuid::now_v7();
        let mut contradictory = InvocationReply::ok(id, None, 1, 1);
        contradictory.fuel_used = None;
        assert_eq!(contradictory.validate(), Err(Refusal::FrameInvalid));

        let mut dead_with_fuel = InvocationReply::worker_failed(id, 1);
        dead_with_fuel.fuel_used = Some(10);
        assert_eq!(dead_with_fuel.validate(), Err(Refusal::FrameInvalid));

        let mut echoing_wrong = InvocationReply::refused(id, Refusal::FrameInvalid, 1, 1);
        echoing_wrong.reason = None;
        assert_eq!(echoing_wrong.validate(), Err(Refusal::FrameInvalid));
    }

    #[test]
    fn http_methods_are_classified_as_reads_and_writes() {
        let mut req = HttpRequest {
            method: "GET".into(),
            url: "https://api.example.com/v1/tickets".into(),
            headers: BTreeMap::new(),
            body: None,
        };
        assert!(!req.is_write());
        for write in ["POST", "PUT", "PATCH", "DELETE", "post"] {
            req.method = write.into();
            assert!(req.is_write(), "{write} must be a write");
        }
        for read in ["HEAD", "OPTIONS", "get"] {
            req.method = read.into();
            assert!(!req.is_write(), "{read} must be a read");
        }
    }

    fn egress() -> EgressScopes {
        let mut scopes = EgressScopes::new();
        scopes.insert(
            "telemetry".into(),
            vec![
                "https://api.example.com/v1".into(),
                "https://billing.example.com".into(),
            ],
        );
        scopes
    }

    #[test]
    fn egress_authorization_checks_profile_scheme_host_and_prefix() {
        let scopes = egress();
        assert_eq!(
            authorize_egress(&scopes, "telemetry", "https://api.example.com/v1/tickets"),
            Ok(())
        );
        assert_eq!(
            authorize_egress(&scopes, "telemetry", "https://billing.example.com/any"),
            Ok(())
        );
        // Unknown profile, empty profile, non-https scheme.
        assert_eq!(
            authorize_egress(&scopes, "absent", "https://api.example.com/v1"),
            Err(Refusal::EgressProfileMissing)
        );
        assert_eq!(
            authorize_egress(&scopes, "telemetry", "http://api.example.com/v1"),
            Err(Refusal::EgressSchemeDenied)
        );
        // Host outside the allowlist, prefix outside the scope, credentials.
        assert_eq!(
            authorize_egress(&scopes, "telemetry", "https://evil.example.net/v1"),
            Err(Refusal::EgressHostDenied)
        );
        assert_eq!(
            authorize_egress(&scopes, "telemetry", "https://api.example.com/v2/tickets"),
            Err(Refusal::EgressHostDenied)
        );
        assert_eq!(
            authorize_egress(&scopes, "telemetry", "https://user:pw@api.example.com/v1"),
            Err(Refusal::EgressHostDenied)
        );
        // A prefix must match on a path boundary, not on a string prefix.
        assert_eq!(
            authorize_egress(&scopes, "telemetry", "https://api.example.com/v10"),
            Err(Refusal::EgressHostDenied)
        );
        // Query strings do not defeat the path check.
        assert_eq!(
            authorize_egress(&scopes, "telemetry", "https://api.example.com/v1?x=1"),
            Ok(())
        );
        let mut empty = EgressScopes::new();
        empty.insert("telemetry".into(), vec![]);
        assert_eq!(
            authorize_egress(&empty, "telemetry", "https://api.example.com/v1"),
            Err(Refusal::EgressProfileMissing)
        );
    }
}
