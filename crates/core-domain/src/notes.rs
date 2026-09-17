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
///   (template-derived, i.e. `template_item_id` set) is checked or marked
///   N-A (Wave 12 lane 6: shared not-applicable state — an item the org
///   deemed not applicable can never block a closure). Ad-hoc
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
        .filter(|i| i.template_item_id.is_some() && !i.checked && !i.na)
        .map(|i| i.text.clone())
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(CloseBlocked { missing })
    }
}

/// Why a close attempt hit the Tasks-gate (docs/notes-tasks-workbar.md §4.1):
/// every item still UNASSIGNED — neither checked nor N-A. The texts ride
/// the typed 409 so any closing surface (raw API callers included) renders
/// the exact outstanding rows without a second round-trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TasksUnassigned {
    pub items: Vec<(crate::ids::ChecklistItemId, String, bool)>,
}

/// The close-gate verdict: a ticket may move to a CLOSED category only when
/// every checklist item has a terminal decision (`checked` or `na`).
/// Deliberately independent of the board's `require_checklist_close`
/// posture — that toggle governs the REQUIRED-item gate; this one is the
/// "no unassigned status at close" rule for ALL items (ad-hoc included).
/// An empty checklist passes vacuously, exactly like `enforce_pre_close`.
pub fn enforce_tasks_assigned(items: &[ChecklistItem]) -> Result<(), TasksUnassigned> {
    let open: Vec<_> = items
        .iter()
        .filter(|i| !i.checked && !i.na)
        .map(|i| {
            (
                i.id,
                i.text.clone(),
                i.template_item_id.is_some(),
            )
        })
        .collect();
    if open.is_empty() {
        Ok(())
    } else {
        Err(TasksUnassigned { items: open })
    }
}

/// The standardized note fragments the server composes for task state
/// transitions (§3.3/§3.4). The lead, the classification suffix, and the
/// idempotency-key tag travel together; `workflow_meta.kind` mirrors
/// `task_note_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskNoteKind {
    Completed,
    Na,
    Reopen,
    CloseAll,
}

impl TaskNoteKind {
    /// `workflow_meta.kind` value.
    pub const fn as_str(self) -> &'static str {
        match self {
            TaskNoteKind::Completed => "task_complete",
            TaskNoteKind::Na => "task_na",
            TaskNoteKind::Reopen => "task_reopen",
            TaskNoteKind::CloseAll => "task_closeall",
        }
    }

    /// The idempotency-key tag for the append claim
    /// (`task-{tag}:{ticket}:{item[:transition]}`-shaped, §3.3).
    pub const fn key_tag(self) -> &'static str {
        match self {
            TaskNoteKind::Completed => "task-complete",
            TaskNoteKind::Na => "task-na",
            TaskNoteKind::Reopen => "task-reopen",
            TaskNoteKind::CloseAll => "task-closeall",
        }
    }
}

/// Composes the auto-append body THROUGH the standard formatter, so a
/// workflow line reads exactly like a tech-typed ledger line (§3.3):
/// bullet + capitalized, bold lead. E.g.
/// `- **Completed:** Fix DNS forwarding`.
pub fn compose_task_note(kind: TaskNoteKind, detail: &str) -> String {
    let lead = match kind {
        TaskNoteKind::Completed => "Completed",
        TaskNoteKind::Na => "N/A",
        TaskNoteKind::Reopen => "Reopened",
        TaskNoteKind::CloseAll => "Completed",
    };
    // B4 for the detail part: the bold lead makes the composed line a
    // markdown-lead line, which the formatter's capitalization pass skips
    // verbatim — so compose_task_note capitalizes the detail's first letter
    // itself (same first-letter rule; acronyms never downcased).
    let detail = match detail.find(|c: char| c.is_alphabetic()) {
        Some(idx) => {
            let mut cs = detail[idx..].chars();
            let first = cs.next().expect("idx found");
            format!(
                "{}{}{}",
                &detail[..idx],
                first.to_uppercase(),
                cs.as_str()
            )
        }
        None => detail.to_string(),
    };
    format_note_body(&format!("**{lead}:** {detail}"))
}

/// The shared note-body formatter (docs/notes-tasks-workbar.md §2.4) —
/// ONE implementation mirrored by the web client (`formatNoteBody`). Pure
/// function over the draft; idempotent by construction. Rules in order:
///
/// - B1 — trim leading/trailing whitespace lines; collapse 3+ consecutive
///   blank lines to 1;
/// - B2 — every non-empty line becomes a canonical `- ` bullet; user-typed
///   `- * • –` markers are normalized into it; ordered numerals (`1.`, `1)`)
///   are PRESERVED as typed;
/// - B3 — up to 3 spaces of intentional indentation survive under the bullet
///   (sub-bullets);
/// - B4 — sentence capitalization: the line's first CHARACTER is uppercased,
///   nothing else touched (acronyms/proper nouns stay safe);
/// - L — links stay raw verbatim (L1/L2 pass through; L3: a bare
///   protocol-less URL line gets `https://` prepended, deterministic);
/// - B6 — `format(format(x)) == format(x)`.
///
/// The server composes auto-appends through this fn and stores plain
/// markdown; the client applies it to drafts at submit. No terminal
/// punctuation is ever added (B5).
pub fn format_note_body(draft: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for raw in draft.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue; // B1: blank lines never survive as bullets
        }
        // B3: up to 3 leading spaces of intentional indentation survive;
        // 4+ is flow text — indent resets to zero, whitespace goes.
        let leading = raw.len() - raw.trim_start_matches(' ').len();
        let (indent, stripped) = if leading > 3 {
            (0, raw.trim_start().to_string())
        } else {
            (leading, raw.to_string())
        };
        let stripped = stripped.as_str();
        // B2/B3: relative to the CONTENT position (after the preserved
        // indent) — the marker walk and trim operate on the trimmed line
        // so a restore-pass can never double-print a bullet.
        let content = stripped.trim_start().trim_start();
        let marker_stripped = content
            .strip_prefix("- ")
            .or_else(|| content.strip_prefix("* "))
            .or_else(|| content.strip_prefix("• "))
            .or_else(|| content.strip_prefix("– "))
            .unwrap_or(content);
        // B2 exception: ordered numerals are PRESERVED as typed (the tech
        // chose a numbered list; do not re-bullet it). "1." / "12)" forms.
        let numerable = ordered_numeral_start(marker_stripped);
        // L3: a line consisting SOLELY of a bare host URL gets https://.
        let text = if !numerable && !marker_stripped.contains(' ') && looks_like_bare_host(marker_stripped)
        {
            format!("https://{marker_stripped}")
        } else {
            marker_stripped.to_string()
        };
        // B4: sentence capitalization — the first ALPHABETIC character is
        // uppercased, everything else untouched (digits/acronyms safe).
        // Verbatim exceptions: raw URLs (L1), markdown links (L2), and
        // markdown bold leads — capitalizing those would corrupt the
        // token or mis-case a URL scheme.
        let skip_cap = text.starts_with("http://")
            || text.starts_with("https://")
            || text.starts_with('[')
            || text.starts_with("**");
        let text = if skip_cap {
            text
        } else {
            match text.find(|c: char| c.is_alphabetic()) {
                Some(idx) => {
                    let (head, tail) = text.split_at(idx);
                    let mut cs = tail.chars();
                    let first = cs.next().expect("idx found");
                    format!("{head}{}{}", first.to_uppercase(), cs.as_str())
                }
                None => text,
            }
        };
        if numerable {
            lines.push(format!("{}{}", " ".repeat(indent), text));
        } else {
            lines.push(format!("{}- {}", " ".repeat(indent), text));
        }
    }
    lines.join("\n")
}

fn ordered_numeral_start(line: &str) -> bool {
    let digits: String = line.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return false;
    }
    let rest = &line[digits.len()..];
    rest.starts_with(". ") || rest.starts_with(") ")
}

fn looks_like_bare_host(line: &str) -> bool {
    // Deliberately narrow: a dot in the first whitespace-free token,
    // bracketless (markdown link syntax never qualifies), no leading/
    // trailing dot, and no explicit scheme (those pass through verbatim).
    if line.starts_with('[') {
        return false;
    }
    let head = line.split([' ', '/']).next().unwrap_or("");
    head.contains('.')
        && !head.starts_with('.')
        && !head.ends_with('.')
        && !line.starts_with("http://")
        && !line.starts_with("https://")
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
            na: false,
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
    fn na_required_items_never_block_and_missing_na_defaults_false() {
        // N-A REQUIRED items are exempt: the shared not-applicable state is
        // close-gate truth, not a per-user view.
        let mut na = item("legacy on-prem check", false, true);
        na.na = true;
        assert_eq!(enforce_pre_close(&[na], true), Ok(()));

        // Backward compatibility: a payload without the `na` key (older
        // goldens, older emitters) deserializes as false and keeps blocking
        // exactly as before.
        let legacy: ChecklistItem = serde_json::from_value(serde_json::json!({
            "id": Uuid::now_v7(),
            "org_id": Uuid::now_v7(),
            "ticket_id": Uuid::now_v7(),
            "text": "client notified",
            "checked": false,
            "sort_order": 0,
            "checked_by": null,
            "checked_at": null,
            "template_item_id": Uuid::now_v7(),
        }))
        .expect("legacy payload without na deserializes");
        assert!(!legacy.na);
        assert!(enforce_pre_close(&[legacy], true).is_err());
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

    // --- format_note_body (docs/notes-tasks-workbar.md §2.4) ----------------

    fn fmt(s: &str) -> String {
        format_note_body(s)
    }

    #[test]
    fn bullets_every_line_canonically() {
        assert_eq!(fmt("fixed printer\ndiagnosed cable"), "- Fixed printer\n- Diagnosed cable");
    }

    #[test]
    fn normalizes_typed_markers() {
        assert_eq!(fmt("- a\n* b\n• c\n– d"), "- A\n- B\n- C\n- D");
    }

    #[test]
    fn preserves_ordered_numerals() {
        assert_eq!(fmt("1. boot scan\n2) run diags"), "1. Boot scan\n2) Run diags");
    }

    #[test]
    fn keeps_up_to_three_indent_spaces() {
        assert_eq!(fmt("  - sub item"), "  - Sub item");
        assert_eq!(fmt("    four spaces"), "- Four spaces");
    }

    #[test]
    fn capitalizes_first_char_only() {
        assert_eq!(fmt("dns forwarded after TTL refresh"), "- Dns forwarded after TTL refresh");
        assert_eq!(fmt("IBM drive replaced"), "- IBM drive replaced");
    }

    #[test]
    fn no_terminal_punctuation_added() {
        assert_eq!(fmt("works"), "- Works");
    }

    #[test]
    fn blank_lines_collapse_and_trim() {
        assert_eq!(fmt("\n\n\na\n\n\n\n\nb\n\n"), "- A\n- B");
    }

    #[test]
    fn empty_draft_formats_to_empty() {
        assert_eq!(fmt(""), "");
        assert_eq!(fmt("  \n \t "), "");
    }

    #[test]
    fn links_pass_verbatim_l1_l2() {
        // L1: full URLs with a scheme stay untouched by L3 handling.
        assert_eq!(fmt("https://example.com/x?y=1 &z"), "- https://example.com/x?y=1 &z");
        // L2: markdown link syntax passes through untouched.
        assert_eq!(fmt("[docs](https://example.com/docs)"), "- [docs](https://example.com/docs)");
    }

    #[test]
    fn bare_host_line_gets_https_prefix_l3() {
        assert_eq!(fmt("www.example.com/status"), "- https://www.example.com/status");
        assert_eq!(fmt("example.com/docs"), "- https://example.com/docs");
        // Not a URL line: has spaces or is markdown -> untouched.
        assert_eq!(fmt("example.com hosted here"), "- Example.com hosted here");
    }

    #[test]
    fn compose_task_note_bulleted_bold_lead() {
        assert_eq!(
            compose_task_note(TaskNoteKind::Completed, "Fix DNS forwarding"),
            "- **Completed:** Fix DNS forwarding"
        );
        assert_eq!(
            compose_task_note(TaskNoteKind::Na, "Customer already resolved via vendor"),
            "- **N/A:** Customer already resolved via vendor"
        );
        assert_eq!(TaskNoteKind::Completed.key_tag(), "task-complete");
        assert_eq!(TaskNoteKind::CloseAll.as_str(), "task_closeall");
    }

    #[test]
    fn format_is_idempotent() {
        let samples = [
            "- a\n* b\n1. c\nhttps://example.com\n\n\n\nd",
            "**Completed:** Fix DNS forwarding",
            "•  mixed   bullets\n  - sub\n2) second",
            "",
            "\n\n\nsingle",
        ];
        for s in samples {
            let once = fmt(s);
            assert_eq!(fmt(&once), once, "not idempotent for {s:?}");
        }
    }

    use proptest::prelude::*;

    proptest! {
        /// Idempotence property for every generated draft.
        #[test]
        fn format_idempotent_for_all_drafts(parts in proptest::collection::vec("[A-Za-z0-9 .,/\\-*•–:]{0,24}", 0..8usize)) {
            let draft = parts.join("\n");
            let once = format_note_body(&draft);
            prop_assert_eq!(format_note_body(&once), once, "not idempotent for {:?}", draft);
        }
    }

    proptest! {
        /// L1/L2 invariants: explicit-scheme URLs and markdown links survive
        /// verbatim inside a bulleted line (capitalization of the URL's first
        /// letter is deterministic and idempotent-consistent for 'H').
        #[test]
        fn urls_survive(scheme in proptest::sample::select(vec!["http", "https"]), host in "[a-z]{3,10}", path in "[a-z0-9/]{0,12}") {
            let draft = format!("{0}://{1}.example.com/{2}", scheme, host, path);
            let expected = format!("{0}://{1}.example.com/{2}", scheme, host, path);
            let once = format_note_body(&draft);
            prop_assert!(once.contains(&expected));
            prop_assert_eq!(format_note_body(&once), once);
        }
    }

    // --- close gate ---------------------------------------------------------

    use uuid::Uuid as Id;

    fn task(text: &str, checked: bool, na: bool) -> ChecklistItem {
        ChecklistItem {
            id: Id::now_v7(),
            org_id: Id::nil(),
            ticket_id: Id::nil(),
            text: text.to_string(),
            checked,
            na,
            sort_order: 0,
            checked_by: None,
            checked_at: None,
            template_item_id: None,
        }
    }

    #[test]
    fn tasks_gate_blocks_only_unassigned() {
        let items = vec![task("done one", true, false), task("n-a one", false, true), task("still open", false, false)];
        let blocked = enforce_tasks_assigned(&items).expect_err("one open");
        assert_eq!(blocked.items.len(), 1);
        assert_eq!(blocked.items[0].1, "still open");
    }

    #[test]
    fn tasks_gate_vacuous_pass_on_empty_or_all_decided() {
        assert_eq!(enforce_tasks_assigned(&[]), Ok(()));
        let items = vec![task("a", true, false), task("b", false, true)];
        assert_eq!(enforce_tasks_assigned(&items), Ok(()));
    }

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
