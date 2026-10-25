//! The attachment verdict backfill (Wave 55, rows F30 and F31 —
//! `p55-attachment-verdicts`).
//!
//! Purpose: close the one way the Wave 54 admission rule can lock a working
//! install out of its own files. Since `p54-attachment-scan`, a download in a
//! deployment WITH a scanner refuses an object that has no verdict on record,
//! because an absent verdict there means the bytes entered the store without
//! passing the gate. That is the right answer for bytes written after the gate
//! existed. It is the wrong answer for the ones written before it: every file
//! stored by an install that had no scanner has no verdict, and turning a
//! scanner on today would refuse all of them at every door until someone
//! re-uploaded each one by hand.
//!
//! The ruling (Wave 54 follow-up F30) is refuse-until-backfilled, and this is
//! the backfill. It walks the stored objects that have no verdict, reads each
//! one's bytes back from the object store, and puts them through the SAME
//! [`crate::scan_gate::gate_with`] the upload and inbound doors call, writing
//! the SAME verdict rows through [`core_db::attachment_scans`]. Nothing here
//! reimplements the gate: a second copy of "what a scanner answer means" is
//! exactly the drift this lane exists to remove.
//!
//! Four properties the command's safety rests on, all enforced here rather
//! than promised by the caller:
//!
//! 1. **Only objects without a verdict are scanned.** The walk's predicate is
//!    `NOT EXISTS (SELECT 1 FROM attachment_scans ...)` (`core-db`'s
//!    [`core_db::attachments::list_unverdict_objects`]), so a second run finds
//!    nothing and pays for nothing.
//! 2. **A recorded verdict is never rewritten.** `record_admitted` keeps the
//!    first admission; the walk only ever targets digests with no row, and a
//!    concurrent writer that lands one first is counted `already`, not
//!    overwritten.
//! 3. **A no-scanner deployment does nothing.** The documented none mode
//!    writes `skipped` for every upload it sees, so a backfill there would
//!    stamp `skipped` on history and call it an examination. It refuses
//!    instead, and the doors keep streaming those objects as Wave 54 decided.
//! 4. **A scanner outage stops the walk.** Bytes are not recorded as clean
//!    because a later object might scan: the first [`ApiError::Unavailable`]
//!    ends the pass and reports what it had done, so the run is resumable and
//!    the report is honest about where it stopped.
//!
//! Bytes are re-hashed before the verdict is written. The digest is the
//! identity the row is keyed by, so bytes that do not hash to it are a
//! different object and get no verdict — an operator should hear about that
//! store, not receive a verdict about something else.
//!
//! Non-purpose: no HTTP surface, no re-scan of admitted objects (that would be
//! a separate, deliberate pass), no retention policy, and no repair of a store
//! that has lost bytes — those are reported, never invented.

use crate::error::{ApiError, ApiResult};
use crate::scan_gate;
use core_db::attachments::UnverdictObject;
use core_domain::entities::ScanStatus;
use core_mail::ScanProvider;
use core_store::ObjectStore;
use futures_util::StreamExt as _;
use sha2::Digest as _;
use sqlx::PgPool;
use uuid::Uuid;

/// Objects fetched per query batch. Each batch is one read; the writes happen
/// per object so a crash mid-walk leaves finished verdicts committed.
pub const DEFAULT_BATCH: i64 = 200;

/// What the walk found before it touched anything: the number a `--dry-run`
/// reports and the number an executing run is confirmed against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillPlan {
    /// Objects per org, ordered by org id.
    pub per_org: Vec<(Uuid, i64)>,
    /// Every object with no verdict on record.
    pub total: i64,
}

/// What one run did, per outcome. Counts, not prose: the CLI prints them,
/// and a test asserts them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackfillReport {
    /// Objects the walk planned to judge. A second run reports 0.
    pub found: i64,
    /// Bytes a scanner examined and passed.
    pub clean: u64,
    /// Bytes recorded `skipped` — only reachable with a scanner whose own
    /// answer says skipped (a scripted or future adapter), never the none mode.
    pub skipped: u64,
    /// Bytes held: a signature was found. The object keeps its bytes and the
    /// operator's release path now owns it.
    pub quarantined: u64,
    /// A verdict for this digest landed between the plan and the write.
    pub already: u64,
    /// Objects with no verdict that the walk could not judge, with the reason
    /// per object. These keep no verdict, so the next run tries them again.
    pub unreadable: u64,
    /// One line per unreadable object (`<org>/<sha256>: <reason>`).
    pub failures: Vec<String>,
    /// Set when the scanner stopped answering: the walk ended there, and
    /// nothing was recorded for the object it was judging.
    pub aborted: Option<String>,
}

/// The objects with no verdict, counted per org. Read-only: a dry run is this
/// call and nothing else.
///
/// # Errors
/// [`ApiError::Validation`] when the connection cannot read across orgs (see
/// [`require_deployment_scope`]); [`ApiError`] when the database cannot be
/// read.
pub async fn plan(pool: &PgPool, org_id: Option<Uuid>) -> ApiResult<BackfillPlan> {
    require_deployment_scope(pool).await?;
    let per_org = core_db::attachments::unverdict_counts(pool, org_id).await?;
    let total = per_org.iter().map(|(_, count)| *count).sum();
    Ok(BackfillPlan { per_org, total })
}

/// Refuses a connection whose row-level security scope hides the objects the
/// walk exists to find.
///
/// The walk reads across organizations and its reads run on the pool, outside
/// any transaction that carries `app.org_id`. Under the RLS app role every
/// such query returns zero rows, so the command would print a clean, empty
/// plan, write nothing, and look like a deployment that has nothing to
/// backfill — the exact failure the erasure verb's owner check exists to
/// prevent. Narrowing with `org_id` does not change this: a pooled read is
/// still unscoped, so the deployment role (the migration role) is what the
/// command needs either way.
///
/// # Errors
/// [`ApiError::Validation`] when the connection is the RLS app role.
pub async fn require_deployment_scope(pool: &PgPool) -> ApiResult<()> {
    let owner: bool = sqlx::query_scalar(
        r#"SELECT current_setting('is_superuser') = 'on'
              OR EXISTS (
                   SELECT 1 FROM pg_roles
                    WHERE rolname = current_user AND (rolbypassrls OR rolsuper)
                 )
              OR EXISTS (
                   SELECT 1 FROM pg_class
                    WHERE relname = 'orgs' AND pg_get_userbyid(relowner) = current_user
                 )"#,
    )
    .fetch_one(pool)
    .await?;
    if owner {
        return Ok(());
    }
    Err(ApiError::Validation(
        "this connection cannot walk every organization: connect with the deployment (migration) \
         role, not the RLS-constrained app role — under the app role every read here returns zero \
         rows and the backfill would report an empty plan while changing nothing"
            .into(),
    ))
}

/// Walks the objects with no verdict and records one per object.
///
/// `expected` is the count a dry run reported. When it does not match the set
/// this run is about to judge, the run refuses BEFORE scanning anything: the
/// operator's confirmation is about a plan, and the plan has moved.
///
/// # Errors
/// [`ApiError::Unavailable`] only via [`BackfillReport::aborted`] (which is the
/// resumable case); [`ApiError`] for a database or store failure that makes the
/// outcome unknown, and [`ApiError::Validation`] for the two refusals above.
pub async fn run(
    pool: &PgPool,
    store: &dyn ObjectStore,
    scanner: &dyn ScanProvider,
    org_id: Option<Uuid>,
    batch: i64,
    expected: Option<i64>,
) -> ApiResult<BackfillReport> {
    if !scan_gate::scanner_is_installed(scanner) {
        return Err(ApiError::Validation(
            "this deployment has no malware scanner configured (EMAIL_SCAN is not clamd), so a \
             backfill has nothing to decide: recording `skipped` for every stored object would \
             claim an examination that never happened. The doors stream unverdict objects in \
             this mode, exactly as before the gate existed."
                .into(),
        ));
    }
    let batch = batch.max(1);

    let plan = plan(pool, org_id).await?;
    let mut report = BackfillReport {
        found: plan.total,
        ..BackfillReport::default()
    };
    if let Some(expected) = expected {
        if expected != plan.total {
            return Err(ApiError::Validation(format!(
                "the plan moved between the dry run and this run: {} object(s) have no verdict \
                 now, {expected} were confirmed, so nothing was scanned. Re-run --dry-run.",
                plan.total
            )));
        }
    }

    let mut cursor: Option<(Uuid, String)> = None;
    loop {
        let page =
            core_db::attachments::list_unverdict_objects(pool, org_id, cursor.clone(), batch)
                .await?;
        if page.is_empty() {
            break;
        }
        for object in page {
            cursor = Some((object.org_id, object.sha256.clone()));
            match judge(pool, store, scanner, &object).await {
                Ok(Outcome::Clean) => report.clean += 1,
                Ok(Outcome::Skipped) => report.skipped += 1,
                Ok(Outcome::Quarantined) => report.quarantined += 1,
                Ok(Outcome::Already) => report.already += 1,
                Ok(Outcome::Unreadable(reason)) => {
                    report.unreadable += 1;
                    report
                        .failures
                        .push(format!("{}/{}: {reason}", object.org_id, object.sha256));
                }
                Err(ApiError::Unavailable(reason)) => {
                    report.aborted = Some(reason);
                    return Ok(report);
                }
                Err(error) => return Err(error),
            }
        }
    }
    Ok(report)
}

/// One object's verdict, in the terms the report counts.
enum Outcome {
    Clean,
    Skipped,
    Quarantined,
    /// A verdict for this digest already existed when the write landed.
    Already,
    /// No bytes, or bytes that are not the object the row describes. Either
    /// way there is nothing to put a verdict on.
    Unreadable(String),
}

/// Reads one object's bytes, gates them, and records what the gate said.
async fn judge(
    pool: &PgPool,
    store: &dyn ObjectStore,
    scanner: &dyn ScanProvider,
    object: &UnverdictObject,
) -> ApiResult<Outcome> {
    // A verdict row requires a non-empty object (`size_bytes > 0`); a zero
    // byte row cannot be described by it, so report it instead of failing the
    // whole walk on one CHECK constraint.
    if object.size_bytes <= 0 {
        return Ok(Outcome::Unreadable(
            "the row records no bytes (size_bytes <= 0), which the verdict table cannot describe"
                .into(),
        ));
    }
    let mut stored = match store.get(&object.storage_key).await {
        Ok(stored) => stored,
        Err(core_store::StoreError::NotFound(_)) => {
            return Ok(Outcome::Unreadable(format!(
                "no object at {}",
                object.storage_key
            )))
        }
        Err(error) => {
            return Err(ApiError::Internal(anyhow::anyhow!(
                "reading {}: {error}",
                object.storage_key
            )))
        }
    };
    let mut bytes = Vec::with_capacity(stored.size_bytes as usize);
    while let Some(chunk) = stored.stream.next().await {
        let chunk = chunk.map_err(|error| {
            ApiError::Internal(anyhow::anyhow!("reading {}: {error}", object.storage_key))
        })?;
        bytes.extend_from_slice(&chunk);
    }

    // The digest is the verdict's identity. Bytes that do not hash to it are a
    // different object; recording a verdict for them would attach one file's
    // examination to another file's row.
    let digest = hex::encode(sha2::Sha256::digest(&bytes));
    if digest != object.sha256 {
        return Ok(Outcome::Unreadable(format!(
            "the stored bytes hash to {digest}, not to the row's {}",
            object.sha256
        )));
    }

    let verdict = scan_gate::gate_with(scanner, &bytes).await?;
    let mut tx = pool.begin().await.map_err(ApiError::from)?;
    // Required under the RLS app role and a no-op under a role that bypasses
    // RLS, which is what a cross-org walk needs.
    core_db::set_org(&mut tx, object.org_id).await?;

    if verdict.status == ScanStatus::Infected {
        scan_gate::quarantine_in_tx(
            &mut tx,
            store,
            scan_gate::Rejection {
                org_id: object.org_id,
                sha256: &object.sha256,
                filename: &object.filename,
                content_type: &object.content_type,
                size_bytes: object.size_bytes,
                bytes: bytes::Bytes::from(bytes),
                ticket_id: object.ticket_id,
                uploaded_by_user_id: object.uploaded_by_user_id,
                verdict: &verdict,
            },
        )
        .await?;
        tx.commit().await.map_err(ApiError::from)?;
        return Ok(Outcome::Quarantined);
    }

    let status = verdict.status;
    let outcome = core_db::attachment_scans::record_admitted(
        &mut tx,
        core_db::attachment_scans::NewScan {
            org_id: object.org_id,
            sha256: &object.sha256,
            status,
            detail: verdict.detail.as_deref(),
            scanner: verdict.scanner,
            size_bytes: object.size_bytes,
            content_type: &object.content_type,
            filename: &object.filename,
            storage_key: &object.storage_key,
            ticket_id: object.ticket_id,
            uploaded_by_user_id: object.uploaded_by_user_id,
            uploaded_by_portal_user_id: object.uploaded_by_portal_user_id,
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(match outcome {
        core_db::attachment_scans::AdmitOutcome::Recorded(_) if status == ScanStatus::Skipped => {
            Outcome::Skipped
        }
        core_db::attachment_scans::AdmitOutcome::Recorded(_) => Outcome::Clean,
        core_db::attachment_scans::AdmitOutcome::AlreadyRecorded(_) => Outcome::Already,
    })
}
