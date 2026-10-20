//! The TypeSafe System One adapter — the seam's first kind and test
//! case (ADR 0010 §1). Vendor knowledge lives ENTIRELY here: `POST
//! /v1/systemone` with `{ model, state, questions }`, pinned `jev-*`
//! model ids, and 429/529 exponential backoff. Swapping vendors
//! touches this file and the registry — never a call site.

use crate::provider::{
    normalize_distribution, Answer, DecisionError, DecisionProvider, DecisionRequest,
    DecisionResponse, Question,
};
use core_domain::retry::{self, Subsystem};
use serde_json::{json, Value};

/// The provider's retry budget and spacing come from the `decision` retry
/// policy (429/529 with exponential spacing, an honest refusal after the
/// budget, never a busy-loop — ADR 0010 §6). The numbers used to live here as
/// `MAX_ATTEMPTS` and `BASE_BACKOFF_MS`.
fn decision_policy() -> core_domain::retry::RetryPolicy {
    retry::policy(Subsystem::Decision)
}

pub struct SystemOneProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl SystemOneProvider {
    pub fn new(http: reqwest::Client, base_url: &str, api_key: &str) -> Self {
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
        }
    }

    fn validate_model(&self, model: &str) -> Result<(), DecisionError> {
        if model.starts_with("jev-") && model != "jev-latest" {
            Ok(())
        } else {
            Err(DecisionError::InvalidModel {
                model: model.to_string(),
                reason: "system_one models are pinned jev-* ids (jev-latest is not a pin)"
                    .to_string(),
            })
        }
    }

    fn question_wire(q: &Question) -> Value {
        match q {
            Question::Choice {
                id,
                instructions,
                options,
            } => json!({
                "id": id, "type": "choice",
                "instructions": instructions, "options": options,
            }),
            Question::Score {
                id,
                instructions,
                rubric,
            } => json!({
                "id": id, "type": "score",
                "instructions": instructions, "rubric": rubric,
            }),
            Question::Noul { id, instructions } => json!({
                "id": id, "type": "noul", "instructions": instructions,
            }),
        }
    }

    fn request_body(req: &DecisionRequest) -> Value {
        json!({
            "model": req.model,
            "state": req.state,
            "questions": req.questions.iter().map(Self::question_wire).collect::<Vec<_>>(),
        })
    }

    /// Parse + validate the wire response against the asked questions:
    /// one answer per question id, distributions match widths and
    /// normalize, picked/level indexes are in range.
    fn parse_response(
        req: &DecisionRequest,
        body: &Value,
    ) -> Result<DecisionResponse, DecisionError> {
        let answers_wire = body
            .get("answers")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                DecisionError::Protocol("response missing 'answers' array".to_string())
            })?;
        let usage = json!({
            "prompt_tokens": body.pointer("/usage/input_tokens").and_then(Value::as_i64).unwrap_or(0),
            "completion_tokens": 0,
        });

        let mut answers = Vec::with_capacity(req.questions.len());
        for q in &req.questions {
            let wire = answers_wire
                .iter()
                .find(|a| a.get("id").and_then(Value::as_str) == Some(q.id()))
                .ok_or_else(|| {
                    DecisionError::Protocol(format!(
                        "question '{}' missing from answer set",
                        q.id()
                    ))
                })?;
            let confidence = wire
                .get("confidence")
                .and_then(Value::as_f64)
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
            let distribution: Vec<f64> = wire
                .get("distribution")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_f64).collect())
                .unwrap_or_default();
            let answer = match q {
                Question::Choice { id, options, .. } => {
                    let distribution = normalize_distribution(&distribution, options.len(), id)?;
                    let picked = wire.get("picked").and_then(Value::as_u64).ok_or_else(|| {
                        DecisionError::Protocol(format!("choice '{id}' missing 'picked'"))
                    })? as usize;
                    if picked >= options.len() {
                        return Err(DecisionError::Protocol(format!(
                            "choice '{id}': picked {picked} out of range"
                        )));
                    }
                    Answer::Choice {
                        id: id.clone(),
                        picked,
                        distribution,
                        confidence,
                    }
                }
                Question::Score { id, rubric, .. } => {
                    let distribution = normalize_distribution(&distribution, rubric.len(), id)?;
                    let level = wire.get("level").and_then(Value::as_u64).ok_or_else(|| {
                        DecisionError::Protocol(format!("score '{id}' missing 'level'"))
                    })? as usize;
                    if level >= rubric.len() {
                        return Err(DecisionError::Protocol(format!(
                            "score '{id}': level {level} out of range"
                        )));
                    }
                    Answer::Score {
                        id: id.clone(),
                        level,
                        distribution,
                        confidence,
                    }
                }
                Question::Noul { id, .. } => {
                    let distribution = normalize_distribution(&distribution, 2, id)?;
                    let probability = distribution[1];
                    Answer::Noul {
                        id: id.clone(),
                        probability,
                    }
                }
            };
            answers.push(answer);
        }

        let usage: serde_json::Result<crate::provider::QuestionUsage> =
            serde_json::from_value(usage);
        Ok(DecisionResponse {
            answers,
            usage: usage.map_err(|e| DecisionError::Protocol(e.to_string()))?,
        })
    }
}

#[async_trait::async_trait]
impl DecisionProvider for SystemOneProvider {
    fn kind(&self) -> &'static str {
        "system_one"
    }

    async fn answer(&self, req: DecisionRequest) -> Result<DecisionResponse, DecisionError> {
        self.validate_model(&req.model)?;
        if req.questions.is_empty() {
            return Ok(DecisionResponse {
                answers: Vec::new(),
                usage: Default::default(),
            });
        }

        let url = format!("{}/systemone", self.base_url);
        let body = Self::request_body(&req);
        let policy = decision_policy();
        let max_attempts = policy.budget.limit().unwrap_or(3);
        for attempt in 1..=max_attempts {
            let response = self
                .http
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| DecisionError::Transport(e.to_string()))?;
            let status = response.status();
            // 429 (rate limit) and 529 (vendor "site overloaded") are
            // the retryable pair; exponential spacing with jitter,
            // Retry-After honored when present.
            if status.as_u16() == 429 || status.as_u16() == 529 {
                let retry_after_secs = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok());
                if attempt < max_attempts {
                    let backoff = retry_after_secs
                        .map(std::time::Duration::from_secs)
                        .unwrap_or_else(|| policy.delay(attempt, retry::sample()));
                    tokio::time::sleep(backoff).await;
                    continue;
                }
                return Err(DecisionError::RateLimited { retry_after_secs });
            }
            if !status.is_success() {
                return Err(DecisionError::Upstream {
                    status: status.as_u16(),
                });
            }
            let parsed: Value = response
                .json()
                .await
                .map_err(|e| DecisionError::Protocol(format!("response body is not JSON: {e}")))?;
            return Self::parse_response(&req, &parsed);
        }
        unreachable!("retry loop returns inside every arm")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice_question() -> Question {
        Question::Choice {
            id: "board".into(),
            instructions: "Route the ticket".into(),
            options: vec!["helpdesk".into(), "projects".into()],
        }
    }

    fn ok_body() -> Value {
        json!({
            "answers": [{ "id": "board", "picked": 1,
                          "distribution": [0.25, 0.75], "confidence": 0.9 }],
            "usage": { "input_tokens": 321 }
        })
    }

    async fn spawn_stub(routes: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, routes).await.unwrap() });
        format!("http://{addr}/v1")
    }

    fn request(model: &str) -> DecisionRequest {
        DecisionRequest {
            model: model.into(),
            state: json!({ "subject": "printer on fire" }),
            questions: vec![choice_question()],
        }
    }

    #[tokio::test]
    async fn rejects_non_jev_models_before_any_http() {
        let provider = SystemOneProvider::new(reqwest::Client::new(), "http://127.0.0.1:1/v1", "k");
        let err = provider.answer(request("gpt-4o")).await.unwrap_err();
        assert!(matches!(err, DecisionError::InvalidModel { .. }));
        let err = provider.answer(request("jev-latest")).await.unwrap_err();
        assert!(matches!(err, DecisionError::InvalidModel { .. }));
    }

    #[tokio::test]
    async fn parses_choice_answers_with_normalized_distribution() {
        let app = axum::Router::new().route(
            "/v1/systemone",
            axum::routing::post(|| async { axum::Json(ok_body()) }),
        );
        let base = spawn_stub(app).await;
        let provider = SystemOneProvider::new(reqwest::Client::new(), &base, "k");
        let out = provider.answer(request("jev-1.13.0")).await.unwrap();
        assert_eq!(out.usage.prompt_tokens, 321);
        assert_eq!(out.answers.len(), 1);
        match &out.answers[0] {
            Answer::Choice {
                picked,
                distribution,
                confidence,
                ..
            } => {
                assert_eq!(*picked, 1);
                assert_eq!(distribution, &vec![0.25, 0.75]);
                assert!((confidence - 0.9).abs() < f64::EPSILON);
            }
            other => panic!("expected choice answer, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn retries_529_with_backoff_then_succeeds() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        let hits = Arc::new(AtomicU32::new(0));
        let hits2 = hits.clone();
        let app = axum::Router::new().route(
            "/v1/systemone",
            axum::routing::post(move || {
                let hits = hits2.clone();
                async move {
                    if hits.fetch_add(1, Ordering::SeqCst) == 0 {
                        axum::response::Response::builder()
                            .status(529)
                            .body(axum::body::Body::empty())
                            .unwrap()
                    } else {
                        axum::response::Response::builder()
                            .status(200)
                            .header("content-type", "application/json")
                            .body(axum::body::Body::from(ok_body().to_string()))
                            .unwrap()
                    }
                }
            }),
        );
        let base = spawn_stub(app).await;
        let provider = SystemOneProvider::new(reqwest::Client::new(), &base, "k");
        let out = provider.answer(request("jev-1.13.0")).await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert_eq!(out.answers.len(), 1);
    }

    #[tokio::test]
    async fn rate_limited_after_exhausting_retries() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        let hits = Arc::new(AtomicU32::new(0));
        let hits2 = hits.clone();
        let app = axum::Router::new().route(
            "/v1/systemone",
            axum::routing::post(move || {
                let hits = hits2.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    axum::response::Response::builder()
                        .status(429)
                        .header("retry-after", "1")
                        .body(axum::body::Body::empty())
                        .unwrap()
                }
            }),
        );
        let base = spawn_stub(app).await;
        let provider = SystemOneProvider::new(reqwest::Client::new(), &base, "k");
        let err = provider.answer(request("jev-1.13.0")).await.unwrap_err();
        match err {
            DecisionError::RateLimited {
                retry_after_secs: Some(secs),
            } => assert_eq!(secs, 1),
            other => panic!("expected RateLimited, got {other:?}"),
        }
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn protocol_errors_are_typed() {
        let app = axum::Router::new().route(
            "/v1/systemone",
            axum::routing::post(|| async { axum::Json(json!({ "answers": [] })) }),
        );
        let base = spawn_stub(app).await;
        let provider = SystemOneProvider::new(reqwest::Client::new(), &base, "k");
        let err = provider.answer(request("jev-1.13.0")).await.unwrap_err();
        assert!(matches!(err, DecisionError::Protocol(_)));
    }
}
