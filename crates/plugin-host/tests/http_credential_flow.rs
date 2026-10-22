//! The COMPOSED credentialed path, against a real database: guest →
//! `http_request` → host-call frame (pipe or channel) →
//! `exec::TransactionHostCalls` → capability re-check → `core_domain::egress`
//! re-screen → `core_db::plugin_credentials::resolve` → the parent transport
//! (W53-12 phase 2).
//!
//! Why this file exists: each layer is proven on its own elsewhere and none of
//! them proves the composition. `core-db`'s suite proves the resolve refuses a
//! withdrawn grant; `egress.rs` proves a refusal reply becomes
//! `plugin_credential_refused`; the core-api tests prove a sealed credential
//! reaches the wire and nothing comes back. What nobody has run is the whole
//! chain, with both isolation transports, where the ORDER is the security
//! property: the capability is re-derived, the URL is re-screened, and the
//! resolve refuses, all before the transport is reached at all.
//!
//! So the transport in this test is a DOUBLE that asserts whether it was called:
//!
//! - granted binding → it is called once per invocation, with the record's
//!   CIPHERTEXT in `SealedCredential` and never the plaintext;
//! - withdrawn grant → it is not called, and the guest sees
//!   `plugin_credential_refused`. That is the sentence "the request never
//!   leaves" as an assertion instead of a claim.
//!
//! It needs a database, so it is `#[ignore]`d and the group gate runs it:
//!
//! ```text
//! cargo test -p plugin-host --test http_credential_flow -- --ignored
//! ```
//!
//! `cargo test -p plugin-host` stays green without a database. The URL comes
//! from `MARULA_TEST_DATABASE_URL` or `DATABASE_URL`. Nothing resets the schema:
//! the test makes its own org, folder, record and installs.

use std::sync::{Arc, Mutex};

use chrono::Utc;
use core_db::plugin_credentials::{self, BindOutcome, NewBinding, ResolveOutcome};
use core_domain::entities::plugins::{
    ConnectorProfile, PluginCapability, PluginCatalogEntry, PluginInstall, PluginInstallStatus,
    PluginManifestV2, PluginSurfaceProfile, PluginVersion, SurfaceSlot, SurfaceWidget,
    SurfaceWidgetKind, PLUGIN_RUNTIME_INTERFACE_VERSION,
};
use core_store::memory::MemoryStore;
use core_store::{plugin_artifact_key, ObjectStore};
use plugin_host::abi::{HttpRequest, HttpResponse, Refusal};
use plugin_host::egress::{HttpEgress, SealedCredential};
use plugin_host::exec::{invoke_surface_with, PoolSlot, SurfaceCall, SurfaceOutcome};
use plugin_host::isolation::ModuleArtifact;
use plugin_host::runtime::{IsolationMode, PluginPool, PoolConfig};
use plugin_host::surface::SurfaceKind;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// A guest that asks for `https://api.acme.test/v1` under the `acme` profile and
/// returns the reply frame unchanged. Memory: profile at 0 (`acme`, 4 bytes),
/// the JSON request at 16 (49 bytes), `marula_alloc` hands out space from 1024.
const ASK_ACME: &str = r#"
    (module
      (import "marula" "http_request" (func $http (param i32 i32 i32 i32) (result i64)))
      (memory (export "memory") 1)
      (data (i32.const 0) "acme")
      (data (i32.const 16) "{\"method\":\"GET\",\"url\":\"https://api.acme.test/v1\"}")
      (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
      (func (export "marula_free") (param i32) (param i32))
      (func (export "marula_entry") (param i32) (param i32) (result i64)
        (call $http (i32.const 0) (i32.const 4) (i32.const 16) (i32.const 49))))
"#;

/// The transport the executor hands the request to. It records every call and
/// answers a fixed body; whether it was called is the assertion.
#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<(String, Option<SealedCredential>)>>,
}

impl HttpEgress for Recorder {
    fn send<'a>(
        &'a mut self,
        target: &'a core_domain::egress::EgressTarget,
        _request: &'a HttpRequest,
        credential: Option<&'a SealedCredential>,
        _timeout: std::time::Duration,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<HttpResponse, Refusal>> + Send + 'a>,
    > {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((target.as_str().to_string(), credential.cloned()));
        Box::pin(async move {
            Ok(HttpResponse {
                status: 200,
                headers: Default::default(),
                body: "{\"ok\":true}".to_string(),
            })
        })
    }
}

impl Recorder {
    fn count(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

fn artifact(source: &str) -> ModuleArtifact {
    let bytes = wat::parse_str(source).expect("valid WAT");
    ModuleArtifact {
        digest: ModuleArtifact::digest_of(&bytes),
        bytes,
    }
}

fn manifest() -> PluginManifestV2 {
    serde_json::from_value(json!({
        "name": "Credentialed egress probe",
        "version": "1.0.0",
        "interface_version": PLUGIN_RUNTIME_INTERFACE_VERSION,
        "description": "Calls one allowlisted host under one egress profile.",
        "capabilities": [
            PluginCapability::UiWidgets.as_str(),
            PluginCapability::AutomationReads.as_str()
        ],
        "connector_profile": ConnectorProfile {
            webhook_endpoints: Vec::new(),
            scopes: Vec::new(),
            routing: None,
            outbound_scopes: [(
                "acme".to_string(),
                vec!["https://api.acme.test/v1".to_string()],
            )]
            .into_iter()
            .collect(),
        },
        "surface_profile": PluginSurfaceProfile {
            widgets: vec![SurfaceWidget {
                id: "egress_probe".into(),
                title: "Egress probe".into(),
                slot: SurfaceSlot::TicketPane,
                kind: SurfaceWidgetKind::List,
                props_schema: None,
                default_props: None,
            }],
            ..PluginSurfaceProfile::default()
        },
    }))
    .expect("the manifest is the v2 shape")
}

async fn admin_pool() -> PgPool {
    let url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("the composed egress test needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    pool
}

struct Seeded {
    org_id: Uuid,
    install: PluginInstall,
    entry_key: String,
    folder_id: Uuid,
    record_id: Uuid,
    owner_role: Uuid,
    sealed: Vec<u8>,
}

/// One org, one restricted folder holding one record with a `secret` field, one
/// grant for the owner role, and one enabled install whose manifest declares the
/// `acme` profile.
async fn seed(pool: &PgPool, store: &dyn ObjectStore, slug: &str) -> Seeded {
    let org_id = Uuid::now_v7();
    let catalog_id = Uuid::now_v7();
    let install_id = Uuid::now_v7();
    let probe = artifact(ASK_ACME);
    store
        .put(
            &plugin_artifact_key(org_id, &probe.digest),
            "application/wasm",
            probe.bytes.clone().into(),
        )
        .await
        .expect("the module is stored under its digest");

    let mut tx = pool.begin().await.expect("transaction");
    core_db::set_org(&mut tx, org_id).await.expect("org scope");
    sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
        .bind(org_id)
        .bind("Credentialed egress org")
        .bind(slug)
        .execute(&mut *tx)
        .await
        .expect("org row");
    let roles = core_db::orgs::seed_default_roles(&mut tx, org_id)
        .await
        .expect("default roles");
    let owner_role = roles
        .get(core_domain::policy::BuiltinRole::Owner.as_str())
        .copied()
        .expect("owner role");

    let entry = PluginCatalogEntry {
        id: catalog_id,
        slug: format!("egress-probe-{slug}"),
        name: "Credentialed egress probe".into(),
        description: "Calls one allowlisted host.".into(),
        publisher: "marula".into(),
        origin_org_id: Some(org_id),
        latest_version: Some("1.0.0".into()),
        interface_version: PLUGIN_RUNTIME_INTERFACE_VERSION,
        revoked_at: None,
        revoked_reason: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let version = PluginVersion {
        id: Uuid::now_v7(),
        catalog_id,
        version: "1.0.0".into(),
        manifest: manifest(),
        module_digest: probe.digest.clone(),
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
        module_digest: probe.digest.clone(),
        status: PluginInstallStatus::Enabled,
        granted_capabilities: vec![
            PluginCapability::UiWidgets.as_str().to_string(),
            PluginCapability::AutomationReads.as_str().to_string(),
        ],
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
    core_db::plugin_surface::sync_surface_section(
        &mut tx,
        org_id,
        install_id,
        SurfaceKind::Widget.as_str(),
        &[(
            format!("plugin.{}.egress_probe", entry.slug),
            "Egress probe".to_string(),
            json!({ "slot": "ticket_pane", "kind": "list" }),
        )],
    )
    .await
    .expect("surface entry");

    // A restricted folder: the category grant is what the bind and the resolve
    // both consult, so a folder nobody granted exercises the real gate.
    let seed = core_db::secret_vault::list_org_folders(&mut tx, org_id)
        .await
        .expect("seeded folders")
        .into_iter()
        .find(|folder| folder.slug == "documents")
        .expect("the documents seed folder");
    let folder_id = Uuid::now_v7();
    core_db::secret_vault::create_folder(
        &mut tx,
        org_id,
        None,
        Some(seed.id),
        folder_id,
        "vendor-keys",
        "Vendor API keys",
        core_domain::entities::SecretFolderKind::ApiKeys,
        false,
        true,
        Uuid::now_v7(),
    )
    .await
    .expect("restricted folder");
    core_db::secret_vault::upsert_grant(
        &mut tx,
        org_id,
        owner_role,
        folder_id,
        true,
        false,
        Uuid::now_v7(),
    )
    .await
    .expect("read grant");
    let record_id = Uuid::now_v7();
    // The bytes are a marker, not a seal: this crate has no key. What the test
    // needs is that the executor carries the RECORD'S BYTES through, and that
    // they are not the plaintext it stands for.
    let sealed = b"sealed::acme-tenant-token".to_vec();
    core_db::secret_vault::create_record(
        &mut tx,
        org_id,
        None,
        folder_id,
        record_id,
        core_domain::entities::SecretRecordKind::ApiKey,
        "Acme tenant token",
        None,
        None,
        None,
        None,
        None,
        &[("secret".to_string(), sealed.clone())],
        Uuid::now_v7(),
    )
    .await
    .expect("record");
    let bound = plugin_credentials::bind(
        &mut tx,
        org_id,
        install_id,
        &NewBinding {
            profile: "acme",
            secret_record_id: record_id,
            header_name: "x-api-key",
            bound_by: None,
            bound_role_ids: vec![owner_role],
        },
    )
    .await
    .expect("bind");
    assert!(
        matches!(bound, BindOutcome::Bound(_)),
        "the fixture's bind must succeed, got {bound:?}"
    );
    tx.commit().await.expect("seed commits");

    Seeded {
        org_id,
        entry_key: format!("plugin.{}.egress_probe", install.slug),
        install,
        folder_id,
        record_id,
        owner_role,
        sealed,
    }
}

async fn slot(pool: &PgPool, mode: IsolationMode) -> PoolSlot {
    let slot = PoolSlot::new();
    slot.get_with(|| async move {
        PluginPool::start(PoolConfig {
            mode,
            workers: 1,
            worker_bin: Some(env!("CARGO_BIN_EXE_marula-plugin-worker").into()),
            health: Some(pool.clone()),
            sandbox_limits: Default::default(),
        })
        .await
    })
    .await
    .expect("the pool starts");
    slot
}

async fn ask(
    pool: &PgPool,
    store: &dyn ObjectStore,
    slot: &PoolSlot,
    seeded: &Seeded,
    egress: &mut Recorder,
) -> SurfaceOutcome {
    let mut tx = pool.begin().await.expect("transaction");
    core_db::set_org(&mut tx, seeded.org_id)
        .await
        .expect("org scope");
    let outcome = invoke_surface_with(
        &mut tx,
        pool,
        store,
        slot,
        egress,
        &SurfaceCall {
            org_id: seeded.org_id,
            entry_key: &seeded.entry_key,
            surface: SurfaceKind::Widget,
            params: &json!({}),
            ticket_id: None,
        },
    )
    .await
    .expect("the invocation runs");
    tx.commit().await.expect("commit");
    outcome
}

/// Both transports, one scenario: the credential goes out, and a withdrawn grant
/// stops the next call before the transport is reached.
#[tokio::test]
#[ignore = "needs Postgres via MARULA_TEST_DATABASE_URL or DATABASE_URL; run with --ignored at the gate"]
async fn a_bound_credential_reaches_the_transport_and_a_withdrawn_grant_stops_it() {
    let pool = admin_pool().await;
    let store = Arc::new(MemoryStore::new());
    let slug = format!("egress-{}", Uuid::now_v7().simple());
    let seeded = seed(&pool, &*store, &slug).await;

    for mode in [IsolationMode::InProcess, IsolationMode::Pool] {
        let slot = slot(&pool, mode).await;
        let mut egress = Recorder::default();

        // --- granted: the transport is reached, with ciphertext ------------
        let outcome = ask(&pool, &*store, &slot, &seeded, &mut egress).await;
        assert_eq!(
            outcome.outcome,
            plugin_host::abi::InvocationOutcome::Ok,
            "{mode:?}: {:?}",
            outcome.reason
        );
        assert_eq!(egress.count(), 1, "{mode:?}: the transport is called once");
        {
            let calls = egress
                .calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let (url, credential) = &calls[0];
            assert_eq!(url, "https://api.acme.test/v1", "the screened target");
            let credential = credential
                .as_ref()
                .expect("the guest's profile has a binding");
            assert_eq!(credential.header_name, "x-api-key");
            assert_eq!(
                credential.sealed, seeded.sealed,
                "the resolve hands the RECORD'S BYTES to the transport"
            );
            assert!(
                !credential
                    .sealed
                    .windows(15)
                    .any(|w| w == b"acme-tenant-tok"),
                "and never a plaintext form of them"
            );
        }

        // --- withdrawn: the transport is NOT reached -----------------------
        let mut tx = pool.begin().await.expect("transaction");
        core_db::set_org(&mut tx, seeded.org_id)
            .await
            .expect("org scope");
        core_db::secret_vault::delete_grant(
            &mut tx,
            seeded.org_id,
            seeded.owner_role,
            seeded.folder_id,
        )
        .await
        .expect("the grant is withdrawn");
        tx.commit().await.expect("commit");

        let mut egress = Recorder::default();
        let outcome = ask(&pool, &*store, &slot, &seeded, &mut egress).await;
        assert_eq!(
            outcome.outcome,
            plugin_host::abi::InvocationOutcome::Refused,
            "{mode:?}: a withdrawn grant is a refusal"
        );
        assert_eq!(
            outcome.skip_reason(),
            "plugin_credential_refused",
            "{mode:?}: code 25, one code for every cause"
        );
        assert_eq!(
            egress.count(),
            0,
            "{mode:?}: the request never left — the transport was not called"
        );

        // --- unbound: the request still goes out, WITHOUT a credential -----
        //
        // A profile nobody bound is not a refusal. The profile is consented and
        // the host is allowlisted, so this is the uncredentialed call Wave 53
        // shipped; the binding's existence is what makes a credential required.
        let mut tx = pool.begin().await.expect("transaction");
        core_db::set_org(&mut tx, seeded.org_id)
            .await
            .expect("org scope");
        core_db::secret_vault::upsert_grant(
            &mut tx,
            seeded.org_id,
            seeded.owner_role,
            seeded.folder_id,
            true,
            false,
            Uuid::now_v7(),
        )
        .await
        .expect("grant restored");
        plugin_credentials::unbind(&mut tx, seeded.org_id, seeded.install.id, "acme")
            .await
            .expect("consent withdrawn by the operator");
        tx.commit().await.expect("commit");

        let mut egress = Recorder::default();
        let outcome = ask(&pool, &*store, &slot, &seeded, &mut egress).await;
        assert_eq!(
            outcome.outcome,
            plugin_host::abi::InvocationOutcome::Ok,
            "{mode:?}: an unbound profile is an uncredentialed call, not a refusal"
        );
        assert_eq!(egress.count(), 1, "{mode:?}: the request went out");
        {
            let calls = egress
                .calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            assert!(
                calls[0].1.is_none(),
                "{mode:?}: no credential was presented, because none is bound"
            );
        }

        // Bind again for the next transport's run.
        let mut tx = pool.begin().await.expect("transaction");
        core_db::set_org(&mut tx, seeded.org_id)
            .await
            .expect("org scope");
        let rebound = plugin_credentials::bind(
            &mut tx,
            seeded.org_id,
            seeded.install.id,
            &NewBinding {
                profile: "acme",
                secret_record_id: seeded.record_id,
                header_name: "x-api-key",
                bound_by: None,
                bound_role_ids: vec![seeded.owner_role],
            },
        )
        .await
        .expect("rebind");
        assert!(matches!(rebound, BindOutcome::Bound(_)), "{mode:?}");
        tx.commit().await.expect("commit");
    }
}

/// The resolve itself, at the layer the executor calls: a granted binding answers
/// with the record's bytes, and the same call after the grant is withdrawn
/// answers `GrantRevoked` rather than bytes.
#[tokio::test]
#[ignore = "needs Postgres via MARULA_TEST_DATABASE_URL or DATABASE_URL; run with --ignored at the gate"]
async fn the_resolve_the_executor_calls_refuses_a_withdrawn_grant() {
    let pool = admin_pool().await;
    let store = Arc::new(MemoryStore::new());
    let slug = format!("egress-resolve-{}", Uuid::now_v7().simple());
    let seeded = seed(&pool, &*store, &slug).await;

    let mut tx = pool.begin().await.expect("transaction");
    core_db::set_org(&mut tx, seeded.org_id)
        .await
        .expect("org scope");
    match plugin_credentials::resolve(&mut tx, seeded.org_id, seeded.install.id, "acme")
        .await
        .expect("resolve")
    {
        ResolveOutcome::Granted(credential) => {
            assert_eq!(credential.sealed, seeded.sealed);
            assert_eq!(credential.record_id, seeded.record_id);
        }
        other => panic!("a granted binding must resolve, got {other:?}"),
    }
    core_db::secret_vault::delete_grant(
        &mut tx,
        seeded.org_id,
        seeded.owner_role,
        seeded.folder_id,
    )
    .await
    .expect("withdraw");
    assert!(
        matches!(
            plugin_credentials::resolve(&mut tx, seeded.org_id, seeded.install.id, "acme")
                .await
                .expect("resolve"),
            ResolveOutcome::GrantRevoked
        ),
        "the resolve re-checks the grant instead of trusting the consent"
    );
    tx.commit().await.expect("commit");
}
