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

/// Fixed width of the unified-search embedding vectors (migration 0062
/// stores `vector(768)`): [`BUNDLED_EMBEDDING_MODEL`]'s native width, and the
/// width a cloud embedder is asked for (OpenAI text-embedding-3 truncates to
/// it). A provider that cannot produce this width leaves the vector leg dark —
/// search degrades to FTS, never stores a wrong-width vector.
pub const SEARCH_EMBEDDING_DIM: usize = 768;

/// Provider the bundled embedding model runs behind: the local `ollama`
/// runtime the `ai` compose profile starts. It is local on purpose. A host
/// that is too small for the model, or a re-index that has to beat the
/// overnight window, names a cloud provider instead — the override, not the
/// default.
pub const BUNDLED_EMBEDDING_PROVIDER: &str = "ollama";

/// The embedding model an install gets when nobody configured one:
/// EmbeddingGemma 2, text-only variant, 271M parameters, Apache 2.0. It is
/// 768 dimensions native and Matryoshka to 512 / 256 / 128, so it fills
/// [`SEARCH_EMBEDDING_DIM`] with no schema change and no truncation.
///
/// The multimodal build of the same model (`embeddinggemma-2:740m-q8_0`) is
/// the text tower plus a 473M projector for images and audio. It costs the
/// same text throughput and a larger resident set, so it is the override for
/// an install that needs media chunks, not the default. An instance names a
/// different model with `MARULA_EMBEDDING_MODEL`; the catalogue lane owns the
/// selection surface that replaces that variable.
pub const BUNDLED_EMBEDDING_MODEL: &str = "embeddinggemma-2:270m-q8_0-text";

/// One org's embedding configuration after its settings are read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingConfig {
    /// Provider id: `ollama` for the bundled local path, `openai` for the
    /// cloud override, `none` for the documented none mode, which makes no
    /// outgoing embedding call at all.
    pub provider: String,
    /// The model this org named, if any. `None` means the provider's own
    /// default applies, which is [`BUNDLED_EMBEDDING_MODEL`] locally.
    pub model: Option<String>,
    /// True when no provider was configured and this is the bundled default.
    pub provider_is_default: bool,
}

/// Resolve an org's embedding configuration from its raw settings values.
///
/// Unset is not the same as off. An install that never touched
/// `ai.completion_provider` gets the bundled local embedder. An org that set
/// it to `none` asked for the vector leg to stay dark and keeps it, which is
/// the documented none mode rather than a degraded default.
pub fn resolve_embedding_config(provider: Option<&str>, model: Option<&str>) -> EmbeddingConfig {
    let provider = provider.unwrap_or("").trim();
    let model = model
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("none"))
        .map(str::to_string);
    if provider.is_empty() {
        return EmbeddingConfig {
            provider: BUNDLED_EMBEDDING_PROVIDER.to_string(),
            model,
            provider_is_default: true,
        };
    }
    EmbeddingConfig {
        provider: provider.to_ascii_lowercase(),
        model,
        provider_is_default: false,
    }
}

/// A request sent to a [`CompletionProvider::embed`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingRequest {
    /// Texts to embed; one vector is returned per input, in order.
    pub inputs: Vec<String>,
    /// Optional model identifier requested by the caller (or the provider
    /// default configured for the org).
    pub model: Option<String>,
}

/// A response produced by [`CompletionProvider::embed`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingResponse {
    /// One vector per input, in input order. Implementations must return
    /// exactly [`SEARCH_EMBEDDING_DIM`] floats per input.
    pub embeddings: Vec<Vec<f32>>,
    /// The concrete model that produced the vectors.
    pub model: String,
    /// Token counts, if provided by the provider.
    pub usage: Option<CompletionUsage>,
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

    /// Embeds a batch of texts — one [`SEARCH_EMBEDDING_DIM`]-wide vector
    /// per input, in order — for the hybrid search vector leg (§4.5). The
    /// default is the none mode: providers that cannot embed leave the
    /// vector leg dark and search degrades to the lexical legs; callers
    /// treat every `Err` the same way and never fail a search because AI is
    /// unconfigured or down. Implementations must never log input text or
    /// key material.
    fn embed(
        &self,
        _req: &EmbeddingRequest,
    ) -> impl std::future::Future<Output = Result<EmbeddingResponse, CompletionError>> + Send {
        async {
            Err(CompletionError::Unavailable {
                reason: "embedding capability is unconfigured (none mode default)".to_string(),
            })
        }
    }
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
            max_tokens: Some(10),
        };
        let err = provider.complete(&req).await.unwrap_err();
        match err {
            CompletionError::Unavailable { reason } => {
                assert!(reason.contains("none mode"));
            }
            other => panic!("expected Unavailable error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn embed_defaults_to_typed_unavailable_without_any_impl() {
        // Any provider — even one written before the vector leg existed —
        // gets a usable none-mode embed for free: the vector leg degrades,
        // search never breaks, and no wrong-width vector can ever be stored.
        let provider = NoneCompletionProvider;
        let req = EmbeddingRequest {
            inputs: vec!["Mail relay down".to_string()],
            model: None,
        };
        match provider.embed(&req).await {
            Err(CompletionError::Unavailable { reason }) => {
                assert!(reason.contains("embedding"));
            }
            other => panic!("expected default Unavailable, got {:?}", other),
        }
    }

    #[test]
    fn search_embedding_dim_is_stable() {
        assert_eq!(SEARCH_EMBEDDING_DIM, 768);
        assert_eq!(crate::completion::SEARCH_EMBEDDING_DIM, 768);
    }

    #[test]
    fn unset_settings_resolve_to_the_bundled_local_embedder() {
        let config = resolve_embedding_config(None, None);
        assert_eq!(config.provider, BUNDLED_EMBEDDING_PROVIDER);
        assert_eq!(config.provider, "ollama");
        assert!(config.provider_is_default);
        assert_eq!(
            config.model, None,
            "no model named means the provider default"
        );

        let blank = resolve_embedding_config(Some("   "), Some("   "));
        assert_eq!(blank, config, "whitespace is not a configuration");
    }

    #[test]
    fn explicit_none_keeps_the_vector_leg_dark() {
        let config = resolve_embedding_config(Some("none"), None);
        assert_eq!(config.provider, "none");
        assert!(!config.provider_is_default);
    }

    #[test]
    fn a_named_provider_and_model_survive_resolution() {
        let config = resolve_embedding_config(Some("OpenAI"), Some(" text-embedding-3-small "));
        assert_eq!(config.provider, "openai");
        assert_eq!(config.model.as_deref(), Some("text-embedding-3-small"));
        assert!(!config.provider_is_default);
    }

    #[test]
    fn a_none_model_falls_back_to_the_provider_default() {
        let config = resolve_embedding_config(Some("ollama"), Some("none"));
        assert_eq!(config.provider, "ollama");
        assert_eq!(config.model, None);
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
