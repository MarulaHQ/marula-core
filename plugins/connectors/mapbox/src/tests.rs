//! Fixture-server tests: the Mapbox adapter against local endpoints, no
//! live API, no key material beyond the fixture string.

use crate::{MapboxConfig, MapboxRoutingProvider};
use axum::{extract::Query, routing::get, Json, Router};
use core_routing::{
    GeoPoint, GeocodeRequest, IsochroneRequest, MatrixRequest, Place, RouteRequest, RoutingError,
    RoutingProvider, TravelMode,
};
use std::collections::HashMap;

async fn spawn(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let addr = listener.local_addr().expect("fixture addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("fixture server");
    });
    format!("http://{addr}")
}

fn token_of(query: &HashMap<String, String>) -> Option<String> {
    query.get("access_token").cloned()
}

async fn matrix(
    Query(query): Query<HashMap<String, String>>,
) -> Json<serde_json::Value> {
    assert_eq!(token_of(&query).as_deref(), Some("pk.fixture"));
    assert_eq!(query.get("annotations").map(String::as_str), Some("duration,distance"));
    Json(serde_json::json!({
        "code": "Ok",
        "durations": [[600.0]],
        "distances": [[5000.0]],
    }))
}

async fn directions(
    Query(query): Query<HashMap<String, String>>,
) -> Json<serde_json::Value> {
    assert_eq!(token_of(&query).as_deref(), Some("pk.fixture"));
    Json(serde_json::json!({
        "code": "Ok",
        "routes": [{
            "legs": [{ "duration": 600.0, "distance": 5000.0 }],
            "duration": 600.0,
            "distance": 5000.0
        }]
    }))
}

async fn geocode(
    Query(query): Query<HashMap<String, String>>,
) -> Json<serde_json::Value> {
    assert_eq!(token_of(&query).as_deref(), Some("pk.fixture"));
    assert_eq!(query.get("q").map(String::as_str), Some("Unter den Linden 1"));
    Json(serde_json::json!({
        "features": [{
            "properties": {
                "full_address": "Unter den Linden 1, Berlin",
                "coordinates": { "latitude": 52.517, "longitude": 13.3889 }
            }
        }]
    }))
}

async fn isochrone(
    Query(query): Query<HashMap<String, String>>,
) -> Json<serde_json::Value> {
    assert_eq!(token_of(&query).as_deref(), Some("pk.fixture"));
    assert_eq!(query.get("contours_minutes").map(String::as_str), Some("10,20"));
    Json(serde_json::json!({
        "features": [
            { "properties": { "contour": 10 }, "geometry": { "type": "Polygon", "coordinates": [] } },
            { "properties": { "contour": 20 }, "geometry": { "type": "Polygon", "coordinates": [] } }
        ]
    }))
}

async fn unauthorized() -> (axum::http::StatusCode, &'static str) {
    (axum::http::StatusCode::UNAUTHORIZED, "bad token pk.fixture")
}

fn provider_for(base: String, profile: &str) -> MapboxRoutingProvider {
    let mut config = MapboxConfig::driving("pk.fixture");
    config.base_url = base;
    config.profile = profile.into();
    MapboxRoutingProvider::new(config).expect("provider")
}

#[tokio::test]
async fn mapbox_matrix_route_geocode_isochrone_against_fixtures() {
    let app = Router::new()
        .route("/directions-matrix/v1/mapbox/driving/{coords}", get(matrix))
        .route("/directions/v5/mapbox/driving/{coords}", get(directions))
        .route("/search/geocode/v6/forward", get(geocode))
        .route("/isochrone/v1/mapbox/driving/{lonlat}", get(isochrone));
    let base = spawn(app).await;
    let provider = provider_for(base, "driving");
    assert_eq!(provider.info().id, "mapbox");
    assert_eq!(provider.info().quality, core_routing::RoutingQuality::Measured);

    let response = provider
        .matrix(&MatrixRequest::new(
            vec![Place::at(GeoPoint::new(52.5, 13.4))],
            vec![Place::at(GeoPoint::new(52.6, 13.5))],
            TravelMode::Driving,
        ))
        .await
        .expect("matrix");
    assert_eq!(response.cell(0, 0).unwrap().duration_seconds, Some(600));

    let route = provider
        .route(&RouteRequest::new(
            Place::at(GeoPoint::new(52.5, 13.4)),
            Place::at(GeoPoint::new(52.6, 13.5)),
            TravelMode::Driving,
        ))
        .await
        .expect("route");
    assert_eq!(route.total_distance_meters, Some(5000));

    let hits = provider
        .geocode(&GeocodeRequest::new("Unter den Linden 1"))
        .await
        .expect("geocode");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].point.latitude, 52.517);

    let isochrones = provider
        .isochrone(&IsochroneRequest {
            center: GeoPoint::new(52.5, 13.4),
            mode: TravelMode::Driving,
            contours_minutes: vec![10, 20],
            depart_at: None,
        })
        .await
        .expect("isochrone");
    assert_eq!(isochrones.contours.len(), 2);
}

#[tokio::test]
async fn mapbox_token_never_appears_in_errors_or_debug() {
    let app = Router::new().route(
        "/directions-matrix/v1/mapbox/driving/{coords}",
        get(unauthorized),
    );
    let base = spawn(app).await;
    let provider = provider_for(base, "driving");
    let error = provider
        .matrix(&MatrixRequest::new(
            vec![Place::at(GeoPoint::new(52.5, 13.4))],
            vec![Place::at(GeoPoint::new(52.6, 13.5))],
            TravelMode::Driving,
        ))
        .await
        .expect_err("must fail");
    assert!(matches!(error, RoutingError::Unauthorized));

    let mut config = MapboxConfig::driving("pk.fixture");
    config.base_url = "http://fixture.invalid".into();
    let debug = format!("{config:?}");
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("pk.fixture"));
}
