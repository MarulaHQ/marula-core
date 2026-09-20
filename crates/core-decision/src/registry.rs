//! The provider-kind descriptor registry (ADR 0010 §1: "a trait +
//! provider-kind descriptor, not a TypeSafe client"). A future decision
//! model is a new descriptor + factory arm — call sites never change.

use crate::provider::DecisionProvider;
use crate::system_one::SystemOneProvider;

/// One decision-provider kind. `pinned_models` carries the curated
/// default pins (admins may catalog other `jev-*` pins; answer
/// provenance always records the exact id that answered).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderKindDescriptor {
    pub kind: &'static str,
    pub label: &'static str,
    pub default_base_url: &'static str,
    pub pinned_models: &'static [&'static str],
}

pub const REGISTRY: &[ProviderKindDescriptor] = &[ProviderKindDescriptor {
    kind: "system_one",
    label: "TypeSafe System One",
    default_base_url: "https://api.typesafe.ai/v1",
    pinned_models: &["jev-1.13.0"],
}];

pub fn descriptor(kind: &str) -> Option<&'static ProviderKindDescriptor> {
    REGISTRY.iter().find(|d| d.kind == kind)
}

/// Build the boxed provider for a kind. `base_url`/`api_key` come from
/// org config + the sealed ModelCredential slot; unknown kinds return
/// None (typed unconfigured posture upstream — never a panic).
pub fn make_provider(
    kind: &str,
    base_url: &str,
    api_key: &str,
    http: reqwest::Client,
) -> Option<Box<dyn DecisionProvider>> {
    match kind {
        "system_one" => Some(Box::new(SystemOneProvider::new(http, base_url, api_key))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_one_is_the_first_kind() {
        let d = descriptor("system_one").expect("system_one registered");
        assert_eq!(d.pinned_models, &["jev-1.13.0"]);
        assert!(d.pinned_models.iter().all(|m| m.starts_with("jev-")));
        assert!(descriptor("frontier_llm").is_none());
    }

    #[test]
    fn unknown_kinds_build_nothing() {
        let http = reqwest::Client::new();
        assert!(make_provider("nope", "http://localhost", "k", http.clone()).is_none());
        assert!(make_provider("system_one", "http://localhost", "k", http).is_some());
    }
}
