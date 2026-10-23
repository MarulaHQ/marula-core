//! The plugin worker: a long-lived child process that runs one guest invocation
//! at a time (ADR 0023 Decision 2).
//!
//! It speaks [`plugin_host::wire`] on stdin/stdout — **stdout is protocol only**
//! — and writes every diagnostic to stderr. It authorizes nothing: the frame it
//! receives is the parent's resolved answer, it re-validates that frame into an
//! empty `InvocationState`, and it can only run the module whose sha256 the
//! parent named.
//!
//! ## The host-call bridge (W53-11)
//!
//! A guest's `kv_get` runs inside a wasmtime `Caller`, synchronously, in this
//! process: there is no runtime here and no database. So this binary installs
//! [`plugin_host::kv::BridgeKvStore`], which writes a host-call frame to stdout
//! and blocks on the matching reply from stdin. The parent answers it with its
//! own connection and its own record of the invocation
//! (`plugin_host::runtime::HostCallService`). This process still holds no
//! tenant identity and no credential.
//!
//! Two facts make the re-entrant read safe, and both are structural rather than
//! hoped for:
//!
//! 1. The main loop releases both pipes before it calls into the sandbox: it
//!    reads a frame, drops the guard, runs the invocation, and only then reads
//!    again. The bridge therefore never contends with it.
//! 2. Neither side is buffered. The pipes are duplicated descriptors read with
//!    `read_exact` on a `File`, so a read takes exactly the bytes it asked for
//!    and cannot swallow the next frame. (A `BufReader` here would be a bug: two
//!    readers over one descriptor, one of them reading ahead.)
//!
//! It exits when stdin closes, when it cannot parse a frame (that is the
//! parent's bug, and guessing is worse than dying), or after answering an
//! invocation that found it above its resident ceiling (exit code
//! [`EXIT_RETIRED`], so the parent recycles it without losing the answer it just
//! wrote).
//!
//! Every limit lives in [`plugin_host::isolation`]; this binary is transport,
//! dispatch, the host-call pipe and its own resident-set check.

//! # What this binary must not link
//!
//! No HTTP client, no TLS stack, no credential, no key. `plugin-host` has no
//! `reqwest` dependency at all (W53-12 moved the transport into core-api), and
//! the transitive path through `core-store`'s object layer is not reachable from
//! anything this binary calls, so the linker drops it. That is measurable, not
//! assumed:
//!
//! ```text
//! cargo build -p plugin-host --bin marula-plugin-worker
//! nm -a .kilo/target-shared/debug/marula-plugin-worker | grep -c reqwest   # 0
//! ```
//!
//! If you add a dependency or a call here, run that before you commit it. A
//! worker that can construct a request is a worker where a guest's next escape
//! is a network egress rather than a trap.

use std::fs::File;
use std::io::{Read, Write};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use plugin_host::abi::{InvocationReply, Refusal};
use plugin_host::bridge::{HostCallChannel, HostCallClient};
use plugin_host::egress::{self, BridgeHttp};
use plugin_host::host_fns::register_all;
use plugin_host::isolation::{LoadError, ModuleArtifact, Sandbox};
use plugin_host::kv::{self, BridgeKvStore};
use plugin_host::runtime::{forward_logs, over_resident_ceiling, RESIDENT_CHECK_EVERY};
use plugin_host::wire::{self, HostCallFrame, HostReplyFrame, InvokeFrame, ReplyFrame};

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

    let pipes = match Pipes::open() {
        Ok(pipes) => Arc::new(pipes),
        Err(error) => {
            eprintln!("plugin-worker: stdio: {error}");
            return ExitCode::FAILURE;
        }
    };
    let channel = Arc::new(WorkerChannel {
        pipes: Arc::clone(&pipes),
    });
    // ONE client for both features, installed before the hello frame so no
    // invocation can run with a seam unset. The parent expects each invocation's
    // host calls to be numbered 0, 1, 2, … across every verb, so the KV store and
    // the HTTP transport must share one counter — see `plugin_host::bridge`.
    //
    // The HTTP transport is a frame, not a client: this process has no network
    // library to link (W53-12 moved the transport parent-side, which is why
    // `plugin-host` no longer depends on `reqwest`).
    let client = HostCallClient::new(channel);
    let bridge = Arc::new(BridgeKvStore::with_client(Arc::clone(&client)));
    kv::install(Arc::clone(&bridge) as Arc<dyn kv::KvStore>);
    egress::install(Arc::new(BridgeHttp::with_client(client)));

    // Announce ourselves before reading anything: the parent's `start()` proves
    // the binary speaks this protocol at boot instead of on a user request.
    match wire::encode_hello(std::process::id())
        .map_err(|error| error.to_string())
        .and_then(|frame| pipes.write(&frame))
    {
        Ok(()) => {}
        Err(error) => {
            eprintln!("plugin-worker: hello: {error}");
            return ExitCode::FAILURE;
        }
    }

    let mut invocations: u64 = 0;
    loop {
        let body = match pipes.read() {
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
        let (reply, module_cached, evicted) = run_one(&sandbox, &bridge, &frame, started);
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
        if let Err(error) = pipes.write(&encoded) {
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

/// The worker's two ends of the protocol, shared with the bridge.
///
/// Duplicated descriptors rather than `stdin().lock()`/`stdout().lock()`,
/// because a host function runs while the main loop is inside `sandbox.run`: a
/// std lock held across that call would be re-entered by the bridge and
/// self-deadlock, and the std handles buffer. See this file's header.
struct Pipes {
    input: Mutex<File>,
    output: Mutex<File>,
}

impl Pipes {
    fn open() -> Result<Pipes, String> {
        use std::os::fd::AsFd;
        let input = std::io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .map(File::from)
            .map_err(|error| format!("stdin: {error}"))?;
        let output = std::io::stdout()
            .as_fd()
            .try_clone_to_owned()
            .map(File::from)
            .map_err(|error| format!("stdout: {error}"))?;
        Ok(Pipes {
            input: Mutex::new(input),
            output: Mutex::new(output),
        })
    }

    /// One length-prefixed body. `Ok(None)` is a clean EOF between frames.
    fn read(&self) -> Result<Option<Vec<u8>>, String> {
        let mut input = self
            .input
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        read_body(&mut *input)
    }

    fn write(&self, frame: &[u8]) -> Result<(), String> {
        let mut output = self
            .output
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        write_frame(&mut *output, frame)
    }
}

/// The bridge's transport: one frame out, the matching frame back, no locks
/// held across the pair beyond each single operation.
impl HostCallChannel for WorkerChannel {
    fn exchange(&self, frame: &HostCallFrame) -> Result<HostReplyFrame, String> {
        let encoded = wire::encode_host_call(frame).map_err(|error| error.to_string())?;
        self.pipes.write(&encoded)?;
        // The main loop is not reading here (see the header), and the parent is
        // the only writer on the other end of this pipe.
        let body = self
            .pipes
            .read()?
            .ok_or_else(|| "the parent closed the pipe during a host call".to_string())?;
        let reply = wire::decode_host_reply(&body).map_err(|error| error.to_string())?;
        if reply.invocation_id != frame.invocation_id {
            return Err("the parent answered another invocation".to_string());
        }
        Ok(reply)
    }
}

struct WorkerChannel {
    pipes: Arc<Pipes>,
}

/// Runs one frame: load the module if the parent sent it, then execute — with
/// the bridge bound to this invocation for exactly the duration of the run.
///
/// A load failure is a REFUSED reply, not an exit: the worker is healthy and the
/// parent is the one that named a digest whose bytes do not match, which is a
/// frame-level refusal (`plugin_abi_frame_invalid`) and not a worker death.
fn run_one(
    sandbox: &Sandbox,
    bridge: &BridgeKvStore,
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
    // The store installed at start is the bridge; binding the invocation is
    // what makes its frames nameable. A guest that somehow runs without it is
    // answered `plugin_not_implemented` rather than a guessed invocation id.
    bridge.begin_invocation(invocation_id);
    let outcome = sandbox.run(&module, request);
    bridge.end_invocation();

    match outcome {
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
