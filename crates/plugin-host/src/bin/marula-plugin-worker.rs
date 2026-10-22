//! The plugin worker: a long-lived child process that runs one guest invocation
//! at a time (ADR 0023 Decision 2).
//!
//! It speaks [`plugin_host::wire`] on stdin/stdout — **stdout is protocol only**
//! — and writes every diagnostic to stderr. It authorizes nothing: the frame it
//! receives is the parent's resolved answer, it re-validates that frame into an
//! empty `InvocationState`, and it can only run the module whose sha256 the
//! parent named.
//!
//! It exits when stdin closes, when it cannot parse a frame (that is the
//! parent's bug, and guessing is worse than dying), or after answering an
//! invocation that found it above its resident ceiling (exit code
//! [`EXIT_RETIRED`], so the parent recycles it without losing the answer it just
//! wrote).
//!
//! Every limit lives in [`plugin_host::isolation`]; this binary is transport,
//! dispatch and its own resident-set check.

use std::io::{Read, Write};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use plugin_host::abi::{InvocationReply, Refusal};
use plugin_host::host_fns::register_all;
use plugin_host::isolation::{LoadError, ModuleArtifact, Sandbox};
use plugin_host::runtime::{forward_logs, over_resident_ceiling, RESIDENT_CHECK_EVERY};
use plugin_host::wire::{self, InvokeFrame, ReplyFrame};

/// The worker answered its last invocation and is retiring because its resident
/// set is above ADR 0023 Decision 4's ceiling. Distinct from `1` (it could not
/// start) so a supervisor can tell "recycle me" from "I am broken".
pub const EXIT_RETIRED: u8 = 70;

fn main() -> ExitCode {
    let sandbox = match Sandbox::new(Arc::new(register_all())) {
        Ok(sandbox) => sandbox,
        Err(error) => {
            eprintln!("plugin-worker: sandbox: {error}");
            return ExitCode::FAILURE;
        }
    };

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();

    // Announce ourselves before reading anything: the parent's `start()` proves
    // the binary speaks this protocol at boot instead of on a user request.
    match wire::encode_hello(std::process::id())
        .map_err(|error| error.to_string())
        .and_then(|frame| write_frame(&mut output, &frame))
    {
        Ok(()) => {}
        Err(error) => {
            eprintln!("plugin-worker: hello: {error}");
            return ExitCode::FAILURE;
        }
    }

    let mut invocations: u64 = 0;
    loop {
        let body = match read_body(&mut input) {
            Ok(Some(body)) => body,
            // A clean EOF between frames: the parent dropped our stdin.
            Ok(None) => return ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("plugin-worker: read: {error}");
                return ExitCode::FAILURE;
            }
        };
        let frame = match wire::decode_invoke(&body) {
            Ok(frame) => frame,
            Err(error) => {
                eprintln!("plugin-worker: frame: {error}");
                return ExitCode::FAILURE;
            }
        };

        let started = Instant::now();
        let (reply, module_cached, evicted) = run_one(&sandbox, &frame, started);
        invocations += 1;

        // Measure the resident set on the way out of the invocation, never
        // inside it: reading /proc is 5-10 µs and the per-call budget is 9.38.
        let retire = invocations.is_multiple_of(RESIDENT_CHECK_EVERY) && over_resident_ceiling();
        let frame = ReplyFrame {
            reply,
            module_cached,
            retire,
            evicted,
        };
        let encoded = match wire::encode_reply(&frame) {
            Ok(encoded) => encoded,
            // The reply is bounded by the caps in `wire`; a failure here means a
            // bug in this process, and a silent drop would be a lost audit row.
            Err(error) => {
                eprintln!("plugin-worker: encode: {error}");
                return ExitCode::FAILURE;
            }
        };
        if let Err(error) = write_frame(&mut output, &encoded) {
            // The parent is gone. Nothing to answer and nothing to record.
            eprintln!("plugin-worker: write: {error}");
            return ExitCode::FAILURE;
        }
        if retire {
            eprintln!("plugin-worker: resident ceiling reached, retiring");
            return ExitCode::from(EXIT_RETIRED);
        }
    }
}

/// Runs one frame: load the module if the parent sent it, then execute.
///
/// A load failure is a REFUSED reply, not an exit: the worker is healthy and the
/// parent is the one that named a digest whose bytes do not match, which is a
/// frame-level refusal (`plugin_abi_frame_invalid`) and not a worker death.
fn run_one(
    sandbox: &Sandbox,
    frame: &InvokeFrame,
    started: Instant,
) -> (InvocationReply, bool, Option<String>) {
    let invocation_id = frame.request.invocation_id;
    let loaded = match (&frame.module, sandbox.get(&frame.digest)) {
        (Some(bytes), _) => sandbox.load_with_eviction(&ModuleArtifact {
            digest: frame.digest.clone(),
            bytes: bytes.clone(),
        }),
        (None, Some(module)) => Ok((module, None)),
        // The parent always sends the bytes for a digest this worker did not
        // acknowledge, so this is a protocol bug rather than a condition.
        (None, None) => Err(LoadError::NotCached(frame.digest.clone())),
    };
    let (module, evicted) = match loaded {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("plugin-worker: {error}");
            return (
                InvocationReply::refused(
                    invocation_id,
                    Refusal::FrameInvalid,
                    0,
                    elapsed_us(started),
                ),
                false,
                None,
            );
        }
    };

    let module_cached = sandbox.contains(&frame.digest);
    let request = &frame.request;
    let context = (request.org_id, request.install_id, invocation_id);
    match sandbox.run(&module, request) {
        Ok(output) => {
            forward_logs(context, &output.logs);
            (
                InvocationReply::ok(
                    invocation_id,
                    output.output,
                    output.fuel_used,
                    elapsed_us(started),
                ),
                module_cached,
                evicted,
            )
        }
        Err(failure) => {
            forward_logs(context, &failure.logs);
            (
                InvocationReply::refused(
                    invocation_id,
                    failure.refusal,
                    failure.fuel_used,
                    elapsed_us(started),
                ),
                module_cached,
                evicted,
            )
        }
    }
}

fn elapsed_us(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// Blocking length-prefixed read. `Ok(None)` is a clean EOF between frames.
fn read_body<R: Read>(input: &mut R) -> Result<Option<Vec<u8>>, String> {
    let mut len = [0u8; 4];
    match input.read_exact(&mut len) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(format!("length prefix: {error}")),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > wire::MAX_FRAME_BYTES {
        return Err(format!(
            "frame of {len} bytes is above the {}-byte ceiling",
            wire::MAX_FRAME_BYTES
        ));
    }
    let mut body = vec![0u8; len];
    input
        .read_exact(&mut body)
        .map_err(|error| format!("body: {error}"))?;
    Ok(Some(body))
}

fn write_frame<W: Write>(output: &mut W, frame: &[u8]) -> Result<(), String> {
    output.write_all(frame).map_err(|error| error.to_string())?;
    output.flush().map_err(|error| error.to_string())
}
