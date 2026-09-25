//! Routing service (Wave 32 lane 7): provider construction, travel
//! estimation, and journey context assembly.
//!
//! Purpose: the one place that turns operator config + org policy + a
//! technician's day into routing calls. The seam crate (`core-routing`)
//! stays pure; this module owns the DB reads (sites, start points,
//! settings) and the fallback ladder: a configured provider when
//! registered, else the built-in heuristic from the org's per-mile
//! table. Every estimate carries its provenance.
//!
//! Non-purpose: HTTP views (routes/routing.rs), writes to appointments
//! (routes/scheduling.rs owns save-time booking), suggestion thresholds
//! (core-domain::routing owns the math).

use crate::config::{Config, RoutingChoice};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use chrono::{DateTime, NaiveDate, Utc};
use core_domain::entities::{Site, TravelOrigin};
use core_domain::routing::TravelPlace;
use core_domain::settings::TravelPolicy;
use core_routing::{
    ContractConfig, ContractRoutingProvider, GeoPoint, HeuristicConfig, HeuristicRoutingProvider,
    MatrixRequest, MatrixResponse, OsrmConfig, OsrmDialect, OsrmRoutingProvider, Place,
    RoutingError, RoutingProvider, RoutingRegistry, TravelMode,
};
use mapbox_routing::{MapboxConfig, MapboxRoutingProvider};
use std::sync::Arc;
use uuid::Uuid;

/// Builds the process-wide provider registry from operator config. A
/// provider that fails construction stays DECLARED (`Missing` status):
/// the failure is visible at `/api/dispatch/routing`, and estimates fall
/// back to the heuristic instead of failing the boot.
pub fn build_registry(config: &Config) -> Arc<RoutingRegistry> {
    let registry = RoutingRegistry::new();
    let configured_id: String = match &config.routing {
        RoutingChoice::None => return Arc::new(registry),
        RoutingChoice::Osrm { base_url } => {
            register_or_declare(
                &registry,
                "osrm",
                OsrmRoutingProvider::new(OsrmConfig::new(
                    base_url.clone(),
                    OsrmDialect::Osrm,
                    TravelMode::Driving,
                ))
                .map(|provider| Arc::new(provider) as Arc<dyn RoutingProvider>),
            );
            "osrm".into()
        }
        RoutingChoice::Ors {
            base_url,
            api_key,
        } => {
            let mut osrm = OsrmConfig::new(
                base_url.clone(),
                OsrmDialect::OpenRouteService,
                TravelMode::Driving,
            );
            osrm.api_key = api_key.clone();
            register_or_declare(
                &registry,
                "osrm",
                OsrmRoutingProvider::new(osrm)
                    .map(|provider| Arc::new(provider) as Arc<dyn RoutingProvider>),
            );
            "osrm".into()
        }
        RoutingChoice::Mapbox {
            access_token,
            profile,
        } => {
            let mut mapbox = MapboxConfig::driving(access_token.clone());
            mapbox.profile = profile.clone();
            register_or_declare(
                &registry,
                "mapbox",
                MapboxRoutingProvider::new(mapbox)
                    .map(|provider| Arc::new(provider) as Arc<dyn RoutingProvider>),
            );
            "mapbox".into()
        }
        RoutingChoice::Contract { id, base_url, token } => {
            let mut contract = ContractConfig::new(
                format!("contract:{id}"),
                format!("{id} routing pack"),
                base_url.clone(),
            );
            contract.token = token.clone();
            let contract_id = contract.id.clone();
            register_or_declare(
                &registry,
                &contract_id,
                ContractRoutingProvider::new(contract)
                    .map(|provider| Arc::new(provider) as Arc<dyn RoutingProvider>),
            );
            contract_id
        }
    };
    registry.set_configured(Some(configured_id));
    Arc::new(registry)
}

fn register_or_declare(
    registry: &RoutingRegistry,
    id: &str,
    provider: Result<Arc<dyn RoutingProvider>, RoutingError>,
) {
    match provider {
        Ok(provider) => registry.register(provider),
        Err(error) => tracing::error!(
            provider = id,
            %error,
            "routing provider failed to build — estimates fall back to the heuristic"
        ),
    }
}

/// Loads the org's folded routing/travel policy.
pub async fn travel_policy_for(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
) -> ApiResult<TravelPolicy> {
    let overrides = core_db::settings::list_org_overrides(&mut *conn, org_id).await?;
    let folded: Vec<(String, serde_json::Value)> = overrides
        .into_iter()
        .map(|row| (row.key, row.value))
        .collect();
    Ok(core_domain::settings::travel_policy(&folded))
}

/// The org's reporting currency (expenses and reimbursement rollups).
pub async fn base_currency(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
) -> ApiResult<String> {
    let org = core_db::orgs::get_org(&mut *conn, org_id).await?;
    Ok(org.base_currency)
}

/// The heuristic provider built from the org's table — the honest
/// NONE-mode engine.
pub fn heuristic_provider(policy: &TravelPolicy) -> ApiResult<HeuristicRoutingProvider> {
    HeuristicRoutingProvider::new(HeuristicConfig::from_settings(
        policy.heuristic_minutes_per_mile,
        policy.heuristic_overhead_minutes,
        policy.heuristic_road_factor_percent,
    ))
    .map_err(|e| ApiError::Internal(anyhow::anyhow!("heuristic config: {e}")))
}

/// Resolves the provider an estimate should use: the configured registry
/// provider, else `None` (callers build the heuristic). The registry's
/// active provider is returned even when its quality is heuristic (a
/// third-party heuristic pack is still the operator's choice).
pub fn active_provider(state: &AppState) -> Option<Arc<dyn RoutingProvider>> {
    state.routing.active()
}

/// The provider a request actually routes through: the registry's active
/// provider when configured, else the org's heuristic instance. Keeps the
/// borrow alive for batched calls.
pub enum ActiveRouting {
    Registry(Arc<dyn RoutingProvider>),
    Heuristic(HeuristicRoutingProvider),
}

impl ActiveRouting {
    #[must_use]
    pub fn provider(&self) -> &dyn RoutingProvider {
        match self {
            ActiveRouting::Registry(provider) => provider.as_ref(),
            ActiveRouting::Heuristic(provider) => provider,
        }
    }
}

pub fn active_routing(state: &AppState, policy: &TravelPolicy) -> ApiResult<ActiveRouting> {
    match active_provider(state) {
        Some(provider) => Ok(ActiveRouting::Registry(provider)),
        None => Ok(ActiveRouting::Heuristic(heuristic_provider(policy)?)),
    }
}

/// Where a journey starts, resolved for one technician-day.
#[derive(Debug, Clone)]
pub struct StartPoint {
    pub place: TravelPlace,
    /// `explicit` (dispatcher set this day) or `previous_day_site`
    /// (auto-suggested continuity).
    pub source: String,
}

impl StartPoint {
    #[must_use]
    pub fn origin(&self) -> TravelOrigin {
        TravelOrigin {
            kind: self.place.kind.clone(),
            label: self.place.label.clone(),
            site_id: self.place.site_id,
            latitude: self.place.latitude,
            longitude: self.place.longitude,
            source: Some(self.source.clone()),
        }
    }
}

/// The destination place of a site, with the coordinates the seam needs.
#[must_use]
pub fn site_place(site: &Site) -> TravelPlace {
    TravelPlace {
        kind: "site".into(),
        label: Some(site.name.clone()),
        latitude: site.latitude,
        longitude: site.longitude,
        site_id: Some(site.id),
    }
}

/// Resolves the day's start point: the explicit dispatcher row first,
/// then the previous day's LAST booked site (the logical continuation of
/// yesterday's route). `None` is a legal, typed answer — the caller
/// surfaces a `missing_start_point` warning instead of guessing.
pub async fn resolve_start_point(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    day: NaiveDate,
) -> ApiResult<Option<StartPoint>> {
    if let Some(row) =
        core_db::routing::get_journey_start_point(&mut *conn, org_id, user_id, day).await?
    {
        let place = match row.kind.as_str() {
            "site" => match row.site_id {
                Some(site_id) => {
                    let site = core_db::clients::get_site(&mut *conn, org_id, site_id).await?;
                    let mut place = site_place(&site);
                    // A dispatcher may pin explicit coordinates over the
                    // site record; those win.
                    if row.latitude.is_some() && row.longitude.is_some() {
                        place.latitude = row.latitude;
                        place.longitude = row.longitude;
                    }
                    place
                }
                None => TravelPlace {
                    kind: "site".into(),
                    label: row.label.clone(),
                    latitude: row.latitude,
                    longitude: row.longitude,
                    site_id: None,
                },
            },
            _ => TravelPlace {
                kind: row.kind.clone(),
                label: row.label.clone(),
                latitude: row.latitude,
                longitude: row.longitude,
                site_id: row.site_id,
            },
        };
        return Ok(Some(StartPoint {
            place,
            source: "explicit".into(),
        }));
    }

    // Auto-suggestion: yesterday's last on-site stop becomes today's
    // start (the van is already there). No yesterday, no sites, or no
    // coordinates — no suggestion; the warning path owns the honesty.
    let previous_day = day - chrono::Duration::days(1);
    let from = previous_day
        .and_hms_opt(0, 0, 0)
        .expect("midnight is valid")
        .and_utc();
    let to = from + chrono::Duration::days(1);
    let appointments = core_db::scheduling::list_appointments_between(
        &mut *conn,
        org_id,
        &[user_id],
        from,
        to,
    )
    .await?;
    let last_site_id = appointments
        .iter()
        .rev()
        .find(|a| a.site_id.is_some())
        .and_then(|a| a.site_id);
    if let Some(site_id) = last_site_id {
        if let Ok(site) = core_db::clients::get_site(&mut *conn, org_id, site_id).await {
            let place = site_place(&site);
            if place.has_coordinates() {
                return Ok(Some(StartPoint {
                    place,
                    source: "previous_day_site".into(),
                }));
            }
        }
    }
    Ok(None)
}

/// A place from a start-point/destination snapshot for the seam.
#[must_use]
pub fn place_from_travel_place(place: &TravelPlace) -> Place {
    match place.coordinates() {
        Some((latitude, longitude)) => Place::at(GeoPoint::new(latitude, longitude))
            .labelled(place.label.clone().unwrap_or_else(|| place.kind.clone())),
        None => Place::address(
            place
                .label
                .clone()
                .unwrap_or_else(|| format!("{} start point", place.kind)),
        )
        .labelled(place.label.clone().unwrap_or_else(|| place.kind.clone())),
    }
}

/// One matrix over an ordered node list (start last, or stops only).
pub async fn matrix_for_nodes(
    provider: &dyn RoutingProvider,
    nodes: &[Place],
    mode: TravelMode,
    depart_at: Option<DateTime<Utc>>,
) -> Result<MatrixResponse, RoutingError> {
    let mut request = MatrixRequest::new(nodes.to_vec(), nodes.to_vec(), mode);
    if let Some(at) = depart_at {
        request = request.departing(at);
    }
    provider.matrix(&request).await
}

/// The typed reason a travel estimate could not be produced.
#[derive(Debug, Clone)]
pub enum TravelSkip {
    NoStartPoint,
    NoDestinationCoordinates,
    Provider(String),
}

impl TravelSkip {
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            TravelSkip::NoStartPoint => "missing_start_point",
            TravelSkip::NoDestinationCoordinates => "missing_coordinates",
            TravelSkip::Provider(reason) => reason,
        }
    }
}

/// Estimates one leg with provenance: configured provider when present,
/// else the org's heuristic table.
pub async fn estimate_leg(
    state: &AppState,
    policy: &TravelPolicy,
    from: &TravelPlace,
    to: &TravelPlace,
    depart_at: DateTime<Utc>,
) -> Result<core_routing::TravelEstimate, TravelSkip> {
    if !from.has_coordinates() || !to.has_coordinates() {
        return Err(TravelSkip::NoDestinationCoordinates);
    }
    let estimate = match active_provider(state) {
        Some(provider) => core_routing::estimate_travel(
            provider.as_ref(),
            &place_from_travel_place(from),
            &place_from_travel_place(to),
            TravelMode::Driving,
            Some(depart_at),
        )
        .await
        .map_err(|e| TravelSkip::Provider(e.label().to_string()))?,
        None => {
            let heuristic = heuristic_provider(policy).map_err(|_| {
                TravelSkip::Provider("provider_unavailable".to_string())
            })?;
            core_routing::estimate_travel(
                &heuristic,
                &place_from_travel_place(from),
                &place_from_travel_place(to),
                TravelMode::Driving,
                Some(depart_at),
            )
            .await
            .map_err(|e| TravelSkip::Provider(e.label().to_string()))?
        }
    };
    Ok(estimate)
}
