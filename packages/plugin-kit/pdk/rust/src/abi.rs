//! The guest half of ABI v1: the limits, the packing convention and the refusal
//! vocabulary, mirrored from `crates/plugin-host/src/abi.rs` by hand.
//!
//! The mirror is checked, not promised: `crates/plugin-host` has a test that
//! parses `packages/plugin-kit/abi/host-abi.v1.json` and compares it with the
//! live registry, and this crate has a test that every string below appears in
//! that same file. A refusal added on one side and not the other fails a build.

/// Input per invocation, in bytes.
pub const MAX_INPUT_BYTES: usize = 1024 * 1024;
/// Output per invocation, in bytes.
pub const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
/// One `log` line, in bytes.
pub const MAX_LOG_LINE_BYTES: usize = 4096;
/// `log` lines per invocation.
pub const MAX_LOG_LINES: u32 = 100;
/// Namespace label: `[a-z0-9_.-]{1,64}`.
pub const KV_NAMESPACE_MAX_BYTES: usize = 64;
/// One KV key, in bytes.
pub const KV_KEY_MAX_BYTES: usize = 512;
/// One KV value, in bytes.
pub const KV_VALUE_MAX_BYTES: usize = 1024 * 1024;

/// The numbers an author checks against before a call, so an oversized value is
/// a local error rather than a refusal round trip.
pub const LIMITS: Limits = Limits {
    input_bytes: MAX_INPUT_BYTES,
    output_bytes: MAX_OUTPUT_BYTES,
    log_line_bytes: MAX_LOG_LINE_BYTES,
    log_lines: MAX_LOG_LINES,
    kv_namespace_bytes: KV_NAMESPACE_MAX_BYTES,
    kv_key_bytes: KV_KEY_MAX_BYTES,
    kv_value_bytes: KV_VALUE_MAX_BYTES,
};

/// See [`LIMITS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub log_line_bytes: usize,
    pub log_lines: u32,
    pub kv_namespace_bytes: usize,
    pub kv_key_bytes: usize,
    pub kv_value_bytes: usize,
}

/// A typed refusal from the host. The string is authoritative (it is what the
/// audit row carries); the code is the in-band `u32` form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    CapabilityNotGranted,
    LogBudgetExceeded,
    KvNamespaceDenied,
    KvQuotaExceeded,
    KvValueTooLarge,
    EgressProfileMissing,
    EgressHostDenied,
    EgressSchemeDenied,
    EgressResponseTooLarge,
    EventTypeNotDeclared,
    QueryUnknown,
    WorkerFailed,
    NotImplemented,
    UnknownHostFn,
    FrameInvalid,
    InputTooLarge,
    OutputTooLarge,
}

impl Refusal {
    pub const ALL: [Refusal; 17] = [
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
    ];

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
        }
    }

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
        }
    }

    pub fn from_code(code: u32) -> Option<Self> {
        Self::ALL.iter().copied().find(|r| r.code() == code)
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|r| r.as_str() == value)
    }
}

/// The tag byte of a success reply frame.
pub const REPLY_TAG_OK: u8 = 0;
/// The tag byte of a refusal reply frame.
pub const REPLY_TAG_ERR: u8 = 1;

/// Pack `(pointer, length)` into the `i64` a data-returning call returns.
pub const fn pack(ptr: u32, len: u32) -> i64 {
    (((ptr as u64) << 32) | (len as u64)) as i64
}

/// Inverse of [`pack`].
pub const fn unpack(packed: i64) -> (u32, u32) {
    let bits = packed as u64;
    ((bits >> 32) as u32, bits as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The catalogue is the shared artefact; a refusal that exists here but not
    /// there would never be documented to an author.
    const ABI_JSON: &str = include_str!("../../../abi/host-abi.v1.json");

    #[test]
    fn every_refusal_is_documented_in_the_shared_catalogue() {
        for refusal in Refusal::ALL {
            assert!(
                ABI_JSON.contains(refusal.as_str()),
                "{} is missing from abi/host-abi.v1.json",
                refusal.as_str()
            );
        }
    }

    #[test]
    fn codes_and_strings_are_unique_and_round_trip() {
        for refusal in Refusal::ALL {
            assert_ne!(refusal.code(), 0);
            assert_eq!(Refusal::from_code(refusal.code()), Some(refusal));
            assert_eq!(Refusal::parse(refusal.as_str()), Some(refusal));
        }
    }

    #[test]
    fn pack_round_trips() {
        assert_eq!(unpack(pack(1024, 32)), (1024, 32));
        assert_eq!(unpack(pack(0, 0)), (0, 0));
        assert_eq!(unpack(pack(u32::MAX, u32::MAX)), (u32::MAX, u32::MAX));
    }
}
