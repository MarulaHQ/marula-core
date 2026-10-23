//! `http_request`'s two halves: the seam the PARENT implements, and the client
//! the WORKER uses to reach it (Wave 53 lane 12, phase 2).
//!
//! ## Why the request leaves the worker at all
//!
//! A secret can only be decrypted in a process that must never run untrusted
//! wasm. Phase 1 built the binding between an install's egress profile and a
//! vault record; this module is how the request that wears that record is
//! performed somewhere else. The guest names a profile and hands over a request.
//! The worker forwards it. The parent screens it, resolves the binding,
//! decrypts, injects one header and sends — and hands back status, headers and
//! body, which is all the guest ever gets.
//!
//! ## What that costs, said plainly
//!
//! One extra pipe round trip per call, which is noise against the 5,000 ms
//! budget, and the response body crosses the pipe (bounded by
//! `wire::MAX_HOST_CALL_HTTP_BODY_BYTES`, the transport's own 2 MiB cap). What
//! it buys: the worker process ends the wave with **no HTTP client, no
//! credential bytes, no decryption key, and no network library at all** —
//! `reqwest` is not a dependency of this crate, so the worker cannot link one
//! even by accident.
//!
//! ## The allowlist runs twice, from one implementation
//!
//! The worker's `Egress` gate still screens the URL before any frame is written,
//! because an unconsented profile should not cost a round trip. The parent
//! screens again with the same `core_domain::egress` functions against its own
//! copy of the install's `outbound_scopes`, because the worker is the process it
//! isolates. Two calls of one function is defence in depth; two implementations
//! would be the drift this design exists to prevent.

use std::cell::RefCell;
use std::sync::{Arc, RwLock};

use core_domain::egress::EgressTarget;

use crate::abi::{self, Refusal};
use crate::bridge::{CallError, HostCallClient};
use crate::wire::{HostCallOp, HostCallValue};

/// What the parent needs decrypted, and where.
///
/// `sealed` is ciphertext: `core_db::plugin_credentials::resolve` never returns
/// anything else, and this type deliberately has no field a plaintext could ride
/// in on. The three coordinate fields are the AAD the ciphertext was sealed
/// against (`secret_field:{org}:{record}:{field}`), so the caller rebuilds the
/// AAD from the row it read instead of re-deriving a format that lives in
/// core-api.
#[derive(Clone)]
pub struct SealedCredential {
    pub org_id: uuid::Uuid,
    pub record_id: uuid::Uuid,
    pub field_key: String,
    /// The header the value is injected as, lowercased when it was bound.
    pub header_name: String,
    /// Ciphertext. Never plaintext.
    pub sealed: Vec<u8>,
}

/// Hand-written so a log line that formats this prints a length and a header
/// name, never the bytes. Ciphertext is not a disclosure on its own, but a log
/// line nobody chose to write is how one usually gets out.
impl std::fmt::Debug for SealedCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SealedCredential")
            .field("org_id", &self.org_id)
            .field("record_id", &self.record_id)
            .field("field_key", &self.field_key)
            .field("header_name", &self.header_name)
            .field(
                "sealed",
                &format_args!("<{} sealed bytes>", self.sealed.len()),
            )
            .finish()
    }
}

/// The PARENT's half of `http_request`: perform one already-screened request.
///
/// Implemented by core-api (`plugin_http`), which owns the secret key, the
/// client and the DNS pinning. `plugin-host` defines the shape and never
/// implements it, which is what keeps this crate free of a network client.
pub trait HttpEgress: Send {
    /// `target` is the SCREENED URL — the transport fetches that, never the
    /// guest's string. `credential` is present only when the install has a
    /// binding for `target`'s profile. `timeout` is the remaining transport
    /// budget for this invocation, already clamped by the caller.
    fn send<'a>(
        &'a mut self,
        target: &'a EgressTarget,
        request: &'a abi::HttpRequest,
        credential: Option<&'a SealedCredential>,
        timeout: std::time::Duration,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<abi::HttpResponse, Refusal>> + Send + 'a>,
    >;
}

/// The transport a caller that has none is handed.
///
/// It answers `plugin_not_implemented`, which is the honest answer for a path
/// with nowhere to send: the bare [`crate::exec::invoke_surface`] entry point and
/// a unit test. Production core-api always supplies one.
pub struct NoHttpEgress;

impl HttpEgress for NoHttpEgress {
    fn send<'a>(
        &'a mut self,
        _target: &'a EgressTarget,
        _request: &'a abi::HttpRequest,
        _credential: Option<&'a SealedCredential>,
        _timeout: std::time::Duration,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<abi::HttpResponse, Refusal>> + Send + 'a>,
    > {
        Box::pin(async move { Err(Refusal::NotImplemented) })
    }
}

/// The WORKER's half: a `http_request` host entry asks the parent.
pub trait HttpTransport: Send + Sync {
    /// The profile the guest named, and the request it built. No tenant, no
    /// record, no credential.
    fn send(&self, profile: &str, request: &abi::HttpRequest)
        -> Result<abi::HttpResponse, Refusal>;
}

/// The transport both isolation modes install: it is a frame on the parent
/// bridge, and the channel differs rather than the behaviour (ADR 0023
/// Decision 2 — the transport changes nothing about what a plugin may do).
pub struct BridgeHttp {
    client: Arc<HostCallClient>,
}

impl BridgeHttp {
    pub fn with_client(client: Arc<HostCallClient>) -> Self {
        BridgeHttp { client }
    }

    /// From the channel alone, which is how a test builds one.
    pub fn new(channel: Arc<dyn crate::bridge::HostCallChannel>) -> Self {
        BridgeHttp {
            client: HostCallClient::new(channel),
        }
    }
}

impl HttpTransport for BridgeHttp {
    fn send(
        &self,
        profile: &str,
        request: &abi::HttpRequest,
    ) -> Result<abi::HttpResponse, Refusal> {
        let op = HostCallOp::HttpRequest {
            profile: profile.to_string(),
            request: request.clone(),
        };
        match self.client.ask(op) {
            Ok(HostCallValue::Http(response)) => Ok(response),
            // A reply that is not an HTTP answer means the parent answered the
            // wrong verb for this call. That is a broken bridge, not a refusal
            // the guest caused.
            Ok(_) => Err(Refusal::WorkerFailed),
            Err(CallError::Refused(refusal)) => Err(refusal.as_refusal()),
            Err(CallError::NotBound) => Err(Refusal::NotImplemented),
            Err(CallError::Transport(_)) => Err(Refusal::WorkerFailed),
        }
    }
}

thread_local! {
    /// The transport for THIS thread. `in_process` runs two invocations on two
    /// blocking threads, and a process-wide install would let one guest's
    /// request travel on the other's bridge.
    static THREAD_HTTP: RefCell<Option<Arc<dyn HttpTransport>>> = const { RefCell::new(None) };
}

/// The transport for this process, when it is installed once (a pool worker).
static PROCESS_HTTP: RwLock<Option<Arc<dyn HttpTransport>>> = RwLock::new(None);

/// Installs the process-wide transport. Idempotent in effect: a second call
/// replaces the first, which is what a restarted worker wants.
pub fn install(http: Arc<dyn HttpTransport>) {
    *PROCESS_HTTP
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(http);
}

/// The transport this thread resolves to, if any. Thread first, then process.
pub fn current() -> Option<Arc<dyn HttpTransport>> {
    if let Some(http) = THREAD_HTTP.with(|slot| slot.borrow().clone()) {
        return Some(http);
    }
    PROCESS_HTTP
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Runs `f` with `http` as this thread's transport, restoring the previous value
/// afterwards — including when `f` panics, which is what makes it safe from a
/// test that may fail an assertion.
pub fn with_http<T>(http: Arc<dyn HttpTransport>, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<Arc<dyn HttpTransport>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let previous = self.0.take();
            THREAD_HTTP.with(|slot| *slot.borrow_mut() = previous);
        }
    }

    let previous = THREAD_HTTP.with(|slot| slot.borrow_mut().replace(http));
    let _restore = Restore(previous);
    f()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Recording {
        answered: Refusal,
        seen: std::sync::Mutex<Vec<String>>,
    }

    impl HttpTransport for Recording {
        fn send(
            &self,
            profile: &str,
            _request: &abi::HttpRequest,
        ) -> Result<abi::HttpResponse, Refusal> {
            self.seen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(profile.to_string());
            Err(self.answered)
        }
    }

    /// The bridge maps a parent refusal onto the guest's vocabulary, and code 25
    /// survives the trip. A canned channel plays the parent.
    #[test]
    fn a_parent_refusal_reaches_the_guest_as_its_abi_code() {
        use crate::bridge::HostCallChannel;
        use crate::wire::{HostCallFrame, HostCallOutcome, HostCallRefusal, HostReplyFrame};

        struct Canned(HostCallRefusal);
        impl HostCallChannel for Canned {
            fn exchange(&self, frame: &HostCallFrame) -> Result<HostReplyFrame, String> {
                Ok(HostReplyFrame {
                    invocation_id: frame.invocation_id,
                    call_id: frame.call_id,
                    outcome: HostCallOutcome::Refused(self.0),
                })
            }
        }

        let request = abi::HttpRequest {
            method: "GET".into(),
            url: "https://api.acme.test/v1".into(),
            headers: Default::default(),
            body: None,
        };
        let client = crate::bridge::HostCallClient::new(std::sync::Arc::new(Canned(
            HostCallRefusal::CredentialRefused,
        )));
        client.begin_invocation(uuid::Uuid::nil());
        let bridge = BridgeHttp::with_client(std::sync::Arc::clone(&client));
        assert_eq!(
            bridge.send("acme", &request),
            Err(Refusal::CredentialRefused),
            "code 25 is what the guest sees when the credential is refused"
        );
        assert_eq!(Refusal::CredentialRefused.code(), 25);
        assert_eq!(
            Refusal::CredentialRefused.as_str(),
            "plugin_credential_refused"
        );
    }

    #[test]
    fn a_thread_install_wins_over_the_process_one_and_then_restores() {
        let process: Arc<dyn HttpTransport> = Arc::new(Recording {
            answered: Refusal::EgressHostDenied,
            seen: std::sync::Mutex::new(Vec::new()),
        });
        install(Arc::clone(&process));
        let thread: Arc<dyn HttpTransport> = Arc::new(Recording {
            answered: Refusal::CredentialRefused,
            seen: std::sync::Mutex::new(Vec::new()),
        });
        let request = abi::HttpRequest {
            method: "GET".into(),
            url: "https://api.acme.test/v1".into(),
            headers: Default::default(),
            body: None,
        };
        with_http(Arc::clone(&thread), || {
            assert_eq!(
                current()
                    .expect("a thread transport")
                    .send("acme", &request),
                Err(Refusal::CredentialRefused)
            );
        });
        assert_eq!(
            current()
                .expect("the process transport")
                .send("acme", &request),
            Err(Refusal::EgressHostDenied),
            "the thread install must not outlive its scope"
        );
    }
}
