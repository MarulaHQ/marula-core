//! The `RoutingProvider` trait and the shared leg-estimate helper.

use crate::error::RoutingError;
use crate::types::{
    estimate_minutes, GeocodeHit, GeocodeRequest, IsochroneRequest, IsochroneResponse, MatrixCell,
    MatrixRequest, MatrixResponse, Place, RouteRequest, RouteResponse, RoutingCapability,
    RoutingQuality, TimeControl, TravelEstimate, TravelMode,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};

/// Static descriptor every provider carries. Owned strings (not
/// `&'static str`) because out-of-tree contract packs register under a
/// configured id/label.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderInfo {
    /// Stable machine id (`none`, `osrm`, `mapbox`, `contract:traveltime`).
    pub id: String,
    /// Human label for settings/status surfaces.
    pub label: String,
    pub quality: RoutingQuality,
    pub capabilities: Vec<RoutingCapability>,
    pub modes: Vec<TravelMode>,
    /// One matrix request may carry many sources × targets.
    pub batched_matrix: bool,
    /// Which time-control semantics the provider honors.
    pub time_control: TimeControl,
}

impl ProviderInfo {
    #[must_use]
    pub fn supports(&self, capability: RoutingCapability) -> bool {
        self.capabilities.contains(&capability)
    }

    #[must_use]
    pub fn supports_mode(&self, mode: TravelMode) -> bool {
        self.modes.contains(&mode)
    }
}

/// The single seam between scheduling and any distance/duration API.
///
/// Implementations own their vendor mapping. `matrix` is the required
/// primitive — every scheduling need routes through it — while `route`,
/// `geocode`, and `isochrone` default to a typed
/// [`RoutingError::Unsupported`] so a provider only implements what it
/// truthfully advertises in [`ProviderInfo::capabilities`].
#[async_trait]
pub trait RoutingProvider: Send + Sync {
    fn info(&self) -> &ProviderInfo;

    /// Batched N×M durations/distances. Cells the provider cannot fill
    /// are omitted or carry `None` values — never fabricated zeros.
    async fn matrix(&self, request: &MatrixRequest) -> Result<MatrixResponse, RoutingError>;

    async fn route(&self, request: &RouteRequest) -> Result<RouteResponse, RoutingError> {
        let _ = request;
        Err(RoutingError::Unsupported {
            provider: self.info().id.clone(),
            capability: RoutingCapability::Route.as_str(),
        })
    }

    async fn geocode(&self, request: &GeocodeRequest) -> Result<Vec<GeocodeHit>, RoutingError> {
        let _ = request;
        Err(RoutingError::Unsupported {
            provider: self.info().id.clone(),
            capability: RoutingCapability::Geocode.as_str(),
        })
    }

    async fn isochrone(
        &self,
        request: &IsochroneRequest,
    ) -> Result<IsochroneResponse, RoutingError> {
        let _ = request;
        Err(RoutingError::Unsupported {
            provider: self.info().id.clone(),
            capability: RoutingCapability::Isochrone.as_str(),
        })
    }
}

/// Blanket forwarding so `Arc<dyn RoutingProvider>` / `&T` are usable
/// wherever a provider is expected (the range-filter-drafts pattern).
#[async_trait]
impl<T: RoutingProvider + ?Sized> RoutingProvider for std::sync::Arc<T> {
    fn info(&self) -> &ProviderInfo {
        (**self).info()
    }

    async fn matrix(&self, request: &MatrixRequest) -> Result<MatrixResponse, RoutingError> {
        (**self).matrix(request).await
    }

    async fn route(&self, request: &RouteRequest) -> Result<RouteResponse, RoutingError> {
        (**self).route(request).await
    }

    async fn geocode(&self, request: &GeocodeRequest) -> Result<Vec<GeocodeHit>, RoutingError> {
        (**self).geocode(request).await
    }

    async fn isochrone(
        &self,
        request: &IsochroneRequest,
    ) -> Result<IsochroneResponse, RoutingError> {
        (**self).isochrone(request).await
    }
}

/// Estimates one leg (from → to) through a provider. Prefers the batched
/// matrix primitive; falls back to `route` for providers that only speak
/// routes. The departure/arrival time (when given and when the provider
/// advertises it) is forwarded so traffic-aware providers see the real
/// context; a time-independent provider still answers, and the result
/// carries `time_controlled: false` instead of pretending.
pub async fn estimate_travel(
    provider: &dyn RoutingProvider,
    from: &Place,
    to: &Place,
    mode: TravelMode,
    depart_at: Option<DateTime<Utc>>,
) -> Result<TravelEstimate, RoutingError> {
    let info = provider.info();
    if !info.supports_mode(mode) {
        return Err(RoutingError::Unsupported {
            provider: info.id.clone(),
            capability: "mode",
        });
    }
    let forward_time = depart_at.is_some() && info.time_control.departure_aware();
    if info.supports(RoutingCapability::Matrix) {
        let mut request = MatrixRequest::new(vec![from.clone()], vec![to.clone()], mode);
        request.annotations = crate::types::MatrixAnnotations::both();
        if forward_time {
            if let Some(at) = depart_at {
                request = request.departing(at);
            }
        }
        let response = provider.matrix(&request).await?;
        let cell = response.cell(0, 0).ok_or_else(|| {
            RoutingError::Protocol(format!("provider `{}` returned no 0,0 cell", info.id))
        })?;
        let seconds = cell.duration_seconds.ok_or_else(|| {
            RoutingError::Unavailable(format!(
                "provider `{}` reported the leg unreachable",
                info.id
            ))
        })?;
        return Ok(TravelEstimate {
            minutes: estimate_minutes(seconds),
            distance_meters: cell.distance_meters,
            provider: response.provider,
            quality: response.quality,
            time_controlled: forward_time,
        });
    }
    if info.supports(RoutingCapability::Route) {
        let mut request = RouteRequest::new(from.clone(), to.clone(), mode);
        if forward_time {
            request.depart_at = depart_at;
        }
        let response = provider.route(&request).await?;
        let seconds = response.total_duration_seconds.ok_or_else(|| {
            RoutingError::Unavailable(format!("provider `{}` returned no route duration", info.id))
        })?;
        return Ok(TravelEstimate {
            minutes: estimate_minutes(seconds),
            distance_meters: response.total_distance_meters,
            provider: response.provider,
            quality: response.quality,
            time_controlled: forward_time,
        });
    }
    Err(RoutingError::Unsupported {
        provider: info.id.clone(),
        capability: RoutingCapability::Matrix.as_str(),
    })
}

/// Maps a vendor duration/distance pair onto a matrix cell, applying the
/// seam's canonical rounding (seconds whole, metres whole). Public so
/// first-party and relay provider crates normalize identically.
#[must_use]
pub fn cell_from_values(
    source_index: usize,
    target_index: usize,
    duration_seconds: Option<f64>,
    distance_meters: Option<f64>,
) -> MatrixCell {
    MatrixCell {
        source_index,
        target_index,
        duration_seconds: duration_seconds
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(|v| v.round() as i64),
        distance_meters: distance_meters
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(|v| v.round() as i64),
    }
}
