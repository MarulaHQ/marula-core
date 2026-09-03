//! Fixture-capture tooling (TASKLIST U7, upgrade-compatibility-policy §8).
//!
//! Purpose: support the per-tag upgrade fixtures in `fixtures/upgrades/` —
//! `meridian fixture manifest` emits the U5 customization manifest for one
//! org as JSON, exactly the envelope the API export produces and the U6
//! preflight consumes. Capture procedure (documented in
//! fixtures/upgrades/README.md): migrate a scratch database to the tag,
//! seed the production-like org, dump it, and emit this manifest alongside.
//!
//! Non-purpose: no seeding and no dumping here (those are plain SQL +
//! `pg_dump`, kept as committed files for review); nothing secret-bearing —
//! the manifest loader structurally excludes signing secrets and key hashes.

use anyhow::Context;

/// Emits the org's customization manifest (U5 envelope) as pretty JSON to
/// stdout, or to `out` when given.
pub async fn emit_manifest(
    database_url: &str,
    org_id: uuid::Uuid,
    out: Option<std::path::PathBuf>,
) -> anyhow::Result<()> {
    let pool = core_db::connect(database_url).await?;
    let mut tx = pool
        .begin()
        .await
        .context("beginning manifest transaction")?;
    core_db::set_org(&mut tx, org_id)
        .await
        .context("scoping transaction to org")?;
    let manifest =
        core_db::manifest::load_customization_manifest(&mut tx, org_id, chrono::Utc::now())
            .await
            .context("loading customization manifest")?;
    tx.commit().await?;
    let json = serde_json::to_string_pretty(&manifest).context("serializing manifest")?;
    match out {
        Some(path) => {
            std::fs::write(&path, json + "\n")
                .with_context(|| format!("writing {}", path.display()))?;
            tracing::info!("manifest written to {}", path.display());
        }
        None => println!("{json}"),
    }
    Ok(())
}
