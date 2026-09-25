//! Self-hosted OSRM / OpenRouteService adapter — the first measured
//! provider (TASKLIST lane-7 part A).
//!
//! Two dialects share one adapter because they share one job (a
//! self-hosted distance matrix + route server):
//!
//! * [`OsrmDialect::Osrm`] — the OSRM `table`/`route` HTTP API
//!   (`/table/v1/{profile}/{lon,lat;…}`).
//! * [`OsrmDialect::OpenRouteService`] — the ORS v2 JSON API
//!   (`POST /v2/matrix/{profile}`, `POST /v2/directions/{profile}`),
//!   with an optional API key for a hosted instance.
//!
//! Static graphs carry no traffic model, so the adapter advertises
//! [`TimeControl::None`]: a departure time is never silently claimed to
//! have shaped the answer (the result says `time_controlled: false`).
//!
//! Non-purpose: no routing server management, no tile hosting, no
//! credential storage (config comes from the operator).

use crate::error::RoutingError;
use crate::provider::{cell_from_values, ProviderInfo, RoutingProvider};
use crate::types::{
    MatrixCell, MatrixRequest, MatrixResponse, Place, RouteLeg, RouteRequest, RouteResponse,
    RoutingCapability, RoutingQuality, TimeControl, TravelMode,
};
use async_trait::async_trait;
use serde::Deserialize;
use std::time::Duration;

/// Which wire dialect the self-hosted server speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsrmDialect {
    Osrm,
    OpenRouteService,
}

/// Operator configuration for the self-hosted adapter.
#[derive(Debug, Clone, PartialEq)]
pub struct OsrmConfig {
    /// Base URL, e.g. `http://osrm.internal:5000` (no trailing slash
    /// required).
    pub base_url: String,
    pub dialect: OsrmDialect,
    /// Profile path segment (`driving`, `walking`, `cycling` for OSRM;
    /// `driving-car`, `foot-walking`, `cycling-regular` for ORS).
    pub profile: String,
    /// The mode this instance's profile actually serves. Advertised
    /// verbatim — never widened to modes the graph cannot answer.
    pub mode: TravelMode,
    /// ORS API key (hosted instances); sent as `Authorization`. OSRM
    /// self-host needs none.
    pub api_key: Option<String>,
    pub timeout: Duration,
}

impl OsrmConfig {
    #[must_use]
    pub fn new(base_url: impl Into<String>, dialect: OsrmDialect, mode: TravelMode) -> Self {
        let profile = match (dialect, mode) {
            (OsrmDialect::Osrm, TravelMode::Driving) => "driving",
            (OsrmDialect::Osrm, TravelMode::Walking) => "walking",
            (OsrmDialect::Osrm, TravelMode::Cycling) => "cycling",
            (OsrmDialect::Osrm, TravelMode::PublicTransport) => "driving",
            (OsrmDialect::OpenRouteService, TravelMode::Driving) => "driving-car",
            (OsrmDialect::OpenRouteService, TravelMode::Walking) => "foot-walking",
            (OsrmDialect::OpenRouteService, TravelMode::Cycling) => "cycling-regular",
            (OsrmDialect::OpenRouteService, TravelMode::PublicTransport) => "driving-car",
        };
        Self {
            base_url: base_url.into(),
            dialect,
            profile: profile.into(),
            mode,
            api_key: None,
            timeout: Duration::from_secs(10),
        }
    }
}

pub struct OsrmRoutingProvider {
    config: OsrmConfig,
    client: reqwest::Client,
    info: ProviderInfo,
}

impl OsrmRoutingProvider {
    pub fn new(config: OsrmConfig) -> Result<Self, RoutingError> {
        if config.base_url.trim().is_empty() {
            return Err(RoutingError::Invalid(
                "OSRM/ORS base_url must not be empty".into(),
            ));
        }
        if config.profile.trim().is_empty() {
            return Err(RoutingError::Invalid(
                "OSRM/ORS profile must not be empty".into(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| RoutingError::Unavailable(format!("http client: {e}")))?;
        let info = ProviderInfo {
            id: "osrm".into(),
            label: match config.dialect {
                OsrmDialect::Osrm => "Self-hosted OSRM".into(),
                OsrmDialect::OpenRouteService => "Self-hosted OpenRouteService".into(),
            },
            quality: RoutingQuality::Measured,
            capabilities: vec![RoutingCapability::Matrix, RoutingCapability::Route],
            modes: vec![config.mode],
            batched_matrix: true,
            time_control: TimeControl::None,
        };
        Ok(Self {
            config,
            client,
            info,
        })
    }

    fn base(&self) -> String {
        self.config.base_url.trim_end_matches('/').to_string()
    }

    fn resolved_point(place: &Place) -> Result<(f64, f64), RoutingError> {
        place
            .point
            .filter(|p| p.is_plausible())
            .map(|p| (p.longitude, p.latitude))
            .ok_or_else(|| {
                RoutingError::Invalid(
                    "OSRM/ORS matrix needs resolved coordinates; geocode the place first".into(),
                )
            })
    }

    async fn get_json_query(
        &self,
        url: String,
        query: &[(&str, String)],
    ) -> Result<serde_json::Value, RoutingError> {
        let response = self
            .client
            .get(url)
            .query(query)
            .send()
            .await
            .map_err(|e| RoutingError::Unavailable(transport_label(&e)))?;
        decode_response(response).await
    }

    async fn post_json(
        &self,
        url: String,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, RoutingError> {
        let mut request = self.client.post(url).json(body);
        if let Some(key) = &self.config.api_key {
            request = request.header("Authorization", key);
        }
        let response = request
            .send()
            .await
            .map_err(|e| RoutingError::Unavailable(transport_label(&e)))?;
        decode_response(response).await
    }

    fn coords_path(points: &[(f64, f64)]) -> String {
        points
            .iter()
            .map(|(lon, lat)| format!("{lon},{lat}"))
            .collect::<Vec<_>>()
            .join(";")
    }
}

/// Maps transport errors onto the seam's vocabulary without echoing a URL
/// that might carry a key.
fn transport_label(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "request timed out".into()
    } else if error.is_connect() {
        "connection failed".into()
    } else {
        "transport error".into()
    }
}

async fn decode_response(response: reqwest::Response) -> Result<serde_json::Value, RoutingError> {
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(RoutingError::Unauthorized);
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let retry_after_secs = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<i64>().ok());
        return Err(RoutingError::RateLimited { retry_after_secs });
    }
    let status_code = status.as_u16();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(RoutingError::upstream(status_code, &body));
    }
    response
        .json::<serde_json::Value>()
        .await
        .map_err(|e| RoutingError::Protocol(format!("unreadable provider body: {e}")))
}

fn f64_grid_to_cells(
    durations: Option<&Vec<Vec<Option<f64>>>>,
    distances: Option<&Vec<Vec<Option<f64>>>>,
    source_count: usize,
    target_count: usize,
) -> Result<Vec<MatrixCell>, RoutingError> {
    let mut cells = Vec::with_capacity(source_count * target_count);
    for i in 0..source_count {
        for j in 0..target_count {
            let duration = durations
                .and_then(|grid| grid.get(i))
                .and_then(|row| row.get(j))
                .copied()
                .flatten();
            let distance = distances
                .and_then(|grid| grid.get(i))
                .and_then(|row| row.get(j))
                .copied()
                .flatten();
            cells.push(cell_from_values(i, j, duration, distance));
        }
    }
    Ok(cells)
}

#[derive(Debug, Deserialize)]
struct OsrmTable {
    code: Option<String>,
    durations: Option<Vec<Vec<Option<f64>>>>,
    distances: Option<Vec<Vec<Option<f64>>>>,
}

#[derive(Debug, Deserialize)]
struct OrsMatrix {
    durations: Option<Vec<Vec<Option<f64>>>>,
    distances: Option<Vec<Vec<Option<f64>>>>,
}

#[derive(Debug, Deserialize)]
struct OsrmRouteBody {
    code: Option<String>,
    routes: Option<Vec<OsrmRoute>>,
}

#[derive(Debug, Deserialize)]
struct OsrmRoute {
    legs: Option<Vec<OsrmLeg>>,
    duration: Option<f64>,
    distance: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct OsrmLeg {
    duration: Option<f64>,
    distance: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct OrsDirectionsBody {
    features: Option<Vec<OrsFeature>>,
}

#[derive(Debug, Deserialize)]
struct OrsFeature {
    properties: Option<OrsProperties>,
}

#[derive(Debug, Deserialize)]
struct OrsProperties {
    segments: Option<Vec<OsrmLeg>>,
    summary: Option<OsrmLeg>,
}

#[async_trait]
impl RoutingProvider for OsrmRoutingProvider {
    fn info(&self) -> &ProviderInfo {
        &self.info
    }

    async fn matrix(&self, request: &MatrixRequest) -> Result<MatrixResponse, RoutingError> {
        request.validate()?;
        if request.mode != self.config.mode {
            return Err(RoutingError::Unsupported {
                provider: self.info.id.clone(),
                capability: "mode",
            });
        }
        let mut points = Vec::with_capacity(request.sources.len() + request.targets.len());
        for place in request.sources.iter().chain(request.targets.iter()) {
            points.push(Self::resolved_point(place)?);
        }
        let source_count = request.sources.len();
        let target_count = request.targets.len();
        let source_indices: Vec<String> = (0..source_count).map(|i| i.to_string()).collect();
        let target_indices: Vec<String> = (source_count..source_count + target_count)
            .map(|i| i.to_string())
            .collect();
        let cells = match self.config.dialect {
            OsrmDialect::Osrm => {
                let url = format!(
                    "{}/table/v1/{}/{}",
                    self.base(),
                    self.config.profile,
                    Self::coords_path(&points)
                );
                let sources = source_indices.join(";");
                let destinations = target_indices.join(";");
                let body = self
                    .get_json_query(
                        url,
                        &[
                            ("annotations", "duration,distance".to_string()),
                            ("sources", sources),
                            ("destinations", destinations),
                        ],
                    )
                    .await?;
                let table: OsrmTable = serde_json::from_value(body)
                    .map_err(|e| RoutingError::Protocol(format!("unexpected OSRM table: {e}")))?;
                if table.code.as_deref().is_some_and(|c| c != "Ok") {
                    return Err(RoutingError::Protocol(format!(
                        "OSRM table refused: {}",
                        table.code.unwrap_or_default()
                    )));
                }
                f64_grid_to_cells(
                    table.durations.as_ref(),
                    table.distances.as_ref(),
                    source_count,
                    target_count,
                )?
            }
            OsrmDialect::OpenRouteService => {
                let url = format!("{}/v2/matrix/{}", self.base(), self.config.profile);
                let body = serde_json::json!({
                    "locations": points.iter().map(|(lon, lat)| [lon, lat]).collect::<Vec<_>>(),
                    "sources": (0..source_count).collect::<Vec<_>>(),
                    "destinations": (0..target_count).collect::<Vec<_>>(),
                    "metrics": ["duration", "distance"],
                    "resolve_locations": false,
                });
                let response = self.post_json(url, &body).await?;
                let matrix: OrsMatrix = serde_json::from_value(response)
                    .map_err(|e| RoutingError::Protocol(format!("unexpected ORS matrix: {e}")))?;
                f64_grid_to_cells(
                    matrix.durations.as_ref(),
                    matrix.distances.as_ref(),
                    source_count,
                    target_count,
                )?
            }
        };
        Ok(MatrixResponse {
            provider: self.info.id.clone(),
            quality: RoutingQuality::Measured,
            cells,
        })
    }

    async fn route(&self, request: &RouteRequest) -> Result<RouteResponse, RoutingError> {
        request.validate()?;
        if request.mode != self.config.mode {
            return Err(RoutingError::Unsupported {
                provider: self.info.id.clone(),
                capability: "mode",
            });
        }
        let mut places = Vec::with_capacity(request.waypoints.len() + 2);
        places.push(request.origin.clone());
        places.extend(request.waypoints.iter().cloned());
        places.push(request.destination.clone());
        let mut points = Vec::with_capacity(places.len());
        for place in &places {
            points.push(Self::resolved_point(place)?);
        }
        let (legs, total_duration, total_distance) = match self.config.dialect {
            OsrmDialect::Osrm => {
                let url = format!(
                    "{}/route/v1/{}/{}",
                    self.base(),
                    self.config.profile,
                    Self::coords_path(&points)
                );
                let body = self
                    .get_json_query(
                        url,
                        &[
                            ("overview", "false".to_string()),
                            ("alternatives", "false".to_string()),
                            ("steps", "false".to_string()),
                        ],
                    )
                    .await?;
                let parsed: OsrmRouteBody = serde_json::from_value(body)
                    .map_err(|e| RoutingError::Protocol(format!("unexpected OSRM route: {e}")))?;
                if parsed.code.as_deref().is_some_and(|c| c != "Ok") {
                    return Err(RoutingError::Protocol(format!(
                        "OSRM route refused: {}",
                        parsed.code.unwrap_or_default()
                    )));
                }
                let route = parsed
                    .routes
                    .and_then(|mut r| r.pop())
                    .ok_or_else(|| RoutingError::Protocol("OSRM returned no route".into()))?;
                let legs = route
                    .legs
                    .unwrap_or_default()
                    .into_iter()
                    .map(|l| RouteLeg {
                        duration_seconds: l.duration.map(|d| d.round() as i64),
                        distance_meters: l.distance.map(|d| d.round() as i64),
                    })
                    .collect::<Vec<_>>();
                (
                    legs,
                    route.duration.map(|d| d.round() as i64),
                    route.distance.map(|d| d.round() as i64),
                )
            }
            OsrmDialect::OpenRouteService => {
                let url = format!("{}/v2/directions/{}", self.base(), self.config.profile);
                let body = serde_json::json!({
                    "coordinates": points.iter().map(|(lon, lat)| [lon, lat]).collect::<Vec<_>>(),
                    "instructions": false,
                });
                let response = self.post_json(url, &body).await?;
                let parsed: OrsDirectionsBody = serde_json::from_value(response).map_err(|e| {
                    RoutingError::Protocol(format!("unexpected ORS directions: {e}"))
                })?;
                let feature = parsed
                    .features
                    .and_then(|mut f| f.pop())
                    .ok_or_else(|| RoutingError::Protocol("ORS returned no route".into()))?;
                let properties = feature
                    .properties
                    .ok_or_else(|| RoutingError::Protocol("ORS route has no properties".into()))?;
                let legs = properties
                    .segments
                    .unwrap_or_default()
                    .into_iter()
                    .map(|l| RouteLeg {
                        duration_seconds: l.duration.map(|d| d.round() as i64),
                        distance_meters: l.distance.map(|d| d.round() as i64),
                    })
                    .collect::<Vec<_>>();
                let summary = properties.summary;
                (
                    legs,
                    summary.as_ref().and_then(|s| s.duration).map(|d| d.round() as i64),
                    summary
                        .as_ref()
                        .and_then(|s| s.distance)
                        .map(|d| d.round() as i64),
                )
            }
        };
        Ok(RouteResponse {
            provider: self.info.id.clone(),
            quality: RoutingQuality::Measured,
            legs,
            total_duration_seconds: total_duration,
            total_distance_meters: total_distance,
            geometry: None,
        })
    }
}
