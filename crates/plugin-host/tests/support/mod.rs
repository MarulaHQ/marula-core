//! Shared seeding for the database-backed conformance tests.
//!
//! These tests need the same four rows before they can say anything: an org, a
//! catalog entry with a pinned version, an enabled install with a grant set, and
//! the surface entry the executor resolves first. Writing that four times is how
//! two tests end up disagreeing about which grant they proved.
//!
//! Nothing here is production code and nothing here is compiled into the crate:
//! `tests/support/mod.rs` is a module each integration test declares, so the
//! helpers are only reachable from the tests that ask for them.

#![allow(dead_code)]

use chrono::Utc;
use core_domain::entities::plugins::{
    PluginCapability, PluginCatalogEntry, PluginInstall, PluginInstallStatus, PluginManifestV2,
    PluginVersion, PLUGIN_RUNTIME_INTERFACE_VERSION,
};
use core_store::{plugin_artifact_key, ObjectStore};
use plugin_host::isolation::ModuleArtifact;
use plugin_host::runtime::{IsolationMode, PluginPool, PoolConfig};
use plugin_host::surface::SurfaceKind;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

/// How one install should be seeded. The defaults are an enabled install with
/// one widget entry and no grants — the deny-by-default starting point.
pub struct Seed {
    pub slug: String,
    pub name: String,
    pub capabilities: Vec<PluginCapability>,
    pub kind: SurfaceKind,
    pub entry_id: String,
    pub entry_label: String,
    pub definition: Value,
    pub status: PluginInstallStatus,
    pub revoked: bool,
    pub artifact: ModuleArtifact,
}

impl Seed {
    /// A widget install holding `capabilities`, with one list widget entry.
    pub fn widget(
        slug: &str,
        artifact: &ModuleArtifact,
        capabilities: &[PluginCapability],
    ) -> Self {
        Seed {
            slug: slug.to_string(),
            name: format!("Conformance probe {slug}"),
            capabilities: capabilities.to_vec(),
            kind: SurfaceKind::Widget,
            entry_id: "probe".to_string(),
            entry_label: "Probe".to_string(),
            definition: json!({ "slot": "ticket_pane", "kind": "list" }),
            status: PluginInstallStatus::Enabled,
            revoked: false,
            artifact: artifact.clone(),
        }
    }

    /// An action install: the entry carries the consented effect the executor
    /// applies, and nothing the guest answers can change it.
    pub fn action(
        slug: &str,
        artifact: &ModuleArtifact,
        capabilities: &[PluginCapability],
        effect: &str,
    ) -> Self {
        Seed {
            kind: SurfaceKind::Action,
            entry_id: "probe_action".to_string(),
            entry_label: "Probe action".to_string(),
            definition: json!({
                "effect": effect,
                "node_contract": { "emits": ["ticket.comment_added"] },
            }),
            ..Seed::widget(slug, artifact, capabilities)
        }
    }

    pub fn disabled(mut self) -> Self {
        self.status = PluginInstallStatus::Disabled;
        self
    }

    pub fn revoked(mut self) -> Self {
        self.revoked = true;
        self
    }
}

/// What the seed produced, with the entry key the executor is asked for.
pub struct Seeded {
    pub org_id: Uuid,
    pub catalog_id: Uuid,
    pub install: PluginInstall,
    pub entry_key: String,
}

/// The manifest the install pinned. It mirrors the seed's capability list and,
/// for an action, declares the action kind under the surface profile — which is
/// what makes the install's pins and its registry row agree.
///
/// Built from JSON rather than from the struct literals: the v2 manifest has more
/// optional fields than this probe needs, and a `.default()` on a struct that
/// gains a field later would silently seed a different manifest.
pub fn manifest(seed: &Seed) -> PluginManifestV2 {
    let surface = match seed.kind {
        SurfaceKind::Widget => json!({
            "widgets": [{
                "id": seed.entry_id,
                "title": seed.entry_label,
                "slot": "ticket_pane",
                "kind": "list",
            }],
        }),
        _ => json!({
            "actions": [{
                "id": seed.entry_id,
                "label": seed.entry_label,
                "description": "Conformance probe action.",
                "effect": "add_internal_comment",
                "params_schema": {
                    "type": "object",
                    "properties": { "body": { "type": "string", "maxLength": 500 } },
                },
            }],
        }),
    };
    serde_json::from_value(json!({
        "name": seed.name,
        "version": "1.0.0",
        "interface_version": PLUGIN_RUNTIME_INTERFACE_VERSION,
        "description": "A conformance probe.",
        "capabilities": seed
            .capabilities
            .iter()
            .map(|capability| capability.as_str())
            .collect::<Vec<_>>(),
        "surface_profile": surface,
    }))
    .expect("the manifest is the v2 shape")
}

/// The admin pool the conformance tests run against.
pub async fn admin_pool(max_connections: u32) -> PgPool {
    let url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("this test needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(std::time::Duration::from_secs(30))
        .connect(&url)
        .await
        .expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    pool
}

/// One org, one catalog entry, one version, one install and one surface entry.
///
/// The module bytes go into the store under the key `lifecycle::load_module`
/// will read, so the install is exercised through the same artifacts a
/// deployment serves rather than a shortcut around the pin.
pub async fn seed_install(pool: &PgPool, store: &dyn ObjectStore, seed: &Seed) -> Seeded {
    let org_id = Uuid::now_v7();
    let catalog_id = Uuid::now_v7();
    let version_id = Uuid::now_v7();
    let install_id = Uuid::now_v7();

    store
        .put(
            &plugin_artifact_key(org_id, &seed.artifact.digest),
            "application/wasm",
            seed.artifact.bytes.clone().into(),
        )
        .await
        .expect("the module is stored under its digest");

    let mut tx = pool.begin().await.expect("transaction");
    core_db::set_org(&mut tx, org_id).await.expect("org scope");
    sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
        .bind(org_id)
        .bind(format!("Conformance org {}", seed.slug))
        .bind(format!("{}-{}", seed.slug, Uuid::now_v7().simple()))
        .execute(&mut *tx)
        .await
        .expect("org row");

    let entry = PluginCatalogEntry {
        id: catalog_id,
        slug: seed.slug.clone(),
        name: seed.name.clone(),
        description: "A conformance probe.".into(),
        publisher: "marula".into(),
        origin_org_id: Some(org_id),
        latest_version: Some("1.0.0".into()),
        interface_version: PLUGIN_RUNTIME_INTERFACE_VERSION,
        revoked_at: seed.revoked.then(Utc::now),
        revoked_reason: seed
            .revoked
            .then(|| "withdrawn by its publisher".to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let version = PluginVersion {
        id: version_id,
        catalog_id,
        version: "1.0.0".into(),
        manifest: manifest(seed),
        module_digest: seed.artifact.digest.clone(),
        created_at: Utc::now(),
    };
    core_db::plugins::create_catalog_entry(&mut tx, &entry, &version)
        .await
        .expect("catalog entry");

    let install = PluginInstall {
        id: install_id,
        org_id,
        catalog_id,
        slug: entry.slug.clone(),
        version: "1.0.0".into(),
        module_digest: seed.artifact.digest.clone(),
        status: seed.status,
        granted_capabilities: seed
            .capabilities
            .iter()
            .map(|capability| capability.as_str().to_string())
            .collect(),
        installed_by: None,
        installed_at: Utc::now(),
        updated_at: Utc::now(),
        revoked_at: None,
        revoked_by: None,
        revoked_reason: None,
        previous_version: None,
        previous_module_digest: None,
        previous_capabilities: Vec::new(),
    };
    core_db::plugins::create_install(&mut tx, &install)
        .await
        .expect("install row");

    let entry_key = format!("plugin.{}.{}", entry.slug, seed.entry_id);
    core_db::plugin_surface::sync_surface_section(
        &mut tx,
        org_id,
        install_id,
        seed.kind.as_str(),
        &[(
            entry_key.clone(),
            seed.entry_label.clone(),
            seed.definition.clone(),
        )],
    )
    .await
    .expect("surface entry");

    tx.commit().await.expect("seed commits");
    Seeded {
        org_id,
        catalog_id,
        install,
        entry_key,
    }
}

/// A board, one open status and one ticket in that org, so an effect has a
/// target that belongs to the tenant under test.
pub async fn seed_ticket(pool: &PgPool, org_id: Uuid) -> Uuid {
    let board_id = Uuid::now_v7();
    let status_id = Uuid::now_v7();
    let ticket_id = Uuid::now_v7();

    let mut tx = pool.begin().await.expect("transaction");
    core_db::set_org(&mut tx, org_id).await.expect("org scope");
    sqlx::query("INSERT INTO boards (id, org_id, name, is_default) VALUES ($1, $2, $3, true)")
        .bind(board_id)
        .bind(org_id)
        .bind(format!("Conformance board {}", Uuid::now_v7().simple()))
        .execute(&mut *tx)
        .await
        .expect("board row");
    sqlx::query(
        "INSERT INTO board_statuses (id, board_id, org_id, name, category, is_entry)
         VALUES ($1, $2, $3, 'New', 'open', true)",
    )
    .bind(status_id)
    .bind(board_id)
    .bind(org_id)
    .execute(&mut *tx)
    .await
    .expect("status row");
    sqlx::query(
        "INSERT INTO tickets (id, org_id, number, board_id, status_id, title, category)
         VALUES ($1, $2, $3, $4, $5, 'Conformance ticket', 'open')",
    )
    .bind(ticket_id)
    .bind(org_id)
    .bind(format!("C{}", Uuid::now_v7().simple()))
    .bind(board_id)
    .bind(status_id)
    .execute(&mut *tx)
    .await
    .expect("ticket row");
    tx.commit().await.expect("ticket seeds commit");
    ticket_id
}

/// The pool slot a caller owns, wired to the worker binary this package builds
/// and to the database the quarantine rows live in.
pub async fn pool_slot(pool: &PgPool, workers: usize) -> plugin_host::exec::PoolSlot {
    use plugin_host::exec::PoolSlot;
    let slot = PoolSlot::new();
    slot.get_with(|| async move {
        PluginPool::start(PoolConfig {
            mode: IsolationMode::Pool,
            workers,
            worker_bin: Some(env!("CARGO_BIN_EXE_marula-plugin-worker").into()),
            health: Some(pool.clone()),
            sandbox_limits: Default::default(),
        })
        .await
    })
    .await
    .expect("the pool starts from this package's worker binary");
    slot
}

/// A guest built from WAT, addressed by its digest.
pub fn artifact(source: &str) -> ModuleArtifact {
    let bytes = wat::parse_str(source).expect("valid WAT");
    ModuleArtifact {
        digest: ModuleArtifact::digest_of(&bytes),
        bytes,
    }
}
