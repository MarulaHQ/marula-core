//! The parent ↔ worker framing protocol (ADR 0023 Decisions 2 and 7).
//!
//! One request frame in, one reply frame out, one invocation in flight per
//! worker. The transport is the worker's own stdin/stdout pair: **stdout is
//! protocol only**, every diagnostic goes to stderr. A frame is a 4-byte
//! little-endian body length followed by that many bytes, and a body is a
//! fixed header (magic, protocol version, kind) plus the fields below.
//!
//! ```text
//! request  := u32 len, u32 magic, u8 version, u8 kind=1,
//!             uuid invocation_id, uuid org_id, uuid install_id,
//!             str entry, bytes payload,
//!             str[] granted, str[] declared_events,
//!             (str name, str[] scopes)[] egress,
//!             str digest, u8 has_module [bytes module]
//!
//! hello    := u32 len, u32 magic, u8 version, u8 kind=3, u32 worker_pid
//!             (written by the worker once, before it reads anything)
//!
//! reply    := u32 len, u32 magic, u8 version, u8 kind=2,
//!             uuid invocation_id, u8 outcome,
//!             u8 has_reason [str reason], u8 has_output [bytes output],
//!             u8 has_fuel [u64 fuel_used], u64 duration_us,
//!             u8 module_cached, u8 retire, u8 has_evicted [str digest]
//!
//! host     := u32 len, u32 magic, u8 version, u8 kind=4,
//!             uuid invocation_id, u64 call_id, u8 verb, <the verb's arguments>
//!
//! host reply := u32 len, u32 magic, u8 version, u8 kind=5,
//!             uuid invocation_id, u64 call_id, u8 status,
//!             u8 shape [the shape's own fields]
//! ```
//!
//! The two host-call frames are the middle of one invocation: the worker writes
//! a `host` frame, blocks on its stdin for the matching `host reply`, and hands
//! the answer to the guest. They exist because a host function runs inside a
//! wasmtime `Caller` in the worker process, synchronously, where there is no
//! database and no runtime — so the worker asks the parent instead of the parent
//! giving the worker a connection (ADR 0023 Decision 2: the worker authorizes
//! nothing, so it may not hold a tenant's handle either).
//!
//! **A host-call frame carries no authority.** It names an invocation (a
//! correlator), a call number (a correlator), a verb and that verb's arguments.
//! It does not name an org, an install or a capability: the parent answers from
//! its own record of the invocation it dispatched. See
//! [`HostCallOp::verb`] for the verbs and `runtime::HostCallService` for the
//! side that serves them.
//!
//! `str` is `u32 len` plus that many UTF-8 bytes and `bytes` is `u32 len` plus
//! that many bytes. Counts are `u16` and every length is checked against the
//! bytes actually present before anything is allocated, so a truncated or
//! lying frame is a typed [`WireError`] rather than a panic or a half-built
//! request.
//!
//! ## Why a hand-rolled codec rather than serde
//!
//! The two frames carry an invocation payload of up to 1 MiB and, on first use,
//! a module of up to 4 MiB. JSON would either quadruple those bytes (`Vec<u8>`
//! serialises as an array of numbers) or need a second dependency to keep them
//! binary, and the per-invocation encode/decode would land on a path the ADR
//! budgets in microseconds. This codec is about the size of the serde
//! attributes it replaces, has no version skew of its own, and is exhaustively
//! tested against truncation in [`tests`].
//!
//! ## What a forged frame can and cannot do
//!
//! The worker never authorizes anything: a request frame's `granted`,
//! `declared_events` and `egress` are the parent's resolved answer, and the
//! worker re-validates them into an [`crate::host_fns::InvocationState`] that
//! starts empty. A frame whose `granted` names a capability the closed
//! vocabulary does not define is refused whole by
//! [`abi::InvocationRequest::validate`], which the worker calls before it
//! compiles a module. Nothing in this module widens a grant, and the reply
//! direction carries no authority at all — the parent validates the reply with
//! [`abi::InvocationReply::validate`] and treats a failure as a worker failure.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::abi::{self, InvocationOutcome, InvocationReply, InvocationRequest, Refusal};

/// `"MRPW"` — Marula plugin worker. A frame that does not start with it is not
/// our frame, whoever wrote it.
pub const MAGIC: u32 = 0x4D52_5057;
/// Bumped when a field changes shape. The parent refuses to start a worker that
/// speaks another version rather than guessing which fields it may read.
///
/// **2** added the host-call pair. That is a bump rather than a compatible
/// addition on purpose: a v1 worker cannot parse a `host reply`, so a mixed
/// deployment (a v2 core-api in front of a v1 binary) would block a guest's
/// first `kv_get` forever instead of failing. The handshake in
/// `runtime::spawn_worker` is where that mismatch belongs, and it is loud there.
pub const PROTOCOL_VERSION: u8 = 2;

/// The largest module artifact a worker will accept, in bytes. Mirrors the
/// distribution lane's per-member cap
/// (`distribution::MAX_MEMBER_BYTES`): an artifact bigger than a member of a
/// signed bundle cannot be one this deployment admitted.
pub const MAX_MODULE_BYTES: usize = 4 * 1024 * 1024;

/// The largest body either side will read: a full module, a full payload and
/// room for the metadata beside them.
pub const MAX_FRAME_BYTES: usize = MAX_MODULE_BYTES + abi::MAX_INPUT_BYTES + 64 * 1024;

/// Frame kind byte for a request.
pub const KIND_INVOKE: u8 = 1;
/// Frame kind byte for a reply.
pub const KIND_REPLY: u8 = 2;
/// Frame kind byte for the worker's opening hello. It exists so the parent
/// learns at BOOT that the binary it spawned speaks this protocol, rather than
/// on the first user request.
pub const KIND_HELLO: u8 = 3;
/// Frame kind byte for a host call: the worker asks the parent to do something
/// it cannot do itself (W53-11's bridge). Written on stdout mid-invocation.
pub const KIND_HOST_CALL: u8 = 4;
/// Frame kind byte for a host call's answer: the parent replies on the worker's
/// stdin, and the worker returns it to the guest.
pub const KIND_HOST_REPLY: u8 = 5;

/// The largest value one host call carries either way. The ABI's own cap
/// (`kv_value_bytes`), restated here so a decoder refuses an oversized frame
/// before it allocates the bytes.
pub const MAX_HOST_CALL_VALUE_BYTES: usize = abi::KV_VALUE_MAX_BYTES;
/// The largest label (namespace, key, prefix, cursor) one host call carries.
pub const MAX_HOST_CALL_LABEL_BYTES: usize = abi::KV_KEY_MAX_BYTES;
/// The most keys one `kv_list` reply carries: the ABI's page size.
pub const MAX_HOST_CALL_PAGE: usize = abi::KV_LIST_PAGE as usize;

/// The largest egress profile name an `http_request` frame carries. Mirrors the
/// manifest rule (`PluginManifestV2::validate_outbound_scopes`:
/// `[a-z][a-z0-9_-]{0,63}`), so a name the manifest could not declare is not a
/// name the frame can carry.
pub const MAX_HOST_CALL_PROFILE_BYTES: usize = 64;
/// The largest URL an `http_request` frame carries. The same number the domain
/// guard stops at (`core_domain::egress::MAX_URL_BYTES`), restated here because
/// this module is the framing layer and does not link the guard.
pub const MAX_HOST_CALL_URL_BYTES: usize = core_domain::egress::MAX_URL_BYTES;
/// The most headers one `http_request` carries in either direction.
pub const MAX_HOST_CALL_HEADERS: usize = 64;
/// The largest single header name or value a frame carries. A header the ABI
/// would have refused at the guest edge, arriving in a frame, is refused before
/// it is allocated.
pub const MAX_HOST_CALL_HEADER_BYTES: usize = 4096;
/// The largest HTTP response body a host reply carries. The transport's own cap
/// (`abi::HTTP_RESPONSE_MAX_BYTES`), restated so the decoder refuses before it
/// allocates.
pub const MAX_HOST_CALL_HTTP_BODY_BYTES: usize = abi::HTTP_RESPONSE_MAX_BYTES;

/// A framing failure. Every variant is the parent's or a test's problem, never
/// a guest's: the parent turns all of them into
/// [`InvocationOutcome::WorkerFailed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    /// The body is shorter than the field it claims, or a length overruns it.
    Truncated,
    /// Magic, version or kind byte does not belong to this protocol.
    NotOurFrame,
    /// A string was not UTF-8, a refusal word was not in the vocabulary, an
    /// outcome byte was not one of the three, or a boolean byte was not 0/1.
    Malformed,
    /// A frame the encoder was asked to build, or the decoder was handed, is
    /// above [`MAX_FRAME_BYTES`] (or a module above [`MAX_MODULE_BYTES`]).
    TooLarge,
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            WireError::Truncated => "truncated frame",
            WireError::NotOurFrame => "not a plugin-worker frame",
            WireError::Malformed => "malformed frame",
            WireError::TooLarge => "frame above MAX_FRAME_BYTES",
        })
    }
}

impl std::error::Error for WireError {}

/// One request: the authorized invocation, the digest of the module it runs
/// on, and that module's bytes when the worker does not already have it.
///
/// The digest travels on every frame and the bytes only on the first: a worker
/// restart forgets its cache, so the pool re-sends rather than assuming. The
/// digest is what the worker verifies the bytes against; it is not a hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvokeFrame {
    pub digest: String,
    pub module: Option<Vec<u8>>,
    pub request: InvocationRequest,
}

/// One reply: the audit-ready [`InvocationReply`] and the two facts the pool
/// needs about the worker that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyFrame {
    pub reply: InvocationReply,
    /// The worker holds this invocation's module in its compiled cache, so the
    /// parent may omit the bytes next time. `false` means "send them again";
    /// the pool never assumes otherwise.
    pub module_cached: bool,
    /// The worker is over a resource ceiling and will exit after this reply.
    /// The pool retires the member and spawns a fresh one; the invocation this
    /// reply describes is still complete and valid.
    pub retire: bool,
    /// The digest this reply's module load pushed out of the worker's cache.
    /// The parent drops it from its own "already sent" set, so the next
    /// invocation for it carries the bytes again instead of being refused.
    pub evicted: Option<String>,
}

// ---------------------------------------------------------------------------
// Host calls: the worker asks the parent (W53-11)
// ---------------------------------------------------------------------------

/// The verb a host-call frame names.
///
/// One byte on the wire, and the numbers are the contract:
///
/// ```text
/// 1 kv_get   2 kv_put   3 kv_delete   4 kv_list   5 http_request
/// ```
///
/// `6` and `7` stay reserved for `emit_event` and `read_model_query`. A code no
/// verb has is refused with [`WireError::Malformed`] instead of guessed, which
/// is what lets a later lane add one without changing what today's frames mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostVerb {
    KvGet,
    KvPut,
    KvDelete,
    KvList,
    /// The parent performs the request (Wave 53 lane 12, phase 2). The worker
    /// has no client and no credential: it forwards the guest's request and
    /// hands back the answer.
    HttpRequest,
}

impl HostVerb {
    pub const fn code(self) -> u8 {
        match self {
            HostVerb::KvGet => 1,
            HostVerb::KvPut => 2,
            HostVerb::KvDelete => 3,
            HostVerb::KvList => 4,
            HostVerb::HttpRequest => 5,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            HostVerb::KvGet => "kv_get",
            HostVerb::KvPut => "kv_put",
            HostVerb::KvDelete => "kv_delete",
            HostVerb::KvList => "kv_list",
            HostVerb::HttpRequest => "http_request",
        }
    }

    /// `None` for a code no verb has. A later lane's verb is a later lane's
    /// frame; this decoder does not accept what it cannot serve.
    pub const fn parse(code: u8) -> Option<Self> {
        match code {
            1 => Some(HostVerb::KvGet),
            2 => Some(HostVerb::KvPut),
            3 => Some(HostVerb::KvDelete),
            4 => Some(HostVerb::KvList),
            5 => Some(HostVerb::HttpRequest),
            _ => None,
        }
    }
}

/// One host call, with the verb's arguments and nothing else.
///
/// No org, no install, no capability, no grant: see this module's header. The
/// guest names a LABEL (a namespace and a key); the parent composes the identity
/// from the invocation it dispatched, so the most a forged frame can do is ask
/// about a key inside the install that is already running.
///
/// `HttpRequest` follows the same rule with a different noun: the frame carries
/// the PROFILE the manifest declared and the guest's own request, and nothing
/// that could name a tenant, a vault record or a credential. The parent looks
/// the binding up from its own record of the invocation (W53-12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostCallOp {
    KvGet {
        namespace: String,
        key: String,
    },
    KvPut {
        namespace: String,
        key: String,
        value: Vec<u8>,
        /// The ABI's `kv_put` has no expiry argument (Decision 3), so a guest
        /// cannot set one. The field exists because the store's signature has
        /// one and a restore re-applies an expiry; the host always sends
        /// `None`.
        expires_at: Option<DateTime<Utc>>,
    },
    KvDelete {
        namespace: String,
        key: String,
    },
    KvList {
        namespace: String,
        prefix: String,
        cursor: Option<String>,
    },
    HttpRequest {
        /// The profile the guest named, already checked against the install's
        /// consented `outbound_scopes` by the worker's gate. The parent checks
        /// it again against its own copy, because the worker is the process it
        /// isolates.
        profile: String,
        /// The guest's request, parsed from the ABI frame.
        request: abi::HttpRequest,
    },
}

impl HostCallOp {
    pub const fn verb(&self) -> HostVerb {
        match self {
            HostCallOp::KvGet { .. } => HostVerb::KvGet,
            HostCallOp::KvPut { .. } => HostVerb::KvPut,
            HostCallOp::KvDelete { .. } => HostVerb::KvDelete,
            HostCallOp::KvList { .. } => HostVerb::KvList,
            HostCallOp::HttpRequest { .. } => HostVerb::HttpRequest,
        }
    }
}

/// What a host call answered, when it answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostCallValue {
    /// `kv_put`, `kv_delete`: there is nothing to hand back.
    Unit,
    /// `kv_get` and the key was not there. Absence is not a refusal: the ABI
    /// gives `kv_get` one code and it is for a namespace, not a key.
    Absent,
    /// `kv_get` and the key was there.
    Value(Vec<u8>),
    /// `kv_delete` and whether the key was there. The guest cannot see this
    /// (the ABI's answer is only "did it work"); it is carried because the
    /// [`crate::kv::KvStore`] contract promises it.
    Existed(bool),
    /// One `kv_list` page.
    Page {
        keys: Vec<String>,
        cursor: Option<String>,
    },
    /// One `http_request` answer: status, the response headers the guest may
    /// see, and the body. The credential is not here and cannot be: the parent
    /// added it to the request it sent, and nothing about the request comes
    /// back (W53-12).
    Http(abi::HttpResponse),
}

/// Why the parent refused a host call.
///
/// A closed set, because it is the parent's half of the guest's vocabulary:
/// [`HostCallRefusal::as_refusal`] maps each one onto one ABI [`Refusal`], and
/// that mapping is the only place the two vocabularies meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostCallRefusal {
    /// The store refused the write on quota (`plugin_kv_quota_exceeded`).
    QuotaExceeded,
    /// The invocation has spent its host-call allowance
    /// (`runtime::MAX_HOST_CALLS_PER_INVOCATION`) or its transport budget. The
    /// ABI has no code of its own for this, so it is reported as
    /// `plugin_fuel_exhausted`: the invocation's allowance for work is what ran
    /// out.
    BudgetExhausted,
    /// Nothing on this path can serve the call (`plugin_not_implemented`):
    /// the pool was handed no service, which is the in-process transport and
    /// the bare `PluginRuntime::invoke` entry point today.
    Unavailable,
    /// The store, or the parent's own bookkeeping, failed
    /// (`plugin_worker_failed`). The one refusal that counts against the
    /// install's health.
    Failed,
    /// The install does not hold the capability the call needs
    /// (`plugin_capability_not_granted`). The parent re-derives this from the
    /// install's own grant set, so a worker that skipped its gate cannot spend
    /// a capability the install does not have.
    CapabilityNotGranted,
    /// The profile is not in the install's consented egress scopes
    /// (`plugin_egress_profile_missing`).
    EgressProfileMissing,
    /// The URL, or the address it resolved to, is outside the consented scope
    /// (`plugin_egress_host_denied`).
    EgressHostDenied,
    /// The scheme is not https (`plugin_egress_scheme_denied`).
    EgressSchemeDenied,
    /// The response is above the ABI's cap
    /// (`plugin_egress_response_too_large`).
    EgressResponseTooLarge,
    /// A header, a method or a URL shape the ABI would have refused
    /// (`plugin_abi_frame_invalid`). A guest naming a host-owned header, or the
    /// header its own credential is injected as, lands here.
    FrameInvalid,
    /// The profile needs a credential and the parent could not present one
    /// (`plugin_credential_refused`): no binding, a withdrawn category grant,
    /// or a record with no material field. One code for all three, so a guest
    /// cannot probe the shape of an org's vault.
    CredentialRefused,
}

impl HostCallRefusal {
    pub const fn code(self) -> u8 {
        match self {
            HostCallRefusal::QuotaExceeded => 1,
            HostCallRefusal::BudgetExhausted => 2,
            HostCallRefusal::Unavailable => 3,
            HostCallRefusal::Failed => 4,
            HostCallRefusal::CapabilityNotGranted => 5,
            HostCallRefusal::EgressProfileMissing => 6,
            HostCallRefusal::EgressHostDenied => 7,
            HostCallRefusal::EgressSchemeDenied => 8,
            HostCallRefusal::EgressResponseTooLarge => 9,
            HostCallRefusal::FrameInvalid => 10,
            HostCallRefusal::CredentialRefused => 11,
        }
    }

    pub const fn parse(code: u8) -> Option<Self> {
        match code {
            1 => Some(HostCallRefusal::QuotaExceeded),
            2 => Some(HostCallRefusal::BudgetExhausted),
            3 => Some(HostCallRefusal::Unavailable),
            4 => Some(HostCallRefusal::Failed),
            5 => Some(HostCallRefusal::CapabilityNotGranted),
            6 => Some(HostCallRefusal::EgressProfileMissing),
            7 => Some(HostCallRefusal::EgressHostDenied),
            8 => Some(HostCallRefusal::EgressSchemeDenied),
            9 => Some(HostCallRefusal::EgressResponseTooLarge),
            10 => Some(HostCallRefusal::FrameInvalid),
            11 => Some(HostCallRefusal::CredentialRefused),
            _ => None,
        }
    }

    /// The ABI code the guest sees. Every variant has one, and this is the only
    /// place the host-call vocabulary and the ABI's meet.
    pub const fn as_refusal(self) -> abi::Refusal {
        match self {
            HostCallRefusal::QuotaExceeded => abi::Refusal::KvQuotaExceeded,
            HostCallRefusal::BudgetExhausted => abi::Refusal::FuelExhausted,
            HostCallRefusal::Unavailable => abi::Refusal::NotImplemented,
            HostCallRefusal::Failed => abi::Refusal::WorkerFailed,
            HostCallRefusal::CapabilityNotGranted => abi::Refusal::CapabilityNotGranted,
            HostCallRefusal::EgressProfileMissing => abi::Refusal::EgressProfileMissing,
            HostCallRefusal::EgressHostDenied => abi::Refusal::EgressHostDenied,
            HostCallRefusal::EgressSchemeDenied => abi::Refusal::EgressSchemeDenied,
            HostCallRefusal::EgressResponseTooLarge => abi::Refusal::EgressResponseTooLarge,
            HostCallRefusal::FrameInvalid => abi::Refusal::FrameInvalid,
            HostCallRefusal::CredentialRefused => abi::Refusal::CredentialRefused,
        }
    }
}

/// The answer to one host call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostCallOutcome {
    Ok(HostCallValue),
    Refused(HostCallRefusal),
}

impl From<HostCallValue> for HostCallOutcome {
    fn from(value: HostCallValue) -> Self {
        HostCallOutcome::Ok(value)
    }
}

/// One host call: the correlators, the verb and its arguments.
///
/// `call_id` is the worker's own monotonic counter for the invocation. The
/// parent requires it to be exactly the next number it expects, so a duplicate,
/// a replay or a frame from an earlier invocation is a protocol failure rather
/// than an answer somebody has to interpret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCallFrame {
    pub invocation_id: Uuid,
    pub call_id: u64,
    pub op: HostCallOp,
}

/// The parent's answer to one [`HostCallFrame`], echoed back on both
/// correlators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostReplyFrame {
    pub invocation_id: Uuid,
    pub call_id: u64,
    pub outcome: HostCallOutcome,
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u16(&mut self, value: usize) -> Result<(), WireError> {
        let value = u16::try_from(value).map_err(|_| WireError::TooLarge)?;
        self.bytes.extend_from_slice(&value.to_le_bytes());
        Ok(())
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn uuid(&mut self, value: Uuid) {
        self.bytes.extend_from_slice(value.as_bytes());
    }

    fn str(&mut self, value: &str) -> Result<(), WireError> {
        self.u32(u32::try_from(value.len()).map_err(|_| WireError::TooLarge)?);
        self.bytes.extend_from_slice(value.as_bytes());
        Ok(())
    }

    fn bytes(&mut self, value: &[u8]) -> Result<(), WireError> {
        self.u32(u32::try_from(value.len()).map_err(|_| WireError::TooLarge)?);
        self.bytes.extend_from_slice(value);
        Ok(())
    }

    fn finish(self) -> Result<Vec<u8>, WireError> {
        if self.bytes.len() > MAX_FRAME_BYTES {
            return Err(WireError::TooLarge);
        }
        let mut framed = Vec::with_capacity(self.bytes.len() + 4);
        framed.extend_from_slice(&(self.bytes.len() as u32).to_le_bytes());
        framed.extend_from_slice(&self.bytes);
        Ok(framed)
    }
}

fn header(kind: u8) -> Writer {
    let mut writer = Writer {
        bytes: Vec::with_capacity(256),
    };
    writer.u32(MAGIC);
    writer.u8(PROTOCOL_VERSION);
    writer.u8(kind);
    writer
}

/// Encodes the worker's opening frame: its pid, so a parent can line the frame
/// up with the child it spawned.
pub fn encode_hello(worker_pid: u32) -> Result<Vec<u8>, WireError> {
    let mut writer = header(KIND_HELLO);
    writer.u32(worker_pid);
    writer.finish()
}

/// Decodes a hello frame, returning the worker's pid.
pub fn decode_hello(body: &[u8]) -> Result<u32, WireError> {
    if body.len() > MAX_FRAME_BYTES {
        return Err(WireError::TooLarge);
    }
    let mut reader = Reader::new(body);
    reader.header(KIND_HELLO)?;
    let pid = reader.u32()?;
    reader.done()?;
    Ok(pid)
}

/// Encodes one request into the bytes written to a worker's stdin.
pub fn encode_invoke(frame: &InvokeFrame) -> Result<Vec<u8>, WireError> {
    let request = &frame.request;
    if request.payload.len() > abi::MAX_INPUT_BYTES {
        return Err(WireError::TooLarge);
    }
    if let Some(module) = &frame.module {
        if module.len() > MAX_MODULE_BYTES {
            return Err(WireError::TooLarge);
        }
    }

    let mut writer = header(KIND_INVOKE);
    writer.uuid(request.invocation_id);
    writer.uuid(request.org_id);
    writer.uuid(request.install_id);
    writer.str(&request.entry)?;
    writer.bytes(&request.payload)?;

    writer.u16(request.granted.len())?;
    for key in &request.granted {
        writer.str(key)?;
    }
    writer.u16(request.declared_events.len())?;
    for event in &request.declared_events {
        writer.str(event)?;
    }
    writer.u16(request.egress.len())?;
    for (profile, scopes) in &request.egress {
        writer.str(profile)?;
        writer.u16(scopes.len())?;
        for scope in scopes {
            writer.str(scope)?;
        }
    }

    writer.str(&frame.digest)?;
    match &frame.module {
        Some(module) => {
            writer.u8(1);
            writer.bytes(module)?;
        }
        None => writer.u8(0),
    }
    writer.finish()
}

/// Encodes one reply into the bytes written to a worker's stdout.
pub fn encode_reply(frame: &ReplyFrame) -> Result<Vec<u8>, WireError> {
    let reply = &frame.reply;
    if let Some(output) = &reply.output {
        if output.len() > abi::MAX_OUTPUT_BYTES {
            return Err(WireError::TooLarge);
        }
    }

    let mut writer = header(KIND_REPLY);
    writer.uuid(reply.invocation_id);
    writer.u8(match reply.outcome {
        InvocationOutcome::Ok => 0,
        InvocationOutcome::Refused => 1,
        InvocationOutcome::WorkerFailed => 2,
    });
    match &reply.reason {
        Some(reason) => {
            writer.u8(1);
            writer.str(reason.as_str())?;
        }
        None => writer.u8(0),
    }
    match &reply.output {
        Some(output) => {
            writer.u8(1);
            writer.bytes(output)?;
        }
        None => writer.u8(0),
    }
    match reply.fuel_used {
        Some(fuel) => {
            writer.u8(1);
            writer.u64(fuel);
        }
        None => writer.u8(0),
    }
    writer.u64(reply.duration_us);
    writer.u8(u8::from(frame.module_cached));
    writer.u8(u8::from(frame.retire));
    match &frame.evicted {
        Some(digest) => {
            writer.u8(1);
            writer.str(digest)?;
        }
        None => writer.u8(0),
    }
    writer.finish()
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Reader { bytes, at: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], WireError> {
        let end = self.at.checked_add(len).ok_or(WireError::Truncated)?;
        if end > self.bytes.len() {
            return Err(WireError::Truncated);
        }
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }

    fn bool(&mut self) -> Result<bool, WireError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(WireError::Malformed),
        }
    }

    fn u16(&mut self) -> Result<usize, WireError> {
        let bytes = self.take(2)?;
        Ok(usize::from(u16::from_le_bytes([bytes[0], bytes[1]])))
    }

    fn u32(&mut self) -> Result<u32, WireError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn u64(&mut self) -> Result<u64, WireError> {
        let bytes = self.take(8)?;
        let mut array = [0u8; 8];
        array.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(array))
    }

    fn uuid(&mut self) -> Result<Uuid, WireError> {
        let bytes = self.take(16)?;
        let mut array = [0u8; 16];
        array.copy_from_slice(bytes);
        Ok(Uuid::from_bytes(array))
    }

    fn str(&mut self) -> Result<String, WireError> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| WireError::Malformed)
    }

    /// A host call's label: a string that may not exceed
    /// [`MAX_HOST_CALL_LABEL_BYTES`]. Refused before the bytes are copied, so a
    /// frame that claims a 4 MiB namespace allocates nothing.
    fn label(&mut self) -> Result<String, WireError> {
        let len = self.u32()? as usize;
        if len > MAX_HOST_CALL_LABEL_BYTES {
            return Err(WireError::TooLarge);
        }
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| WireError::Malformed)
    }

    fn bytes(&mut self, cap: usize) -> Result<Vec<u8>, WireError> {
        let len = self.u32()? as usize;
        if len > cap {
            return Err(WireError::TooLarge);
        }
        Ok(self.take(len)?.to_vec())
    }

    fn header(&mut self, kind: u8) -> Result<(), WireError> {
        if self.u32()? != MAGIC || self.u8()? != PROTOCOL_VERSION || self.u8()? != kind {
            return Err(WireError::NotOurFrame);
        }
        Ok(())
    }

    fn done(&self) -> Result<(), WireError> {
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err(WireError::Malformed)
        }
    }
}

/// Decodes a request body (the bytes after the 4-byte length prefix).
pub fn decode_invoke(body: &[u8]) -> Result<InvokeFrame, WireError> {
    if body.len() > MAX_FRAME_BYTES {
        return Err(WireError::TooLarge);
    }
    let mut reader = Reader::new(body);
    reader.header(KIND_INVOKE)?;

    let invocation_id = reader.uuid()?;
    let org_id = reader.uuid()?;
    let install_id = reader.uuid()?;
    let entry = reader.str()?;
    let payload = reader.bytes(abi::MAX_INPUT_BYTES)?;

    let granted_count = reader.u16()?;
    let mut granted = Vec::with_capacity(granted_count.min(64));
    for _ in 0..granted_count {
        granted.push(reader.str()?);
    }

    let events_count = reader.u16()?;
    let mut declared_events = Vec::with_capacity(events_count.min(64));
    for _ in 0..events_count {
        declared_events.push(reader.str()?);
    }

    let profile_count = reader.u16()?;
    let mut egress = abi::EgressScopes::new();
    for _ in 0..profile_count {
        let name = reader.str()?;
        let scope_count = reader.u16()?;
        let mut scopes = Vec::with_capacity(scope_count.min(64));
        for _ in 0..scope_count {
            scopes.push(reader.str()?);
        }
        egress.insert(name, scopes);
    }

    let digest = reader.str()?;
    let module = if reader.bool()? {
        Some(reader.bytes(MAX_MODULE_BYTES)?)
    } else {
        None
    };
    reader.done()?;

    Ok(InvokeFrame {
        digest,
        module,
        request: InvocationRequest {
            invocation_id,
            org_id,
            install_id,
            entry,
            payload,
            granted,
            declared_events,
            egress,
        },
    })
}

/// Decodes a reply body (the bytes after the 4-byte length prefix).
pub fn decode_reply(body: &[u8]) -> Result<ReplyFrame, WireError> {
    if body.len() > MAX_FRAME_BYTES {
        return Err(WireError::TooLarge);
    }
    let mut reader = Reader::new(body);
    reader.header(KIND_REPLY)?;

    let invocation_id = reader.uuid()?;
    let outcome = match reader.u8()? {
        0 => InvocationOutcome::Ok,
        1 => InvocationOutcome::Refused,
        2 => InvocationOutcome::WorkerFailed,
        _ => return Err(WireError::Malformed),
    };
    let reason = if reader.bool()? {
        let word = reader.str()?;
        Some(Refusal::parse(&word).ok_or(WireError::Malformed)?)
    } else {
        None
    };
    let output = if reader.bool()? {
        Some(reader.bytes(abi::MAX_OUTPUT_BYTES)?)
    } else {
        None
    };
    let fuel_used = if reader.bool()? {
        Some(reader.u64()?)
    } else {
        None
    };
    let duration_us = reader.u64()?;
    let module_cached = reader.bool()?;
    let retire = reader.bool()?;
    let evicted = if reader.bool()? {
        Some(reader.str()?)
    } else {
        None
    };
    reader.done()?;

    Ok(ReplyFrame {
        reply: InvocationReply {
            invocation_id,
            outcome,
            reason,
            output,
            fuel_used,
            duration_us,
        },
        module_cached,
        retire,
        evicted,
    })
}

// ---------------------------------------------------------------------------
// Host calls: the codec
// ---------------------------------------------------------------------------

/// Encodes one host call into the bytes the worker writes to its stdout.
pub fn encode_host_call(frame: &HostCallFrame) -> Result<Vec<u8>, WireError> {
    let mut writer = header(KIND_HOST_CALL);
    writer.uuid(frame.invocation_id);
    writer.u64(frame.call_id);
    writer.u8(frame.op.verb().code());
    match &frame.op {
        HostCallOp::KvGet { namespace, key } | HostCallOp::KvDelete { namespace, key } => {
            label(&mut writer, namespace)?;
            label(&mut writer, key)?;
        }
        HostCallOp::KvPut {
            namespace,
            key,
            value,
            expires_at,
        } => {
            label(&mut writer, namespace)?;
            label(&mut writer, key)?;
            if value.len() > MAX_HOST_CALL_VALUE_BYTES {
                return Err(WireError::TooLarge);
            }
            writer.bytes(value)?;
            match expires_at {
                Some(at) => {
                    writer.u8(1);
                    writer.u64(u64::from_le_bytes(at.timestamp_millis().to_le_bytes()));
                }
                None => writer.u8(0),
            }
        }
        HostCallOp::KvList {
            namespace,
            prefix,
            cursor,
        } => {
            label(&mut writer, namespace)?;
            label(&mut writer, prefix)?;
            match cursor {
                Some(cursor) => {
                    writer.u8(1);
                    label(&mut writer, cursor)?;
                }
                None => writer.u8(0),
            }
        }
        HostCallOp::HttpRequest { profile, request } => {
            if profile.len() > MAX_HOST_CALL_PROFILE_BYTES {
                return Err(WireError::TooLarge);
            }
            writer.str(profile)?;
            encode_http_request(&mut writer, request)?;
        }
    }
    writer.finish()
}

/// The request half of `http_request`: method, URL, guest headers, body.
///
/// Four caps and every one of them is checked before the bytes are written, so
/// a frame that claims a giant URL or header count is refused rather than
/// sized.
fn encode_http_request(writer: &mut Writer, request: &abi::HttpRequest) -> Result<(), WireError> {
    if request.method.len() > MAX_HOST_CALL_PROFILE_BYTES
        || request.url.len() > MAX_HOST_CALL_URL_BYTES
        || request.headers.len() > MAX_HOST_CALL_HEADERS
    {
        return Err(WireError::TooLarge);
    }
    if let Some(body) = &request.body {
        if body.len() > abi::MAX_INPUT_BYTES {
            return Err(WireError::TooLarge);
        }
    }
    writer.str(&request.method)?;
    writer.str(&request.url)?;
    writer.u16(request.headers.len())?;
    for (name, value) in &request.headers {
        if name.len() > MAX_HOST_CALL_HEADER_BYTES || value.len() > MAX_HOST_CALL_HEADER_BYTES {
            return Err(WireError::TooLarge);
        }
        writer.str(name)?;
        writer.str(value)?;
    }
    match &request.body {
        Some(body) => {
            writer.u8(1);
            writer.bytes(body.as_bytes())?;
        }
        None => writer.u8(0),
    }
    Ok(())
}

/// The response half: status, headers, body. Bounded by the transport's own cap
/// so the bytes the guest's answer allocates cannot exceed what the transport
/// already refused to buffer.
fn encode_http_response(
    writer: &mut Writer,
    response: &abi::HttpResponse,
) -> Result<(), WireError> {
    if response.headers.len() > MAX_HOST_CALL_HEADERS
        || response.body.len() > MAX_HOST_CALL_HTTP_BODY_BYTES
    {
        return Err(WireError::TooLarge);
    }
    writer.u16(usize::from(response.status))?;
    writer.u16(response.headers.len())?;
    for (name, value) in &response.headers {
        if name.len() > MAX_HOST_CALL_HEADER_BYTES || value.len() > MAX_HOST_CALL_HEADER_BYTES {
            return Err(WireError::TooLarge);
        }
        writer.str(name)?;
        writer.str(value)?;
    }
    writer.bytes(response.body.as_bytes())?;
    Ok(())
}

fn label(writer: &mut Writer, value: &str) -> Result<(), WireError> {
    if value.len() > MAX_HOST_CALL_LABEL_BYTES {
        return Err(WireError::TooLarge);
    }
    writer.str(value)
}

/// Encodes the parent's answer into the bytes it writes to the worker's stdin.
pub fn encode_host_reply(frame: &HostReplyFrame) -> Result<Vec<u8>, WireError> {
    let mut writer = header(KIND_HOST_REPLY);
    writer.uuid(frame.invocation_id);
    writer.u64(frame.call_id);
    match &frame.outcome {
        HostCallOutcome::Refused(refusal) => writer.u8(refusal.code()),
        HostCallOutcome::Ok(value) => {
            writer.u8(0);
            match value {
                HostCallValue::Unit => writer.u8(0),
                HostCallValue::Absent => writer.u8(1),
                HostCallValue::Value(value) => {
                    if value.len() > MAX_HOST_CALL_VALUE_BYTES {
                        return Err(WireError::TooLarge);
                    }
                    writer.u8(2);
                    writer.bytes(value)?;
                }
                HostCallValue::Page { keys, cursor } => {
                    if keys.len() > MAX_HOST_CALL_PAGE {
                        return Err(WireError::TooLarge);
                    }
                    writer.u8(3);
                    writer.u16(keys.len())?;
                    for key in keys {
                        label(&mut writer, key)?;
                    }
                    match cursor {
                        Some(cursor) => {
                            writer.u8(1);
                            label(&mut writer, cursor)?;
                        }
                        None => writer.u8(0),
                    }
                }
                HostCallValue::Existed(existed) => {
                    writer.u8(4);
                    writer.u8(u8::from(*existed));
                }
                HostCallValue::Http(response) => {
                    writer.u8(5);
                    encode_http_response(&mut writer, response)?;
                }
            }
        }
    }
    writer.finish()
}

/// Decodes a host-call body (the bytes after the 4-byte length prefix).
pub fn decode_host_call(body: &[u8]) -> Result<HostCallFrame, WireError> {
    if body.len() > MAX_FRAME_BYTES {
        return Err(WireError::TooLarge);
    }
    let mut reader = Reader::new(body);
    reader.header(KIND_HOST_CALL)?;
    let invocation_id = reader.uuid()?;
    let call_id = reader.u64()?;
    let verb = HostVerb::parse(reader.u8()?).ok_or(WireError::Malformed)?;
    let op = match verb {
        HostVerb::KvGet => HostCallOp::KvGet {
            namespace: reader.label()?,
            key: reader.label()?,
        },
        HostVerb::KvDelete => HostCallOp::KvDelete {
            namespace: reader.label()?,
            key: reader.label()?,
        },
        HostVerb::KvPut => {
            let namespace = reader.label()?;
            let key = reader.label()?;
            let value = reader.bytes(MAX_HOST_CALL_VALUE_BYTES)?;
            let expires_at = if reader.bool()? {
                let millis = i64::from_le_bytes(reader.u64()?.to_le_bytes());
                Some(DateTime::from_timestamp_millis(millis).ok_or(WireError::Malformed)?)
            } else {
                None
            };
            HostCallOp::KvPut {
                namespace,
                key,
                value,
                expires_at,
            }
        }
        HostVerb::KvList => {
            let namespace = reader.label()?;
            let prefix = reader.label()?;
            let cursor = if reader.bool()? {
                Some(reader.label()?)
            } else {
                None
            };
            HostCallOp::KvList {
                namespace,
                prefix,
                cursor,
            }
        }
        HostVerb::HttpRequest => {
            let profile = reader.str()?;
            if profile.len() > MAX_HOST_CALL_PROFILE_BYTES {
                return Err(WireError::TooLarge);
            }
            HostCallOp::HttpRequest {
                profile,
                request: decode_http_request(&mut reader)?,
            }
        }
    };
    reader.done()?;
    Ok(HostCallFrame {
        invocation_id,
        call_id,
        op,
    })
}

/// Decodes a host-reply body (the bytes after the 4-byte length prefix).
pub fn decode_host_reply(body: &[u8]) -> Result<HostReplyFrame, WireError> {
    if body.len() > MAX_FRAME_BYTES {
        return Err(WireError::TooLarge);
    }
    let mut reader = Reader::new(body);
    reader.header(KIND_HOST_REPLY)?;
    let invocation_id = reader.uuid()?;
    let call_id = reader.u64()?;
    let status = reader.u8()?;
    let outcome = if status == 0 {
        let value = match reader.u8()? {
            0 => HostCallValue::Unit,
            1 => HostCallValue::Absent,
            2 => HostCallValue::Value(reader.bytes(MAX_HOST_CALL_VALUE_BYTES)?),
            3 => {
                let count = reader.u16()?;
                if count > MAX_HOST_CALL_PAGE {
                    return Err(WireError::TooLarge);
                }
                let mut keys = Vec::with_capacity(count.min(MAX_HOST_CALL_PAGE));
                for _ in 0..count {
                    keys.push(reader.label()?);
                }
                let cursor = if reader.bool()? {
                    Some(reader.label()?)
                } else {
                    None
                };
                HostCallValue::Page { keys, cursor }
            }
            4 => HostCallValue::Existed(reader.bool()?),
            5 => HostCallValue::Http(decode_http_response(&mut reader)?),
            _ => return Err(WireError::Malformed),
        };
        HostCallOutcome::Ok(value)
    } else {
        HostCallOutcome::Refused(HostCallRefusal::parse(status).ok_or(WireError::Malformed)?)
    };
    reader.done()?;
    Ok(HostReplyFrame {
        invocation_id,
        call_id,
        outcome,
    })
}

/// The request half of an `http_request` frame, after the profile.
fn decode_http_request(reader: &mut Reader<'_>) -> Result<abi::HttpRequest, WireError> {
    let method = reader.str()?;
    if method.len() > MAX_HOST_CALL_PROFILE_BYTES {
        return Err(WireError::TooLarge);
    }
    let url = reader.str()?;
    if url.len() > MAX_HOST_CALL_URL_BYTES {
        return Err(WireError::TooLarge);
    }
    let count = reader.u16()?;
    if count > MAX_HOST_CALL_HEADERS {
        return Err(WireError::TooLarge);
    }
    let mut headers = std::collections::BTreeMap::new();
    for _ in 0..count {
        let name = reader.str()?;
        let value = reader.str()?;
        if name.len() > MAX_HOST_CALL_HEADER_BYTES || value.len() > MAX_HOST_CALL_HEADER_BYTES {
            return Err(WireError::TooLarge);
        }
        headers.insert(name, value);
    }
    let body = if reader.bool()? {
        Some(
            String::from_utf8(reader.bytes(abi::MAX_INPUT_BYTES)?)
                .map_err(|_| WireError::Malformed)?,
        )
    } else {
        None
    };
    Ok(abi::HttpRequest {
        method,
        url,
        headers,
        body,
    })
}

/// The response half of a host reply: status, headers, body.
fn decode_http_response(reader: &mut Reader<'_>) -> Result<abi::HttpResponse, WireError> {
    let status = reader.u16()?;
    let count = reader.u16()?;
    if count > MAX_HOST_CALL_HEADERS {
        return Err(WireError::TooLarge);
    }
    let mut headers = std::collections::BTreeMap::new();
    for _ in 0..count {
        let name = reader.str()?;
        let value = reader.str()?;
        if name.len() > MAX_HOST_CALL_HEADER_BYTES || value.len() > MAX_HOST_CALL_HEADER_BYTES {
            return Err(WireError::TooLarge);
        }
        headers.insert(name, value);
    }
    let body = String::from_utf8(reader.bytes(MAX_HOST_CALL_HTTP_BODY_BYTES)?)
        .map_err(|_| WireError::Malformed)?;
    Ok(abi::HttpResponse {
        status: u16::try_from(status).map_err(|_| WireError::Malformed)?,
        headers,
        body,
    })
}

/// The kind byte of a body that belongs to this protocol, without decoding the
/// rest of it. The parent uses it to tell a host call from the invocation's
/// reply on one stream.
pub fn frame_kind(body: &[u8]) -> Result<u8, WireError> {
    if body.len() > MAX_FRAME_BYTES {
        return Err(WireError::TooLarge);
    }
    let mut reader = Reader::new(body);
    if reader.u32()? != MAGIC || reader.u8()? != PROTOCOL_VERSION {
        return Err(WireError::NotOurFrame);
    }
    let kind = reader.u8()?;
    match kind {
        KIND_INVOKE | KIND_REPLY | KIND_HELLO | KIND_HOST_CALL | KIND_HOST_REPLY => Ok(kind),
        _ => Err(WireError::NotOurFrame),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module() -> Vec<u8> {
        b"\0asm\x01\0\0\0".to_vec()
    }

    fn request() -> InvocationRequest {
        let mut egress = abi::EgressScopes::new();
        egress.insert(
            "telemetry".into(),
            vec!["https://api.example.com/v1".into()],
        );
        InvocationRequest {
            invocation_id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            install_id: Uuid::now_v7(),
            entry: "plugin.triage-note-action.triage_note".into(),
            payload: br#"{"ticket":"01J"}"#.to_vec(),
            granted: vec!["automation.actions".into(), "events.emit".into()],
            declared_events: vec!["ticket.escalated".into()],
            egress,
        }
    }

    #[test]
    fn an_invoke_frame_round_trips_with_and_without_its_module_bytes() {
        for module in [None, Some(module())] {
            let frame = InvokeFrame {
                digest: "a".repeat(64),
                module,
                request: request(),
            };
            let framed = encode_invoke(&frame).expect("encode");
            let declared = u32::from_le_bytes([framed[0], framed[1], framed[2], framed[3]]);
            assert_eq!(declared as usize, framed.len() - 4);
            assert_eq!(decode_invoke(&framed[4..]).unwrap(), frame);
        }
    }

    #[test]
    fn a_reply_frame_round_trips_for_all_three_outcomes() {
        let id = Uuid::now_v7();
        for reply in [
            InvocationReply::ok(id, Some(b"{}".to_vec()), 38_909, 5),
            InvocationReply::ok(id, None, 1, 1),
            InvocationReply::refused(id, Refusal::LogBudgetExceeded, 12, 3),
            InvocationReply::worker_failed(id, 700),
        ] {
            for (module_cached, retire) in [(false, false), (true, false), (true, true)] {
                let frame = ReplyFrame {
                    reply: reply.clone(),
                    module_cached,
                    retire,
                    evicted: if module_cached {
                        None
                    } else {
                        Some("b".repeat(64))
                    },
                };
                let framed = encode_reply(&frame).expect("encode");
                assert_eq!(decode_reply(&framed[4..]).unwrap(), frame);
            }
        }
    }

    #[test]
    fn the_hello_frame_carries_the_workers_pid_and_nothing_else() {
        let framed = encode_hello(4242).expect("encode");
        assert_eq!(decode_hello(&framed[4..]), Ok(4242));
        assert_eq!(
            decode_hello(&framed[4..framed.len() - 1]),
            Err(WireError::Truncated)
        );
        let mut body = framed[4..].to_vec();
        body[5] = KIND_REPLY;
        assert_eq!(decode_hello(&body), Err(WireError::NotOurFrame));
    }

    #[test]
    fn every_truncation_of_a_frame_is_a_typed_error_and_never_a_panic() {
        let frame = InvokeFrame {
            digest: "a".repeat(64),
            module: Some(module()),
            request: request(),
        };
        let framed = encode_invoke(&frame).expect("encode");
        let body = &framed[4..];
        for cut in 0..body.len() {
            assert!(
                decode_invoke(&body[..cut]).is_err(),
                "a {cut}-byte prefix decoded"
            );
        }
        assert!(decode_invoke(body).is_ok());
    }

    #[test]
    fn trailing_bytes_are_refused_rather_than_ignored() {
        let mut framed = encode_invoke(&InvokeFrame {
            digest: "a".repeat(64),
            module: None,
            request: request(),
        })
        .expect("encode");
        framed.push(0);
        assert_eq!(decode_invoke(&framed[4..]), Err(WireError::Malformed));
    }

    #[test]
    fn a_foreign_magic_version_or_kind_is_not_our_frame() {
        let framed = encode_invoke(&InvokeFrame {
            digest: "a".repeat(64),
            module: None,
            request: request(),
        })
        .expect("encode");
        let body = &framed[4..];

        let mut wrong_magic = body.to_vec();
        wrong_magic[0] ^= 0xff;
        assert_eq!(decode_invoke(&wrong_magic), Err(WireError::NotOurFrame));

        let mut wrong_version = body.to_vec();
        wrong_version[4] = PROTOCOL_VERSION + 1;
        assert_eq!(decode_invoke(&wrong_version), Err(WireError::NotOurFrame));

        let mut wrong_kind = body.to_vec();
        wrong_kind[5] = KIND_REPLY;
        assert_eq!(decode_invoke(&wrong_kind), Err(WireError::NotOurFrame));
    }

    #[test]
    fn a_lying_payload_length_is_refused_before_it_is_read() {
        let frame = InvokeFrame {
            digest: "a".repeat(64),
            module: None,
            request: request(),
        };
        let framed = encode_invoke(&frame).expect("encode");
        let mut body = framed[4..].to_vec();
        // The payload length word sits after header(6), three uuids (48) and
        // the entry string. Find it by decoding the prefix length instead of
        // hard-coding an offset.
        let entry_len_offset = 6 + 48;
        let entry_len = u32::from_le_bytes([
            body[entry_len_offset],
            body[entry_len_offset + 1],
            body[entry_len_offset + 2],
            body[entry_len_offset + 3],
        ]) as usize;
        let payload_len_offset = entry_len_offset + 4 + entry_len;
        // A length that is legal for a payload but larger than the bytes the
        // frame actually carries: the reader must not allocate it.
        let lying = u32::try_from(body.len()).expect("small frame");
        body[payload_len_offset..payload_len_offset + 4].copy_from_slice(&lying.to_le_bytes());
        assert_eq!(decode_invoke(&body), Err(WireError::Truncated));
    }

    #[test]
    fn an_unknown_refusal_word_or_outcome_byte_is_malformed() {
        let reply = ReplyFrame {
            reply: InvocationReply::refused(Uuid::now_v7(), Refusal::GuestTrap, 1, 1),
            module_cached: false,
            retire: false,
            evicted: None,
        };
        let framed = encode_reply(&reply).expect("encode");
        let body = &framed[4..];

        let mut bad_outcome = body.to_vec();
        bad_outcome[22] = 9;
        assert_eq!(decode_reply(&bad_outcome), Err(WireError::Malformed));

        // Rewrite the reason word to something no refusal produces.
        let mut bad_reason = body.to_vec();
        let at = bad_reason
            .windows(4)
            .position(|w| w == b"trap")
            .expect("the reason word is in the frame");
        bad_reason[at..at + 4].copy_from_slice(b"opal");
        assert_eq!(decode_reply(&bad_reason), Err(WireError::Malformed));
    }

    #[test]
    fn an_oversize_payload_module_or_output_is_refused_at_encode_time() {
        let mut oversize = request();
        oversize.payload = vec![0; abi::MAX_INPUT_BYTES + 1];
        assert_eq!(
            encode_invoke(&InvokeFrame {
                digest: "a".repeat(64),
                module: None,
                request: oversize
            }),
            Err(WireError::TooLarge)
        );

        assert_eq!(
            encode_invoke(&InvokeFrame {
                digest: "a".repeat(64),
                module: Some(vec![0; MAX_MODULE_BYTES + 1]),
                request: request(),
            }),
            Err(WireError::TooLarge)
        );

        let reply = ReplyFrame {
            reply: InvocationReply::ok(
                Uuid::now_v7(),
                Some(vec![0; abi::MAX_OUTPUT_BYTES + 1]),
                1,
                1,
            ),
            module_cached: false,
            retire: false,
            evicted: None,
        };
        assert_eq!(encode_reply(&reply), Err(WireError::TooLarge));
    }

    #[test]
    fn a_decoder_that_is_handed_an_oversize_body_refuses_before_parsing() {
        let body = vec![0u8; MAX_FRAME_BYTES + 1];
        assert_eq!(decode_invoke(&body), Err(WireError::TooLarge));
        assert_eq!(decode_reply(&body), Err(WireError::TooLarge));
        assert_eq!(decode_host_call(&body), Err(WireError::TooLarge));
        assert_eq!(decode_host_reply(&body), Err(WireError::TooLarge));
        assert_eq!(frame_kind(&body), Err(WireError::TooLarge));
    }

    fn call(op: HostCallOp) -> HostCallFrame {
        HostCallFrame {
            invocation_id: Uuid::now_v7(),
            call_id: 7,
            op,
        }
    }

    fn every_op() -> Vec<HostCallOp> {
        vec![
            HostCallOp::KvGet {
                namespace: "cache".into(),
                key: "k".into(),
            },
            HostCallOp::KvPut {
                namespace: "cache".into(),
                key: "k".into(),
                value: vec![0xff, 0x00, 0xfe],
                expires_at: None,
            },
            HostCallOp::KvPut {
                namespace: "cache".into(),
                key: "k".into(),
                value: Vec::new(),
                expires_at: Some(DateTime::from_timestamp_millis(1_700_000_000_123).expect("ts")),
            },
            HostCallOp::KvDelete {
                namespace: "cache".into(),
                key: "k".into(),
            },
            HostCallOp::KvList {
                namespace: "cache".into(),
                prefix: "a".into(),
                cursor: None,
            },
            HostCallOp::KvList {
                namespace: "cache".into(),
                prefix: String::new(),
                cursor: Some("a1".into()),
            },
            HostCallOp::HttpRequest {
                profile: "acme".into(),
                request: abi::HttpRequest {
                    method: "POST".into(),
                    url: "https://api.acme.test/v1/tickets?state=open".into(),
                    headers: [("content-type".to_string(), "application/json".to_string())]
                        .into_iter()
                        .collect(),
                    body: Some("{\"title\":\"x\"}".into()),
                },
            },
            HostCallOp::HttpRequest {
                profile: "acme".into(),
                request: abi::HttpRequest {
                    method: "GET".into(),
                    url: "https://api.acme.test/v1".into(),
                    headers: Default::default(),
                    body: None,
                },
            },
        ]
    }

    fn every_outcome() -> Vec<HostCallOutcome> {
        vec![
            HostCallOutcome::Ok(HostCallValue::Unit),
            HostCallOutcome::Ok(HostCallValue::Absent),
            HostCallOutcome::Ok(HostCallValue::Value(vec![1, 2, 3])),
            HostCallOutcome::Ok(HostCallValue::Existed(true)),
            HostCallOutcome::Ok(HostCallValue::Existed(false)),
            HostCallOutcome::Ok(HostCallValue::Page {
                keys: vec!["a1".into(), "a2".into()],
                cursor: Some("a2".into()),
            }),
            HostCallOutcome::Ok(HostCallValue::Page {
                keys: Vec::new(),
                cursor: None,
            }),
            HostCallOutcome::Refused(HostCallRefusal::QuotaExceeded),
            HostCallOutcome::Refused(HostCallRefusal::BudgetExhausted),
            HostCallOutcome::Refused(HostCallRefusal::Unavailable),
            HostCallOutcome::Refused(HostCallRefusal::Failed),
            HostCallOutcome::Refused(HostCallRefusal::CapabilityNotGranted),
            HostCallOutcome::Refused(HostCallRefusal::EgressProfileMissing),
            HostCallOutcome::Refused(HostCallRefusal::EgressHostDenied),
            HostCallOutcome::Refused(HostCallRefusal::EgressSchemeDenied),
            HostCallOutcome::Refused(HostCallRefusal::EgressResponseTooLarge),
            HostCallOutcome::Refused(HostCallRefusal::FrameInvalid),
            HostCallOutcome::Refused(HostCallRefusal::CredentialRefused),
            HostCallOutcome::Ok(HostCallValue::Http(abi::HttpResponse {
                status: 201,
                headers: [("content-type".to_string(), "application/json".to_string())]
                    .into_iter()
                    .collect(),
                body: "{\"ok\":true}".into(),
            })),
            HostCallOutcome::Ok(HostCallValue::Http(abi::HttpResponse {
                status: 204,
                headers: Default::default(),
                body: String::new(),
            })),
        ]
    }

    #[test]
    fn a_host_call_round_trips_for_every_verb() {
        for op in every_op() {
            let frame = call(op.clone());
            let framed = encode_host_call(&frame).expect("encode");
            let declared = u32::from_le_bytes([framed[0], framed[1], framed[2], framed[3]]);
            assert_eq!(declared as usize, framed.len() - 4);
            assert_eq!(decode_host_call(&framed[4..]).expect("decode"), frame);
            assert_eq!(frame_kind(&framed[4..]), Ok(KIND_HOST_CALL));
        }
    }

    #[test]
    fn a_host_reply_round_trips_for_every_outcome() {
        for outcome in every_outcome() {
            let frame = HostReplyFrame {
                invocation_id: Uuid::now_v7(),
                call_id: 11,
                outcome,
            };
            let framed = encode_host_reply(&frame).expect("encode");
            assert_eq!(decode_host_reply(&framed[4..]).expect("decode"), frame);
            assert_eq!(frame_kind(&framed[4..]), Ok(KIND_HOST_REPLY));
        }
    }

    #[test]
    fn a_host_call_frame_carries_no_tenant_identity() {
        // The property the whole bridge rests on. A frame is built for an
        // invocation whose org and install are known to the test, and NEITHER
        // identifier may appear in the bytes: a worker that runs untrusted code
        // cannot name a tenant because the frame has no field to put one in.
        let org_id = Uuid::parse_str("018f0a5e-0000-7000-8000-00000000aaaa").expect("uuid");
        let install_id = Uuid::parse_str("018f0a5e-0000-7000-8000-00000000bbbb").expect("uuid");
        let op = HostCallOp::KvGet {
            namespace: "cache".into(),
            key: "k".into(),
        };
        let framed = encode_host_call(&HostCallFrame {
            invocation_id: Uuid::parse_str("018f0a5e-0000-7000-8000-00000000cccc").expect("uuid"),
            call_id: 1,
            op,
        })
        .expect("encode");
        for (name, id) in [("org", org_id), ("install", install_id)] {
            assert!(
                !framed.windows(16).any(|window| window == id.as_bytes()),
                "the host-call frame carries an {name} id"
            );
        }
    }

    /// The `http_request` frame carries the guest's request and a profile name,
    /// and nothing that could name a tenant, a record or a credential.
    ///
    /// The credential is the point of the verb, so the absence is asserted on
    /// the bytes: a frame that could carry `Authorization: Bearer …` would be a
    /// credential channel from the process that runs untrusted wasm.
    #[test]
    fn an_http_frame_carries_no_credential_and_no_tenant() {
        let op = HostCallOp::HttpRequest {
            profile: "acme".into(),
            request: abi::HttpRequest {
                method: "GET".into(),
                url: "https://api.acme.test/v1".into(),
                headers: Default::default(),
                body: None,
            },
        };
        let framed = encode_host_call(&HostCallFrame {
            invocation_id: Uuid::parse_str("018f0a5e-0000-7000-8000-00000000cccc").expect("uuid"),
            call_id: 1,
            op,
        })
        .expect("encode");
        for forbidden in [
            b"authorization".as_slice(),
            b"x-api-key".as_slice(),
            b"header_name".as_slice(),
            b"sealed".as_slice(),
            b"record".as_slice(),
        ] {
            assert!(
                !framed
                    .windows(forbidden.len())
                    .any(|window| window == forbidden),
                "the http frame must not carry {:?}",
                String::from_utf8_lossy(forbidden)
            );
        }
        for id in [
            "018f0a5e-0000-7000-8000-00000000aaaa",
            "018f0a5e-0000-7000-8000-00000000bbbb",
        ] {
            let bytes = Uuid::parse_str(id).expect("uuid").into_bytes();
            assert!(
                !framed.windows(16).any(|window| window == bytes),
                "the http frame carries a tenant id"
            );
        }
    }

    #[test]
    fn a_profile_over_the_cap_is_refused_at_encode_time() {
        let op = HostCallOp::HttpRequest {
            profile: "a".repeat(MAX_HOST_CALL_PROFILE_BYTES + 1),
            request: abi::HttpRequest {
                method: "GET".into(),
                url: "https://api.acme.test/v1".into(),
                headers: Default::default(),
                body: None,
            },
        };
        assert_eq!(
            encode_host_call(&call(op)),
            Err(WireError::TooLarge),
            "a profile name the manifest could not declare is refused before it is written"
        );
    }

    #[test]
    fn a_verb_no_frame_defines_is_malformed_and_never_guessed() {
        let frame = call(HostCallOp::KvDelete {
            namespace: "cache".into(),
            key: "k".into(),
        });
        let framed = encode_host_call(&frame).expect("encode");
        let mut body = framed[4..].to_vec();
        // header(6) + invocation uuid(16) + call_id(8) -> the verb byte.
        // 5 is `http_request` since W53-12; 6 stays reserved for
        // `emit_event`, so an unknown code still refuses rather than guesses.
        body[30] = 6;
        assert_eq!(decode_host_call(&body), Err(WireError::Malformed));
        body[30] = 0;
        assert_eq!(decode_host_call(&body), Err(WireError::Malformed));
        // And the codes the verbs do use are stable, because a later lane's verb
        // must slot in beside them rather than renumber them.
        for (code, verb) in [
            (1, HostVerb::KvGet),
            (2, HostVerb::KvPut),
            (3, HostVerb::KvDelete),
            (4, HostVerb::KvList),
            (5, HostVerb::HttpRequest),
        ] {
            assert_eq!(HostVerb::parse(code), Some(verb));
            assert_eq!(verb.code(), code);
        }
        for reserved in 6..=7 {
            assert_eq!(
                HostVerb::parse(reserved),
                None,
                "verb {reserved} is reserved for a later frame"
            );
        }
    }

    #[test]
    fn every_truncation_of_a_host_frame_is_a_typed_error() {
        for op in every_op() {
            let framed = encode_host_call(&call(op)).expect("encode");
            let body = &framed[4..];
            for cut in 0..body.len() {
                assert!(
                    decode_host_call(&body[..cut]).is_err(),
                    "a {cut}-byte prefix of a host call decoded"
                );
            }
            assert!(decode_host_call(body).is_ok());
        }
        for outcome in every_outcome() {
            let framed = encode_host_reply(&HostReplyFrame {
                invocation_id: Uuid::now_v7(),
                call_id: 1,
                outcome,
            })
            .expect("encode");
            let body = &framed[4..];
            for cut in 0..body.len() {
                assert!(
                    decode_host_reply(&body[..cut]).is_err(),
                    "a {cut}-byte prefix of a host reply decoded"
                );
            }
            assert!(decode_host_reply(body).is_ok());
        }
    }

    #[test]
    fn a_host_frame_above_its_own_caps_is_refused_before_it_is_allocated() {
        // Encode refuses to build one.
        assert_eq!(
            encode_host_call(&call(HostCallOp::KvPut {
                namespace: "cache".into(),
                key: "k".into(),
                value: vec![0; MAX_HOST_CALL_VALUE_BYTES + 1],
                expires_at: None,
            })),
            Err(WireError::TooLarge)
        );
        assert_eq!(
            encode_host_call(&call(HostCallOp::KvGet {
                namespace: "a".repeat(MAX_HOST_CALL_LABEL_BYTES + 1),
                key: "k".into(),
            })),
            Err(WireError::TooLarge)
        );
        assert_eq!(
            encode_host_reply(&HostReplyFrame {
                invocation_id: Uuid::now_v7(),
                call_id: 1,
                outcome: HostCallOutcome::Ok(HostCallValue::Page {
                    keys: vec!["k".into(); MAX_HOST_CALL_PAGE + 1],
                    cursor: None,
                }),
            }),
            Err(WireError::TooLarge)
        );

        // And a decoder refuses a frame that lies about a length.
        let framed = encode_host_call(&call(HostCallOp::KvPut {
            namespace: "cache".into(),
            key: "k".into(),
            value: vec![0; 4],
            expires_at: None,
        }))
        .expect("encode");
        let mut body = framed[4..].to_vec();
        // Find the value's length word: header(6), uuid(16), call_id(8), verb(1),
        // "cache"(4+5), "k"(4+1).
        let value_len_at = 6 + 16 + 8 + 1 + 4 + 5 + 4 + 1;
        body[value_len_at..value_len_at + 4].copy_from_slice(
            &u32::try_from(MAX_HOST_CALL_VALUE_BYTES + 1)
                .unwrap()
                .to_le_bytes(),
        );
        assert_eq!(decode_host_call(&body), Err(WireError::TooLarge));
    }

    #[test]
    fn a_reply_that_is_not_an_answer_to_a_host_call_is_not_our_frame() {
        let invoke = encode_invoke(&InvokeFrame {
            digest: "a".repeat(64),
            module: None,
            request: request(),
        })
        .expect("encode");
        assert_eq!(frame_kind(&invoke[4..]), Ok(KIND_INVOKE));
        assert_eq!(
            decode_host_call(&invoke[4..]),
            Err(WireError::NotOurFrame),
            "an invocation frame is not a host call"
        );

        let mut foreign_kind = encode_host_call(&call(HostCallOp::KvGet {
            namespace: "cache".into(),
            key: "k".into(),
        }))
        .expect("encode");
        foreign_kind[5] = 9;
        assert_eq!(frame_kind(&foreign_kind[4..]), Err(WireError::NotOurFrame));
    }
}
