//! The out-of-tree routing adapter: any provider pack that implements the
//! documented Meridian Routing Contract over HTTP (ADR 0014, normative
//! wire shape in `docs/routing-providers.md`).
//!
//! This is how premium/relay routing connectors (TravelTime, Geoapify, …)
//! reach the core with ZERO core changes: the pack hosts the documented
//! endpoints, the operator points `ROUTING_PROVIDER=contract` at it, and
//! this adapter speaks the contract. Vendor mapping, entitlement, and
//! credentials live in the pack; the core never names a vendor and never
//! checks a license (ADR 0013 §4).
//!
//! The pack declares its capability set, modes, quality, and time control
//! in the operator config (which mirrors its plugin manifest); the
//! adapter enforces the declaration before every call — an undeclared
//! capability is a typed refusal, not an attempt.

use crate::error::RoutingError;
use crate::provider::{cell_from_values, ProviderInfo, RoutingProvider};
use crate::types::{
    GeocodeHit, GeocodeRequest, IsochroneContour, IsochroneRequest, IsochroneResponse, MatrixCell,
    MatrixRequest, MatrixResponse, RouteLeg, RouteRequest, RouteResponse, RoutingCapability,
    RoutingQuality, TimeControl, TravelMode,
};
use async_trait::async_trait;
use serde::Deserialize;
use std::time::Duration;

/// Operator configuration for one contract pack. Every field mirrors the
/// pack's plugin manifest (`connector_profile.routing`) so install-time
/// consent shows exactly what the adapter will honor.
#[derive(Debug, Clone, PartialEq)]
pub struct ContractConfig {
    /// Stable provider id the registry activates, e.g. `contract:traveltime`.
    pub id: String,
    /// Human label for status surfaces, e.g. `TravelTime (relay pack)`.
    pub label: String,
    /// Base URL of the pack's contract root (no trailing slash required).
    pub base_url: String,
    /// Bearer token the pack expects, when it expects one. Never logged,
    /// never placed in a URL.
    pub token: Option<String>,
    /// Declared measurement quality; relay packs are `Measured`.
    pub quality: RoutingQuality,
    /// Declared capability set. Calls outside it are typed refusals.
    pub capabilities: Vec<RoutingCapability>,
    pub modes: Vec<TravelMode>,
    pub time_control: TimeControl,
    pub timeout: Duration,
}

impl ContractConfig {
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            base_url: base_url.into(),
            token: None,
            quality: RoutingQuality::Measured,
            capabilities: vec![RoutingCapability::Matrix],
            modes: vec![TravelMode::Driving],
            time_control: TimeControl::Departure,
            timeout: Duration::from_secs(15),
        }
    }
}

pub struct ContractRoutingProvider {
    config: ContractConfig,
    client: reqwest::Client,
    info: ProviderInfo,
}

#[derive(Debug, Deserialize)]
struct MatrixBody {
    #[serde(default)]
    cells: Vec<ContractCell>,
}

#[derive(Debug, Deserialize)]
struct ContractCell {
    source_index: usize,
    target_index: usize,
    #[serde(default)]
    duration_seconds: Option<f64>,
    #[serde(default)]
    distance_meters: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct RouteBody {
    #[serde(default)]
    legs: Vec<ContractLeg>,
    #[serde(default)]
    total_duration_seconds: Option<f64>,
    #[serde(default)]
    total_distance_meters: Option<f64>,
    #[serde(default)]
    geometry: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ContractLeg {
    #[serde(default)]
    duration_seconds: Option<f64>,
    #[serde(default)]
    distance_meters: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct GeocodeBody {
    #[serde(default)]
    hits: Vec<ContractHit>,
}

#[derive(Debug, Deserialize)]
struct ContractHit {
    label: String,
    latitude: f64,
    longitude: f64,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default)]
    provider_place_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IsochroneBody {
    #[serde(default)]
    contours: Vec<ContractContour>,
}

#[derive(Debug, Deserialize)]
struct ContractContour {
    minutes: u16,
    geometry_geojson: String,
}

impl ContractRoutingProvider {
    pub fn new(config: ContractConfig) -> Result<Self, RoutingError> {
        if config.id.trim().is_empty() {
            return Err(RoutingError::Invalid("contract provider id is empty".into()));
        }
        if config.base_url.trim().is_empty() {
            return Err(RoutingError::Invalid(
                "contract provider base_url is empty".into(),
            ));
        }
        if config.capabilities.is_empty() {
            return Err(RoutingError::Invalid(
                "contract provider declares no capabilities".into(),
            ));
        }
        if config.modes.is_empty() {
            return Err(RoutingError::Invalid(
                "contract provider declares no travel modes".into(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| RoutingError::Unavailable(format!("http client: {e}")))?;
        let info = ProviderInfo {
            id: config.id.clone(),
            label: config.label.clone(),
            quality: config.quality,
            capabilities: config.capabilities.clone(),
            modes: config.modes.clone(),
            batched_matrix: true,
            time_control: config.time_control,
        };
        Ok(Self {
            config,
            client,
            info,
        })
    }

    fn require(&self, capability: RoutingCapability) -> Result<(), RoutingError> {
        if self.config.capabilities.contains(&capability) {
            Ok(())
        } else {
            Err(RoutingError::Unsupported {
                provider: self.info.id.clone(),
                capability: capability.as_str(),
            })
        }
    }

    fn require_mode(&self, mode: TravelMode) -> Result<(), RoutingError> {
        if self.config.modes.contains(&mode) {
            Ok(())
        } else {
            Err(RoutingError::Unsupported {
                provider: self.info.id.clone(),
                capability: "mode",
            })
        }
    }

    async fn post(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value, RoutingError> {
        let url = format!("{}{}", self.config.base_url.trim_end_matches('/'), path);
        let mut request = self.client.post(url).json(body);
        if let Some(token) = &self.config.token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                RoutingError::Unavailable("contract request timed out".into())
            } else if error.is_connect() {
                RoutingError::Unavailable("contract connection failed".into())
            } else {
                RoutingError::Unavailable("contract transport error".into())
            }
        })?;
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
        let code = status.as_u16();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(RoutingError::upstream(code, &text));
        }
        response
            .json::<serde_json::Value>()
            .await
            .map_err(|e| RoutingError::Protocol(format!("unreadable contract body: {e}")))
    }
}

#[async_trait]
impl RoutingProvider for ContractRoutingProvider {
    fn info(&self) -> &ProviderInfo {
        &self.info
    }

    async fn matrix(&self, request: &MatrixRequest) -> Result<MatrixResponse, RoutingError> {
        self.require(RoutingCapability::Matrix)?;
        self.require_mode(request.mode)?;
        request.validate()?;
        let body = serde_json::json!({
            "mode": request.mode.as_str(),
            "depart_at": request.depart_at,
            "arrive_by": request.arrive_by,
            "annotations": {
                "duration": request.annotations.duration,
                "distance": request.annotations.distance,
            },
            "sources": request.sources,
            "targets": request.targets,
        });
        let response = self.post("/routing/matrix", &body).await?;
        let parsed: MatrixBody = serde_json::from_value(response)
            .map_err(|e| RoutingError::Protocol(format!("unexpected contract matrix: {e}")))?;
        let cells = parsed
            .cells
            .into_iter()
            .map(|cell| {
                cell_from_values(
                    cell.source_index,
                    cell.target_index,
                    cell.duration_seconds,
                    cell.distance_meters,
                )
            })
            .collect::<Vec<MatrixCell>>();
        Ok(MatrixResponse {
            provider: self.info.id.clone(),
            quality: self.config.quality,
            cells,
        })
    }

    async fn route(&self, request: &RouteRequest) -> Result<RouteResponse, RoutingError> {
        self.require(RoutingCapability::Route)?;
        self.require_mode(request.mode)?;
        request.validate()?;
        let body = serde_json::json!({
            "mode": request.mode.as_str(),
            "origin": request.origin,
            "destination": request.destination,
            "waypoints": request.waypoints,
            "depart_at": request.depart_at,
            "arrive_by": request.arrive_by,
        });
        let response = self.post("/routing/route", &body).await?;
        let parsed: RouteBody = serde_json::from_value(response)
            .map_err(|e| RoutingError::Protocol(format!("unexpected contract route: {e}")))?;
        Ok(RouteResponse {
            provider: self.info.id.clone(),
            quality: self.config.quality,
            legs: parsed
                .legs
                .into_iter()
                .map(|leg| RouteLeg {
                    duration_seconds: leg.duration_seconds.map(|v| v.round() as i64),
                    distance_meters: leg.distance_meters.map(|v| v.round() as i64),
                })
                .collect(),
            total_duration_seconds: parsed.total_duration_seconds.map(|v| v.round() as i64),
            total_distance_meters: parsed.total_distance_meters.map(|v| v.round() as i64),
            geometry: parsed.geometry,
        })
    }

    async fn geocode(&self, request: &GeocodeRequest) -> Result<Vec<GeocodeHit>, RoutingError> {
        self.require(RoutingCapability::Geocode)?;
        request.validate()?;
        let body = serde_json::json!({
            "query": request.query,
            "near": request.near,
            "limit": request.limit,
            "country": request.country,
        });
        let response = self.post("/routing/geocode", &body).await?;
        let parsed: GeocodeBody = serde_json::from_value(response)
            .map_err(|e| RoutingError::Protocol(format!("unexpected contract geocode: {e}")))?;
        Ok(parsed
            .hits
            .into_iter()
            .map(|hit| GeocodeHit {
                label: hit.label,
                point: crate::types::GeoPoint::new(hit.latitude, hit.longitude),
                confidence: hit.confidence,
                provider_place_id: hit.provider_place_id,
            })
            .collect())
    }

    async fn isochrone(
        &self,
        request: &IsochroneRequest,
    ) -> Result<IsochroneResponse, RoutingError> {
        self.require(RoutingCapability::Isochrone)?;
        self.require_mode(request.mode)?;
        request.validate()?;
        let body = serde_json::json!({
            "center": request.center,
            "mode": request.mode.as_str(),
            "contours_minutes": request.contours_minutes,
            "depart_at": request.depart_at,
        });
        let response = self.post("/routing/isochrone", &body).await?;
        let parsed: IsochroneBody = serde_json::from_value(response)
            .map_err(|e| RoutingError::Protocol(format!("unexpected contract isochrone: {e}")))?;
        Ok(IsochroneResponse {
            provider: self.info.id.clone(),
            quality: self.config.quality,
            contours: parsed
                .contours
                .into_iter()
                .map(|c| IsochroneContour {
                    minutes: c.minutes,
                    geometry_geojson: c.geometry_geojson,
                })
                .collect(),
        })
    }
}
