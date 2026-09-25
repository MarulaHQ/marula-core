//! The built-in NONE-mode provider: scheduling works with no routing
//! server, honestly labeled.
//!
//! The estimate is great-circle distance × a road factor, plus a fixed
//! overhead, at a per-mile pace from the typed table (org settings in the
//! core; the crate takes the folded config). Every response carries
//! [`RoutingQuality::Heuristic`], and pairs beyond the configured radius
//! are returned as unreachable rather than as a fabricated number.

use crate::error::RoutingError;
use crate::provider::{cell_from_values, ProviderInfo, RoutingProvider};
use crate::types::{
    GeoPoint, MatrixCell, MatrixRequest, MatrixResponse, RoutingCapability, RoutingQuality,
    TimeControl, TravelMode,
};
use async_trait::async_trait;

/// The wire id of the NONE-mode provider.
pub const HEURISTIC_PROVIDER_ID: &str = "none";

const METERS_PER_MILE: f64 = 1_609.344;

/// The folded per-mile table. Integer org settings map onto this exactly
/// (`minutes_per_mile` minutes per mile, `road_factor_percent`/100 road
/// factor, `overhead_minutes` fixed). Distances beyond
/// `max_radius_miles` are unreachable — the heuristic refuses to guess
/// across a state line.
#[derive(Debug, Clone, PartialEq)]
pub struct HeuristicConfig {
    pub minutes_per_mile: f64,
    pub overhead_minutes: f64,
    pub road_factor: f64,
    pub max_radius_miles: f64,
}

impl Default for HeuristicConfig {
    fn default() -> Self {
        Self {
            minutes_per_mile: 2.0,
            overhead_minutes: 5.0,
            road_factor: 1.25,
            max_radius_miles: 150.0,
        }
    }
}

impl HeuristicConfig {
    /// Folds the typed settings' integer units onto the float table.
    /// Callers pass already-validated values (settings validation is the
    /// gate); this constructor is defensive anyway.
    #[must_use]
    pub fn from_settings(
        minutes_per_mile: i64,
        overhead_minutes: i64,
        road_factor_percent: i64,
    ) -> Self {
        Self {
            minutes_per_mile: (minutes_per_mile.max(1)) as f64,
            overhead_minutes: overhead_minutes.max(0) as f64,
            road_factor: (road_factor_percent.clamp(100, 300) as f64) / 100.0,
            max_radius_miles: 150.0,
        }
    }

    pub fn validate(&self) -> Result<(), RoutingError> {
        if !(self.minutes_per_mile.is_finite() && self.minutes_per_mile > 0.0) {
            return Err(RoutingError::Invalid(
                "heuristic minutes_per_mile must be positive".into(),
            ));
        }
        if !(self.overhead_minutes.is_finite() && self.overhead_minutes >= 0.0) {
            return Err(RoutingError::Invalid(
                "heuristic overhead_minutes must be >= 0".into(),
            ));
        }
        if !(self.road_factor.is_finite() && self.road_factor >= 1.0) {
            return Err(RoutingError::Invalid(
                "heuristic road_factor must be >= 1.0 (a straight line is a lower bound)".into(),
            ));
        }
        if !(self.max_radius_miles.is_finite() && self.max_radius_miles > 0.0) {
            return Err(RoutingError::Invalid(
                "heuristic max_radius_miles must be positive".into(),
            ));
        }
        Ok(())
    }

    /// Minutes for one great-circle pair, or `None` past the radius.
    #[must_use]
    pub fn minutes_for_points(&self, from: &GeoPoint, to: &GeoPoint) -> Option<(i64, i64)> {
        let road_meters = from.haversine_meters(to) * self.road_factor;
        let miles = road_meters / METERS_PER_MILE;
        if miles > self.max_radius_miles {
            return None;
        }
        let minutes = self.overhead_minutes + miles * self.minutes_per_mile;
        Some(((minutes * 60.0).round() as i64, road_meters.round() as i64))
    }
}

/// The NONE-mode provider instance. Constructible per request from org
/// settings — it holds no I/O and no state.
#[derive(Debug, Clone)]
pub struct HeuristicRoutingProvider {
    config: HeuristicConfig,
    info: ProviderInfo,
}

impl HeuristicRoutingProvider {
    pub fn new(config: HeuristicConfig) -> Result<Self, RoutingError> {
        config.validate()?;
        Ok(Self {
            config,
            info: ProviderInfo {
                id: HEURISTIC_PROVIDER_ID.into(),
                label: "Built-in travel estimate".into(),
                quality: RoutingQuality::Heuristic,
                capabilities: vec![RoutingCapability::Matrix],
                modes: vec![TravelMode::Driving],
                batched_matrix: true,
                time_control: TimeControl::None,
            },
        })
    }

    #[must_use]
    pub fn config(&self) -> &HeuristicConfig {
        &self.config
    }
}

impl Default for HeuristicRoutingProvider {
    fn default() -> Self {
        Self::new(HeuristicConfig::default()).expect("default heuristic config is valid")
    }
}

#[async_trait]
impl RoutingProvider for HeuristicRoutingProvider {
    fn info(&self) -> &ProviderInfo {
        &self.info
    }

    async fn matrix(&self, request: &MatrixRequest) -> Result<MatrixResponse, RoutingError> {
        request.validate()?;
        if !self.info.supports_mode(request.mode) {
            return Err(RoutingError::Unsupported {
                provider: self.info.id.clone(),
                capability: "mode",
            });
        }
        let mut cells = Vec::with_capacity(request.sources.len() * request.targets.len());
        for (i, source) in request.sources.iter().enumerate() {
            for (j, target) in request.targets.iter().enumerate() {
                let (duration_seconds, distance_meters) = match (source.point, target.point) {
                    (Some(from), Some(to)) if from.is_plausible() && to.is_plausible() => {
                        match self.config.minutes_for_points(&from, &to) {
                            Some((seconds, meters)) => (Some(seconds as f64), Some(meters as f64)),
                            None => (None, None),
                        }
                    }
                    // The heuristic cannot geocode; an address-only place
                    // is an honest hole.
                    _ => (None, None),
                };
                cells.push(cell_from_values(
                    i,
                    j,
                    duration_seconds,
                    distance_meters,
                ));
            }
        }
        Ok(MatrixResponse {
            provider: self.info.id.clone(),
            quality: RoutingQuality::Heuristic,
            cells,
        })
    }
}

/// Convenience alias so callers can build "the none-mode matrix" for one
/// test or one off-line path without importing cell types.
#[must_use]
pub fn heuristic_cell(
    config: &HeuristicConfig,
    from: &GeoPoint,
    to: &GeoPoint,
) -> MatrixCell {
    let (duration_seconds, distance_meters) = config
        .minutes_for_points(from, to)
        .map(|(s, m)| (Some(s as f64), Some(m as f64)))
        .unwrap_or((None, None));
    cell_from_values(0, 0, duration_seconds, distance_meters)
}
