//! The `CompletionProvider` trait (§4.7 traits at the edge) and BYOK seam.
//!
//! Purpose: define the trait boundary that LLM / completion integrations sit
//! behind (e.g. vector search summaries, ticket categorization, draft replies,
//! or embedding helpers). Every external AI dependency sits behind this
//! trait; no vendor SDK leaks past it.
//!
//! Non-purpose: no HTTP transport or vendor API clients here — pure domain
//! definitions and the built-in [`NoneCompletionProvider`] (the documented
//! none mode, which returns a typed unavailable error without network calls).

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Role of a message in a completion conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

impl MessageRole {
    pub const fn as_str(&self) -> &'static str {
        match self {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        }
    }
}

/// One chat message in a completion request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CompletionMessage {
    pub role: MessageRole,
    pub content: String,
}

/// A request sent to a [`CompletionProvider`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CompletionRequest {
    /// Ordered conversation messages.
    pub messages: Vec<CompletionMessage>,
    /// Optional model identifier requested by the caller (or provider default).
    pub model: Option<String>,
    /// Temperature for generation (0.0 = deterministic, 1.0 = creative).
    pub temperature: Option<f32>,
    /// Maximum completion tokens to generate.
    pub max_tokens: Option<u32>,
}

/// Token usage statistics returned by a completion provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CompletionUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// A response produced by a [`CompletionProvider`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CompletionResponse {
    /// The generated completion text.
    pub content: String,
    /// The concrete model that generated the completion.
    pub model: String,
    /// Token counts, if provided by the provider.
    pub usage: Option<CompletionUsage>,
}

/// Typed error returned by a [`CompletionProvider`].
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompletionError {
    /// Provider is disabled or unconfigured (none mode).
    #[error("completion provider unavailable: {reason}")]
    Unavailable { reason: String },
    /// Provider rate limit or quota exceeded.
    #[error("completion provider rate limited: {reason}")]
    RateLimited { reason: String },
    /// Input prompt or configuration is invalid.
    #[error("invalid completion request: {reason}")]
    InvalidRequest { reason: String },
    /// Provider backend error or upstream failure.
    #[error("completion provider backend error: {reason}")]
    Backend { reason: String },
}

/// The completion provider trait boundary (§4.7): every LLM/completion
/// dependency sits behind this trait; no vendor SDK leaks past it.
///
/// Implementations must be `Send + Sync`. The async shape mirrors
/// [`crate::notifications::MailTransport`] (RPITIT, dyn-free) — the
/// vector-search lane provides real HTTP adapters; the shipped default is
/// [`NoneCompletionProvider`] (standalone-first doctrine).
pub trait CompletionProvider: Send + Sync {
    /// Human/machine name of the provider (e.g. `"none"`, `"openai"`,
    /// `"anthropic"`, `"ollama"`).
    fn name(&self) -> &str;

    /// Generates one completion or fails with a typed error. Implementations
    /// must never log prompt or key material and must never block the
    /// caller's event loop.
    fn complete(
        &self,
        req: &CompletionRequest,
    ) -> impl std::future::Future<Output = Result<CompletionResponse, CompletionError>> + Send;
}

/// Built-in none-mode provider: always returns a typed [`CompletionError::Unavailable`].
/// Zero network calls, zero external dependencies.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoneCompletionProvider;

impl CompletionProvider for NoneCompletionProvider {
    fn name(&self) -> &str {
        "none"
    }

    async fn complete(
        &self,
        _req: &CompletionRequest,
    ) -> Result<CompletionResponse, CompletionError> {
        Err(CompletionError::Unavailable {
            reason: "completion provider is unconfigured (none mode default)".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn none_mode_returns_typed_unavailable() {
        let provider = NoneCompletionProvider;
        assert_eq!(provider.name(), "none");
        let req = CompletionRequest {
            messages: vec![CompletionMessage {
                role: MessageRole::User,
                content: "Hello".to_string(),
            }],
            model: None,
            temperature: None,
            max_tokens: None,
        };
        let err = provider.complete(&req).await.unwrap_err();
        match err {
            CompletionError::Unavailable { reason } => {
                assert!(reason.contains("none mode"));
            }
            other => panic!("expected Unavailable error, got {:?}", other),
        }
    }

    #[test]
    fn error_display_formatting() {
        let err = CompletionError::RateLimited {
            reason: "quota exceeded".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "completion provider rate limited: quota exceeded"
        );
    }

    #[test]
    fn error_wire_round_trip() {
        let err = CompletionError::Unavailable {
            reason: "unconfigured".to_string(),
        };
        let wire = serde_json::to_value(&err).expect("serialize");
        assert_eq!(wire["type"], "unavailable");
        let back: CompletionError = serde_json::from_value(wire).expect("deserialize");
        assert_eq!(back, err);
    }
}
