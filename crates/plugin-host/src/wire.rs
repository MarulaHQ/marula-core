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
//! ```
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

use uuid::Uuid;

use crate::abi::{self, InvocationOutcome, InvocationReply, InvocationRequest, Refusal};

/// `"MRPW"` — Marula plugin worker. A frame that does not start with it is not
/// our frame, whoever wrote it.
pub const MAGIC: u32 = 0x4D52_5057;
/// Bumped when a field changes shape. The parent refuses to start a worker that
/// speaks another version rather than guessing which fields it may read.
pub const PROTOCOL_VERSION: u8 = 1;

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
    }
}
