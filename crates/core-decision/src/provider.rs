//! Typed questions in, typed answers out (ADR 0010 §1).
//!
//! The trait speaks the *shape* of the decision-model class — never a
//! vendor type. One dispatch = one speculative fan-out request: all
//! questions for one trigger ride a single provider call (ADR 0011 §4;
//! per-question HTTP calls are an anti-pattern and this API makes them
//! unrepresentable).

use serde::{Deserialize, Serialize};

/// One typed question. Free-form text does not exist in this space
/// (ADR 0011 §4: the class generates no text and returns no rationales).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Question {
    /// Pick one of ≤255 options; the provider returns the full
    /// normalized probability distribution over the options.
    Choice {
        id: String,
        instructions: String,
        options: Vec<String>,
    },
    /// Score against an org-defined ordered rubric (level 0..len-1).
    Score {
        id: String,
        instructions: String,
        rubric: Vec<String>,
    },
    /// Yes/no probability. No separate confidence exists — the
    /// probability IS the signal (ADR 0010 §5).
    Noul { id: String, instructions: String },
}

impl Question {
    pub fn id(&self) -> &str {
        match self {
            Question::Choice { id, .. }
            | Question::Score { id, .. }
            | Question::Noul { id, .. } => id,
        }
    }

    /// The expected distribution width for this question.
    pub fn distribution_width(&self) -> usize {
        match self {
            Question::Choice { options, .. } => options.len(),
            Question::Score { rubric, .. } => rubric.len(),
            Question::Noul { .. } => 2,
        }
    }
}

/// One typed answer, carrying the FULL normalized probability
/// distribution (recorded for audit + calibration — ADR 0011 §4),
/// provider-defined confidence, and the pinned model id that answered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Answer {
    Choice {
        id: String,
        /// Index into the question's options.
        picked: usize,
        distribution: Vec<f64>,
        confidence: f64,
    },
    Score {
        id: String,
        /// Index into the question's rubric levels.
        level: usize,
        distribution: Vec<f64>,
        confidence: f64,
    },
    Noul {
        id: String,
        /// P(yes) in [0, 1]. Thresholds are per-org config; ≈0.5 routes
        /// to review, never acts (ADR 0010 §5).
        probability: f64,
    },
}

impl Answer {
    pub fn id(&self) -> &str {
        match self {
            Answer::Choice { id, .. } | Answer::Score { id, .. } | Answer::Noul { id, .. } => id,
        }
    }
}

/// Input tokens the provider consumed (the class bills input only —
/// output is free per the research record).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
}

/// The whole fan-out's usage + provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionResponse {
    pub answers: Vec<Answer>,
    pub usage: QuestionUsage,
}

/// One dispatch ask. `state` is JSON-able composed content (text-only
/// for the class — no images) and `questions` is the whole typed map
/// for the trigger.
#[derive(Debug, Clone)]
pub struct DecisionRequest {
    /// Pinned model id (e.g. `jev-1.13.0`, never only `jev-latest` —
    /// ADR 0010 §1; answer provenance records exactly this id).
    pub model: String,
    pub state: serde_json::Value,
    pub questions: Vec<Question>,
}

/// Typed provider failures. Rate limits and upstream outages are
/// retryable at the transport layer (429/529 exponential backoff in the
/// adapter); everything surface-facing degrades typed, never 5xx into a
/// human workflow (ADR 0008 posture, inherited by 0010 §6).
#[derive(Debug, thiserror::Error)]
pub enum DecisionError {
    #[error("decision provider rate limited (429/529 after retries)")]
    RateLimited { retry_after_secs: Option<u64> },
    #[error("decision provider returned status {status}")]
    Upstream { status: u16 },
    #[error("decision provider transport failure: {0}")]
    Transport(String),
    #[error("model id '{model}' is not valid for this provider kind: {reason}")]
    InvalidModel { model: String, reason: String },
    #[error("provider response failed validation: {0}")]
    Protocol(String),
}

/// The seam every decision provider speaks. Object-safe so the
/// provider-kind registry can hand out boxed providers without call
/// sites seeing a vendor type.
#[async_trait::async_trait]
pub trait DecisionProvider: Send + Sync {
    fn kind(&self) -> &'static str;

    /// Dispatch the WHOLE question map in one speculative fan-out.
    async fn answer(&self, req: DecisionRequest) -> Result<DecisionResponse, DecisionError>;
}

/// Normalize a returned distribution: clamp negatives, renormalize to a
/// sum of ~1, and refuse wrong widths. Providers occasionally return
/// near-sums (0.999…); the seam normalizes honestly instead of
/// recording vectors that don't sum.
pub(crate) fn normalize_distribution(
    raw: &[f64],
    width: usize,
    question_id: &str,
) -> Result<Vec<f64>, DecisionError> {
    if raw.len() != width {
        return Err(DecisionError::Protocol(format!(
            "question '{question_id}': distribution width {} != expected {width}",
            raw.len()
        )));
    }
    if raw.iter().any(|p| !p.is_finite() || *p < -1e-9) {
        return Err(DecisionError::Protocol(format!(
            "question '{question_id}': distribution carries a negative or non-finite entry"
        )));
    }
    let sum: f64 = raw.iter().sum();
    if !sum.is_finite() || sum <= 0.0 {
        return Err(DecisionError::Protocol(format!(
            "question '{question_id}': distribution sums to {sum}"
        )));
    }
    Ok(raw.iter().map(|p| (p / sum).clamp(0.0, 1.0)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_rescales_near_sums() {
        let out = normalize_distribution(&[0.4999, 0.4999], 2, "q").unwrap();
        assert!((out[0] - 0.5).abs() < 1e-9);
        assert!((out[0] + out[1] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn normalize_refuses_wrong_width_and_negatives() {
        assert!(normalize_distribution(&[0.5, 0.5], 3, "q").is_err());
        assert!(normalize_distribution(&[-0.5, 1.5], 2, "q").is_err());
        assert!(normalize_distribution(&[f64::NAN, 1.0], 2, "q").is_err());
    }
}
