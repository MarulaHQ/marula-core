//! The parent bridge: one channel, one invocation binding, one call counter.
//!
//! A worker cannot reach anything the parent owns — no database, no socket, no
//! credential. What it can do is ask, one frame at a time, on the pipe it was
//! started with; this module is the worker's end of that conversation.
//!
//! ## Why there is ONE counter
//!
//! The parent requires `call_id` to be exactly the next number it expects
//! (`runtime`'s correlator check). Two independent counters — one for the KV
//! store, one for the HTTP transport — would both start at zero and the second
//! call of either kind would arrive as a duplicate. So the counter lives here,
//! once, and both features share it: [`crate::kv::BridgeKvStore`] and
//! [`crate::egress::BridgeHttp`] are two faces of one client.
//!
//! ## Why the invocation is bound per run, not per store
//!
//! The channel is installed once at worker start (there is one worker per
//! process and its pipes never change). The invocation id is not, so
//! [`HostCallClient::begin_invocation`] sets it immediately before the sandbox
//! runs and [`HostCallClient::end_invocation`] clears it afterwards. A call
//! with no bound invocation is a transport error rather than a frame with a
//! guessed id: the parent would refuse it anyway, and inventing one would put a
//! wrong correlator in a log.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use uuid::Uuid;

use crate::abi::Refusal;
use crate::wire::{
    self, HostCallFrame, HostCallOp, HostCallRefusal, HostCallValue, HostReplyFrame,
};

/// The transport one call is handed to. The worker binary implements it over
/// the stdin/stdout pair it already owns; a test implements it with a queue.
///
/// `Err` is a transport failure — a closed pipe, a frame that will not decode,
/// an answer to another call. It is never a refusal: a refusal is a well-formed
/// [`wire::HostCallOutcome::Refused`] the parent chose, which is a different
/// thing.
pub trait HostCallChannel: Send + Sync {
    fn exchange(&self, frame: &HostCallFrame) -> Result<HostReplyFrame, String>;
}

/// What a host call produced: an answer, a refusal, or a broken bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    /// The parent chose to refuse. The guest sees the ABI code the refusal
    /// maps onto.
    Refused(HostCallRefusal),
    /// No invocation is bound, so there is nothing to attribute a call to.
    /// Distinct from a transport failure because it is not one: the caller is
    /// asking outside the window an invocation is running.
    NotBound,
    /// The bridge itself failed. The message is for the log, never for the
    /// guest.
    Transport(String),
}

impl CallError {
    /// The typed refusal a guest sees. A broken bridge is
    /// [`Refusal::WorkerFailed`], which is what it is: the mechanism failed and
    /// the caller must fail the node instead of recording a decision.
    pub fn refusal(&self) -> Refusal {
        match self {
            CallError::Refused(refusal) => refusal.as_refusal(),
            CallError::NotBound => Refusal::NotImplemented,
            CallError::Transport(_) => Refusal::WorkerFailed,
        }
    }
}

impl From<HostCallRefusal> for CallError {
    fn from(refusal: HostCallRefusal) -> Self {
        CallError::Refused(refusal)
    }
}

/// The shared client: frames out, checked answers back.
pub struct HostCallClient {
    channel: Arc<dyn HostCallChannel>,
    invocation: RwLock<Option<Uuid>>,
    next_call: AtomicU64,
}

impl HostCallClient {
    pub fn new(channel: Arc<dyn HostCallChannel>) -> Arc<Self> {
        Arc::new(HostCallClient {
            channel,
            invocation: RwLock::new(None),
            next_call: AtomicU64::new(0),
        })
    }

    /// Binds the client to the invocation about to run. The counter restarts at
    /// zero, because the parent counts calls per invocation.
    pub fn begin_invocation(&self, invocation_id: Uuid) {
        *self
            .invocation
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(invocation_id);
        self.next_call.store(0, Ordering::SeqCst);
    }

    /// Unbinds the invocation. A host call after this point fails as a
    /// transport error rather than being attributed to the invocation that just
    /// finished.
    pub fn end_invocation(&self) {
        *self
            .invocation
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    /// Whether an invocation is currently bound. A caller that needs to know
    /// before building a frame asks here.
    pub fn bound_invocation(&self) -> Option<Uuid> {
        *self
            .invocation
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// One frame out, one frame back, with both correlators checked.
    pub fn ask(&self, op: HostCallOp) -> Result<HostCallValue, CallError> {
        let invocation_id = self.bound_invocation().ok_or(CallError::NotBound)?;
        let call_id = self.next_call.fetch_add(1, Ordering::SeqCst);
        let reply = self
            .channel
            .exchange(&HostCallFrame {
                invocation_id,
                call_id,
                op,
            })
            .map_err(CallError::Transport)?;
        if reply.invocation_id != invocation_id || reply.call_id != call_id {
            // A stale or crossed reply would otherwise be handed to the guest
            // as if it answered the call it just made.
            return Err(CallError::Transport(
                "the parent answered a different host call".to_string(),
            ));
        }
        match reply.outcome {
            wire::HostCallOutcome::Ok(value) => Ok(value),
            wire::HostCallOutcome::Refused(refusal) => Err(CallError::Refused(refusal)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A channel that records what it was asked and answers from a queue.
    struct Canned {
        seen: Mutex<Vec<HostCallFrame>>,
        answers: Mutex<Vec<Result<HostReplyFrame, String>>>,
    }

    impl Canned {
        fn new(answers: Vec<Result<HostReplyFrame, String>>) -> Arc<Self> {
            Arc::new(Canned {
                seen: Mutex::new(Vec::new()),
                answers: Mutex::new(answers),
            })
        }
    }

    impl HostCallChannel for Canned {
        fn exchange(&self, frame: &HostCallFrame) -> Result<HostReplyFrame, String> {
            self.seen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(frame.clone());
            self.answers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(0)
        }
    }

    fn frame(call_id: u64) -> HostReplyFrame {
        HostReplyFrame {
            invocation_id: Uuid::nil(),
            call_id,
            outcome: wire::HostCallOutcome::Ok(HostCallValue::Unit),
        }
    }

    #[test]
    fn the_counter_is_shared_so_two_features_never_repeat_a_call() {
        // The point of this module: a KV call and an HTTP call in the same
        // invocation must be numbered 0 and 1, not 0 and 0.
        let channel = Canned::new(vec![Ok(frame(0)), Ok(frame(1))]);
        let client = HostCallClient::new(channel.clone());
        client.begin_invocation(Uuid::nil());
        client
            .ask(HostCallOp::KvDelete {
                namespace: "ns".into(),
                key: "k".into(),
            })
            .expect("first call");
        client
            .ask(HostCallOp::HttpRequest {
                profile: "acme".into(),
                request: crate::abi::HttpRequest {
                    method: "GET".into(),
                    url: "https://api.acme.test/v1".into(),
                    headers: Default::default(),
                    body: None,
                },
            })
            .expect("second call");
        let seen = channel
            .seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(seen[0].call_id, 0);
        assert_eq!(seen[1].call_id, 1);
        assert_eq!(seen[1].op.verb().as_str(), "http_request");
    }

    #[test]
    fn an_answer_to_another_call_is_a_transport_failure() {
        let channel = Canned::new(vec![Ok(frame(7))]);
        let client = HostCallClient::new(channel);
        client.begin_invocation(Uuid::nil());
        let error = client
            .ask(HostCallOp::KvDelete {
                namespace: "ns".into(),
                key: "k".into(),
            })
            .expect_err("a crossed reply is not an answer");
        assert!(matches!(error, CallError::Transport(_)));
        assert_eq!(error.refusal(), Refusal::WorkerFailed);
    }

    #[test]
    fn a_call_with_no_bound_invocation_never_reaches_the_channel() {
        let channel = Canned::new(vec![]);
        let client = HostCallClient::new(channel.clone());
        let error = client
            .ask(HostCallOp::KvDelete {
                namespace: "ns".into(),
                key: "k".into(),
            })
            .expect_err("nothing is bound");
        assert!(matches!(error, CallError::NotBound));
        assert_eq!(error.refusal(), Refusal::NotImplemented);
        assert!(
            channel
                .seen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty(),
            "an unbound call must not be guessed onto the wire"
        );
    }

    #[test]
    fn the_counter_restarts_when_a_new_invocation_binds() {
        let channel = Canned::new(vec![Ok(frame(0)), Ok(frame(0))]);
        let client = HostCallClient::new(channel.clone());
        client.begin_invocation(Uuid::nil());
        client
            .ask(HostCallOp::KvDelete {
                namespace: "ns".into(),
                key: "k".into(),
            })
            .expect("first invocation");
        client.end_invocation();
        client.begin_invocation(Uuid::nil());
        client
            .ask(HostCallOp::KvDelete {
                namespace: "ns".into(),
                key: "k".into(),
            })
            .expect("second invocation");
        let seen = channel
            .seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(
            seen.iter().map(|f| f.call_id).collect::<Vec<_>>(),
            vec![0, 0]
        );
    }
}
