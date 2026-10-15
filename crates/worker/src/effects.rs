//! worker/effects — the HTTP seam to core-api's internal machine surface.
//!
//! Purpose: every worker-computed effect (SLA verdicts, automation
//! effects) crosses THIS seam as a plain POST with a bearer token. The
//! trait exists so tests can record calls without any network; the real
//! client never touches the database itself (§4.1: only core-api writes).
//!
//! Non-purpose: no retries here — durability is the job queue's business;
//! this module performs exactly one attempt per invocation and reports the
//! outcome classification back.

use serde_json::Value;

/// Outcome classes the queue runner maps to done/retry/dead decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectOutcome {
    /// 2xx — effect applied (or idempotently skipped).
    Applied,
    /// 4xx — permanent rejection; retrying cannot succeed.
    Rejected,
    /// Network/5xx — transient; the job should retry with backoff.
    Transient,
}

/// The machine-to-core-api seam. `body` is the pre-serialized request for
/// the target endpoint; `path` selects it ("/api/internal/sla/verdict"
/// or "/api/internal/effects").
pub trait EffectApi: Send + Sync {
    fn post(
        &self,
        path: &'static str,
        body: Value,
    ) -> impl std::future::Future<Output = EffectOutcome> + Send;

    /// The same call, returning the response body too. Most effects are
    /// fire-and-forget (the outcome classification is all the queue needs),
    /// but a workflow NODE's output is what a downstream `value.reference`
    /// reads, so the executor needs the body back. The default drops it —
    /// a test double that never sets an output gets `Null` for free.
    fn post_json(
        &self,
        path: &'static str,
        body: Value,
    ) -> impl std::future::Future<Output = (EffectOutcome, Value)> + Send {
        async move { (self.post(path, body).await, Value::Null) }
    }
}

/// Production client: reqwest with rustls, one attempt, no redirects.
#[derive(Clone)]
pub struct ReqwestEffectApi {
    base_url: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl ReqwestEffectApi {
    pub fn new(base_url: String, token: Option<String>) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
        }
    }

    /// One attempt, returning the classification and the decoded body when
    /// the call succeeded. `post` throws the body away; `post_json` keeps it.
    async fn call(&self, path: &'static str, body: Value) -> (EffectOutcome, Value) {
        let Some(token) = &self.token else {
            tracing::warn!(
                path,
                "MARULA_INTERNAL_TOKEN unset; skipping effect call (job will dead-letter)"
            );
            return (EffectOutcome::Rejected, Value::Null);
        };
        let url = format!("{}{path}", self.base_url);
        let response = self
            .http
            .post(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await;
        match response {
            Ok(resp) if resp.status().is_success() => {
                let value = resp.json::<Value>().await.unwrap_or(Value::Null);
                (EffectOutcome::Applied, value)
            }
            Ok(resp) if resp.status().is_client_error() => {
                tracing::warn!(status = %resp.status(), path, "core-api rejected effect");
                (EffectOutcome::Rejected, Value::Null)
            }
            Ok(resp) => {
                tracing::error!(status = %resp.status(), path, "core-api error on effect");
                (EffectOutcome::Transient, Value::Null)
            }
            Err(error) => {
                tracing::error!(%error, path, "effect call transport failure");
                (EffectOutcome::Transient, Value::Null)
            }
        }
    }
}

impl EffectApi for ReqwestEffectApi {
    async fn post(&self, path: &'static str, body: Value) -> EffectOutcome {
        self.call(path, body).await.0
    }

    async fn post_json(&self, path: &'static str, body: Value) -> (EffectOutcome, Value) {
        self.call(path, body).await
    }
}

/// Test double recording every posted body.
pub struct RecordingEffectApi {
    pub calls: std::sync::Mutex<Vec<(&'static str, Value)>>,
    pub outcome: EffectOutcome,
    /// The body `post_json` returns. `Null` unless set, so a fire-and-forget
    /// test needs nothing.
    pub output: Value,
}

impl RecordingEffectApi {
    pub fn new(outcome: EffectOutcome) -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
            outcome,
            output: Value::Null,
        }
    }

    /// Sets the body every `post_json` call returns.
    pub fn with_output(mut self, output: Value) -> Self {
        self.output = output;
        self
    }

    pub fn recorded(&self) -> Vec<(&'static str, Value)> {
        self.calls.lock().expect("mutex").clone()
    }
}

impl EffectApi for RecordingEffectApi {
    async fn post(&self, path: &'static str, body: Value) -> EffectOutcome {
        self.calls.lock().expect("mutex").push((path, body));
        self.outcome
    }

    async fn post_json(&self, path: &'static str, body: Value) -> (EffectOutcome, Value) {
        self.calls.lock().expect("mutex").push((path, body));
        (self.outcome, self.output.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn recording_api_captures_path_and_body() {
        let api = RecordingEffectApi::new(EffectOutcome::Applied);
        let out = api
            .post("/api/internal/sla/verdict", json!({ "x": 1 }))
            .await;
        assert_eq!(out, EffectOutcome::Applied);
        let calls = api.recorded();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "/api/internal/sla/verdict");
        assert_eq!(calls[0].1["x"], 1);
    }
}
