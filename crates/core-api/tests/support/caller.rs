//! One caller identity per request, minted by construction.
//!
//! Why this module exists (Wave 54 follow-up F7): the auth gates
//! (`auth_lockouts`) are DURABLE and keyed on the resolved client IP, and the
//! resolution has a third posture besides "socket peer" and "trusted proxy
//! chain" — a `oneshot` request that carries no `X-Forwarded-For` at all
//! collapses into the single [`core_api::client_ip::UNKNOWN`] bucket. Every
//! suite that sent without the header therefore shared one bucket with every
//! other headerless send in the same binary, and one case's armed lock could
//! refuse another case's sign-in. Wave 54 lane S2a fixed the helper that read
//! a v7 UUID's timestamp bytes for its address; the shared bucket itself
//! stayed, because nothing stopped a suite from omitting the header.
//!
//! The fix is structural rather than another careful helper inside one file:
//! [`fresh_caller_ip`] hands out a distinct address per call, [`CallerIp`]
//! turns one into the header pair a flow suite's `send` helper takes, and
//! [`CallerIp::stamp`] puts it on a `Request` that has none. The flow suites
//! in this crate stamp every request they build, so a case cannot land in a
//! bucket another case wrote.
//!
//! # The address space
//!
//! `198.18.0.0/15` — RFC 2544 benchmarking space. It is never routed, it
//! holds 131,072 addresses (two /16s), and [`core_api::client_ip`] treats it
//! as an ordinary untrusted public client, so the resolver takes the
//! forwarded hop and the gates see a real address rather than a synthetic
//! token. The default trust set (`127/8`, `10/8`, `172.16/12`,
//! `192.168/16`, ULA) does not overlap it.
//!
//! # The uniqueness contract
//!
//! Within one process every call is a distinct slot of the block: a call
//! counter advances one slot at a time, so the first 131,072 calls in a
//! binary cannot collide, whatever the test threads do. The starting slot is
//! drawn from the RANDOM TAIL of a v7 UUID mixed with the pid, never from the
//! UUID's timestamp prefix — that prefix is what made Wave 54's helper hand
//! the same value to every caller in a run — so a rerun inside the durable
//! lock window starts somewhere else.
//!
//! Across processes the starting slots are independent draws from 131,072
//! positions. Two binaries whose draws land within (their call count) of each
//! other share a few addresses; at the couple hundred calls these suites mint
//! that is a fraction of a percent, and a suite whose rows must survive a
//! parallel binary's run already salts its keys per instance
//! (`config.jwt_secret`, see `auth_timing_flow`). Two calls in one process
//! never depend on that bound.
//!
//! A suite that needs a caller address to be *stable* across several requests
//! (a lock that one request arms and the next one must observe) holds one
//! [`CallerIp`] for the whole case and reuses it. That is per-case identity,
//! which is the point; it is never a value two cases share.

#![allow(dead_code)]

use std::fmt;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

/// Addresses in `198.18.0.0/15`: two /16s of 65,536.
const BLOCK_SLOTS: u32 = 131_072;

/// Slot handed to the next call. Shared by every thread in the process, so
/// parallel test cases draw disjoint slots.
static NEXT_SLOT: AtomicU32 = AtomicU32::new(0);

/// The slot this process started at. Drawn once; see the module docs for why
/// it is random rather than the counter's zero.
static BLOCK_BASE: OnceLock<u32> = OnceLock::new();

/// A caller address minted for one test case.
///
/// Keep one for as long as a case needs a stable identity (an armed lock the
/// next request must hit), and mint a new one per case. Never store one in a
/// `static` or hand it to a shared helper: sharing is the bug this type
/// exists to retire.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CallerIp(String);

impl CallerIp {
    /// A caller address no other call in this process has returned.
    pub fn fresh() -> Self {
        let slot = block_base().wrapping_add(NEXT_SLOT.fetch_add(1, Ordering::Relaxed));
        CallerIp(address_for(slot).to_string())
    }

    /// The address as a string — what a gate is keyed on once hashed.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `("x-forwarded-for", address)` for the flow suites' tuple-slice send
    /// helpers.
    pub fn forwarded_for(&self) -> (&'static str, &str) {
        ("x-forwarded-for", self.as_str())
    }

    /// Stamps this caller on a request that carries no `X-Forwarded-For`.
    ///
    /// A request that already names its caller is returned untouched: an
    /// explicit header is a deliberate choice (a stable identity, a proxy
    /// chain under test), and overwriting it would quietly change what the
    /// case is proving.
    pub fn stamp<B>(&self, mut request: axum::http::Request<B>) -> axum::http::Request<B> {
        if !request.headers().contains_key("x-forwarded-for") {
            request.headers_mut().insert(
                "x-forwarded-for",
                axum::http::HeaderValue::from_str(self.as_str())
                    .expect("a minted address is a valid header value"),
            );
        }
        request
    }
}

impl fmt::Display for CallerIp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A fresh caller address. Shorthand for [`CallerIp::fresh`].
pub fn fresh_caller_ip() -> CallerIp {
    CallerIp::fresh()
}

fn block_base() -> u32 {
    *BLOCK_BASE.get_or_init(|| {
        let nonce = uuid::Uuid::now_v7();
        let bytes = nonce.as_bytes();
        // Bytes 10..14 are `rand_c` — the random tail. Bytes 0..6 are the
        // millisecond timestamp and move together for every caller minted in
        // one run, which is the exact collision Wave 54 shipped.
        let tail = u32::from_be_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
        // Mixing the pid in keeps two processes on one host apart more often
        // than two independent draws would, without making the base
        // predictable enough for a rerun to repeat a live lock's bucket.
        let pid = std::process::id().wrapping_mul(0x9E37_79B1);
        (tail ^ pid) % BLOCK_SLOTS
    })
}

/// Maps a slot onto `198.18.0.0/15`: bit 16 picks the /16, the low 16 bits
/// fill the host half.
fn address_for(slot: u32) -> Ipv4Addr {
    let slot = slot % BLOCK_SLOTS;
    Ipv4Addr::new(
        198,
        18 + ((slot >> 16) & 1) as u8,
        ((slot >> 8) & 0xFF) as u8,
        (slot & 0xFF) as u8,
    )
}
