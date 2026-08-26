//! Workflow customization policy — explicit per-board status transitions.
//!
//! Purpose: the pure decision core for "may this ticket move from status A
//! to status B on this board?". Boards either run on the semantic-category
//! floor (no explicit transitions configured) or on their own explicit
//! adjacency matrix; behavior always binds to categories, never labels (§5).
//!
//! Non-purpose: no I/O, no permission checks (that is `policy`), no
//! checklist/SLA side rules — those compose AROUND this verdict at the API
//! layer.

use crate::entities::BoardStatus;
use crate::status::TicketStatusCategory;
use uuid::Uuid;

/// One directed edge in a board's explicit transition matrix. Both ids must
/// belong to statuses of the SAME board (enforced here, and by composite FKs
/// in storage).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TransitionEdge {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub from_status_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub to_status_id: Uuid,
}

impl TransitionEdge {
    /// Builds an edge from a pair of status ids.
    pub fn new(from_status_id: Uuid, to_status_id: Uuid) -> Self {
        Self {
            from_status_id,
            to_status_id,
        }
    }
}

/// Every reason a proposed status change can be rejected. Exhaustive: the
/// compiler forces callers (and tests) to handle each case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionRejection {
    /// The source id is not one of the board's statuses (stale client state,
    /// or a status from another org that RLS hid).
    UnknownFromStatus,
    /// The target id is not one of the board's statuses.
    UnknownToStatus,
    /// Both ids are known but belong to different boards.
    CrossBoardTarget,
    /// Source equals target — a no-op, not a lifecycle transition.
    SelfTransition,
    /// The category floor forbids closed → open: reopening requires an
    /// intermediate active/waiting step so closure stays an explicit act.
    ClosedToOpenReopenProhibited,
    /// The board runs an explicit matrix and this edge is not listed in it.
    NotInExplicitMatrix,
}

impl TransitionRejection {
    /// Human-readable denial reason for API errors and audit entries.
    pub fn message(&self) -> String {
        match self {
            Self::UnknownFromStatus => "current status is not defined on this board".into(),
            Self::UnknownToStatus => "target status is not defined on this board".into(),
            Self::CrossBoardTarget => "target status belongs to a different board".into(),
            Self::SelfTransition => "ticket is already in that status".into(),
            Self::ClosedToOpenReopenProhibited => {
                "closed tickets cannot return directly to open; move through \
                 an active or waiting status first"
                    .into()
            }
            Self::NotInExplicitMatrix => {
                "this board's workflow does not allow that status transition".into()
            }
        }
    }
}

/// The semantic-category floor applied when a board has NO explicit
/// transitions: any category may flow to any other EXCEPT closed → open
/// (identity is rejected earlier as `SelfTransition`). Customization adds
/// depth; it never changes this baseline's semantics.
pub const fn default_category_allows(from: TicketStatusCategory, to: TicketStatusCategory) -> bool {
    !(matches!(from, TicketStatusCategory::Closed) && matches!(to, TicketStatusCategory::Open))
}

/// Validates a proposed `from → to` status change for one board.
///
/// `statuses` is the full status set of the ticket's board (org-scoped by
/// the caller); `edges` is the board's explicit adjacency — empty means the
/// semantic floor governs, non-empty means the matrix IS the workflow.
pub fn validate_transition(
    statuses: &[BoardStatus],
    edges: &[TransitionEdge],
    from_status_id: Uuid,
    to_status_id: Uuid,
) -> Result<(), TransitionRejection> {
    let from = statuses
        .iter()
        .find(|s| s.id == from_status_id)
        .ok_or(TransitionRejection::UnknownFromStatus)?;
    let to = statuses
        .iter()
        .find(|s| s.id == to_status_id)
        .ok_or(TransitionRejection::UnknownToStatus)?;

    if from.board_id != to.board_id {
        return Err(TransitionRejection::CrossBoardTarget);
    }
    if from.id == to.id {
        return Err(TransitionRejection::SelfTransition);
    }

    if edges.is_empty() {
        return if default_category_allows(from.category, to.category) {
            Ok(())
        } else {
            Err(TransitionRejection::ClosedToOpenReopenProhibited)
        };
    }

    if edges
        .iter()
        .any(|e| e.from_status_id == from.id && e.to_status_id == to.id)
    {
        Ok(())
    } else {
        Err(TransitionRejection::NotInExplicitMatrix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a status with deterministic fields; only id/board/category
    /// matter to the validator.
    fn status(id: Uuid, board: Uuid, name: &str, category: TicketStatusCategory) -> BoardStatus {
        BoardStatus {
            id,
            board_id: board,
            org_id: Uuid::nil(),
            name: name.into(),
            category,
            sort_order: 0,
            is_entry: false,
            color: None,
        }
    }

    fn fixture() -> (Uuid, Vec<BoardStatus>) {
        let board = Uuid::now_v7();
        let statuses = vec![
            status(Uuid::now_v7(), board, "New", TicketStatusCategory::Open),
            status(
                Uuid::now_v7(),
                board,
                "Working",
                TicketStatusCategory::InProgress,
            ),
            status(
                Uuid::now_v7(),
                board,
                "Blocked",
                TicketStatusCategory::Waiting,
            ),
            status(Uuid::now_v7(), board, "Done", TicketStatusCategory::Closed),
        ];
        (board, statuses)
    }

    // --- transition-matrix unit tests -------------------------------------

    #[test]
    fn floor_allows_every_category_pair_except_closed_to_open() {
        const ALL: [TicketStatusCategory; 4] = [
            TicketStatusCategory::Open,
            TicketStatusCategory::InProgress,
            TicketStatusCategory::Waiting,
            TicketStatusCategory::Closed,
        ];
        for from in ALL {
            for to in ALL {
                assert_eq!(
                    default_category_allows(from, to),
                    !(from == TicketStatusCategory::Closed && to == TicketStatusCategory::Open),
                    "{from:?} → {to:?}"
                );
            }
        }
    }

    #[test]
    fn empty_matrix_accepts_floor_transitions_end_to_end() {
        let (_, statuses) = fixture();
        for from in &statuses {
            for to in &statuses {
                let verdict = validate_transition(&statuses, &[], from.id, to.id);
                match (from.category, to.category) {
                    (TicketStatusCategory::Closed, TicketStatusCategory::Open) => {
                        assert_eq!(
                            verdict,
                            Err(TransitionRejection::ClosedToOpenReopenProhibited)
                        );
                    }
                    _ if from.id == to.id => {
                        assert_eq!(verdict, Err(TransitionRejection::SelfTransition));
                    }
                    _ => assert_eq!(verdict, Ok(()), "{} → {}", from.name, to.name),
                }
            }
        }
    }

    #[test]
    fn explicit_matrix_is_authoritative_and_restrictive() {
        let (_, statuses) = fixture();
        // Only New → Working and Working → Done are legal.
        let edges = vec![
            TransitionEdge::new(statuses[0].id, statuses[1].id),
            TransitionEdge::new(statuses[1].id, statuses[3].id),
        ];
        assert_eq!(
            validate_transition(&statuses, &edges, statuses[0].id, statuses[1].id),
            Ok(())
        );
        assert_eq!(
            validate_transition(&statuses, &edges, statuses[0].id, statuses[2].id),
            Err(TransitionRejection::NotInExplicitMatrix),
            "floor would allow New → Blocked; the matrix must win"
        );
        assert_eq!(
            validate_transition(&statuses, &edges, statuses[3].id, statuses[1].id),
            Err(TransitionRejection::NotInExplicitMatrix),
            "explicit matrices may forbid even what the floor allows"
        );
    }

    #[test]
    fn explicit_matrix_may_permit_closed_to_open_reopen() {
        let (_, statuses) = fixture();
        let edges = vec![TransitionEdge::new(statuses[3].id, statuses[0].id)];
        assert_eq!(
            validate_transition(&statuses, &edges, statuses[3].id, statuses[0].id),
            Ok(()),
            "customization may add depth beyond the floor"
        );
    }

    #[test]
    fn all_invalid_rejections_are_reported() {
        let (_, statuses_a) = fixture();
        let (board_b, _) = fixture();

        let stranger = Uuid::now_v7();
        assert_eq!(
            validate_transition(&statuses_a, &[], stranger, statuses_a[1].id),
            Err(TransitionRejection::UnknownFromStatus)
        );
        assert_eq!(
            validate_transition(&statuses_a, &[], statuses_a[0].id, stranger),
            Err(TransitionRejection::UnknownToStatus)
        );

        let foreign = status(stranger, board_b, "Other", TicketStatusCategory::Open);
        let mut mixed = statuses_a.clone();
        mixed.push(foreign);
        assert_eq!(
            validate_transition(&mixed, &[], statuses_a[0].id, stranger),
            Err(TransitionRejection::CrossBoardTarget)
        );

        assert_eq!(
            validate_transition(&statuses_a, &[], statuses_a[2].id, statuses_a[2].id),
            Err(TransitionRejection::SelfTransition)
        );
    }

    #[test]
    fn rejection_messages_are_nonempty_and_distinct() {
        let all = [
            TransitionRejection::UnknownFromStatus,
            TransitionRejection::UnknownToStatus,
            TransitionRejection::CrossBoardTarget,
            TransitionRejection::SelfTransition,
            TransitionRejection::ClosedToOpenReopenProhibited,
            TransitionRejection::NotInExplicitMatrix,
        ];
        let mut seen = std::collections::HashSet::new();
        for r in all {
            let m = r.message();
            assert!(!m.is_empty());
            assert!(seen.insert(m), "duplicate rejection message");
        }
    }

    // --- property tests over random graphs ---------------------------------

    use proptest::prelude::*;

    proptest! {
        /// For non-empty explicit graphs the validator accepts EXACTLY the
        /// known, same-board, non-identity edges present in the graph —
        /// nothing more, nothing less — regardless of categories.
        #[test]
        fn explicit_graph_accepts_exactly_its_edges(
            n in 2usize..=6,
            seed in any::<u64>(),
            edge_flags in proptest::collection::vec(any::<bool>(), 30),
        ) {
            let mut rng_state = seed;
            let mut next_id = || {
                rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                Uuid::from_u64_pair(rng_state, rng_state.rotate_left(32))
            };
            let board = next_id();
            let statuses: Vec<BoardStatus> = (0..n)
                .map(|i| {
                    let cat = [
                        TicketStatusCategory::Open,
                        TicketStatusCategory::InProgress,
                        TicketStatusCategory::Waiting,
                        TicketStatusCategory::Closed,
                    ][i % 4];
                    status(next_id(), board, &format!("s{i}"), cat)
                })
                .collect();

            let pairs: Vec<(usize, usize)> = (0..n)
                .flat_map(|a| (0..n).filter(move |b| *b != a).map(move |b| (a, b)))
                .collect();
            let mut edges = Vec::new();
            let mut present: Vec<bool> = pairs
                .iter()
                .enumerate()
                .map(|(i, _)| edge_flags[i % edge_flags.len()])
                .collect();
            for (i, &(a, b)) in pairs.iter().enumerate() {
                if present[i] {
                    edges.push(TransitionEdge::new(statuses[a].id, statuses[b].id));
                }
            }
            // Precondition of THIS property: the graph is explicit (non-empty).
            // Force one edge in and track it so expectations stay truthful.
            if edges.is_empty() {
                present[0] = true;
                let (a, b) = pairs[0];
                edges.push(TransitionEdge::new(statuses[a].id, statuses[b].id));
            }

            for (i, &(a, b)) in pairs.iter().enumerate() {
                let expected = if present[i] {
                    Ok(())
                } else {
                    Err(TransitionRejection::NotInExplicitMatrix)
                };
                prop_assert_eq!(
                    validate_transition(&statuses, &edges, statuses[a].id, statuses[b].id),
                    expected
                );
            }
        }

        /// With no explicit graph the validator reduces exactly to the
        /// category floor for every pair of randomly-categorized statuses.
        #[test]
        fn empty_graph_reduces_to_category_floor(
            cats_a in proptest::collection::vec(any::<u8>(), 8),
            cats_b in proptest::collection::vec(any::<u8>(), 8),
        ) {
            let board = Uuid::now_v7();
            let pick = |b: u8| [
                TicketStatusCategory::Open,
                TicketStatusCategory::InProgress,
                TicketStatusCategory::Waiting,
                TicketStatusCategory::Closed,
            ][usize::from(b % 4)];
            // Disjoint id spaces so left/right entries stay distinct rows.
            let build = |raw: &[u8], base: u128| -> Vec<BoardStatus> {
                raw.iter()
                    .enumerate()
                    .map(|(i, b)| {
                        status(
                            Uuid::from_u128(base + i as u128),
                            board,
                            &format!("x{base}-{i}"),
                            pick(*b),
                        )
                    })
                    .collect()
            };
            let left = build(&cats_a, 1_000);
            let right = build(&cats_b, 2_000);
            let mut universe = left.clone();
            universe.extend(right.iter().cloned());

            for a in &left {
                for b in &right {
                    let verdict = validate_transition(&universe, &[], a.id, b.id);
                    prop_assert_eq!(
                        verdict.is_ok(),
                        default_category_allows(a.category, b.category),
                        "{}({:?}) → {}({:?})",
                        a.name,
                        a.category,
                        b.name,
                        b.category
                    );
                }
            }
        }
    }
}
