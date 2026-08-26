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
}

impl EffectApi for ReqwestEffectApi {
    async fn post(&self, path: &'static str, body: Value) -> EffectOutcome {
        let Some(token) = &self.token else {
            tracing::warn!(
                path,
                "MERIDIAN_INTERNAL_TOKEN unset; skipping effect call (job will dead-letter)"
            );
            return EffectOutcome::Rejected;
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
            Ok(resp) if resp.status().is_success() => EffectOutcome::Applied,
            Ok(resp) if resp.status().is_client_error() => {
                tracing::warn!(status = %resp.status(), path, "core-api rejected effect");
                EffectOutcome::Rejected
            }
            Ok(resp) => {
                tracing::error!(status = %resp.status(), path, "core-api error on effect");
                EffectOutcome::Transient
            }
            Err(error) => {
                tracing::error!(%error, path, "effect call transport failure");
                EffectOutcome::Transient
            }
        }
    }
}

/// Test double recording every posted body.
pub struct RecordingEffectApi {
    pub calls: std::sync::Mutex<Vec<(&'static str, Value)>>,
    pub outcome: EffectOutcome,
}

impl RecordingEffectApi {
    pub fn new(outcome: EffectOutcome) -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
            outcome,
        }
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
