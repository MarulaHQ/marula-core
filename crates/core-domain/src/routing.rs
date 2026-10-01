//! Routing-aware scheduling policy (Wave 32 lane 7): the pure half of
//! automatic travel — journey warnings and suggestion math.
//!
//! Purpose: turn a technician's ordered stops plus the provider's leg
//! estimates into typed **impossible-travel warnings** (surfaced at
//! assign/edit time) and deterministic **route suggestions** (swap /
//! reassign, suggestion-grade, emitted only when a measured provider is
//! active). No I/O: the caller resolves places and leg minutes.
//!
//! Doctrine: travel ADDS to the booked appointment (never replaces it)
//! and occupies capacity — see `crate::scheduling::occupied_minutes`.
//! Warnings are advisory; the DB EXCLUDE constraint and the dispatcher
//! stay the authority.
//!
//! Non-purpose: provider calls (core-api does those), persistence, and
//! auto-applying a suggestion (a human promotes one).

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A destination or start-point snapshot carried with a stop.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TravelPlace {
    /// `site` | `home` | `office` | `address`.
    pub kind: String,
    pub label: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub site_id: Option<Uuid>,
}

impl TravelPlace {
    #[must_use]
    pub fn has_coordinates(&self) -> bool {
        self.latitude.is_some() && self.longitude.is_some()
    }

    #[must_use]
    pub fn coordinates(&self) -> Option<(f64, f64)> {
        Some((self.latitude?, self.longitude?))
    }
}

/// One stop in a technician's day, in scheduled order.
#[derive(Debug, Clone, PartialEq)]
pub struct TravelStop {
    pub appointment_id: Uuid,
    /// Short human label (ticket number / title) used in warning and
    /// suggestion copy.
    pub label: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub travel_minutes: i64,
    pub travel_provider: Option<String>,
    pub travel_quality: Option<String>,
    /// Where the journey arrives (the appointment's site).
    pub place: Option<TravelPlace>,
}

/// The closed warning vocabulary. UI renders these; tests pin them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TravelWarningKind {
    /// No start point could be resolved for the first leg.
    MissingStartPoint,
    /// A stop's destination has no coordinates — travel unknown.
    MissingCoordinates,
    /// The previous booking ends when (or after) this one starts, yet
    /// travel is required between them.
    ZeroTransfer,
    /// The gap between bookings is shorter than the required travel —
    /// the pair is impossible as scheduled ("close on the map, too far
    /// in the clock").
    InsufficientTransfer,
    /// The provider failed for a leg; the reason is typed.
    ProviderError,
}

impl TravelWarningKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            TravelWarningKind::MissingStartPoint => "missing_start_point",
            TravelWarningKind::MissingCoordinates => "missing_coordinates",
            TravelWarningKind::ZeroTransfer => "zero_transfer",
            TravelWarningKind::InsufficientTransfer => "insufficient_transfer",
            TravelWarningKind::ProviderError => "provider_error",
        }
    }
}

/// One advisory warning about a journey.
#[derive(Debug, Clone, PartialEq)]
pub struct TravelWarning {
    pub kind: TravelWarningKind,
    /// The stop the warning concerns; `None` for a journey-level warning
    /// (missing start point).
    pub appointment_id: Option<Uuid>,
    /// Short human copy (UI shows it verbatim).
    pub message: String,
    pub required_minutes: Option<i64>,
    pub gap_minutes: Option<i64>,
    /// How many minutes short the transfer is (`required - gap`).
    pub short_minutes: Option<i64>,
    /// Typed provider failure label when `kind == ProviderError`.
    pub provider_reason: Option<String>,
}

impl TravelWarning {
    fn new(kind: TravelWarningKind, appointment_id: Option<Uuid>, message: String) -> Self {
        Self {
            kind,
            appointment_id,
            message,
            required_minutes: None,
            gap_minutes: None,
            short_minutes: None,
            provider_reason: None,
        }
    }

    #[must_use]
    pub fn is_blocking(&self) -> bool {
        matches!(
            self.kind,
            TravelWarningKind::ZeroTransfer | TravelWarningKind::InsufficientTransfer
        )
    }
}

/// Everything the warning pass needs. `leg_minutes[i]` is the travel time
/// INTO stop `i` (index 0 = start point → first stop); `leg_errors[i]`
/// carries the typed provider reason when the estimate failed.
#[derive(Debug, Clone)]
pub struct JourneyInput<'a> {
    pub stops: &'a [TravelStop],
    pub leg_minutes: &'a [Option<i64>],
    pub leg_errors: &'a [Option<String>],
    pub has_start_point: bool,
}

/// Computes the journey's warnings in scheduled order. Pure and
/// deterministic — the same inputs always produce the same list.
#[must_use]
pub fn journey_warnings(input: &JourneyInput<'_>) -> Vec<TravelWarning> {
    let mut warnings = Vec::new();
    if input.stops.is_empty() {
        return warnings;
    }
    if !input.has_start_point {
        warnings.push(TravelWarning::new(
            TravelWarningKind::MissingStartPoint,
            None,
            "No journey start point for this day — set one to estimate the first leg.".into(),
        ));
    }
    for (index, stop) in input.stops.iter().enumerate() {
        let unresolved = stop
            .place
            .as_ref()
            .map(|p| !p.has_coordinates())
            .unwrap_or(true);
        if unresolved {
            warnings.push(TravelWarning::new(
                TravelWarningKind::MissingCoordinates,
                Some(stop.appointment_id),
                "This stop has no coordinates, so travel into it cannot be estimated.".into(),
            ));
        }
        if let Some(Some(reason)) = input.leg_errors.get(index) {
            let mut warning = TravelWarning::new(
                TravelWarningKind::ProviderError,
                Some(stop.appointment_id),
                "The travel estimate for this stop failed; the provider reason is recorded.".into(),
            );
            warning.provider_reason = Some(reason.clone());
            warnings.push(warning);
        }
        // Transfer feasibility applies from the SECOND stop on: the first
        // stop's travel lives before the day's first booking and has no
        // predecessor to squeeze against.
        if index == 0 {
            continue;
        }
        let Some(required) = input.leg_minutes.get(index).copied().flatten() else {
            continue;
        };
        if required <= 0 {
            continue;
        }
        let previous = &input.stops[index - 1];
        let gap_minutes = (stop.starts_at - previous.ends_at).num_minutes();
        if gap_minutes <= 0 {
            let mut warning = TravelWarning::new(
                TravelWarningKind::ZeroTransfer,
                Some(stop.appointment_id),
                "Back-to-back bookings with no transfer time for a route that needs some.".into(),
            );
            warning.required_minutes = Some(required);
            warning.gap_minutes = Some(gap_minutes);
            warning.short_minutes = Some(required - gap_minutes);
            warnings.push(warning);
        } else if gap_minutes < required {
            let mut warning = TravelWarning::new(
                TravelWarningKind::InsufficientTransfer,
                Some(stop.appointment_id),
                "The gap between these bookings is shorter than the required travel.".into(),
            );
            warning.required_minutes = Some(required);
            warning.gap_minutes = Some(gap_minutes);
            warning.short_minutes = Some(required - gap_minutes);
            warnings.push(warning);
        }
    }
    warnings
}

/// The closed suggestion vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionKind {
    /// Swap two adjacent stops to shorten the drive.
    SwapAdjacent,
    /// Move a stop to another technician whose day is closer.
    Reassign,
}

impl SuggestionKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            SuggestionKind::SwapAdjacent => "swap_adjacent",
            SuggestionKind::Reassign => "reassign",
        }
    }
}

/// One suggestion-grade route improvement. Nothing here auto-applies —
/// the dispatcher promotes a suggestion on the board.
#[derive(Debug, Clone, PartialEq)]
pub struct TravelSuggestion {
    pub kind: SuggestionKind,
    pub appointment_id: Uuid,
    pub swap_with_appointment_id: Option<Uuid>,
    pub target_user_id: Option<Uuid>,
    pub minutes_saved: i64,
    pub label: String,
}

/// Finds adjacent-stop swaps that save travel time.
///
/// Node indexing: stops are `0..stops.len()`, and node
/// `stops.len()` is the journey start. `lookup(a, b)` returns travel
/// minutes from node `a` to node `b` (`None` when the matrix cannot
/// answer — a suggestion is never built on a hole). Only swaps that save
/// at least `min_savings` minutes are emitted, in scan order, so the
/// output is deterministic.
#[must_use]
pub fn adjacent_swap_suggestions(
    stops: &[TravelStop],
    lookup: impl Fn(usize, usize) -> Option<i64>,
    min_savings: i64,
) -> Vec<TravelSuggestion> {
    let count = stops.len();
    if count < 2 {
        return Vec::new();
    }
    let start_node = count;
    let threshold = min_savings.max(1);
    let mut suggestions = Vec::new();
    for i in 0..count - 1 {
        let a = i;
        let b = i + 1;
        let previous = if i == 0 { start_node } else { i - 1 };
        let next = if b + 1 < count { Some(b + 1) } else { None };

        let Some(current_in) = lookup(previous, a) else {
            continue;
        };
        let Some(current_mid) = lookup(a, b) else {
            continue;
        };
        let Some(swapped_in) = lookup(previous, b) else {
            continue;
        };
        let Some(swapped_mid) = lookup(b, a) else {
            continue;
        };
        // The leg OUT of the swapped pair: zero when the pair ends the day.
        let current_out = match next {
            Some(c) => match lookup(b, c) {
                Some(value) => value,
                None => continue,
            },
            None => 0,
        };
        let swapped_out = match next {
            Some(c) => match lookup(a, c) {
                Some(value) => value,
                None => continue,
            },
            None => 0,
        };
        let current_total = current_in + current_mid + current_out;
        let swapped_total = swapped_in + swapped_mid + swapped_out;
        let savings = current_total - swapped_total;
        if savings < threshold {
            continue;
        }
        let a_label = stops[a]
            .label
            .clone()
            .unwrap_or_else(|| stops[a].appointment_id.to_string());
        let b_label = stops[b]
            .label
            .clone()
            .unwrap_or_else(|| stops[b].appointment_id.to_string());
        suggestions.push(TravelSuggestion {
            kind: SuggestionKind::SwapAdjacent,
            appointment_id: stops[a].appointment_id,
            swap_with_appointment_id: Some(stops[b].appointment_id),
            target_user_id: None,
            minutes_saved: savings,
            label: format!("Swap {a_label} and {b_label} — saves ~{savings} min of travel."),
        });
    }
    suggestions
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 18, hour, minute, 0).unwrap()
    }

    fn stop(id: u8, start: DateTime<Utc>, end: DateTime<Utc>) -> TravelStop {
        TravelStop {
            appointment_id: Uuid::from_u128(id as u128),
            label: Some(format!("T-{id}")),
            starts_at: start,
            ends_at: end,
            travel_minutes: 0,
            travel_provider: None,
            travel_quality: None,
            place: Some(TravelPlace {
                kind: "site".into(),
                label: Some(format!("Site {id}")),
                latitude: Some(52.5),
                longitude: Some(13.4),
                site_id: None,
            }),
        }
    }

    #[test]
    fn zero_and_insufficient_transfers_are_typed_with_numbers() {
        let stops = vec![
            stop(1, at(9, 0), at(10, 0)),
            stop(2, at(10, 0), at(11, 0)),
            stop(3, at(11, 20), at(12, 0)),
        ];
        let legs = vec![Some(15), Some(25), Some(45)];
        let warnings = journey_warnings(&JourneyInput {
            stops: &stops,
            leg_minutes: &legs,
            leg_errors: &[None, None, None],
            has_start_point: true,
        });
        assert_eq!(warnings.len(), 2);
        assert_eq!(warnings[0].kind, TravelWarningKind::ZeroTransfer);
        assert_eq!(warnings[0].required_minutes, Some(25));
        assert_eq!(warnings[0].gap_minutes, Some(0));
        assert_eq!(warnings[0].short_minutes, Some(25));
        assert!(warnings[0].is_blocking());
        assert_eq!(warnings[1].kind, TravelWarningKind::InsufficientTransfer);
        assert_eq!(warnings[1].gap_minutes, Some(20));
        assert_eq!(warnings[1].short_minutes, Some(25));
    }

    #[test]
    fn missing_start_and_missing_coordinates_are_reported() {
        let mut stops = vec![stop(1, at(9, 0), at(10, 0))];
        stops[0].place = None;
        let warnings = journey_warnings(&JourneyInput {
            stops: &stops,
            leg_minutes: &[None],
            leg_errors: &[Some("provider_unavailable".into())],
            has_start_point: false,
        });
        let kinds: Vec<TravelWarningKind> = warnings.iter().map(|w| w.kind).collect();
        assert!(kinds.contains(&TravelWarningKind::MissingStartPoint));
        assert!(kinds.contains(&TravelWarningKind::MissingCoordinates));
        assert!(kinds.contains(&TravelWarningKind::ProviderError));
        assert_eq!(
            warnings
                .iter()
                .find(|w| w.kind == TravelWarningKind::ProviderError)
                .unwrap()
                .provider_reason
                .as_deref(),
            Some("provider_unavailable")
        );
    }

    #[test]
    fn adjacent_swap_suggestions_use_the_matrix_and_gate_on_savings() {
        // Stops A(0), B(1), C(2); start node 3.
        let stops = vec![
            stop(1, at(9, 0), at(10, 0)),
            stop(2, at(10, 30), at(11, 30)),
            stop(3, at(12, 30), at(13, 30)),
        ];
        let lookup = |a: usize, b: usize| -> Option<i64> {
            match (a, b) {
                (3, 0) => Some(10), // start -> A
                (0, 1) => Some(5),  // A -> B
                (1, 2) => Some(40), // B -> C
                (3, 1) => Some(5),  // start -> B
                (1, 0) => Some(5),  // B -> A
                (0, 2) => Some(10), // A -> C
                (2, 1) => Some(10), // C -> B
                _ => None,
            }
        };
        let suggestions = adjacent_swap_suggestions(&stops, lookup, 5);
        assert_eq!(suggestions.len(), 2);
        assert_eq!(suggestions[0].kind, SuggestionKind::SwapAdjacent);
        // Swap A/B: (10+5+40) - (5+5+10) = 35.
        assert_eq!(suggestions[0].minutes_saved, 35);
        assert_eq!(suggestions[0].appointment_id, stops[0].appointment_id);
        assert_eq!(
            suggestions[0].swap_with_appointment_id,
            Some(stops[1].appointment_id)
        );
        // Swap B/C: (5+40) - (10+10) = 25.
        assert_eq!(suggestions[1].minutes_saved, 25);

        // A hole in the matrix suppresses the suggestion instead of
        // guessing, and a high threshold suppresses marginal swaps.
        let holed = |a: usize, b: usize| -> Option<i64> {
            if (a, b) == (0, 2) {
                None
            } else {
                lookup(a, b)
            }
        };
        assert!(adjacent_swap_suggestions(&stops, holed, 5).is_empty());
        assert!(adjacent_swap_suggestions(&stops, lookup, 40).is_empty());
    }
}
