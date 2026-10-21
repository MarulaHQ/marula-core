//! `marula data` — the tenant erasure verb (Wave 52 lane R6).
//!
//! Purpose: `docs/reference/data-lifecycle.md` says what a per-tenant delete
//! means for every surface. This module is the part of that contract a machine
//! executes, so the answer to "did anyone actually delete that tenant" is a
//! receipt row rather than a memory of a psql session.
//!
//! The order of operations is the contract, and it is not interchangeable:
//!
//! 1. Read the org, count every org-scoped table, and collect the object keys —
//!    all BEFORE anything is destroyed, because after the delete there is no row
//!    left to ask.
//! 2. `DELETE FROM orgs`, which cascades through every table that names the org
//!    (including the audit trail, the search index and the RAG vectors).
//! 3. Delete the bytes under the org's store prefix.
//! 4. Write the receipt, whatever step 3 did.
//!
//! Rows first and bytes second, deliberately: a crash between them leaves
//! orphaned bytes, which the object-integrity pass finds and reclaims. The
//! reverse order would leave rows advertising bytes that no longer exist and
//! cannot be recovered.
//!
//! Two things this verb cannot do, and says so instead of implying otherwise:
//! it cannot reach into backups already written (they age out of the backup
//! retention; the contract states the window and the remedy), and it cannot undo
//! itself. Both are why it needs `--confirm` to name the tenant, and why
//! `--dry-run` is the documented first invocation.
//!
//! Non-purpose: no HTTP surface — there is no route in the product that deletes
//! a tenant, by design — no anonymisation mode, and no partial erasure (a
//! surface excluded here would make the receipt a lie).

use anyhow::{bail, Context as _};
use core_db::erasure::{self, ErasureOutcome, Receipt};
use core_store::ObjectStore;
use uuid::Uuid;

/// What the verb was asked to do.
#[derive(Debug, Clone)]
pub struct PurgeArgs {
    pub org_id: Uuid,
    /// Operator identity for the receipt. Required to execute.
    pub requested_by: Option<String>,
    /// Why, in the operator's words. Recorded on the receipt.
    pub reason: Option<String>,
    /// The org's label, repeated back. Executing requires this to match.
    pub confirm: Option<String>,
    /// Report only, never required to name the tenant.
    pub dry_run: bool,
}

/// Executes (or reports) an erasure.
pub async fn purge_org(
    database_url: &str,
    store: &dyn ObjectStore,
    args: &PurgeArgs,
) -> anyhow::Result<()> {
    let pool = core_db::connect(database_url)
        .await
        .context("connecting to the database")?;

    // The cascade is only complete on a role that can see every row and owns the
    // table. The RLS app role would delete zero rows and report success, which
    // is the failure this check exists to prevent.
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
    .fetch_one(&pool)
    .await
    .context("checking the connection's role")?;
    if !owner {
        bail!(
            "this connection cannot erase a tenant: connect as the table owner (the migration \
             role), not the RLS-constrained app role — otherwise the cascade deletes nothing and \
             still reports success"
        );
    }

    let facts = erasure::org_facts(&pool, args.org_id)
        .await
        .context("reading the org")?
        .with_context(|| format!("no org with id {}", args.org_id))?;
    let counts = erasure::surface_counts(&pool, args.org_id)
        .await
        .context("counting org-scoped rows")?;
    let prefix = format!("org/{}", args.org_id);
    let objects = store
        .list(&prefix)
        .await
        .context("listing the org's objects")?;
    let referenced = erasure::object_keys(&pool, args.org_id)
        .await
        .context("collecting object keys")?;

    let total_rows: i64 = counts.values().sum();
    println!("org      {} ({})", facts.label, facts.org_id);
    if let Some(created) = facts.created_at {
        println!("created  {}", created.to_rfc3339());
    }
    println!("users    {}", facts.users);
    println!(
        "rows     {total_rows} across {} org-scoped tables",
        counts.len()
    );
    for (table, count) in &counts {
        println!("         {table:<40} {count}");
    }
    println!(
        "objects  {} in the store under {prefix} ({} keys referenced by rows)",
        objects.len(),
        referenced.len()
    );

    if args.dry_run {
        println!(
            "\nDRY RUN — nothing was deleted. Re-run without --dry-run, with \
             --confirm {} --requested-by <you>, to erase.",
            facts.label
        );
        return Ok(());
    }

    match args.confirm.as_deref() {
        Some(label) if label == facts.label => {}
        Some(label) => bail!(
            "--confirm {label:?} does not match this org ({:?}); nothing was deleted",
            facts.label
        ),
        None => bail!(
            "--confirm <label> is required to erase a tenant; the org's label is {:?} \
             (use --dry-run to see what would go)",
            facts.label
        ),
    }
    let requested_by = args
        .requested_by
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .context("--requested-by is required: the receipt has to say who asked")?
        .to_string();

    let started_at = chrono::Utc::now();
    let purged = erasure::purge_rows(&pool, args.org_id)
        .await
        .context("deleting the org row (and every cascading row)")?;
    if purged == 0 {
        bail!("the org row was already gone; nothing was erased and no receipt was written");
    }
    println!("\nrows deleted: {total_rows} (the org row and its cascade)");

    let mut objects_deleted = 0_i32;
    let mut objects_failed = 0_i32;
    for object in &objects {
        match store.delete(&object.key).await {
            Ok(()) => objects_deleted += 1,
            Err(error) => {
                objects_failed += 1;
                eprintln!("  object delete failed: {} — {error}", object.key);
            }
        }
    }
    println!(
        "objects deleted: {objects_deleted}{}",
        if objects_failed > 0 {
            format!(" ({objects_failed} failed)")
        } else {
            String::new()
        }
    );

    let outcome = if objects_failed == 0 {
        ErasureOutcome::Completed
    } else {
        ErasureOutcome::ObjectsIncomplete
    };
    let receipt = Receipt {
        purged_org_id: args.org_id,
        org_label: facts.label.clone(),
        requested_by,
        started_at,
        finished_at: Some(chrono::Utc::now()),
        rows_deleted: counts,
        objects_deleted,
        objects_failed,
        outcome,
        note: args.reason.clone(),
    };
    let receipt_id = erasure::insert_receipt(&pool, &receipt)
        .await
        .context("writing the erasure receipt")?;
    println!(
        "\nreceipt {receipt_id} — {}\n\
         backups already written are NOT rewritten by an erasure; they age out of the backup \
         retention. See docs/reference/data-lifecycle.md §3.",
        receipt.outcome.as_str()
    );
    if objects_failed > 0 {
        bail!(
            "{objects_failed} objects could not be deleted; the receipt records objects_incomplete. \
             The orphan pass will find them again — investigate before calling the erasure done"
        );
    }
    Ok(())
}
