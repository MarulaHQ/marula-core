//! Host-boundary fuzzing: the three places bytes from one side are parsed by the
//! other.
//!
//! 1. **The parent↔worker frames** (`wire`). The parent parses a frame a child
//!    process wrote, and the child parses one the parent wrote. Both ends are
//!    mealy about sizes and counts, so a wrong length field is the interesting
//!    input, not a wrong signature.
//! 2. **The guest's return word** (`isolation`). A guest can return any `i64`.
//!    The host unpacks it into a pointer and a length and reads guest memory at
//!    that range, so every boundary combination has to be a typed refusal and
//!    never a copy past the memory.
//! 3. **The in-band reply frames** (`abi::parse_frame`). The guest decodes what a
//!    host function wrote, and a truncated or mistagged frame must be an error.
//!
//! The generator is a seeded xorshift, not `proptest`: a failure here is a
//! security finding, and it has to be reproducible by a reader with no extra
//! dependency and no shrinking heuristics. Every case prints its seed.

use std::sync::Arc;

use plugin_host::abi::{self, InvocationRequest, Refusal};
use plugin_host::host_fns::register_all;
use plugin_host::isolation::{ModuleArtifact, Sandbox, SandboxLimits};
use plugin_host::wire::{self, HostCallFrame, HostReplyFrame, InvokeFrame, ReplyFrame};
use uuid::Uuid;

/// xorshift64*, chosen because it is four lines a reviewer can check and its
/// whole state is the seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        (self.next() % bound as u64) as usize
    }

    fn byte(&mut self) -> u8 {
        (self.next() >> 33) as u8
    }
}

fn request() -> InvocationRequest {
    InvocationRequest {
        invocation_id: Uuid::now_v7(),
        org_id: Uuid::now_v7(),
        install_id: Uuid::now_v7(),
        entry: "plugin.fuzz.entry".into(),
        payload: b"{}".to_vec(),
        granted: Vec::new(),
        declared_events: Vec::new(),
        egress: Default::default(),
    }
}

/// One valid frame of each kind, so a mutation starts from bytes the codec
/// accepts rather than from noise.
fn seeds() -> Vec<(&'static str, Vec<u8>)> {
    let invoke = wire::encode_invoke(&InvokeFrame {
        digest: "0".repeat(64),
        module: Some(vec![0x00, 0x61, 0x73, 0x6d]),
        request: request(),
    })
    .expect("a valid invoke frame encodes");
    let reply = wire::encode_reply(&ReplyFrame {
        reply: abi::InvocationReply::refused(Uuid::now_v7(), Refusal::GuestTrap, 17, 42),
        module_cached: false,
        retire: false,
        evicted: None,
    })
    .expect("a valid reply frame encodes");
    let host_call = wire::encode_host_call(&HostCallFrame {
        invocation_id: Uuid::now_v7(),
        call_id: 0,
        op: wire::HostCallOp::KvGet {
            namespace: "cache".into(),
            key: "k".into(),
        },
    })
    .expect("a valid host-call frame encodes");
    let host_reply = wire::encode_host_reply(&HostReplyFrame {
        invocation_id: Uuid::now_v7(),
        call_id: 0,
        outcome: wire::HostCallOutcome::Ok(wire::HostCallValue::Absent),
    })
    .expect("a valid host-reply frame encodes");
    vec![
        ("invoke", invoke),
        ("reply", reply),
        ("host_call", host_call),
        ("host_reply", host_reply),
    ]
}

/// Decodes a frame of `kind`, so a mutant is judged by the same entry point the
/// side that trusts it uses.
fn decode(kind: &str, bytes: &[u8]) -> Result<(), wire::WireError> {
    // The four-byte length prefix is read by the frame reader, not by these
    // codecs, so the body is what they see.
    let body = bytes.get(4..).ok_or(wire::WireError::Truncated)?;
    match kind {
        "invoke" => wire::decode_invoke(body).map(|_| ()),
        "reply" => wire::decode_reply(body).map(|_| ()),
        "host_call" => wire::decode_host_call(body).map(|_| ()),
        "host_reply" => wire::decode_host_reply(body).map(|_| ()),
        other => panic!("unknown frame kind {other}"),
    }
}

/// Every mutation is one of: flip a bit, replace a byte, truncate, extend, or
/// splice two seeds. The invariants are the point: no panic, and a frame that
/// does decode obeys the caps the codec declares.
#[test]
fn mutated_frames_never_panic_and_decoded_frames_obey_their_caps() {
    let seeds = seeds();
    let mut rng = Rng(0x5EED_1234_ABCD_0001);
    let mut decoded = 0usize;
    let mut refused = 0usize;

    for round in 0..4_000 {
        let (kind, seed) = &seeds[rng.below(seeds.len())];
        let mut bytes = seed.clone();
        match rng.below(5) {
            0 => {
                let at = rng.below(bytes.len());
                bytes[at] ^= 1 << (rng.below(8) as u32);
            }
            1 => {
                let at = rng.below(bytes.len());
                bytes[at] = rng.byte();
            }
            2 => {
                let keep = rng.below(bytes.len().max(1));
                bytes.truncate(keep);
            }
            3 => {
                let extra = rng.below(64);
                for _ in 0..extra {
                    bytes.push(rng.byte());
                }
            }
            _ => {
                let (_, other) = &seeds[rng.below(seeds.len())];
                let split = rng.below(bytes.len().max(1));
                let tail = rng.below(other.len().max(1));
                bytes.truncate(split);
                bytes.extend_from_slice(&other[tail..]);
            }
        }

        match decode(kind, &bytes) {
            Ok(()) => {
                decoded += 1;
                if *kind == "invoke" {
                    let frame = wire::decode_invoke(&bytes[4..]).expect("decoded once");
                    assert!(
                        frame.request.payload.len() <= abi::MAX_INPUT_BYTES,
                        "round {round}: a decoded request carries an oversize payload"
                    );
                    if let Some(module) = &frame.module {
                        assert!(
                            module.len() <= wire::MAX_MODULE_BYTES,
                            "round {round}: a decoded module exceeds the frame cap"
                        );
                    }
                }
            }
            Err(_) => refused += 1,
        }
    }

    assert!(
        refused > decoded,
        "the generator produced {decoded} decodable frames of {}: a fuzzer that \
         mostly produces valid frames is testing the happy path",
        decoded + refused
    );
}

/// The guest's return word is four bytes of pointer and four of length, and the
/// guest chooses both. This sweeps the boundary values a hostile guest would
/// pick: zero length, the cap, the cap plus one, a pointer past the memory, a
/// range that wraps, and the whole memory with a one-byte overrun.
#[test]
fn every_boundary_return_word_is_a_typed_answer() {
    let sandbox =
        Sandbox::with_limits(Arc::new(register_all()), SandboxLimits::default()).expect("sandbox");

    // One module, sixteen return words. The memory is one page (65,536 bytes).
    let words: [u64; 16] = [
        0,
        1,
        0x0000_0001_0000_0000, // ptr 1, len 0
        0x0000_0400_0000_0000, // ptr 1024, len 0
        0x0000_0400_0000_0001, // ptr 1024, len 1
        0x0000_0400_0010_0000, // ptr 1024, len 1 MiB (cap)
        0x0000_0400_0010_0001, // ptr 1024, len cap + 1
        0x0000_FFFF_0000_0001, // ptr past the memory
        0x0000_FFFF_FFFF_FFFF, // both fields at their maximum
        0x0000_0000_FFFF_FFFF, // ptr 0, len u32::MAX
        0x0000_FF00_0000_1000, // ptr past memory, plausible length
        0x0000_7FFF_FFFF_FFFF, // ptr past memory, length at max
        0x0000_0400_0001_0000, // 65,536-byte read at 1024: exactly past the page
        0x0000_0400_0000_1001, // ptr 1024, len 4097
        0x8000_0000_0000_0001, // the sign bit set
        0x0000_0000_0000_0000, // the null word again, as the last case
    ];

    for word in words {
        let source = format!(
            r#"(module
                 (memory (export "memory") 1)
                 (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
                 (func (export "marula_free") (param i32) (param i32))
                 (func (export "marula_entry") (param i32) (param i32) (result i64)
                   (i64.const {word})))"#
        );
        let bytes = wat::parse_str(&source).expect("valid WAT");
        let artifact = ModuleArtifact {
            digest: ModuleArtifact::digest_of(&bytes),
            bytes,
        };
        let module = sandbox.load(&artifact).expect("loads");
        match sandbox.run(&module, &request()) {
            Ok(output) => {
                let len = output.output.as_ref().map(Vec::len).unwrap_or(0);
                assert!(
                    len <= abi::MAX_OUTPUT_BYTES,
                    "word {word:#x} returned {len} bytes, over the cap"
                );
                assert!(
                    word != 0x0000_0400_0010_0001 && word != 0x0000_FFFF_FFFF_FFFF,
                    "word {word:#x} claimed more than the cap and was answered Ok"
                );
            }
            Err(failure) => assert!(
                matches!(
                    failure.refusal,
                    Refusal::FrameInvalid | Refusal::OutputTooLarge | Refusal::InputTooLarge
                ),
                "word {word:#x} was refused with {:?}, which is not a frame verdict",
                failure.refusal
            ),
        }
    }
}

/// In-band frames: a truncated, mistagged or oversize frame is an error the
/// guest can act on, and the parser never reads past what it was given.
#[test]
fn in_band_reply_frames_are_parsed_defensively() {
    let mut rng = Rng(0x5EED_1234_ABCD_0002);
    let good = abi::ok_frame(b"hello");
    let bad = abi::err_frame(Refusal::KvQuotaExceeded);

    assert_eq!(abi::parse_frame(&good), Ok(&b"hello"[..]));
    assert_eq!(
        abi::parse_frame(&bad),
        Err(Refusal::KvQuotaExceeded),
        "an error frame names the refusal it carries"
    );
    assert_eq!(abi::parse_frame(&[]), Err(Refusal::FrameInvalid));
    assert_eq!(
        abi::parse_frame(&[abi::REPLY_TAG_OK]),
        Ok(&[][..]),
        "an OK tag with no payload is an empty answer, not a malformed one"
    );

    for round in 0..2_000 {
        let mut frame = if rng.below(2) == 0 {
            good.clone()
        } else {
            bad.clone()
        };
        match rng.below(3) {
            0 => frame.truncate(rng.below(frame.len().max(1))),
            1 => {
                let at = rng.below(frame.len());
                frame[at] = rng.byte();
            }
            _ => {
                frame.push(rng.byte());
                frame.push(rng.byte());
            }
        }
        let parsed = abi::parse_frame(&frame);
        if let Ok(payload) = parsed {
            assert!(
                payload.len() <= abi::MAX_OUTPUT_BYTES,
                "round {round}: a parsed frame carries an oversize payload"
            );
        }
    }

    // Every refusal has an error frame the guest can decode, and the tag byte is
    // not the OK tag.
    for refusal in Refusal::ALL {
        let frame = abi::err_frame(refusal);
        assert_eq!(frame[0], abi::REPLY_TAG_ERR);
        assert_eq!(abi::parse_frame(&frame), Err(refusal));
    }
}
