//! core-documents — DocumentVersion semantics (TASKLIST D4,
//! docs/10-agent-platform.md §6).
//!
//! Purpose: the ONE home for the versioning policy that makes documents
//! immutable, append-only, diffable, and restorable:
//!
//! * [`next_version`] — gap-free 1-based numbering;
//! * [`RestorePlan`] — validates a restore request against the version
//!   list and produces the NEW version row it must create (restores never
//!   rewrite or delete history — the binding §6 rule);
//! * [`validate_version_content`] — shape gates applied to every version
//!   before it is appended;
//! * [`diff_bodies`] — a conservative line-level diff summary clients can
//!   render for any version pair.
//!
//! Non-purpose: no SQL (core-db), no HTTP (core-api). Append-only
//! enforcement is app-layer by convention (audit_events posture): this
//! crate exposes append-shaped operations only; no API here mutates an
//! existing version.

use core_domain::entities::DocumentVersion;

/// Upper bound for document titles.
pub const MAX_TITLE_CHARS: usize = 300;
/// Upper bound for version bodies (1 MiB — documents run longer than KB
/// articles but stay bounded for FTS and the request path).
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Upper bound for change summaries.
pub const MAX_SUMMARY_CHARS: usize = 500;
/// The diff works line-by-line; beyond this many lines per side the
/// summary truncates rather than burn O(n·m) LCS memory.
pub const MAX_DIFF_LINES: usize = 5_000;

#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone)]
pub enum DocumentError {
    #[error("title is required")]
    EmptyTitle,
    #[error("title exceeds {MAX_TITLE_CHARS} characters")]
    TitleTooLong,
    #[error("body exceeds the 1 MiB limit")]
    BodyTooLarge,
    #[error("change summary exceeds {MAX_SUMMARY_CHARS} characters")]
    SummaryTooLong,
    #[error("restored version {requested} does not exist")]
    RestoreTargetMissing { requested: i32 },
}

/// The next version number after `current` (gap-free, 1-based). A fresh
/// document has no versions yet; its first write is version 1.
pub fn next_version(current_max: Option<i32>) -> i32 {
    current_max.unwrap_or(0) + 1
}

/// Validates the content of a version about to be APPENDED.
pub fn validate_version_content(
    title: &str,
    body: &str,
    change_summary: &str,
) -> Result<(), DocumentError> {
    if title.trim().is_empty() {
        return Err(DocumentError::EmptyTitle);
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(DocumentError::TitleTooLong);
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(DocumentError::BodyTooLarge);
    }
    if change_summary.chars().count() > MAX_SUMMARY_CHARS {
        return Err(DocumentError::SummaryTooLong);
    }
    Ok(())
}

/// Everything needed to append a restore-created version. Building the
/// plan validates the target; executing it inserts a NEW row — the only
/// sanctioned way historical content returns.
#[derive(Debug, Clone, PartialEq)]
pub struct RestorePlan {
    /// Title carried into the new version (the target's title, so a
    /// rename after the target is not silently undone by the restore).
    pub title: String,
    /// Body copied verbatim from the target version.
    pub body: String,
    /// The version whose content is being restored.
    pub restored_from_version: i32,
    /// The version number the NEW row receives.
    pub new_version: i32,
    /// Validated change summary (defaults to naming the source version).
    pub change_summary: String,
}

impl RestorePlan {
    /// Plans a restore of `target_version` on top of the document's
    /// existing history (newest version = `current_max`). The target must
    /// exist in `history`; the plan's title/body come from that row so the
    /// copy is exact.
    pub fn build(
        history: &[DocumentVersion],
        current_max: Option<i32>,
        target_version: i32,
        change_summary: &str,
    ) -> Result<Self, DocumentError> {
        let target = history.iter().find(|v| v.version == target_version).ok_or(
            DocumentError::RestoreTargetMissing {
                requested: target_version,
            },
        )?;
        let summary = if change_summary.trim().is_empty() {
            format!("Restored from version {target_version}")
        } else {
            change_summary.trim().to_string()
        };
        let plan = Self {
            title: target.title.clone(),
            body: target.body.clone(),
            restored_from_version: target_version,
            new_version: next_version(current_max),
            change_summary: summary,
        };
        validate_version_content(&plan.title, &plan.body, &plan.change_summary)?;
        Ok(plan)
    }
}

/// One line-level change: a removed line and/or the line that replaced it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DiffEntry {
    /// 1-based line number in the OLD body (absent for pure insertions).
    pub old_line: Option<usize>,
    /// 1-based line number in the NEW body (absent for pure deletions).
    pub new_line: Option<usize>,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// A renderable diff summary between two version bodies.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DiffSummary {
    pub added: usize,
    pub removed: usize,
    /// `true` when the diff hit the [`MAX_DIFF_LINES`] guard and the entry
    /// list is incomplete (counts stay exact where computed; callers must
    /// not treat the list as exhaustive).
    pub truncated: bool,
    pub entries: Vec<DiffEntry>,
}

/// Diffs two version bodies line-by-line via LCS. Whitespace-exact: lines
/// must match byte-for-byte to pair up. Trailing-newline differences are
/// ignored (a body and the same body plus one final newline are equal).
pub fn diff_bodies(old_body: &str, new_body: &str) -> DiffSummary {
    let old_lines: Vec<&str> = lines_of(old_body);
    let new_lines: Vec<&str> = lines_of(new_body);
    let truncated = old_lines.len() > MAX_DIFF_LINES || new_lines.len() > MAX_DIFF_LINES;
    let mut entries = Vec::new();
    if truncated {
        return DiffSummary {
            added: 0,
            removed: 0,
            truncated,
            entries,
        };
    }

    // LCS table (usize; bounded by MAX_DIFF_LINES² which is why the guard
    // exists).
    let n = old_lines.len();
    let m = new_lines.len();
    let mut lcs = vec![0_usize; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[at(i, j)] = if old_lines[i] == new_lines[j] {
                lcs[at(i + 1, j + 1)] + 1
            } else {
                lcs[at(i + 1, j)].max(lcs[at(i, j + 1)])
            }
        }
    }

    let (mut i, mut j) = (0, 0);
    let mut added = 0_usize;
    let mut removed = 0_usize;
    while i < n && j < m {
        if old_lines[i] == new_lines[j] {
            i += 1;
            j += 1;
        } else if lcs[at(i + 1, j)] >= lcs[at(i, j + 1)] {
            removed += 1;
            entries.push(DiffEntry {
                old_line: Some(i + 1),
                new_line: None,
                before: Some(old_lines[i].to_string()),
                after: None,
            });
            i += 1;
        } else {
            added += 1;
            entries.push(DiffEntry {
                old_line: None,
                new_line: Some(j + 1),
                before: None,
                after: Some(new_lines[j].to_string()),
            });
            j += 1;
        }
    }
    while i < n {
        removed += 1;
        entries.push(DiffEntry {
            old_line: Some(i + 1),
            new_line: None,
            before: Some(old_lines[i].to_string()),
            after: None,
        });
        i += 1;
    }
    while j < m {
        added += 1;
        entries.push(DiffEntry {
            old_line: None,
            new_line: Some(j + 1),
            before: None,
            after: Some(new_lines[j].to_string()),
        });
        j += 1;
    }

    DiffSummary {
        added,
        removed,
        truncated,
        entries,
    }
}

/// Splits into display lines, ignoring a single trailing newline so a
/// body and its newline-terminated twin diff as equal.
fn lines_of(body: &str) -> Vec<&str> {
    let stripped = body.strip_suffix('\n').unwrap_or(body);
    if stripped.is_empty() {
        Vec::new()
    } else {
        stripped.split('\n').collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(n: i32, title: &str, body: &str) -> DocumentVersion {
        DocumentVersion {
            id: core_domain::ids::DocumentVersionId::nil(),
            org_id: core_domain::ids::OrgId::nil(),
            document_id: core_domain::ids::DocumentId::nil(),
            version: n,
            title: title.into(),
            body: body.into(),
            change_summary: String::new(),
            author_id: None,
            restored_from_version: None,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn version_numbering_is_gap_free_from_one() {
        assert_eq!(next_version(None), 1);
        assert_eq!(next_version(Some(1)), 2);
        assert_eq!(next_version(Some(41)), 42);
    }

    #[test]
    fn content_validation_bounds() {
        assert_eq!(
            validate_version_content("  ", "", ""),
            Err(DocumentError::EmptyTitle)
        );
        let long = "x".repeat(MAX_TITLE_CHARS + 1);
        assert_eq!(
            validate_version_content(&long, "", ""),
            Err(DocumentError::TitleTooLong)
        );
        let big = vec![b'a'; MAX_BODY_BYTES + 1];
        assert_eq!(
            validate_version_content("t", std::str::from_utf8(&big).unwrap(), ""),
            Err(DocumentError::BodyTooLarge)
        );
        let summary = "y".repeat(MAX_SUMMARY_CHARS + 1);
        assert_eq!(
            validate_version_content("t", "", &summary),
            Err(DocumentError::SummaryTooLong)
        );
        assert!(validate_version_content("Runbook", "# steps\n1. reboot", "added steps").is_ok());
    }

    #[test]
    fn restore_plan_copies_target_exactly_and_numbers_forward() {
        let history = vec![
            version(1, "Runbook", "v1 body"),
            version(2, "Runbook v2", "v2 body"),
            version(3, "Runbook v2", "v3 body"),
        ];
        let plan = RestorePlan::build(&history, Some(3), 1, "").expect("plan");
        assert_eq!(plan.new_version, 4);
        assert_eq!(plan.title, "Runbook");
        assert_eq!(plan.body, "v1 body");
        assert_eq!(plan.restored_from_version, 1);
        // Default summary names the source version.
        assert_eq!(plan.change_summary, "Restored from version 1");

        assert_eq!(
            RestorePlan::build(&history, Some(3), 9, ""),
            Err(DocumentError::RestoreTargetMissing { requested: 9 })
        );
    }

    #[test]
    fn diff_reports_additions_removals_and_replacements() {
        let old = "line1\nline2\nline3";
        let new = "line1\nline2-changed\nline3\nline4";

        let summary = diff_bodies(old, new);
        assert_eq!(summary.added, 2); // the replacement's new half + line4
        assert_eq!(summary.removed, 1);
        assert!(!summary.truncated);
        assert!(summary
            .entries
            .iter()
            .any(|e| e.before.as_deref() == Some("line2") && e.after.is_none()));
        assert!(summary
            .entries
            .iter()
            .any(|e| e.after.as_deref() == Some("line2-changed")));
        assert!(summary
            .entries
            .iter()
            .any(|e| e.after.as_deref() == Some("line4") && e.old_line.is_none()));
    }

    #[test]
    fn diff_treats_trailing_newline_as_equal() {
        let summary = diff_bodies("a\nb", "a\nb\n");
        assert_eq!(summary.added, 0);
        assert_eq!(summary.removed, 0);
        assert!(summary.entries.is_empty());
    }

    #[test]
    fn diff_truncates_huge_bodies_instead_of_burning_memory() {
        let big = "x\n".repeat(MAX_DIFF_LINES + 1);
        let summary = diff_bodies(&big, &big);
        assert!(summary.truncated);
        assert!(summary.entries.is_empty());
    }
}
