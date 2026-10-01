//! The Mapbox adapter. Endpoints (all v1/v5/v6 as documented at
//! docs.mapbox.com):
//!
//! * `GET /directions-matrix/v1/mapbox/{profile}/{lon,lat;…}` — matrix.
//! * `GET /directions/v5/mapbox/{profile}/{lon,lat;…}` — directions.
//! * `GET /search/geocode/v6/forward` — forward geocoding.
//! * `GET /isochrone/v1/mapbox/{profile}/{lon},{lat}` — isochrones.
//!
//! The access token rides the `access_token` query parameter (Mapbox's
//! documented scheme). Because of that, transport errors never echo the
//! request URL — mapped to fixed strings instead.

use async_trait::async_trait;
use core_routing::{
    cell_from_values, GeoPoint, GeocodeHit, GeocodeRequest, IsochroneContour, IsochroneRequest,
    IsochroneResponse, MatrixCell, MatrixRequest, MatrixResponse, Place, ProviderInfo, RouteLeg,
    RouteRequest, RouteResponse, RoutingCapability, RoutingError, RoutingProvider, RoutingQuality,
    TimeControl, TravelMode,
};
use serde::Deserialize;
use std::fmt;
use std::time::Duration;

/// Operator configuration. `Debug` is hand-written to redact the token.
#[derive(Clone, PartialEq)]
pub struct MapboxConfig {
    /// Mapbox access token (`pk.*`/`sk.*`). Never logged or echoed.
    pub access_token: String,
    /// API base, default `https://api.mapbox.com` (tests point this at a
    /// fixture server).
    pub base_url: String,
    /// Mapbox profile path: `driving` | `driving-traffic` | `walking` |
    /// `cycling`.
    pub profile: String,
    /// The mode this provider serves.
    pub mode: TravelMode,
    pub timeout: Duration,
}

impl fmt::Debug for MapboxConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MapboxConfig")
            .field("access_token", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("profile", &self.profile)
            .field("mode", &self.mode)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl MapboxConfig {
    /// Driving profile against the public API.
    #[must_use]
    pub fn driving(access_token: impl Into<String>) -> Self {
        Self {
            access_token: access_token.into(),
            base_url: "https://api.mapbox.com".into(),
            profile: "driving".into(),
            mode: TravelMode::Driving,
            timeout: Duration::from_secs(10),
        }
    }
}

pub struct MapboxRoutingProvider {
    config: MapboxConfig,
    client: reqwest::Client,
    info: ProviderInfo,
}

impl MapboxRoutingProvider {
    pub fn new(config: MapboxConfig) -> Result<Self, RoutingError> {
        if config.access_token.trim().is_empty() {
            return Err(RoutingError::Invalid(
                "Mapbox access token must not be empty".into(),
            ));
        }
        if config.profile.trim().is_empty() {
            return Err(RoutingError::Invalid(
                "Mapbox profile must not be empty".into(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| RoutingError::Unavailable(format!("http client: {e}")))?;
        // Mapbox honors departure time only on the traffic profile; any
        // other profile gets a time-independent (but still measured)
        // duration, which the estimate labels honestly.
        let time_control = if config.profile == "driving-traffic" {
            TimeControl::Departure
        } else {
            TimeControl::None
        };
        let info = ProviderInfo {
            id: "mapbox".into(),
            label: "Mapbox".into(),
            quality: RoutingQuality::Measured,
            capabilities: vec![
                RoutingCapability::Matrix,
                RoutingCapability::Route,
                RoutingCapability::Geocode,
                RoutingCapability::Isochrone,
            ],
            modes: vec![config.mode],
            batched_matrix: true,
            time_control,
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
                    "Mapbox matrix/route need resolved coordinates; geocode the place first".into(),
                )
            })
    }

    async fn get_json(
        &self,
        url: String,
        query: &[(&str, String)],
    ) -> Result<serde_json::Value, RoutingError> {
        let response = self
            .client
            .get(url)
            .query(query)
            .query(&[("access_token", self.config.access_token.clone())])
            .send()
            .await
            .map_err(mapbox_transport)?;
        decode_response(response).await
    }
}

/// Fixed labels — the transport error may embed the request URL (and the
/// token in it), so it is never surfaced.
fn mapbox_transport(error: reqwest::Error) -> RoutingError {
    if error.is_timeout() {
        RoutingError::Unavailable("Mapbox request timed out".into())
    } else if error.is_connect() {
        RoutingError::Unavailable("Mapbox connection failed".into())
    } else {
        RoutingError::Unavailable("Mapbox transport error".into())
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
    let code = status.as_u16();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(RoutingError::upstream(code, &body));
    }
    response
        .json::<serde_json::Value>()
        .await
        .map_err(|e| RoutingError::Protocol(format!("unreadable Mapbox body: {e}")))
}

fn coords_path(points: &[(f64, f64)]) -> String {
    points
        .iter()
        .map(|(lon, lat)| format!("{lon},{lat}"))
        .collect::<Vec<_>>()
        .join(";")
}

#[derive(Debug, Deserialize)]
struct MatrixBody {
    code: Option<String>,
    durations: Option<Vec<Vec<Option<f64>>>>,
    distances: Option<Vec<Vec<Option<f64>>>>,
}

#[derive(Debug, Deserialize)]
struct DirectionsBody {
    code: Option<String>,
    routes: Option<Vec<DirectionsRoute>>,
}

#[derive(Debug, Deserialize)]
struct DirectionsRoute {
    legs: Option<Vec<DirectionsLeg>>,
    duration: Option<f64>,
    distance: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DirectionsLeg {
    duration: Option<f64>,
    distance: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct GeocodeBody {
    features: Option<Vec<GeocodeFeature>>,
}

#[derive(Debug, Deserialize)]
struct GeocodeFeature {
    properties: Option<GeocodeProperties>,
}

#[derive(Debug, Deserialize)]
struct GeocodeProperties {
    full_address: Option<String>,
    name: Option<String>,
    coordinates: Option<GeocodeCoordinates>,
}

#[derive(Debug, Deserialize)]
struct GeocodeCoordinates {
    latitude: Option<f64>,
    longitude: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct IsochroneBody {
    features: Option<Vec<IsochroneFeature>>,
}

#[derive(Debug, Deserialize)]
struct IsochroneFeature {
    properties: Option<IsochroneProperties>,
    geometry: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct IsochroneProperties {
    contour: Option<f64>,
}

impl MapboxRoutingProvider {
    fn matrix_cells(
        body: serde_json::Value,
        source_count: usize,
        target_count: usize,
    ) -> Result<Vec<MatrixCell>, RoutingError> {
        let parsed: MatrixBody = serde_json::from_value(body)
            .map_err(|e| RoutingError::Protocol(format!("unexpected Mapbox matrix: {e}")))?;
        if parsed.code.as_deref().is_some_and(|c| c != "Ok") {
            return Err(RoutingError::Protocol(format!(
                "Mapbox matrix refused: {}",
                parsed.code.unwrap_or_default()
            )));
        }
        let mut cells = Vec::with_capacity(source_count * target_count);
        for i in 0..source_count {
            for j in 0..target_count {
                let duration = parsed
                    .durations
                    .as_ref()
                    .and_then(|grid| grid.get(i))
                    .and_then(|row| row.get(j))
                    .copied()
                    .flatten();
                let distance = parsed
                    .distances
                    .as_ref()
                    .and_then(|grid| grid.get(i))
                    .and_then(|row| row.get(j))
                    .copied()
                    .flatten();
                cells.push(cell_from_values(i, j, duration, distance));
            }
        }
        Ok(cells)
    }
}

#[async_trait]
impl RoutingProvider for MapboxRoutingProvider {
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
        let sources = (0..source_count)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(";");
        let destinations = (source_count..source_count + target_count)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(";");
        let url = format!(
            "{}/directions-matrix/v1/mapbox/{}/{}",
            self.base(),
            self.config.profile,
            coords_path(&points)
        );
        let mut query: Vec<(&str, String)> = vec![
            ("annotations", "duration,distance".to_string()),
            ("sources", sources),
            ("destinations", destinations),
        ];
        if request.annotations.duration && !request.annotations.distance {
            query[0].1 = "duration".to_string();
        }
        if let Some(depart_at) = request.depart_at {
            if self.info.time_control.departure_aware() {
                query.push(("depart_at", depart_at.timestamp().to_string()));
            }
        }
        let body = self.get_json(url, &query).await?;
        Ok(MatrixResponse {
            provider: self.info.id.clone(),
            quality: RoutingQuality::Measured,
            cells: Self::matrix_cells(body, source_count, target_count)?,
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
        let url = format!(
            "{}/directions/v5/mapbox/{}/{}",
            self.base(),
            self.config.profile,
            coords_path(&points)
        );
        let body = self
            .get_json(
                url,
                &[
                    ("overview", "false".to_string()),
                    ("alternatives", "false".to_string()),
                ],
            )
            .await?;
        let parsed: DirectionsBody = serde_json::from_value(body)
            .map_err(|e| RoutingError::Protocol(format!("unexpected Mapbox directions: {e}")))?;
        if parsed.code.as_deref().is_some_and(|c| c != "Ok") {
            return Err(RoutingError::Protocol(format!(
                "Mapbox directions refused: {}",
                parsed.code.unwrap_or_default()
            )));
        }
        let route = parsed
            .routes
            .and_then(|mut routes| routes.pop())
            .ok_or_else(|| RoutingError::Protocol("Mapbox returned no route".into()))?;
        Ok(RouteResponse {
            provider: self.info.id.clone(),
            quality: RoutingQuality::Measured,
            legs: route
                .legs
                .unwrap_or_default()
                .into_iter()
                .map(|leg| RouteLeg {
                    duration_seconds: leg.duration.map(|v| v.round() as i64),
                    distance_meters: leg.distance.map(|v| v.round() as i64),
                })
                .collect(),
            total_duration_seconds: route.duration.map(|v| v.round() as i64),
            total_distance_meters: route.distance.map(|v| v.round() as i64),
            geometry: None,
        })
    }

    async fn geocode(&self, request: &GeocodeRequest) -> Result<Vec<GeocodeHit>, RoutingError> {
        request.validate()?;
        let url = format!("{}/search/geocode/v6/forward", self.base());
        let mut query: Vec<(&str, String)> = vec![
            ("q", request.query.clone()),
            ("limit", request.limit.to_string()),
        ];
        if let Some(near) = request.near {
            query.push(("proximity", format!("{},{}", near.longitude, near.latitude)));
        }
        if let Some(country) = &request.country {
            query.push(("country", country.clone()));
        }
        let body = self.get_json(url, &query).await?;
        let parsed: GeocodeBody = serde_json::from_value(body)
            .map_err(|e| RoutingError::Protocol(format!("unexpected Mapbox geocode: {e}")))?;
        let hits = parsed
            .features
            .unwrap_or_default()
            .into_iter()
            .filter_map(|feature| {
                let properties = feature.properties?;
                let coordinates = properties.coordinates?;
                let point = GeoPoint::new(coordinates.latitude?, coordinates.longitude?);
                if !point.is_plausible() {
                    return None;
                }
                Some(GeocodeHit {
                    label: properties
                        .full_address
                        .or(properties.name)
                        .unwrap_or_else(|| format!("{}, {}", point.latitude, point.longitude)),
                    point,
                    confidence: None,
                    provider_place_id: None,
                })
            })
            .collect();
        Ok(hits)
    }

    async fn isochrone(
        &self,
        request: &IsochroneRequest,
    ) -> Result<IsochroneResponse, RoutingError> {
        request.validate()?;
        if request.mode != self.config.mode {
            return Err(RoutingError::Unsupported {
                provider: self.info.id.clone(),
                capability: "mode",
            });
        }
        let center = format!("{},{}", request.center.longitude, request.center.latitude);
        let url = format!(
            "{}/isochrone/v1/mapbox/{}/{}",
            self.base(),
            self.config.profile,
            center
        );
        let contours = request
            .contours_minutes
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let mut query: Vec<(&str, String)> = vec![("contours_minutes", contours)];
        if let Some(depart_at) = request.depart_at {
            if self.info.time_control.departure_aware() {
                query.push(("depart_at", depart_at.to_rfc3339()));
            }
        }
        let body = self.get_json(url, &query).await?;
        let parsed: IsochroneBody = serde_json::from_value(body)
            .map_err(|e| RoutingError::Protocol(format!("unexpected Mapbox isochrone: {e}")))?;
        let contours = parsed
            .features
            .unwrap_or_default()
            .into_iter()
            .filter_map(|feature| {
                let minutes = feature.properties?.contour? as u16;
                let geometry = feature.geometry?;
                Some(IsochroneContour {
                    minutes,
                    geometry_geojson: serde_json::to_string(&geometry)
                        .unwrap_or_else(|_| "{}".into()),
                })
            })
            .collect();
        Ok(IsochroneResponse {
            provider: self.info.id.clone(),
            quality: RoutingQuality::Measured,
            contours,
        })
    }
}
