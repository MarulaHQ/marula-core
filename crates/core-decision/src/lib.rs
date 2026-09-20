//! The provider-general decision edge (ADR 0010 §1–§2, ADR 0011 §1–§4).
//!
//! One seam, zero vendor types at call sites: JSON-able state in, a typed
//! question map in, typed answers (choice / score / noul with full
//! probability distributions + confidence) out. Kind `system_one` (the
//! TypeSafe adapter, `POST /v1/systemone`, pinned `jev-*` models,
//! 429/529 exponential backoff) ships first as the seam's test case —
//! future decision models plug in as new kinds in [`registry`].
//!
//! Config-time class restrictions (ADR 0011 §4) live in [`config`]:
//! they are enforced BEFORE anything reaches a provider — templates
//! compose derived temporal fields in code, criteria stay admin-owned
//! static text, Choice >255 must be a tiered tree, and the composed
//! state is budget-checked (~32k tokens) with a typed refusal — never
//! silent truncation.
//!
//! Non-purpose: no admission policy (surface/grant/caps — the core-api
//! AI pipeline), no state composition from entity snapshots (consumer
//! lanes), no persistence (core-db), no UI.

pub mod config;
pub mod provider;
pub mod registry;
pub mod starters;
pub mod system_one;

pub use config::{
    validate_config, ChoiceOptions, DerivedField, QuestionConfig, QuestionKind, StateTemplate,
    SurfaceConfig, TemplateField, Thresholds, CONFIG_REFUSAL_REASONS, STATE_BUDGET_TOKENS,
};
pub use provider::{
    Answer, DecisionError, DecisionProvider, DecisionRequest, DecisionResponse, Question,
    QuestionUsage,
};
pub use registry::{descriptor, make_provider, ProviderKindDescriptor, REGISTRY};
pub use system_one::SystemOneProvider;
