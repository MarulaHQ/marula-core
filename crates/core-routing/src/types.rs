//! Canonical routing vocabulary shared by every provider implementation.
//!
//! Units are fixed: durations in whole seconds, distances in metres,
//! coordinates in WGS-84 decimal degrees. Providers normalize their
//! vendor payloads onto these shapes and never leak vendor types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The hard ceiling on one batched matrix request (sources × targets).
/// Providers may impose a lower cap and return [`crate::RoutingError`]
/// instead of silently truncating — a truncated matrix is a wrong matrix.
pub const MAX_MATRIX_CELLS: usize = 2_500;

/// A WGS-84 coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeoPoint {
    pub latitude: f64,
    pub longitude: f64,
}

impl GeoPoint {
    #[must_use]
    pub fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
        }
    }

    #[must_use]
    pub fn is_plausible(&self) -> bool {
        self.latitude.is_finite()
            && self.longitude.is_finite()
            && (-90.0..=90.0).contains(&self.latitude)
            && (-180.0..=180.0).contains(&self.longitude)
    }

    /// Great-circle distance in metres (haversine, mean Earth radius).
    #[must_use]
    pub fn haversine_meters(&self, other: &Self) -> f64 {
        const EARTH_RADIUS_M: f64 = 6_371_008.8;
        let lat1 = self.latitude.to_radians();
        let lat2 = other.latitude.to_radians();
        let dlat = (other.latitude - self.latitude).to_radians();
        let dlon = (other.longitude - self.longitude).to_radians();
        let a = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
        2.0 * EARTH_RADIUS_M * a.sqrt().min(1.0).asin()
    }
}

/// Which time-control semantics a provider can honor. Scheduling asks
/// "how long does the trip take LEAVING at the appointment start", so
/// providers that cannot model departure time (a static self-host OSRM
/// graph) advertise [`TimeControl::None`]: the estimate is still real
/// network geometry, just not traffic-aware, and the result says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeControl {
    None,
    Departure,
    Arrival,
    Both,
}

impl TimeControl {
    #[must_use]
    pub const fn departure_aware(self) -> bool {
        matches!(self, TimeControl::Departure | TimeControl::Both)
    }

    #[must_use]
    pub const fn arrival_aware(self) -> bool {
        matches!(self, TimeControl::Arrival | TimeControl::Both)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            TimeControl::None => "none",
            TimeControl::Departure => "departure",
            TimeControl::Arrival => "arrival",
            TimeControl::Both => "both",
        }
    }
}

/// Travel modes the seam understands. Providers advertise which subset
/// they support; an unsupported mode is a typed refusal, never a silent
/// fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TravelMode {
    Driving,
    Walking,
    Cycling,
    PublicTransport,
}

impl TravelMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            TravelMode::Driving => "driving",
            TravelMode::Walking => "walking",
            TravelMode::Cycling => "cycling",
            TravelMode::PublicTransport => "public_transport",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "driving" => Some(Self::Driving),
            "walking" => Some(Self::Walking),
            "cycling" => Some(Self::Cycling),
            "public_transport" => Some(Self::PublicTransport),
            _ => None,
        }
    }
}

/// The capability set a provider may declare and a request may need.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingCapability {
    /// N×M duration/distance grid (one batched request).
    Matrix,
    /// Turn-by-turn route with per-leg totals.
    Route,
    /// Forward geocoding (address → coordinate).
    Geocode,
    /// Reachability contours around a point.
    Isochrone,
}

impl RoutingCapability {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            RoutingCapability::Matrix => "matrix",
            RoutingCapability::Route => "route",
            RoutingCapability::Geocode => "geocode",
            RoutingCapability::Isochrone => "isochrone",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "matrix" => Some(Self::Matrix),
            "route" => Some(Self::Route),
            "geocode" => Some(Self::Geocode),
            "isochrone" => Some(Self::Isochrone),
            _ => None,
        }
    }
}

/// How the numbers were produced. `Measured` means a routing engine
/// computed them against real network geometry; `Heuristic` means the
/// built-in table estimated them. Dispatcher-facing surfaces label the
/// difference, and suggestion-grade features require `Measured`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingQuality {
    Heuristic,
    Measured,
}

impl RoutingQuality {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            RoutingQuality::Heuristic => "heuristic",
            RoutingQuality::Measured => "measured",
        }
    }
}

/// One endpoint in a request: either a resolved coordinate, an address a
/// provider with the geocode capability may resolve, or a labelled
/// reference the caller already resolved (a site id, a start point).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Place {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<GeoPoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl Place {
    #[must_use]
    pub fn at(point: GeoPoint) -> Self {
        Self {
            point: Some(point),
            address: None,
            label: None,
        }
    }

    #[must_use]
    pub fn address(address: impl Into<String>) -> Self {
        Self {
            point: None,
            address: Some(address.into()),
            label: None,
        }
    }

    #[must_use]
    pub fn labelled(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// True when the place can be routed without a geocode round-trip.
    #[must_use]
    pub fn is_resolved(&self) -> bool {
        self.point.map(|p| p.is_plausible()).unwrap_or(false)
    }
}

/// Which annotations a matrix request needs. Duration-only is the cheap
/// path; distance costs extra on some vendors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixAnnotations {
    pub duration: bool,
    pub distance: bool,
}

impl MatrixAnnotations {
    #[must_use]
    pub const fn both() -> Self {
        Self {
            duration: true,
            distance: true,
        }
    }

    #[must_use]
    pub const fn duration_only() -> Self {
        Self {
            duration: true,
            distance: false,
        }
    }
}

impl Default for MatrixAnnotations {
    fn default() -> Self {
        Self::both()
    }
}

/// A batched N×M matrix request. Time control is explicit: `depart_at`
/// asks "leaving then", `arrive_by` asks "arriving by then" — setting
/// both is invalid, and providers that cannot honor one return a typed
/// refusal rather than silently ignoring it.
#[derive(Debug, Clone)]
pub struct MatrixRequest {
    pub sources: Vec<Place>,
    pub targets: Vec<Place>,
    pub mode: TravelMode,
    pub depart_at: Option<DateTime<Utc>>,
    pub arrive_by: Option<DateTime<Utc>>,
    pub annotations: MatrixAnnotations,
}

impl MatrixRequest {
    #[must_use]
    pub fn new(sources: Vec<Place>, targets: Vec<Place>, mode: TravelMode) -> Self {
        Self {
            sources,
            targets,
            mode,
            depart_at: None,
            arrive_by: None,
            annotations: MatrixAnnotations::both(),
        }
    }

    #[must_use]
    pub fn departing(mut self, at: DateTime<Utc>) -> Self {
        self.depart_at = Some(at);
        self
    }

    #[must_use]
    pub fn arriving_by(mut self, at: DateTime<Utc>) -> Self {
        self.arrive_by = Some(at);
        self
    }

    pub fn validate(&self) -> Result<(), crate::RoutingError> {
        if self.sources.is_empty() || self.targets.is_empty() {
            return Err(crate::RoutingError::Invalid(
                "matrix needs at least one source and one target".into(),
            ));
        }
        if self.sources.len().saturating_mul(self.targets.len()) > MAX_MATRIX_CELLS {
            return Err(crate::RoutingError::Invalid(format!(
                "matrix exceeds {MAX_MATRIX_CELLS} cells; split the batch"
            )));
        }
        if self.depart_at.is_some() && self.arrive_by.is_some() {
            return Err(crate::RoutingError::Invalid(
                "matrix accepts depart_at or arrive_by, never both".into(),
            ));
        }
        if !self.annotations.duration && !self.annotations.distance {
            return Err(crate::RoutingError::Invalid(
                "matrix needs at least one annotation (duration or distance)".into(),
            ));
        }
        Ok(())
    }
}

/// One matrix cell. `duration_seconds` / `distance_meters` are `None`
/// when the pair is unreachable or the vendor returned no value — an
/// honest hole, never a zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatrixCell {
    pub source_index: usize,
    pub target_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_meters: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatrixResponse {
    pub provider: String,
    pub quality: RoutingQuality,
    pub cells: Vec<MatrixCell>,
}

impl MatrixResponse {
    #[must_use]
    pub fn cell(&self, source_index: usize, target_index: usize) -> Option<&MatrixCell> {
        self.cells
            .iter()
            .find(|c| c.source_index == source_index && c.target_index == target_index)
    }
}

/// A route request. `origin` and `destination` are required; intermediate
/// `waypoints` are ordered.
#[derive(Debug, Clone)]
pub struct RouteRequest {
    pub origin: Place,
    pub destination: Place,
    pub waypoints: Vec<Place>,
    pub mode: TravelMode,
    pub depart_at: Option<DateTime<Utc>>,
    pub arrive_by: Option<DateTime<Utc>>,
}

impl RouteRequest {
    #[must_use]
    pub fn new(origin: Place, destination: Place, mode: TravelMode) -> Self {
        Self {
            origin,
            destination,
            waypoints: Vec::new(),
            mode,
            depart_at: None,
            arrive_by: None,
        }
    }

    pub fn validate(&self) -> Result<(), crate::RoutingError> {
        if self.depart_at.is_some() && self.arrive_by.is_some() {
            return Err(crate::RoutingError::Invalid(
                "route accepts depart_at or arrive_by, never both".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteLeg {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_meters: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteResponse {
    pub provider: String,
    pub quality: RoutingQuality,
    pub legs: Vec<RouteLeg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_duration_seconds: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_distance_meters: Option<i64>,
    /// Encoded geometry when the vendor returned it (`polyline`/GeoJSON);
    /// the seam never interprets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GeocodeRequest {
    pub query: String,
    pub near: Option<GeoPoint>,
    /// Clamped by each provider to its own maximum.
    pub limit: u8,
    pub country: Option<String>,
}

impl GeocodeRequest {
    #[must_use]
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            near: None,
            limit: 5,
            country: None,
        }
    }

    pub fn validate(&self) -> Result<(), crate::RoutingError> {
        if self.query.trim().is_empty() {
            return Err(crate::RoutingError::Invalid(
                "geocode query is empty".into(),
            ));
        }
        if self.limit == 0 {
            return Err(crate::RoutingError::Invalid(
                "geocode limit must be at least 1".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeocodeHit {
    pub label: String,
    pub point: GeoPoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_place_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IsochroneRequest {
    pub center: GeoPoint,
    pub mode: TravelMode,
    /// Contour bands in minutes (providers cap the count — a refusal is
    /// typed when the list is longer than they accept).
    pub contours_minutes: Vec<u16>,
    pub depart_at: Option<DateTime<Utc>>,
}

impl IsochroneRequest {
    pub fn validate(&self) -> Result<(), crate::RoutingError> {
        if self.contours_minutes.is_empty() {
            return Err(crate::RoutingError::Invalid(
                "isochrone needs at least one contour".into(),
            ));
        }
        if self.contours_minutes.len() > 4 {
            return Err(crate::RoutingError::Invalid(
                "isochrone accepts at most 4 contours".into(),
            ));
        }
        if self.contours_minutes.contains(&0) {
            return Err(crate::RoutingError::Invalid(
                "isochrone contours must be positive minutes".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IsochroneContour {
    pub minutes: u16,
    /// GeoJSON geometry (Polygon/MultiPolygon) as a JSON string.
    pub geometry_geojson: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IsochroneResponse {
    pub provider: String,
    pub quality: RoutingQuality,
    pub contours: Vec<IsochroneContour>,
}

/// The scheduling-facing result of one leg estimate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TravelEstimate {
    pub minutes: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_meters: Option<i64>,
    pub provider: String,
    pub quality: RoutingQuality,
    /// True when the provider actually honored the departure/arrival time
    /// control the caller asked for; false when it fell back to a
    /// time-independent duration (the honest label for static graphs).
    pub time_controlled: bool,
}

/// Rounds seconds up to whole minutes (a 61-second leg is 2 minutes —
/// never under-estimate transfer feasibility). Zero seconds stays zero.
#[must_use]
pub fn estimate_minutes(duration_seconds: i64) -> i64 {
    if duration_seconds <= 0 {
        return 0;
    }
    (duration_seconds + 59) / 60
}
