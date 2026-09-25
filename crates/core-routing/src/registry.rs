//! The provider registry: registration, active selection, and the typed
//! status surface every scheduling caller reads before it routes.

use crate::error::RoutingError;
use crate::heuristic::HEURISTIC_PROVIDER_ID;
use crate::provider::{ProviderInfo, RoutingProvider};
use crate::types::{RoutingCapability, RoutingQuality, TravelMode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

fn read_lock<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

fn write_lock<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}

/// The active routing posture, in the seam's own vocabulary.
#[derive(Debug, Clone, PartialEq)]
pub enum RoutingStatus {
    /// No provider configured: the built-in heuristic answers every
    /// estimate, and suggestion-grade features stay silent (ADR 0010
    /// visibly-absent posture) because the quality is not measured.
    NoneMode,
    /// Config named a provider the registry does not hold. Estimates
    /// still run on the heuristic; the state is typed and visible.
    Missing { configured: String },
    /// A registered provider is active.
    Configured(ProviderInfo),
}

impl RoutingStatus {
    /// The quality every estimate will carry under this status.
    #[must_use]
    pub fn quality(&self) -> RoutingQuality {
        match self {
            RoutingStatus::Configured(info) => info.quality,
            _ => RoutingQuality::Heuristic,
        }
    }

    /// Suggestion-grade features require measured numbers.
    #[must_use]
    pub fn measured(&self) -> bool {
        self.quality() == RoutingQuality::Measured
    }

    /// Machine label for the degraded states (`None` when healthy).
    #[must_use]
    pub fn degraded_reason(&self) -> Option<&'static str> {
        match self {
            RoutingStatus::NoneMode => Some("no_provider_configured"),
            RoutingStatus::Missing { .. } => Some("provider_not_registered"),
            RoutingStatus::Configured(_) => None,
        }
    }
}

/// Serialized status view for `GET /api/dispatch/routing` and settings
/// surfaces. Stable keys; the UI labels heuristics honestly from `quality`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutingStatusView {
    /// True when a registered, measured provider is active.
    pub measured: bool,
    /// Active provider id — `none` in NONE-mode.
    pub provider_id: String,
    pub provider_label: String,
    pub quality: RoutingQuality,
    pub capabilities: Vec<String>,
    pub modes: Vec<String>,
    /// Suggestion-grade features are offered only for measured providers.
    pub suggestions_available: bool,
    /// Typed label when the posture is degraded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The registry itself. Cheap to clone behind an `Arc`; all access is
/// interior-mutable so a future hot-reload of provider config needs no
/// re-plumbing.
#[derive(Default)]
pub struct RoutingRegistry {
    providers: RwLock<BTreeMap<String, Arc<dyn RoutingProvider>>>,
    configured: RwLock<Option<String>>,
}

impl RoutingRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers (or replaces) a provider by its [`ProviderInfo::id`].
    pub fn register(&self, provider: Arc<dyn RoutingProvider>) {
        let id = provider.info().id.clone();
        write_lock(&self.providers).insert(id, provider);
    }

    /// Names the operator-configured provider. `None` restores NONE-mode.
    pub fn set_configured(&self, id: Option<String>) {
        *write_lock(&self.configured) = id.filter(|s| !s.trim().is_empty());
    }

    #[must_use]
    pub fn configured_id(&self) -> Option<String> {
        read_lock(&self.configured).clone()
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<Arc<dyn RoutingProvider>> {
        read_lock(&self.providers).get(id).cloned()
    }

    /// The provider to route with: the configured one when registered,
    /// `None` otherwise. Callers fall back to the heuristic explicitly
    /// (it is always constructible from org settings, so it is not
    /// stored here).
    #[must_use]
    pub fn active(&self) -> Option<Arc<dyn RoutingProvider>> {
        let configured = self.configured_id()?;
        self.get(&configured)
    }

    /// Resolves the active provider or returns the typed refusal naming
    /// the configured id — the callers' one call site.
    pub fn require_active(&self) -> Result<Arc<dyn RoutingProvider>, RoutingError> {
        let configured = self.configured_id().ok_or(RoutingError::NoProvider)?;
        self.get(&configured)
            .ok_or(RoutingError::NotConfigured { provider: configured })
    }

    #[must_use]
    pub fn status(&self) -> RoutingStatus {
        match self.configured_id() {
            None => RoutingStatus::NoneMode,
            Some(id) => match self.get(&id) {
                Some(provider) => RoutingStatus::Configured(provider.info().clone()),
                None => RoutingStatus::Missing { configured: id },
            },
        }
    }

    /// Every registered provider (config names one; the rest stay
    /// available for a future per-org selection).
    #[must_use]
    pub fn providers(&self) -> Vec<ProviderInfo> {
        read_lock(&self.providers)
            .values()
            .map(|p| p.info().clone())
            .collect()
    }

    /// The serialized status view, with the NONE-mode row filled from the
    /// heuristic provider's constant descriptor. `heuristic_modes` is
    /// always `["driving"]`; the label is stable copy.
    #[must_use]
    pub fn status_view(&self) -> RoutingStatusView {
        match self.status() {
            RoutingStatus::Configured(info) => RoutingStatusView {
                measured: info.quality == RoutingQuality::Measured,
                provider_id: info.id,
                provider_label: info.label,
                quality: info.quality,
                capabilities: info.capabilities.iter().map(|c| c.as_str().into()).collect(),
                modes: info.modes.iter().map(|m| m.as_str().into()).collect(),
                suggestions_available: info.quality == RoutingQuality::Measured,
                reason: None,
            },
            RoutingStatus::NoneMode => RoutingStatusView {
                measured: false,
                provider_id: HEURISTIC_PROVIDER_ID.into(),
                provider_label: "Built-in travel estimate".into(),
                quality: RoutingQuality::Heuristic,
                capabilities: vec![RoutingCapability::Matrix.as_str().into()],
                modes: vec![TravelMode::Driving.as_str().into()],
                suggestions_available: false,
                reason: Some("no_provider_configured".into()),
            },
            RoutingStatus::Missing { configured } => RoutingStatusView {
                measured: false,
                provider_id: configured,
                provider_label: "Configured provider not registered".into(),
                quality: RoutingQuality::Heuristic,
                capabilities: vec![RoutingCapability::Matrix.as_str().into()],
                modes: vec![TravelMode::Driving.as_str().into()],
                suggestions_available: false,
                reason: Some("provider_not_registered".into()),
            },
        }
    }
}
