//! worker/object_gc_sweep — the object store's integrity pass
//! (Wave 52 lane R6, `p52-data-lifecycle-dr`).
//!
//! Purpose: rows and bytes are two stores with no shared transaction, and both
//! directions of drift are silent. This pass walks the store once per interval
//! and closes both: it reclaims bytes no row claims (orphans) and records the
//! rows whose bytes are gone, so a download can say "no longer available"
//! instead of failing as an internal error. `core_db::object_gc` owns the SQL;
//! this module owns the decision about what is safe to delete.
//!
//! Safety is the whole design, because this pass deletes data:
//!
//! * **Off by default.** `MARULA_OBJECT_GC` is unset in a stock install, and an
//!   unset pass does nothing at all. Deletion needs a store configuration on the
//!   process (the worker mounts the same object directory core-api writes), and
//!   an operator who has not set that up must never get "reclaimed 400 MB" —
//!   they would get "every attachment is missing", because an empty store is
//!   exactly what a container with no volume looks like.
//! * **An empty listing is a fault, not a finding.** If the store lists nothing
//!   while rows reference objects, the pass aborts before marking or deleting
//!   anything. This is the single most likely production mistake — the worker
//!   without the objects volume — and it would otherwise be a mass
//!   "missing bytes" event that looks like data loss.
//! * **A grace window on orphans.** Bytes are only deletable once the store says
//!   they are older than `MARULA_OBJECT_GC_GRACE_HOURS` (default 24). An upload
//!   writes the object and then commits the row: deleting on "not referenced
//!   right now" would race every upload in flight. A backend that cannot report
//!   an object's age cannot have its objects deleted.
//! * **`dry_run` exists** because the first run on a real bucket should be a
//!   report, not a delete.
//!
//! Non-purpose: no bucket lifecycle policy (the pass is portable across local-fs
//! and S3), no repair of the bytes themselves (`missing_since` records the state;
//! a re-upload of the same content address is what clears it), and no deletion of
//! anything the grace window or the age check cannot vouch for.

use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use core_store::ObjectStore;
use sqlx::PgPool;
use tokio::sync::watch;
use uuid::Uuid;

use core_db::object_gc::{self, Finding, FindingKind};

/// Default cadence. An hour is often enough that a reclaimed upload is noticed
/// the same day and rare enough that a store listing does not compete with the
/// API's traffic.
pub const DEFAULT_INTERVAL_SECS: u64 = 60 * 60;

/// Default grace before unreferenced bytes are deletable.
pub const DEFAULT_GRACE_HOURS: u64 = 24;

/// Ceiling on findings recorded per pass, so one catastrophic pass cannot fill
/// the ledger. The count is reported either way.
pub const MAX_FINDINGS: usize = 5_000;

/// Ceiling on orphan deletions per pass. Same reasoning as the retention batch
/// cap: a first pass over a neglected bucket does its work in several short
/// passes instead of one long one.
pub const MAX_DELETIONS: usize = 5_000;

/// Whether the pass may change anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcMode {
    /// Not configured: the loop is not even started.
    Off,
    /// Report only. Nothing is deleted, no row is marked.
    DryRun,
    /// Delete orphans and mark missing rows.
    Apply,
}

impl GcMode {
    /// Parses `MARULA_OBJECT_GC` (`off` | `dry_run` | `on`). Unset is `off`;
    /// anything unrecognised is `off` with a warning, never a guess — the value
    /// that turns deletion on has to be spelled correctly.
    #[must_use]
    pub fn parse(value: Option<&str>) -> (Self, Option<String>) {
        let Some(raw) = value else {
            return (Self::Off, None);
        };
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "off" | "0" | "false" => (Self::Off, None),
            "dry_run" | "dry-run" => (Self::DryRun, None),
            "on" | "apply" | "1" | "true" => (Self::Apply, None),
            _ => (
                Self::Off,
                Some(format!(
                    "MARULA_OBJECT_GC={raw:?} is not off, dry_run or on; object GC stays off"
                )),
            ),
        }
    }
}

/// Settings for the pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectGcConfig {
    pub mode: GcMode,
    pub interval_secs: u64,
    pub grace: Duration,
    /// Refused or unrecognised settings, for the boot log.
    pub warnings: Vec<String>,
}

impl ObjectGcConfig {
    /// Resolves the knobs from a key lookup.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let (mode, mode_warning) = GcMode::parse(lookup("MARULA_OBJECT_GC").as_deref());
        let mut warnings = Vec::new();
        if let Some(warning) = mode_warning {
            warnings.push(warning);
        }
        let grace_hours = lookup("MARULA_OBJECT_GC_GRACE_HOURS")
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|hours| *hours > 0)
            .unwrap_or(DEFAULT_GRACE_HOURS);
        if let Some(raw) = lookup("MARULA_OBJECT_GC_GRACE_HOURS") {
            if raw.trim().parse::<u64>().map(|h| h == 0).unwrap_or(true) {
                warnings.push(format!(
                    "MARULA_OBJECT_GC_GRACE_HOURS={raw:?} is not a positive hour count; \
                     using {DEFAULT_GRACE_HOURS}"
                ));
            }
        }
        Self {
            mode,
            interval_secs: lookup("MARULA_OBJECT_GC_SECS")
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|secs| *secs > 0)
                .unwrap_or(DEFAULT_INTERVAL_SECS),
            grace: Duration::from_secs(grace_hours.saturating_mul(3_600)),
            warnings,
        }
    }
}

/// What one pass saw and did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcReport {
    pub mode_applied: bool,
    /// Objects the store listed.
    pub listed: usize,
    /// Keys rows currently claim.
    pub referenced: usize,
    /// Unreferenced objects past the grace window.
    pub orphans: usize,
    /// Orphan objects deleted (0 in dry run).
    pub orphans_deleted: usize,
    /// Orphan delete failures; each one is logged with its key.
    pub delete_failures: usize,
    /// Unreferenced objects skipped because the store could not date them.
    pub age_unknown: usize,
    /// Referenced keys with no bytes in the store.
    pub missing: usize,
    /// Rows newly marked as missing bytes.
    pub marked: u64,
    /// Rows whose bytes came back.
    pub cleared: u64,
    /// Keys found in both directions that a cap kept out of this pass.
    pub deferred: usize,
}

/// One pass. `store` is the same adapter core-api writes through.
pub async fn pass(
    pool: &PgPool,
    store: &dyn ObjectStore,
    config: &ObjectGcConfig,
) -> anyhow::Result<GcReport> {
    let mut report = GcReport::default();
    if config.mode == GcMode::Off {
        return Ok(report);
    }
    let objects = store.list("").await?;
    let refs = object_gc::referenced_keys(pool).await?;
    report.listed = objects.len();
    report.referenced = refs.len();

    // The guard that makes this pass safe on a misconfigured host. A store with
    // no objects and a database full of references is a mounting fault, and the
    // "missing bytes" answer it would otherwise produce is the worst possible
    // reading of it.
    if objects.is_empty() && !refs.is_empty() {
        anyhow::bail!(
            "object store ({}) lists 0 objects while {} rows reference objects; \
             refusing to run — the store is probably not mounted on this process",
            store.name(),
            refs.len()
        );
    }

    let referenced: HashSet<&str> = refs.iter().map(|r| r.storage_key.as_str()).collect();
    let stored: HashSet<&str> = objects.iter().map(|o| o.key.as_str()).collect();
    let now = SystemTime::now();

    let mut orphan_keys: Vec<(String, Option<i64>)> = Vec::new();
    for object in &objects {
        if referenced.contains(object.key.as_str()) {
            continue;
        }
        let age = object
            .last_modified
            .and_then(|modified| now.duration_since(modified).ok());
        match age {
            Some(age) if age >= config.grace => {
                orphan_keys.push((object.key.clone(), Some(object.size_bytes as i64)));
            }
            Some(_) => {}
            None => {
                // No age, no deletion: an object whose timestamp is missing is
                // either a backend that cannot date it or a clock fault, and
                // either way this pass cannot prove it is not the upload that
                // is committing right now.
                report.age_unknown += 1;
            }
        }
    }
    report.orphans = orphan_keys.len();
    if orphan_keys.len() > MAX_DELETIONS {
        report.deferred += orphan_keys.len() - MAX_DELETIONS;
        orphan_keys.truncate(MAX_DELETIONS);
    }

    // Missing bytes: a key rows claim that the store does not have.
    let mut missing_keys: Vec<String> = Vec::new();
    for reference in &refs {
        if !stored.contains(reference.storage_key.as_str())
            && !missing_keys.contains(&reference.storage_key)
        {
            missing_keys.push(reference.storage_key.clone());
        }
    }
    report.missing = missing_keys.len();

    // Keys we marked before and that are healthy again: the re-upload case.
    let previously_missing: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT storage_key FROM object_integrity_findings \
         WHERE kind = 'bytes_missing' AND resolved_at IS NULL",
    )
    .fetch_all(pool)
    .await?;
    let healed: Vec<String> = previously_missing
        .into_iter()
        .filter(|key| stored.contains(key.as_str()))
        .collect();

    if config.mode == GcMode::DryRun {
        return Ok(report);
    }
    report.mode_applied = true;

    if !healed.is_empty() {
        report.cleared = object_gc::clear_missing(pool, &healed).await?;
        tracing::info!(
            healed = healed.len(),
            rows = report.cleared,
            "object integrity: bytes restored"
        );
    }

    if !missing_keys.is_empty() {
        report.marked = object_gc::mark_missing(pool, &missing_keys).await?;
        tracing::warn!(
            keys = missing_keys.len(),
            rows = report.marked,
            "object integrity: rows reference objects the store does not have"
        );
    }

    let pass_id = Uuid::now_v7();
    let index = object_gc::reference_index(pool, &missing_keys).await?;
    let mut findings: Vec<Finding> = Vec::new();
    for key in &missing_keys {
        if findings.len() >= MAX_FINDINGS {
            report.deferred += 1;
            continue;
        }
        let reference = index.get(key);
        findings.push(Finding {
            kind: FindingKind::BytesMissing,
            org_id: reference.and_then(|r| r.org_id),
            storage_key: key.clone(),
            size_bytes: None,
            source_table: reference.map(|r| r.source_table),
            note: None,
        });
    }
    // Delete first, then record only what actually went. A finding that said
    // `orphan_deleted` for a delete that failed is the one kind of evidence this
    // ledger must never produce: a record claiming bytes were reclaimed while
    // they are still there.
    for (key, size) in &orphan_keys {
        match store.delete(key).await {
            Ok(()) => {
                report.orphans_deleted += 1;
                if findings.len() >= MAX_FINDINGS {
                    report.deferred += 1;
                    continue;
                }
                findings.push(Finding {
                    kind: FindingKind::OrphanDeleted,
                    org_id: org_id_from_key(key),
                    storage_key: key.clone(),
                    size_bytes: *size,
                    source_table: None,
                    note: None,
                });
            }
            Err(error) => {
                report.delete_failures += 1;
                tracing::error!(key, %error, "object integrity: orphan delete failed");
            }
        }
    }
    object_gc::record_findings(pool, pass_id, &findings).await?;

    tracing::info!(
        pass = %pass_id,
        store = store.name(),
        listed = report.listed,
        referenced = report.referenced,
        orphans = report.orphans,
        deleted = report.orphans_deleted,
        delete_failures = report.delete_failures,
        age_unknown = report.age_unknown,
        missing = report.missing,
        marked = report.marked,
        cleared = report.cleared,
        "object integrity pass"
    );
    Ok(report)
}

/// The org a key belongs to, from the `org/<uuid>/…` convention the store's key
/// builders enforce. `None` for anything else, which the findings table allows
/// and the operator list shows.
fn org_id_from_key(key: &str) -> Option<Uuid> {
    let mut parts = key.split('/');
    if parts.next()? != "org" {
        return None;
    }
    Uuid::parse_str(parts.next()?).ok()
}

/// The loop. Only started when the mode is not `Off`; a failure backs off two
/// seconds, and the report is logged by [`pass`].
pub async fn run_object_gc(
    pool: PgPool,
    store: std::sync::Arc<dyn ObjectStore>,
    config: ObjectGcConfig,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => return,
            result = pass(&pool, store.as_ref(), &config) => {
                match result {
                    Ok(_) => {
                        tokio::time::sleep(std::time::Duration::from_secs(
                            config.interval_secs.max(60),
                        ))
                        .await;
                    }
                    Err(error) => {
                        tracing::error!(%error, "object integrity pass failed");
                        tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_or_off_means_the_pass_does_nothing() {
        assert_eq!(GcMode::parse(None), (GcMode::Off, None));
        assert_eq!(GcMode::parse(Some("")), (GcMode::Off, None));
        assert_eq!(GcMode::parse(Some("off")), (GcMode::Off, None));
        assert_eq!(GcMode::parse(Some("on")), (GcMode::Apply, None));
        assert_eq!(GcMode::parse(Some("dry-run")), (GcMode::DryRun, None));
    }

    /// The setting that deletes data must not be reachable by a typo.
    #[test]
    fn an_unrecognised_mode_stays_off_and_says_so() {
        let (mode, warning) = GcMode::parse(Some("yes"));
        assert_eq!(mode, GcMode::Off);
        assert!(warning.expect("warning").contains("stays off"));
    }

    #[test]
    fn defaults_are_off_with_a_day_of_grace() {
        let cfg = ObjectGcConfig::from_lookup(|_| None);
        assert_eq!(cfg.mode, GcMode::Off);
        assert_eq!(cfg.interval_secs, DEFAULT_INTERVAL_SECS);
        assert_eq!(cfg.grace, Duration::from_secs(86_400));
        assert!(cfg.warnings.is_empty());
    }

    #[test]
    fn a_zero_grace_hour_count_is_refused_not_honoured() {
        let cfg = ObjectGcConfig::from_lookup(|key| match key {
            "MARULA_OBJECT_GC" => Some("on".into()),
            "MARULA_OBJECT_GC_GRACE_HOURS" => Some("0".into()),
            _ => None,
        });
        assert_eq!(cfg.mode, GcMode::Apply);
        assert_eq!(cfg.grace, Duration::from_secs(86_400));
        assert_eq!(cfg.warnings.len(), 1);
    }

    #[test]
    fn the_org_prefix_is_the_only_thing_trusted_for_attribution() {
        let org = Uuid::nil();
        assert_eq!(
            org_id_from_key(&format!("org/{org}/attachments/abc")),
            Some(org)
        );
        assert_eq!(org_id_from_key("org/not-a-uuid/attachments/abc"), None);
        assert_eq!(org_id_from_key("loose/key"), None);
    }
}
