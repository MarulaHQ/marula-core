//! Time-entry billing verdicts (Wave 41 lane 8) — what is odd about one
//! entry, and the one action that clears it.
//!
//! Purpose: the approval queue's P3 row mark ("3 of 12 flagged, each one has a
//! fix in its row") needs two things from the server, and neither of them is
//! a model's opinion: a closed vocabulary of what is wrong, and a *concrete*
//! remedy the approver can apply with one click. Everything here is therefore
//! mechanical and checkable — an entry that overlaps another of the same
//! person's really does overlap, and a description of two words really is not
//! enough to invoice from.
//!
//! The three verdicts, in the order a billing manager cares:
//!
//! - [`TimeVerdictKind::VagueDescription`] — too little written to sign off.
//!   The remedy is a suggested description assembled from facts the server
//!   already has (the ticket title, the work type), never invented prose.
//! - [`TimeVerdictKind::Overlaps`] — this entry's window intersects another of
//!   the same person's, so the same minutes are being claimed twice. The
//!   remedy is the exact overlap, trimmed.
//! - [`TimeVerdictKind::LongerThanUsual`] — this entry is far longer than the
//!   same person's usual entry for the same kind of work. There is no honest
//!   mechanical remedy here, so the fix is to ask the person who was there.
//!
//! Non-purpose: no billable/non-billable judgement. That is the
//! `decision_billing_audit` model's question and it stays where it is, as a
//! probability on the suggestion. A verdict never approves an entry, never
//! changes its state, and never applies itself: the approver is always a
//! person, and [`TimeVerdictFixKind::Ask`] exists precisely because "ask
//! Amara" is a real answer that sends nothing.

use serde::{Deserialize, Serialize};

/// What is wrong with an entry. A closed set: an open string here would put
/// model prose on a money surface, which is exactly what the pack forbids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TimeVerdictKind {
    VagueDescription,
    Overlaps,
    LongerThanUsual,
}

impl TimeVerdictKind {
    pub const ALL: [Self; 3] = [Self::VagueDescription, Self::Overlaps, Self::LongerThanUsual];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::VagueDescription => "vague_description",
            Self::Overlaps => "overlaps",
            Self::LongerThanUsual => "longer_than_usual",
        }
    }
}

/// The one action that clears a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TimeVerdictFixKind {
    /// Replace the description with the server's factual suggestion.
    UseSuggestion,
    /// Cut the overlap off the entry, in whole minutes.
    TrimMinutes,
    /// No mechanical remedy: the person who was there is the only source.
    Ask,
}

impl TimeVerdictFixKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UseSuggestion => "use_suggestion",
            Self::TrimMinutes => "trim_minutes",
            Self::Ask => "ask",
        }
    }
}

/// A remedy, carrying the value it applies so the client never has to
/// recompute it (and cannot apply a different one than the server meant).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimeVerdictFix {
    pub kind: TimeVerdictFixKind,
    /// Button text, in the pack's voice: "Use suggestion", "Trim 30m",
    /// "Ask the tech".
    pub label: String,
    /// Set for [`TimeVerdictFixKind::UseSuggestion`]: the exact description
    /// to write. Assembled from the ticket title and the work type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_description: Option<String>,
    /// Set for [`TimeVerdictFixKind::TrimMinutes`]: whole minutes to cut,
    /// always positive and never more than the entry holds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trim_minutes: Option<i32>,
}

/// One verdict: what is odd, said in words, plus the fix beside it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimeVerdict {
    pub kind: TimeVerdictKind,
    /// The mark's words, typed and short: "Vague description",
    /// "Overlaps #18355 by 30 min", "Longer than usual, 2:00 vs 0:45".
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<TimeVerdictFix>,
}

/// A description shorter than this cannot be invoiced from. Chosen to match
/// the client's own pass rule (`looksFine` in apps/web/lib/money-ui.ts): the
/// server and the client must agree on which entries the human can skip, or
/// the bulk pass and the strip above it tell different stories.
pub const MIN_DESCRIPTION_CHARS: usize = 20;

/// An entry this many times the person's usual length for the same kind of
/// work wants a second look.
pub const LONG_ENTRY_FACTOR: i64 = 3;

/// The floor for "usual", so one short entry yesterday does not make a normal
/// entry today look extraordinary.
pub const MIN_USUAL_MINUTES: i64 = 30;

/// Everything the verdict needs about the entry under review.
#[derive(Debug, Clone)]
pub struct VerdictInput<'a> {
    /// What the person wrote. `None` is a verdict in itself.
    pub notes: Option<&'a str>,
    /// The entry's duration in minutes.
    pub minutes: i64,
    /// The ticket's human number when the entry is ticket-scoped — the
    /// overlap label names the other entry by it.
    pub ticket_number: Option<&'a str>,
    /// The ticket's title, when ticket-scoped. This, not the number, is what
    /// a suggested description is built from: a number is not a sentence.
    pub ticket_title: Option<&'a str>,
    /// The work type's label, for a factual suggested description.
    pub work_type_label: Option<&'a str>,
    /// The same person's other entries, already narrowed to the ones this one
    /// could collide with or be compared against.
    pub peer_minutes: &'a [i64],
    /// The id of the entry this one overlaps, when it does. The overlap in
    /// whole minutes comes back in the fix.
    pub overlap_with_ticket_number: Option<&'a str>,
    pub overlap_minutes: i64,
}

fn hms(minutes: i64) -> String {
    let m = minutes.max(0);
    format!("{}:{:02}", m / 60, m % 60)
}

/// A description worth billing reads like one: more than a label, and not
/// just the work type typed again.
pub fn is_vague(notes: Option<&str>) -> bool {
    match notes {
        None => true,
        Some(text) => text.trim().chars().count() < MIN_DESCRIPTION_CHARS,
    }
}

/// The factual description the server is willing to stand behind. It is
/// assembled, not written: the ticket's own title, then the kind of work, and
/// nothing else. If the parts are missing there is no suggestion — an empty
/// "Use suggestion" is worse than no button.
pub fn suggested_description(
    ticket_title: Option<&str>,
    work_type_label: Option<&str>,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(title) = ticket_title {
        let trimmed = title.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    if let Some(label) = work_type_label {
        let trimmed = label.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join(" — "))
}

/// The overlap between two half-open windows, in whole minutes. None when they
/// merely touch: back-to-back entries are the normal case, not a collision.
pub fn overlap_minutes(
    a_start: chrono::DateTime<chrono::Utc>,
    a_end: chrono::DateTime<chrono::Utc>,
    b_start: chrono::DateTime<chrono::Utc>,
    b_end: chrono::DateTime<chrono::Utc>,
) -> i64 {
    let start = a_start.max(b_start);
    let end = a_end.min(b_end);
    if end <= start {
        return 0;
    }
    (end - start).num_seconds() / 60
}

/// The median of the peer entries, or `None` when there is not enough history
/// to have a "usual" at all.
pub fn usual_minutes(peers: &[i64]) -> Option<i64> {
    let mut sorted: Vec<i64> = peers.iter().copied().filter(|m| *m > 0).collect();
    if sorted.len() < 3 {
        return None;
    }
    sorted.sort_unstable();
    Some(sorted[sorted.len() / 2])
}

/// The verdicts for one entry, most actionable first. A clean entry returns
/// an empty list — the caller then says so out loud rather than inventing a
/// mark.
pub fn verdicts_for(input: &VerdictInput<'_>) -> Vec<TimeVerdict> {
    let mut out: Vec<TimeVerdict> = Vec::new();

    if input.overlap_minutes > 0 {
        let trim = input.overlap_minutes.min(input.minutes).max(0) as i32;
        let other = input
            .overlap_with_ticket_number
            .map(|n| format!("#{n}"))
            .unwrap_or_else(|| "another entry".to_string());
        out.push(TimeVerdict {
            kind: TimeVerdictKind::Overlaps,
            // The pack's own copy: minutes, not a duration. "by 30 min"
            // reads as an amount to cut; "by 0:30" reads as a clock time.
            label: format!("Overlaps {other} by {} min", input.overlap_minutes),
            fix: (trim > 0).then(|| TimeVerdictFix {
                kind: TimeVerdictFixKind::TrimMinutes,
                label: format!("Trim {}m", trim),
                suggested_description: None,
                trim_minutes: Some(trim),
            }),
        });
    }

    if is_vague(input.notes) {
        out.push(TimeVerdict {
            kind: TimeVerdictKind::VagueDescription,
            label: "Vague description".to_string(),
            fix: suggested_description(input.ticket_title, input.work_type_label).map(|text| {
                TimeVerdictFix {
                    kind: TimeVerdictFixKind::UseSuggestion,
                    label: "Use suggestion".to_string(),
                    suggested_description: Some(text),
                    trim_minutes: None,
                }
            }),
        });
    }

    if let Some(usual) = usual_minutes(input.peer_minutes) {
        let floor = (usual * LONG_ENTRY_FACTOR).max(MIN_USUAL_MINUTES);
        if input.minutes > floor {
            out.push(TimeVerdict {
                kind: TimeVerdictKind::LongerThanUsual,
                label: format!("Longer than usual, {} vs {}", hms(input.minutes), hms(usual)),
                // There is no honest mechanical trim here: the person may have
                // done twice the work. The fix is to ask them.
                fix: Some(TimeVerdictFix {
                    kind: TimeVerdictFixKind::Ask,
                    label: "Ask the tech".to_string(),
                    suggested_description: None,
                    trim_minutes: None,
                }),
            });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: u32, minute: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(&format!("2026-09-29T{hour:02}:{minute:02}:00Z"))
            .expect("fixture timestamp")
            .with_timezone(&chrono::Utc)
    }

    #[test]
    fn two_words_is_not_a_description() {
        assert!(is_vague(None));
        assert!(is_vague(Some("Onsite support")));
        assert!(is_vague(Some("  Fixed printer  ")));
        assert!(!is_vague(Some(
            "Replaced the failed drive and confirmed the array rebuilt."
        )));
    }

    #[test]
    fn a_suggestion_is_assembled_from_facts_or_not_offered() {
        assert_eq!(
            suggested_description(Some("No print from the front desk"), Some("On-site")),
            Some("No print from the front desk — On-site".to_string())
        );
        assert_eq!(suggested_description(Some("Only a title"), None), Some("Only a title".into()));
        assert_eq!(suggested_description(None, None), None);
        assert_eq!(suggested_description(Some("   "), None), None);
    }

    #[test]
    fn back_to_back_entries_do_not_overlap() {
        assert_eq!(overlap_minutes(at(9, 0), at(11, 0), at(11, 0), at(12, 0)), 0);
        assert_eq!(overlap_minutes(at(9, 0), at(11, 0), at(10, 30), at(12, 0)), 30);
        assert_eq!(overlap_minutes(at(9, 0), at(11, 0), at(8, 0), at(9, 30)), 30);
        // A nested entry is fully overlapped.
        assert_eq!(overlap_minutes(at(9, 0), at(11, 0), at(9, 30), at(10, 0)), 30);
    }

    #[test]
    fn usual_needs_enough_history_to_mean_anything() {
        assert_eq!(usual_minutes(&[60, 45]), None);
        assert_eq!(usual_minutes(&[90, 60, 30]), Some(60));
        // A zero-minute entry is not an example of anybody's usual work, so it
        // is dropped rather than dragging the median down to nothing.
        assert_eq!(usual_minutes(&[90, 60, 30, 0]), Some(60));
    }

    #[test]
    fn an_overlap_carries_the_exact_minutes_to_cut() {
        let out = verdicts_for(&VerdictInput {
            notes: Some("Replaced the failed drive and confirmed the rebuild."),
            minutes: 120,
            ticket_number: Some("18355"),
            ticket_title: Some("Router swap"),
            work_type_label: Some("On-site"),
            peer_minutes: &[45, 60, 60],
            overlap_with_ticket_number: Some("18355"),
            overlap_minutes: 30,
        });
        assert_eq!(out.len(), 1, "one verdict, and it is the overlap: {out:?}");
        assert_eq!(out[0].kind, TimeVerdictKind::Overlaps);
        assert_eq!(out[0].label, "Overlaps #18355 by 30 min");
        let fix = out[0].fix.as_ref().expect("a trim is offered");
        assert_eq!(fix.kind, TimeVerdictFixKind::TrimMinutes);
        assert_eq!(fix.trim_minutes, Some(30));
        assert_eq!(fix.label, "Trim 30m");
    }

    #[test]
    fn a_vague_entry_offers_the_factual_description() {
        let out = verdicts_for(&VerdictInput {
            notes: Some("Onsite"),
            minutes: 60,
            ticket_number: Some("18310"),
            ticket_title: Some("Printer not printing"),
            work_type_label: Some("On-site"),
            peer_minutes: &[],
            overlap_with_ticket_number: None,
            overlap_minutes: 0,
        });
        let vague = out
            .iter()
            .find(|v| v.kind == TimeVerdictKind::VagueDescription)
            .expect("a vague description is flagged");
        let fix = vague.fix.as_ref().expect("a suggestion is offered");
        assert_eq!(fix.kind, TimeVerdictFixKind::UseSuggestion);
        assert_eq!(
            fix.suggested_description.as_deref(),
            Some("Printer not printing — On-site")
        );
    }

    #[test]
    fn a_vague_entry_with_nothing_to_say_offers_no_button() {
        let out = verdicts_for(&VerdictInput {
            notes: None,
            minutes: 60,
            ticket_number: None,
            ticket_title: None,
            work_type_label: None,
            peer_minutes: &[],
            overlap_with_ticket_number: None,
            overlap_minutes: 0,
        });
        assert_eq!(out.len(), 1);
        assert!(out[0].fix.is_none(), "no parts, no button — never a bare offer");
    }

    #[test]
    fn a_long_entry_asks_rather_than_trims() {
        let out = verdicts_for(&VerdictInput {
            notes: Some("Replaced the failed drive and confirmed the array rebuilt."),
            minutes: 180,
            ticket_number: Some("18355"),
            ticket_title: Some("Router swap"),
            work_type_label: Some("On-site"),
            peer_minutes: &[30, 30, 45, 60],
            overlap_with_ticket_number: None,
            overlap_minutes: 0,
        });
        assert_eq!(out.len(), 1, "{out:?}");
        let long = &out[0];
        assert_eq!(long.kind, TimeVerdictKind::LongerThanUsual);
        assert_eq!(long.label, "Longer than usual, 3:00 vs 0:45");
        let fix = long.fix.as_ref().expect("asking is a fix");
        assert_eq!(fix.kind, TimeVerdictFixKind::Ask);
        assert!(fix.trim_minutes.is_none(), "we do not guess a trim");
    }

    #[test]
    fn a_normal_entry_has_no_verdicts_at_all() {
        let out = verdicts_for(&VerdictInput {
            notes: Some("Replaced the failed drive and confirmed the array rebuilt."),
            minutes: 60,
            ticket_number: Some("18355"),
            ticket_title: Some("Router swap"),
            work_type_label: Some("On-site"),
            peer_minutes: &[45, 60, 90],
            overlap_with_ticket_number: None,
            overlap_minutes: 0,
        });
        assert!(out.is_empty(), "a clean entry gets no mark: {out:?}");
    }

    #[test]
    fn durations_speak_the_clusters_grammar() {
        assert_eq!(hms(0), "0:00");
        assert_eq!(hms(45), "0:45");
        assert_eq!(hms(120), "2:00");
        assert_eq!(hms(1155), "19:15");
    }
}
