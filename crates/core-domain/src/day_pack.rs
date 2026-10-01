//! Day-pack domain (Wave 32 lane 8): the pure decisions behind the tech's
//! `/today` trip-preparation panel.
//!
//! Purpose: the scope/target vocabularies of org dispatch rules, the trip
//! segmentation rule (an office touch re-opens a collect point), conservative
//! checklist-mention matching, and the per-trip coverage math.
//!
//! Non-purpose: DB reads and HTTP views. Nothing here reads stock levels,
//! serials, or the movement ledger — the panel plans, the ledger records.

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Org dispatch-rule scopes. Values are stored verbatim in
/// `dispatch_pack_rules.scope_kind`.
pub const PACK_SCOPE_KINDS: [&str; 3] = ["ticket_type", "ticket_category", "board"];

/// Org dispatch-rule targets: a recommended stock item, or a default kit.
pub const PACK_TARGET_KINDS: [&str; 2] = ["item", "kit"];

/// Line provenance, strongest first. A pick wins over a rule, a rule over a
/// checklist mention when the same item lands on the same ticket twice.
pub const PACK_PROVENANCES: [&str; 3] = ["pick", "rule", "checklist"];

/// `ticket_type` / `ticket_category` / `board`.
#[must_use]
pub fn is_valid_scope_kind(kind: &str) -> bool {
    PACK_SCOPE_KINDS.contains(&kind.trim())
}

/// `item` / `kit`.
#[must_use]
pub fn is_valid_target_kind(kind: &str) -> bool {
    PACK_TARGET_KINDS.contains(&kind.trim())
}

/// Stable identity of one grab line across reads: the panel ticks and
/// untick by this key, and the key never embeds a quantity or a location —
/// those change between reads while the line stays the same.
#[must_use]
pub fn line_key(provenance: &str, id: Uuid) -> String {
    format!("{provenance}:{id}")
}

/// One scheduled stop, trimmed to what trip segmentation needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TripStop {
    pub appointment_id: Uuid,
    /// Origin kind of the leg INTO this stop (`home` | `office` | `site` |
    /// `address`) when the travel model knows it.
    pub leg_from_kind: Option<String>,
}

/// A maximal run of stops the tech serves without returning to the office.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TripSegment {
    /// Index of the segment's first stop in the ordered stop list.
    pub start: usize,
    /// Exclusive index of the segment's last stop.
    pub end: usize,
    /// The segment opens where the tech can re-collect: the office when the
    /// day starts there, or when the recorded leg into the first stop left
    /// the office (a logged office return).
    pub opens_at_office: bool,
}

/// Splits an ordered day into trip segments.
///
/// Index 0 always opens the first trip. A later stop opens a new trip when
/// its recorded leg leaves the office AFTER a stop whose leg did not also
/// leave the office — that is the recorded office return, and the tech can
/// re-collect there. A run of office-origin legs stays one trip pack: the
/// plan estimates every leg from base, which is not a mid-day return.
/// Absent leg data leaves one trip pack, the honest default when nothing
/// says otherwise.
#[must_use]
pub fn trip_segments(start_point_kind: Option<&str>, stops: &[TripStop]) -> Vec<TripSegment> {
    let mut segments: Vec<TripSegment> = Vec::new();
    let mut previous_leg: Option<&str> = None;
    for (index, stop) in stops.iter().enumerate() {
        let leg = stop.leg_from_kind.as_deref();
        let opens = index == 0
            || (leg == Some("office") && previous_leg.is_some() && previous_leg != Some("office"));
        if opens {
            if let Some(previous) = segments.last_mut() {
                previous.end = index;
            }
            let opens_at_office = if index == 0 {
                start_point_kind == Some("office")
            } else {
                true
            };
            segments.push(TripSegment {
                start: index,
                end: stops.len(),
                opens_at_office,
            });
        }
        previous_leg = leg;
    }
    segments
}

/// Required quantity for one item across a trip's stops.
///
/// A tool is a physical unit issued once (one holder at a time), so the max
/// across stops is the requirement. A consumable is used per stop, so the
/// quantities add up. Quantities are integer milli-units.
#[must_use]
pub fn required_milli(is_tool: bool, per_stop: &[i64]) -> i64 {
    if is_tool {
        per_stop.iter().copied().max().unwrap_or(0)
    } else {
        per_stop.iter().sum()
    }
}

/// How much more of an item a trip still needs after ticks. Never negative.
#[must_use]
pub fn coverage_shortfall(required: i64, collected: i64) -> i64 {
    (required - collected).max(0)
}

/// One stock item a checklist line might be talking about.
#[derive(Debug, Clone, Copy)]
pub struct ChecklistCandidate<'a> {
    pub name: &'a str,
    pub sku: Option<&'a str>,
    pub part_number: Option<&'a str>,
}

/// Shortest needle that may match. Below this a "cable" task would match
/// every cable SKU in the catalog, which is worse than no match.
const MIN_NEEDLE_CHARS: usize = 4;

/// The index of the stock item a checklist task mentions, if any.
///
/// Matching is deliberately conservative: a lowercased name (or SKU / part
/// number) of at least four characters must appear inside the lowercased
/// task text; the longest needle wins, ties keep the earlier candidate. No
/// stemming, no synonyms — a wrong guess here puts the wrong part in a van,
/// so a miss simply stays a plain checklist reminder.
#[must_use]
pub fn checklist_match(text: &str, candidates: &[ChecklistCandidate<'_>]) -> Option<usize> {
    let haystack = text.trim().to_lowercase();
    if haystack.is_empty() {
        return None;
    }
    let mut best: Option<(usize, usize)> = None;
    for (index, candidate) in candidates.iter().enumerate() {
        let needles = [Some(candidate.name), candidate.sku, candidate.part_number];
        for needle in needles.into_iter().flatten() {
            let needle = needle.trim().to_lowercase();
            let chars = needle.chars().count();
            if chars < MIN_NEEDLE_CHARS {
                continue;
            }
            if !haystack.contains(&needle) {
                continue;
            }
            match best {
                Some((_, length)) if chars <= length => {}
                _ => best = Some((index, chars)),
            }
        }
    }
    best.map(|(index, _)| index)
}

/// Tomorrow's preview is worth showing when the van does not return to the
/// office first: tomorrow has stops and its resolved start point is not the
/// office. These are the typed answers the panel renders.
pub const TOMORROW_DIRECT_TO_SITE: &str = "direct_to_site";
pub const TOMORROW_OFFICE_START: &str = "office_start";
pub const TOMORROW_NO_STOPS: &str = "no_stops";
pub const TOMORROW_START_UNKNOWN: &str = "start_point_unknown";

/// The reason tomorrow's grab list should (or should not) be prepared
/// tonight. A missing start point answers `start_point_unknown`, never an
/// invented "direct to site".
#[must_use]
pub fn tomorrow_preview_reason(has_stops: bool, start_point_kind: Option<&str>) -> &'static str {
    if !has_stops {
        TOMORROW_NO_STOPS
    } else {
        match start_point_kind {
            Some("office") => TOMORROW_OFFICE_START,
            Some(_) => TOMORROW_DIRECT_TO_SITE,
            None => TOMORROW_START_UNKNOWN,
        }
    }
}

/// Whether the preview renders its list tonight (direct-to-site, or unknown
/// with the caveat). Office start and no-stops stay off.
#[must_use]
pub fn tomorrow_preview_shows(reason: &str) -> bool {
    matches!(reason, TOMORROW_DIRECT_TO_SITE | TOMORROW_START_UNKNOWN)
}

/// Whether a date-time is in the past for the due-date honesty line.
#[must_use]
pub fn is_overdue(due_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    due_at.is_some_and(|due| due < now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop(leg_from_kind: Option<&str>) -> TripStop {
        TripStop {
            appointment_id: Uuid::now_v7(),
            leg_from_kind: leg_from_kind.map(str::to_string),
        }
    }

    #[test]
    fn vocabularies_are_closed_and_trimmed() {
        assert!(is_valid_scope_kind("ticket_type"));
        assert!(is_valid_scope_kind(" board "));
        assert!(!is_valid_scope_kind("status"));
        assert!(is_valid_target_kind("item"));
        assert!(is_valid_target_kind("kit"));
        assert!(!is_valid_target_kind("user"));
        assert_eq!(PACK_PROVENANCES[0], "pick");
    }

    #[test]
    fn an_office_leg_opens_a_new_trip_segment() {
        let stops = vec![
            stop(Some("site")),
            stop(Some("site")),
            stop(Some("office")),
            stop(Some("site")),
        ];
        let segments = trip_segments(Some("home"), &stops);
        assert_eq!(
            segments,
            vec![
                TripSegment {
                    start: 0,
                    end: 2,
                    opens_at_office: false
                },
                TripSegment {
                    start: 2,
                    end: 4,
                    opens_at_office: true
                },
            ]
        );
    }

    #[test]
    fn a_run_of_office_legs_stays_one_trip_pack() {
        let stops = vec![
            stop(Some("office")),
            stop(Some("office")),
            stop(Some("office")),
        ];
        let segments = trip_segments(Some("office"), &stops);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].start, 0);
        assert_eq!(segments[0].end, 3);
        assert!(segments[0].opens_at_office);
    }

    #[test]
    fn a_back_to_back_site_day_is_one_trip_pack() {
        let stops = vec![stop(None), stop(None), stop(Some("site"))];
        let segments = trip_segments(Some("home"), &stops);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].start, 0);
        assert_eq!(segments[0].end, 3);
        assert!(!segments[0].opens_at_office);
    }

    #[test]
    fn an_office_day_start_marks_the_first_segment() {
        let stops = vec![stop(Some("office")), stop(None)];
        let segments = trip_segments(Some("office"), &stops);
        assert_eq!(segments.len(), 1);
        assert!(segments[0].opens_at_office);
    }

    #[test]
    fn no_stops_no_segments() {
        assert!(trip_segments(Some("office"), &[]).is_empty());
    }

    #[test]
    fn tools_require_one_unit_and_consumables_add_up() {
        assert_eq!(required_milli(true, &[1000, 1000, 1000]), 1000);
        assert_eq!(required_milli(true, &[]), 0);
        assert_eq!(required_milli(false, &[1500, 1000]), 2500);
        assert_eq!(coverage_shortfall(2500, 1000), 1500);
        assert_eq!(coverage_shortfall(2500, 4000), 0);
    }

    #[test]
    fn checklist_matching_is_conservative_and_longest_wins() {
        let candidates = [
            ChecklistCandidate {
                name: "Patch cable",
                sku: Some("CBL-1"),
                part_number: None,
            },
            ChecklistCandidate {
                name: "Cable tester",
                sku: None,
                part_number: Some("NET-TEST-9"),
            },
        ];
        assert_eq!(
            checklist_match("Swap the patch cable at the desk", &candidates),
            Some(0)
        );
        assert_eq!(
            checklist_match("Run the cable tester on the run", &candidates),
            Some(1)
        );
        // Ambiguous short words do not guess.
        assert_eq!(checklist_match("Check the cable", &candidates), None);
        assert_eq!(checklist_match("", &candidates), None);
        assert_eq!(checklist_match("Replace the switch", &candidates), None);
    }

    #[test]
    fn tomorrow_preview_reasons_are_typed() {
        assert_eq!(
            tomorrow_preview_reason(false, Some("home")),
            TOMORROW_NO_STOPS
        );
        assert_eq!(
            tomorrow_preview_reason(true, Some("office")),
            TOMORROW_OFFICE_START
        );
        assert_eq!(
            tomorrow_preview_reason(true, Some("site")),
            TOMORROW_DIRECT_TO_SITE
        );
        assert_eq!(
            tomorrow_preview_reason(true, Some("home")),
            TOMORROW_DIRECT_TO_SITE
        );
        assert_eq!(tomorrow_preview_reason(true, None), TOMORROW_START_UNKNOWN);
        assert!(tomorrow_preview_shows(TOMORROW_DIRECT_TO_SITE));
        assert!(tomorrow_preview_shows(TOMORROW_START_UNKNOWN));
        assert!(!tomorrow_preview_shows(TOMORROW_OFFICE_START));
        assert!(!tomorrow_preview_shows(TOMORROW_NO_STOPS));
    }

    #[test]
    fn overdue_is_derived_never_stored() {
        let now = Utc::now();
        assert!(is_overdue(Some(now - chrono::Duration::hours(1)), now));
        assert!(!is_overdue(Some(now + chrono::Duration::hours(1)), now));
        assert!(!is_overdue(None, now));
    }
}
