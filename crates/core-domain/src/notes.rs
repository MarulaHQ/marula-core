//! Structured-notes taxonomy + pre-close checklist gate — pure policy core.
//!
//! Purpose: the single source of truth for (a) which note kinds a viewing
//! audience may see — including the portal v1 tiers where even the client
//! IT-manager tier can NEVER read `internal` — and (b) whether a ticket may
//! close given its required checklist state and the board's enforcement
//! posture. Both decisions are pure functions over injected data, so portal
//! surfaces, staff UI, API routes, and reports all inherit one auditable
//! verdict instead of re-implementing visibility ad hoc.
//!
//! Non-purpose: no persistence, no HTTP, no permission checks against
//! roles (that is `policy`) — callers fetch items/templates and feed them
//! in; no note content handling.

use crate::entities::ChecklistItem;
use crate::status::NoteKind;

/// A surface's note-visibility class. Staff surfaces see the full taxonomy;
/// client-facing surfaces (portal v1, client emails) are hard-limited:
/// plain portal tiers get client-visible notes only, and the portal
/// IT-manager tier additionally gets in-house updates. NO portal audience
/// can ever read `internal` notes — this is the enforcement contract the
/// API layer consumes, not a UI convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoteAudience {
    /// Internal staff surface: internal notes, in-house updates, and
    /// client-visible notes are all readable.
    Staff,
    /// Client-facing surface: ONLY `client_visible` notes are readable.
    /// `in_house_update` is deliberately NOT readable at this tier — it is
    /// a staff-coordination variant reserved for the IT-manager tier below.
    Portal,
    /// Portal IT-manager tier (portal v1): `client_visible` PLUS
    /// `in_house_update`. The widening is deliberate and narrow — in-house
    /// updates are the "here's what we're doing internally" coordination
    /// notes an MSP shares with its client's IT lead; `internal` remains
    /// staff-only for EVERY portal role, without exception.
    PortalItManager,
}

/// The kinds an audience may read, in taxonomy order. Portal returns
/// exactly one kind at the contact tier (two at the IT-manager tier) —
/// this slice IS the enforcement contract consumed by query filters
/// (`core-db` passes it as the kind whitelist).
pub const fn visible_kinds(audience: NoteAudience) -> &'static [NoteKind] {
    match audience {
        NoteAudience::Staff => &[
            NoteKind::Internal,
            NoteKind::ClientVisible,
            NoteKind::InHouseUpdate,
        ],
        NoteAudience::Portal => &[NoteKind::ClientVisible],
        NoteAudience::PortalItManager => &[NoteKind::ClientVisible, NoteKind::InHouseUpdate],
    }
}

/// Whether one specific note may be shown to an audience. Prefer
/// `visible_kinds` for queries; use this for per-row decisions (e.g.
/// filtering already-fetched lists or guarding a single-note fetch).
pub const fn visible_to(kind: NoteKind, audience: NoteAudience) -> bool {
    match audience {
        NoteAudience::Staff => true,
        NoteAudience::Portal => matches!(kind, NoteKind::ClientVisible),
        NoteAudience::PortalItManager => {
            matches!(kind, NoteKind::ClientVisible | NoteKind::InHouseUpdate)
        }
    }
}

/// Why a close attempt was refused. `missing` carries the unchecked
/// REQUIRED item texts so clients can render the exact blockers without a
/// second round-trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseBlocked {
    pub missing: Vec<String>,
}

/// Decides a close attempt under the defaults-first enforcement contract:
///
/// - Enforcement is ACTIVE only when the board has a checklist template AND
///   its `require_checklist_close` toggle is on (`enforcement_active`);
/// - When inactive, closing always proceeds regardless of item state;
/// - When active, closing proceeds only while every REQUIRED instance
///   (template-derived, i.e. `template_item_id` set) is checked. Ad-hoc
///   items never block — they are personal notes, not process gates.
///
/// Items with `template_item_id == None` are ignored by design.
pub fn enforce_pre_close(
    items: &[ChecklistItem],
    enforcement_active: bool,
) -> Result<(), CloseBlocked> {
    if !enforcement_active {
        return Ok(());
    }
    let missing: Vec<String> = items
        .iter()
        .filter(|i| i.template_item_id.is_some() && !i.checked)
        .map(|i| i.text.clone())
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(CloseBlocked { missing })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn item(text: &str, checked: bool, required: bool) -> ChecklistItem {
        ChecklistItem {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            ticket_id: Uuid::now_v7(),
            text: text.into(),
            checked,
            sort_order: 0,
            checked_by: None,
            checked_at: None,
            template_item_id: if required { Some(Uuid::now_v7()) } else { None },
        }
    }

    // --- taxonomy visibility ------------------------------------------------

    #[test]
    fn staff_sees_the_full_taxonomy_and_portal_only_client_visible() {
        assert_eq!(
            visible_kinds(NoteAudience::Staff),
            [
                NoteKind::Internal,
                NoteKind::ClientVisible,
                NoteKind::InHouseUpdate
            ]
        );
        assert_eq!(
            visible_kinds(NoteAudience::Portal),
            [NoteKind::ClientVisible]
        );
    }

    #[test]
    fn portal_it_manager_gets_in_house_updates_but_never_internal() {
        // The v1 IT-manager rule, pinned here: the tier widens visibility
        // by EXACTLY one kind. `internal` is unreachable from any portal
        // audience — if this assertion ever fails, the portal has a leak.
        assert_eq!(
            visible_kinds(NoteAudience::PortalItManager),
            [NoteKind::ClientVisible, NoteKind::InHouseUpdate]
        );
        assert!(!visible_to(
            NoteKind::Internal,
            NoteAudience::PortalItManager
        ));
        assert!(visible_to(
            NoteKind::InHouseUpdate,
            NoteAudience::PortalItManager
        ));
    }

    #[test]
    fn per_row_visibility_matrix_is_exact() {
        const ALL: [NoteKind; 3] = [
            NoteKind::Internal,
            NoteKind::ClientVisible,
            NoteKind::InHouseUpdate,
        ];
        for kind in ALL {
            assert!(visible_to(kind, NoteAudience::Staff), "{kind:?} for staff");
            assert_eq!(
                visible_to(kind, NoteAudience::Portal),
                kind == NoteKind::ClientVisible,
                "{kind:?} for portal"
            );
            assert_eq!(
                visible_to(kind, NoteAudience::PortalItManager),
                matches!(kind, NoteKind::ClientVisible | NoteKind::InHouseUpdate),
                "{kind:?} for portal it_manager"
            );
        }
    }

    // --- pre-close gate -------------------------------------------------------

    #[test]
    fn gate_blocks_exactly_the_unchecked_required_items() {
        let items = vec![
            item("root cause documented", true, true),
            item("client notified", false, true),
            item("invoice draft ready", true, true),
            item("private scratch note", false, false),
        ];
        let blocked = enforce_pre_close(&items, true).expect_err("one required open");
        assert_eq!(blocked.missing, vec!["client notified".to_string()]);

        let all_checked = vec![
            item("root cause documented", true, true),
            item("client notified", true, true),
            item("private scratch note", false, false),
        ];
        assert_eq!(enforce_pre_close(&all_checked, true), Ok(()));
    }

    #[test]
    fn ad_hoc_items_never_block_even_when_enforced() {
        let items = vec![item("remember to smile", false, false)];
        assert_eq!(enforce_pre_close(&items, true), Ok(()));
    }

    #[test]
    fn toggle_off_proceeds_regardless_of_state() {
        let items = vec![item("client notified", false, true)];
        assert_eq!(enforce_pre_close(&items, false), Ok(()));
    }

    #[test]
    fn empty_or_absent_checklist_never_blocks_when_active() {
        assert_eq!(enforce_pre_close(&[], true), Ok(()));
    }

    #[test]
    fn missing_list_reports_every_blocker_in_order() {
        let items = vec![
            item("b", false, true),
            item("a", false, true),
            item("c", true, true),
        ];
        let blocked = enforce_pre_close(&items, true).expect_err("two required open");
        assert_eq!(blocked.missing, vec!["b".to_string(), "a".to_string()]);
    }

    // --- property tests ---------------------------------------------------------

    use proptest::prelude::*;

    proptest! {
        /// For every completion state vector: when enforcement is active the
        /// gate blocks IFF at least one required item is unchecked, and the
        /// missing list equals exactly the unchecked required texts in order;
        /// when inactive it NEVER blocks. Ad-hoc items are invisible to the
        /// verdict in both modes.
        #[test]
        fn verdict_matches_completion_states(
            flags in proptest::collection::vec(any::<bool>(), 0..8usize),
            adhoc_flags in proptest::collection::vec(any::<bool>(), 0..4usize),
            active in any::<bool>(),
        ) {
            let mut items = Vec::new();
            for (i, checked) in flags.iter().enumerate() {
                items.push(item(&format!("req-{i}"), *checked, true));
            }
            for (i, checked) in adhoc_flags.iter().enumerate() {
                items.push(item(&format!("adhoc-{i}"), *checked, false));
            }
            let expected_missing: Vec<String> = flags
                .iter()
                .enumerate()
                .filter(|(_, checked)| !**checked)
                .map(|(i, _)| format!("req-{i}"))
                .collect();

            match enforce_pre_close(&items, active) {
                Ok(()) => prop_assert!(expected_missing.is_empty() || !active),
                Err(CloseBlocked { missing }) => {
                    prop_assert!(active);
                    prop_assert_eq!(missing, expected_missing);
                }
            }
        }
    }
}
