//! `marula attachments` — the stored-object maintenance verbs (Wave 55,
//! `p55-attachment-verdicts`).
//!
//! Purpose: give an operator the one command that closes the gap the scan
//! gate's admission rule opened — objects stored before a scanner existed have
//! no verdict, and a deployment that turns a scanner on today refuses every
//! one of them at every door. `backfill-verdicts` walks those objects, reads
//! their bytes from the object store, and records what the configured scanner
//! says. The walk itself lives in [`core_api::scan_backfill`], beside the gate
//! and the verdict writers it must not drift from; this module is the
//! argument handling, the environment wiring, and the reporting.
//!
//! Conventions are the crate's: `--dry-run` reports the plan and writes
//! nothing, and executing needs `--confirm <count>` — the number the dry run
//! printed — exactly as `data purge-org` needs the tenant's label. The count
//! is the operator's statement that they read the plan; the run re-counts
//! before it scans anything and refuses if the plan moved.
//!
//! Two environment dependencies, both shared with the API process on purpose:
//! `EMAIL_SCAN`/`CLAMD_ADDR` select the same scanner adapter core-api builds
//! (`crates/core-api/src/config.rs`), and `OBJECT_STORE`/`S3_*` select the
//! same store through `core_store::StoreChoice::from_env`, so the command
//! reads the bytes the API wrote rather than a second, empty store.
//!
//! Non-purpose: no re-scan of objects that already have a verdict (that is a
//! deliberate, separate pass), no repair of a store missing bytes, and no
//! bypass around the confirmation.

use anyhow::{bail, Context as _};
use core_mail::{ClamdScan, NoopScan, ScanProvider};
use std::sync::Arc;
use uuid::Uuid;

/// What the verb was asked to do.
#[derive(Debug, Clone)]
pub struct BackfillArgs {
    /// Only this org. Absent means every org. The connection must be the
    /// deployment (migration) role either way — see
    /// [`core_api::scan_backfill::require_deployment_scope`].
    pub org_id: Option<Uuid>,
    /// Objects per query batch.
    pub batch_size: i64,
    /// Report only: list the plan and write nothing.
    pub dry_run: bool,
    /// The count the dry run printed. Required to execute.
    pub confirm: Option<i64>,
}

/// Runs the verdict backfill (or reports the plan).
pub async fn backfill_verdicts(database_url: &str, args: &BackfillArgs) -> anyhow::Result<()> {
    let scanner = scanner_from_env();
    let store = core_store::build(&core_store::StoreChoice::from_env());
    let pool = core_db::connect(database_url)
        .await
        .context("connecting to the database")?;

    let plan = core_api::scan_backfill::plan(&pool, args.org_id)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    print_plan(&plan);

    // The no-scanner arm, before any write: the doors stream in this mode, so
    // a backfill here has nothing to decide and must not invent `skipped`
    // records for history (Wave 54's ruling, restated in scan_backfill::run).
    if !core_api::scan_gate::scanner_is_installed(scanner.as_ref()) {
        println!(
            "\nno scanner is configured (EMAIL_SCAN is not set to clamd), so nothing was scanned \
             and nothing was written. The download doors keep streaming these objects in this \
             mode, exactly as they did before the scan gate existed."
        );
        return Ok(());
    }

    if plan.total == 0 {
        println!("\nnothing to do: every stored object already has an admission verdict.");
        return Ok(());
    }

    if args.dry_run {
        println!(
            "\nDRY RUN — nothing was scanned and nothing was written. Re-run with \
             --confirm {} to scan these objects and record a verdict for each.",
            plan.total
        );
        return Ok(());
    }

    match args.confirm {
        Some(confirmed) if confirmed == plan.total => {}
        Some(confirmed) => bail!(
            "--confirm {confirmed} does not match the {} object(s) with no verdict; nothing was \
             scanned. Re-run --dry-run to read the current plan",
            plan.total
        ),
        None => bail!(
            "--confirm <count> is required to write verdicts; the walk found {} object(s) with no \
             verdict (use --dry-run to see them)",
            plan.total
        ),
    }

    let report = core_api::scan_backfill::run(
        &pool,
        store.as_ref(),
        scanner.as_ref(),
        args.org_id,
        args.batch_size,
        Some(plan.total),
    )
    .await
    .map_err(|error| anyhow::anyhow!("{error}"))?;

    println!("\nverdicts recorded this run:");
    println!("  clean       {}", report.clean);
    println!("  skipped     {}", report.skipped);
    println!("  quarantined {}", report.quarantined);
    println!("  already     {}", report.already);
    println!("  unreadable  {}", report.unreadable);
    for failure in &report.failures {
        eprintln!("  unreadable: {failure}");
    }
    if let Some(reason) = &report.aborted {
        bail!(
            "the scanner stopped answering, so the walk ended there: {reason}. Nothing was \
             recorded for the object it was judging; the rest keep no verdict and the next run \
             picks them up"
        );
    }
    if report.unreadable > 0 {
        println!(
            "\n{} object(s) kept no verdict and are listed above. A file whose bytes are missing \
             or do not match its recorded digest cannot be judged; re-upload it instead.",
            report.unreadable
        );
    }
    Ok(())
}

fn print_plan(plan: &core_api::scan_backfill::BackfillPlan) {
    println!(
        "objects with no admission verdict: {} across {} org(s)",
        plan.total,
        plan.per_org.len()
    );
    for (org_id, count) in &plan.per_org {
        println!("  {org_id}  {count}");
    }
}

/// The scanner this process would send bytes to, from the same two variables
/// core-api reads (`crates/core-api/src/config.rs`: `EMAIL_SCAN=clamd` plus
/// `CLAMD_ADDR`, defaulting to the standard clamd address).
fn scanner_from_env() -> Arc<dyn ScanProvider> {
    scanner_for(
        std::env::var("EMAIL_SCAN").ok().as_deref(),
        std::env::var("CLAMD_ADDR").ok().as_deref(),
    )
}

/// The pure half of [`scanner_from_env`], split out so the mapping can be
/// tested without mutating the process environment.
fn scanner_for(email_scan: Option<&str>, clamd_addr: Option<&str>) -> Arc<dyn ScanProvider> {
    match email_scan {
        Some("clamd") => Arc::new(ClamdScan::new(
            clamd_addr.unwrap_or("127.0.0.1:3310").to_string(),
        )),
        _ => Arc::new(NoopScan::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scanner selection has to agree with core-api's, or the backfill
    /// scans with one adapter while the doors judge with another.
    #[test]
    fn scanner_selection_mirrors_the_api_config() {
        assert_eq!(
            scanner_for(Some("clamd"), None).name(),
            "clamd",
            "EMAIL_SCAN=clamd selects the clamd adapter"
        );
        assert_eq!(
            scanner_for(Some("clamd"), Some("scanner.internal:3310")).name(),
            "clamd",
            "CLAMD_ADDR names where, not whether"
        );
        assert_eq!(
            scanner_for(None, None).name(),
            "none",
            "unset EMAIL_SCAN is the documented none mode"
        );
        assert_eq!(
            scanner_for(Some("none"), Some("scanner.internal:3310")).name(),
            "none",
            "any value but clamd is none mode, as in config.rs"
        );
    }
}
