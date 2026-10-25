//! Demo-org seeding and dev QA loops (the private build tracker DX1–DX2).
//!
//! Purpose: ONE deterministic demo dataset callable from two entrypoints —
//! `marula-core-cli seed-demo` (DX1) and the env-gated `/api/dev/*` surface
//! (DX2). Fixed UUIDs/emails make re-runs stable: seeding is all-or-nothing
//! per org and an existing demo org short-circuits with
//! [`DemoError::AlreadySeeded`] instead of double-applying.
//!
//! Dev-tool posture: direct SQL through the ordinary tenant schema (no new
//! migration — 0045 stays head). Domain invariants are reused where the
//! shared query layer already exposes them (ticket number allocation,
//! category denormalization via `tickets::insert_ticket`, builtin role
//! permission sets); everything else is plain inserts under one
//! `app.org_id`-bound transaction so RLS treats it exactly like tenant
//! traffic.
//!
//! Provenance (IF9 seam): every domain-entity write the seeder performs
//! lands an `entity_writes` row with `actor_kind = 'system'` and a matching
//! `entity_write_seqs` allocation, so "who did this" on demo data answers
//! honestly — a dev write — through the same trail the product reads.
//! Handler-level actions additionally append `audit_events` rows as
//! `Actor::System`.
//!
//! Non-purpose: not a fixture framework (no DSL, no partial sets), not a
//! production data generator (content is hand-written demo narrative), and
//! NOT a production bootstrap — the whole module is only reachable behind
//! `MARULA_DEV_TOOLS=1` (CLI refuses without it; the API router is never
//! mounted).

use crate::{set_dev_reset_bypass, set_org, DbError, DbResult};
use chrono::{Datelike, Duration, NaiveDate, NaiveTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

/// Deterministic demo UUID `n`: the ASCII bytes of "marula" followed by a
/// 64-bit counter. Stable across processes by construction.
pub const fn demo_uuid(n: u64) -> Uuid {
    Uuid::from_u128(0x6D65_7269_6469_616E_u128 << 64 | n as u128)
}

/// The demo org's fixed id ("Marula Demo", slug `marula-demo`).
pub const DEMO_ORG_ID: Uuid = demo_uuid(1);
pub const DEMO_ORG_NAME: &str = "Marula Demo";
pub const DEMO_ORG_SLUG: &str = "marula-demo";
pub const DEMO_TICKET_PREFIX: &str = "DEMO";
/// Shared demo login password (argon2-hashed once per seed run).
pub const DEMO_PASSWORD: &str = "demo-password-123";
/// Deterministic alert-source HMAC secret. Dev-only; printed once at seed
/// end so demo ingest POSTs can be signed.
pub const DEMO_ALERT_HMAC_SECRET: &str = "demo-alert-hmac-secret";
/// Deterministic webhook signing secret (same dev-only posture).
pub const DEMO_WEBHOOK_SECRET: &str = "demo-webhook-secret";

/// `entity_writes.actor_id` is NOT NULL (0028); system provenance uses the
/// nil UUID so the trail never points at a demo human.
const SYSTEM_ACTOR_ID: Uuid = Uuid::nil();

/// Key material for sealing demo webhook/alert-source secrets. MUST be the
/// same key core-api runs with, or the test-fire/ingest paths cannot open
/// the ciphertext.
#[derive(Debug, Clone)]
pub struct DemoSeedOptions {
    pub seal_key: [u8; 32],
    /// Where to write the demo attachments' bytes. Set this to the SAME
    /// directory core-api reads from (`OBJECT_STORE_LOCAL_DIR`) or the
    /// download endpoint will have a row with no object behind it.
    ///
    /// `None` still writes the metadata rows, so attachment lists, type
    /// badges and counts all render; only the byte fetch fails, which the
    /// route reports as a missing object rather than corrupting anything.
    /// Tests use `None` because they never download.
    pub object_store_dir: Option<std::path::PathBuf>,
}

/// The dev at-rest key derivation, shared with core-api's config (one
/// source of truth for the dev fallback: SHA-256 over a fixed label plus
/// the JWT secret).
pub fn derive_dev_seal_key(jwt_secret: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"marula-at-rest-secret-key:");
    hasher.update(jwt_secret.as_bytes());
    hasher.finalize().into()
}

#[derive(Debug, thiserror::Error)]
pub enum DemoError {
    #[error("demo org already seeded (org_id {0}) — POST /api/dev/reset-org or use --force to wipe and reseed")]
    AlreadySeeded(Uuid),
    #[error("hours must be between 1 and 720")]
    InvalidHours,
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Sql(#[from] sqlx::Error),
    #[error("secret sealing: {0}")]
    Crypto(String),
    #[error("demo object store: {0}")]
    Internal(String),
}

pub type DemoResult<T> = Result<T, DemoError>;

/// What one seed pass produced. Drives the CLI summary table and the dev
/// API response body; the two secret fields are dev-only and printed once.
#[derive(Debug, Serialize)]
pub struct DemoSeedReport {
    pub org_id: Uuid,
    pub org_name: &'static str,
    pub slug: &'static str,
    pub users: usize,
    pub clients: usize,
    /// One portal login per client (Wave 47 B3) — the number a prospect's
    /// own staff can actually sign in with.
    pub portal_users: usize,
    pub boards: usize,
    pub tickets: usize,
    pub comments: usize,
    pub checklist_items: usize,
    pub timer_sessions: usize,
    pub time_entries: usize,
    pub contracts: usize,
    pub invoices: usize,
    pub kb_articles: usize,
    pub documents: usize,
    pub projects: usize,
    pub appointments: usize,
    pub alert_sources: usize,
    pub alerts: usize,
    pub alert_incidents: usize,
    pub report_schedules: usize,
    pub webhooks: usize,
    /// Counts for the second wave of surfaces (2026-09 seed expansion).
    pub assets: usize,
    pub catalog_items: usize,
    pub quotes: usize,
    pub pipeline_deals: usize,
    pub credit_notes: usize,
    pub tags: usize,
    pub api_keys: usize,
    pub vault_entries: usize,
    pub notifications: usize,
    pub pm_schedules: usize,
    pub automation_rules: usize,
    pub plugin_installs: usize,
    /// Field-ops fixtures promoted from the assets-stock lane's capture
    /// script (Wave 43 lane 7): the stock cluster's loans and purchasing
    /// legs carry real severity states.
    pub vendors: usize,
    pub purchase_orders: usize,
    pub client_loans: usize,
    /// Ticket, comment and document attachments. The byte payloads live in
    /// the object store, so this counts rows and the size badge the UI shows;
    /// `object_bytes` is the total stored, which is 0 when no store was
    /// configured for the seed run.
    pub attachments: usize,
    pub document_attachments: usize,
    pub attachment_store_bytes: u64,
    /// Email edge, both directions.
    pub email_messages: usize,
    pub email_outbound: usize,
    /// CSAT, the secrets manager, contract legs, the time taxonomy and the
    /// purchasing/lending completion legs.
    pub csat_surveys: usize,
    pub secret_records: usize,
    pub contract_change_orders: usize,
    pub time_categories: usize,
    pub purchase_receipts: usize,
    pub purchase_bills: usize,
    pub client_loan_lines: usize,
    /// (email, role) login pairs, in summary-table order.
    pub logins: Vec<(&'static str, &'static str)>,
    /// (email, client name) — one portal seat per client.
    pub portal_logins: Vec<(&'static str, &'static str)>,
    pub password: &'static str,
    pub alert_hmac_secret: &'static str,
    pub webhook_signing_secret: &'static str,
}

/// True when the demo org row exists (org-bound RLS scope applied first).
pub async fn is_seeded(pool: &PgPool) -> DbResult<bool> {
    let mut tx = pool.begin().await?;
    set_org(&mut tx, DEMO_ORG_ID).await?;
    let seeded: Option<i32> = sqlx::query_scalar("SELECT 1 FROM orgs WHERE id = $1")
        .bind(DEMO_ORG_ID)
        .fetch_optional(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(seeded.is_some())
}

/// Seeds the demo org. Idempotent at the org level: an existing demo org is
/// reported as [`DemoError::AlreadySeeded`] and nothing is written.
pub async fn seed(pool: &PgPool, opts: &DemoSeedOptions) -> DemoResult<DemoSeedReport> {
    if is_seeded(pool).await? {
        return Err(DemoError::AlreadySeeded(DEMO_ORG_ID));
    }
    seed_inner(pool, opts).await
}

/// Wipes the demo org (single cascading delete of the org row) and reseeds
/// from scratch. Safe on a fresh database too (delete of a missing org is a
/// no-op).
pub async fn reset(pool: &PgPool, opts: &DemoSeedOptions) -> DemoResult<DemoSeedReport> {
    let mut tx = pool.begin().await?;
    set_org(&mut tx, DEMO_ORG_ID).await?;
    // The ledger tables are append-only by design, so the cascade that wipes
    // the demo org needs the dev-reset escape (migration 0119). Without it a
    // single stock movement makes this tool a one-way door.
    set_dev_reset_bypass(&mut tx).await?;
    sqlx::query("DELETE FROM orgs WHERE id = $1")
        .bind(DEMO_ORG_ID)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    seed_inner(pool, opts).await
}

/// Shifts the demo org's SLA clock columns back by `hours` (negative
/// budgets ⇒ breach territory) so first-response/resolution ladders and
/// breach paths can be exercised on demand. Returns the number of shifted
/// SLA-state rows. Non-closed tickets only; closed tickets keep history.
pub async fn time_travel(pool: &PgPool, hours: i64) -> DemoResult<u64> {
    if !(1..=24 * 30).contains(&hours) {
        return Err(DemoError::InvalidHours);
    }
    let mut tx = pool.begin().await?;
    set_org(&mut tx, DEMO_ORG_ID).await?;
    let result = sqlx::query(
        "UPDATE ticket_sla_state s
         SET first_response_due = s.first_response_due - make_interval(hours => $2::int),
             resolution_due = s.resolution_due - make_interval(hours => $2::int),
             updated_at = now()
         WHERE s.org_id = $1
           AND s.breached = false
           AND EXISTS (
             SELECT 1 FROM tickets t
             JOIN board_statuses st ON st.id = t.status_id
             WHERE t.id = s.ticket_id AND st.category <> 'closed'
           )",
    )
    .bind(DEMO_ORG_ID)
    .bind(hours)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(result.rows_affected())
}

/// The last `limit` outbox envelopes for the demo org, newest first — the
/// dev event-stream viewer's data source.
#[derive(Debug, Serialize)]
pub struct DevEvent {
    pub id: Uuid,
    pub event_type: String,
    pub entity: String,
    pub entity_id: Option<String>,
    pub occurred_at: chrono::DateTime<Utc>,
    pub published_at: Option<chrono::DateTime<Utc>>,
    pub payload: serde_json::Value,
}

pub async fn recent_events(pool: &PgPool, limit: i64) -> DbResult<Vec<DevEvent>> {
    let mut tx = pool.begin().await?;
    set_org(&mut tx, DEMO_ORG_ID).await?;
    let rows = sqlx::query(
        r#"SELECT id, event_type, entity, entity_id, occurred_at, published_at, payload
           FROM outbox_events
           WHERE org_id = $1
           ORDER BY occurred_at DESC, id DESC
           LIMIT $2"#,
    )
    .bind(DEMO_ORG_ID)
    .bind(limit.clamp(1, 500))
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows
        .into_iter()
        .map(|row| DevEvent {
            id: row.get("id"),
            event_type: row.get("event_type"),
            entity: row.get("entity"),
            entity_id: row.get("entity_id"),
            occurred_at: row.get("occurred_at"),
            published_at: row.get("published_at"),
            payload: row.get("payload"),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Seed pass
// ---------------------------------------------------------------------------

/// The whole demo org, one transaction, one wall-clock anchor. Every id the
/// dataset references is fixed (see the offset constants) so the graph of
/// foreign keys is stable by construction.
async fn seed_inner(pool: &PgPool, opts: &DemoSeedOptions) -> DemoResult<DemoSeedReport> {
    let mut tx = pool.begin().await?;
    set_org(&mut tx, DEMO_ORG_ID).await?;

    let now = Utc::now();
    let today = now.date_naive();

    // --- org -------------------------------------------------------------
    crate::orgs::insert_org(
        &mut *tx,
        DEMO_ORG_ID,
        DEMO_ORG_NAME,
        DEMO_ORG_SLUG,
        DEMO_TICKET_PREFIX,
    )
    .await?;

    // Standard taxonomy baseline (Wave 11 lane 3): the demo org gets the
    // same client-agnostic IT/MSP trees a registered org would.
    crate::orgs::seed_default_taxonomy(&mut tx, DEMO_ORG_ID).await?;
    // Time-category taxonomy (Wave 39 lane 1): the ten standard slugs.
    crate::time_categories::seed_default_time_categories(&mut tx, DEMO_ORG_ID).await?;

    // --- builtin roles (fixed ids, shared permission sets) ----------------
    let mut role_ids: Vec<(String, Uuid)> = Vec::new();
    for (index, (name, builtin, permissions, is_system)) in
        crate::orgs::default_roles().into_iter().enumerate()
    {
        let id = demo_uuid(2 + index as u64);
        sqlx::query(
            "INSERT INTO roles (id, org_id, name, builtin, permissions, is_system)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(DEMO_ORG_ID)
        .bind(&name)
        .bind(builtin.map(|b| b.as_str().to_string()))
        .bind(&permissions)
        .bind(is_system)
        .execute(&mut *tx)
        .await?;
        role_ids.push((name, id));
    }
    let role = |name: &str| -> Uuid {
        role_ids
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, id)| *id)
            .expect("builtin role always present")
    };

    // --- SLA policy (the orgs.rs default, pinned to a fixed id) -----------
    let sla_policy_id = demo_uuid(7);
    let sla_targets = serde_json::json!({
        "p1": { "first_response_mins": 15, "resolution_mins": 480 },
        "p2": { "first_response_mins": 60, "resolution_mins": 1440 },
        "p3": { "first_response_mins": 240, "resolution_mins": 4320 },
        "p4": { "first_response_mins": 480, "resolution_mins": 10080 }
    });
    let sla_calendar = serde_json::json!({
        "name": "9to5 M-F",
        "timezone": "UTC",
        "windows": [
            ["09:00","17:00"],["09:00","17:00"],["09:00","17:00"],
            ["09:00","17:00"],["09:00","17:00"], null, null
        ],
        "holidays": []
    });
    sqlx::query(
        "INSERT INTO sla_policies (id, org_id, name, is_default, targets, calendar)
         VALUES ($1, $2, 'Standard', true, $3, $4)",
    )
    .bind(sla_policy_id)
    .bind(DEMO_ORG_ID)
    .bind(&sla_targets)
    .bind(&sla_calendar)
    .execute(&mut *tx)
    .await?;

    // Docs-platform v1 (Wave 11 lane 7): the demo org gets the standard
    // system layout set like any registered org.
    crate::document_layouts::seed_default_document_layouts(&mut tx, DEMO_ORG_ID).await?;

    // --- users (argon2 hashed once, shared) -------------------------------
    let password_hash = hash_demo_password()?;
    let users: [(&str, &str, u64, &str); 4] = [
        ("owner@demo.local", "Fiona Owner", 10, "Owner"),
        (
            "dispatcher@demo.local",
            "Diego Dispatcher",
            11,
            "Dispatcher",
        ),
        ("tech@demo.local", "Tessa Technician", 12, "Technician"),
        // The builtin set has no narrower billing-only seat; the bookkeeper
        // login takes Admin so invoice/payment demos work out of the box.
        ("bookkeeper@demo.local", "Bella Bookkeeper", 13, "Admin"),
    ];
    for (email, display_name, id_offset, role_name) in users {
        let user_id = demo_uuid(id_offset);
        sqlx::query(
            "INSERT INTO users (id, org_id, email, display_name, password_hash, timezone)
             VALUES ($1, $2, $3, $4, $5, 'UTC')",
        )
        .bind(user_id)
        .bind(DEMO_ORG_ID)
        .bind(email)
        .bind(display_name)
        .bind(&password_hash)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)")
            .bind(user_id)
            .bind(role(role_name))
            .execute(&mut *tx)
            .await?;
        trail_system_write(
            &mut tx,
            "user",
            user_id,
            serde_json::json!({ "email": email, "display_name": display_name }),
        )
        .await?;
    }
    let owner = demo_uuid(10);
    let dispatcher = demo_uuid(11);
    let tech = demo_uuid(12);
    let bookkeeper = demo_uuid(13);

    // --- clients / sites / contacts / portal contact ----------------------
    struct DemoClient {
        id_offset: u64,
        /// Global id-offset base for this client's contacts (unique per
        /// client so contact UUIDs can never collide across clients).
        contact_base: u64,
        name: &'static str,
        site: Option<(&'static str, &'static str)>,
        contacts: &'static [(&'static str, &'static str, bool)],
    }
    let client_specs: [DemoClient; 3] = [
        DemoClient {
            id_offset: 20,
            contact_base: 44,
            name: "Acme Manufacturing",
            site: Some(("Acme HQ", "100 Foundry Rd, Dayton")),
            contacts: &[
                ("Dana Reeve", "dana@acme.demo", true),
                ("Marco Pol", "marco@acme.demo", false),
            ],
        },
        DemoClient {
            id_offset: 21,
            contact_base: 47,
            name: "Northwind Legal",
            site: Some(("Northwind Office", "50 Essex St, Boston")),
            contacts: &[("Priya Natarajan", "priya@northwind.demo", true)],
        },
        DemoClient {
            id_offset: 22,
            contact_base: 48,
            name: "Globex Foods",
            site: None,
            contacts: &[("Chris Vale", "chris@globex.demo", true)],
        },
    ];
    for spec in &client_specs {
        let client_id = demo_uuid(spec.id_offset);
        sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, $3)")
            .bind(client_id)
            .bind(DEMO_ORG_ID)
            .bind(spec.name)
            .execute(&mut *tx)
            .await?;
        let site_id = spec.site.map(|(_, _)| demo_uuid(spec.id_offset + 10));
        if let Some((site_name, address)) = spec.site {
            sqlx::query(
                "INSERT INTO sites (id, org_id, client_id, name, address, timezone)
                 VALUES ($1, $2, $3, $4, $5, 'UTC')",
            )
            .bind(site_id)
            .bind(DEMO_ORG_ID)
            .bind(client_id)
            .bind(site_name)
            .bind(address)
            .execute(&mut *tx)
            .await?;
        }
        for (index, (contact_name, email, is_primary)) in spec.contacts.iter().enumerate() {
            sqlx::query(
                "INSERT INTO contacts (id, org_id, client_id, site_id, name, email, is_primary)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(demo_uuid(spec.contact_base + index as u64))
            .bind(DEMO_ORG_ID)
            .bind(client_id)
            .bind(site_id)
            .bind(contact_name)
            .bind(email)
            .bind(is_primary)
            .execute(&mut *tx)
            .await?;
        }
        trail_system_write(
            &mut tx,
            "client",
            client_id,
            serde_json::json!({ "name": spec.name }),
        )
        .await?;
    }
    let acme = demo_uuid(20);
    let northwind = demo_uuid(21);
    let globex = demo_uuid(22);

    // One portal login per client (Wave 47 B3). A prospect who signs in as a
    // client sees that client's rows and nobody else's, so each of the three
    // clients gets its own seat rather than the org sharing Acme's. Email is
    // unique per org case-insensitively (migration 0021), so the three are
    // distinct by construction.
    struct DemoPortalUser {
        id_offset: u64,
        client: Uuid,
        email: &'static str,
        display_name: &'static str,
        /// Which client's portal this login opens — printed in the seed
        /// summary so an operator knows which prospect each seat is for.
        client_name: &'static str,
    }
    let portal_user_specs: [DemoPortalUser; 3] = [
        DemoPortalUser {
            id_offset: 50,
            client: acme,
            email: "portal@acme.demo",
            display_name: "Dana Reeve (Portal)",
            client_name: "Acme Manufacturing",
        },
        DemoPortalUser {
            id_offset: 53,
            client: northwind,
            email: "portal@northwind.demo",
            display_name: "Priya Natarajan (Portal)",
            client_name: "Northwind Legal",
        },
        DemoPortalUser {
            id_offset: 54,
            client: globex,
            email: "portal@globex.demo",
            display_name: "Chris Vale (Portal)",
            client_name: "Globex Foods",
        },
    ];
    let mut portal_user_count = 0usize;
    for spec in &portal_user_specs {
        sqlx::query(
            "INSERT INTO portal_users (id, org_id, client_id, email, display_name, password_hash, role, status)
             VALUES ($1, $2, $3, $4, $5, $6, 'contact', 'active')",
        )
        .bind(demo_uuid(spec.id_offset))
        .bind(DEMO_ORG_ID)
        .bind(spec.client)
        .bind(spec.email)
        .bind(spec.display_name)
        .bind(&password_hash)
        .execute(&mut *tx)
        .await?;
        portal_user_count += 1;
    }

    // --- boards + custom statuses (all four semantic categories each) -----
    struct DemoStatus {
        id_offset: u64,
        name: &'static str,
        category: &'static str,
        sort: i32,
        entry: bool,
    }
    struct DemoBoard {
        id_offset: u64,
        name: &'static str,
        description: &'static str,
        is_default: bool,
        statuses: &'static [DemoStatus],
    }
    let board_specs: [DemoBoard; 2] = [
        DemoBoard {
            id_offset: 60,
            name: "Service Desk",
            description: "Reactive support intake (default board)",
            is_default: true,
            statuses: &[
                DemoStatus {
                    id_offset: 70,
                    name: "New",
                    category: "open",
                    sort: 0,
                    entry: true,
                },
                DemoStatus {
                    id_offset: 71,
                    name: "Triaged",
                    category: "open",
                    sort: 5,
                    entry: false,
                },
                DemoStatus {
                    id_offset: 72,
                    name: "In Progress",
                    category: "in_progress",
                    sort: 10,
                    entry: false,
                },
                DemoStatus {
                    id_offset: 73,
                    name: "Escalated",
                    category: "in_progress",
                    sort: 15,
                    entry: false,
                },
                DemoStatus {
                    id_offset: 74,
                    name: "Waiting",
                    category: "waiting",
                    sort: 20,
                    entry: false,
                },
                DemoStatus {
                    id_offset: 75,
                    name: "Pending Approval",
                    category: "waiting",
                    sort: 25,
                    entry: false,
                },
                DemoStatus {
                    id_offset: 76,
                    name: "Resolved",
                    category: "closed",
                    sort: 30,
                    entry: false,
                },
            ],
        },
        DemoBoard {
            id_offset: 61,
            name: "Field Service",
            description: "On-site and dispatch work",
            is_default: false,
            statuses: &[
                DemoStatus {
                    id_offset: 80,
                    name: "Scheduled",
                    category: "open",
                    sort: 0,
                    entry: true,
                },
                DemoStatus {
                    id_offset: 81,
                    name: "On Site",
                    category: "in_progress",
                    sort: 10,
                    entry: false,
                },
                DemoStatus {
                    id_offset: 82,
                    name: "Parts On Order",
                    category: "waiting",
                    sort: 20,
                    entry: false,
                },
                DemoStatus {
                    id_offset: 83,
                    name: "Completed",
                    category: "closed",
                    sort: 30,
                    entry: false,
                },
            ],
        },
    ];
    for board in &board_specs {
        let board_id = demo_uuid(board.id_offset);
        crate::boards::create_board(
            &mut *tx,
            DEMO_ORG_ID,
            board_id,
            board.name,
            Some(board.description),
            board.is_default,
        )
        .await?;
        for status in board.statuses {
            sqlx::query(
                "INSERT INTO board_statuses (id, board_id, org_id, name, category, sort_order, is_entry)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(demo_uuid(status.id_offset))
            .bind(board_id)
            .bind(DEMO_ORG_ID)
            .bind(status.name)
            .bind(status.category)
            .bind(status.sort)
            .bind(status.entry)
            .execute(&mut *tx)
            .await?;
        }
        trail_system_write(
            &mut tx,
            "board",
            board_id,
            serde_json::json!({ "name": board.name }),
        )
        .await?;
    }
    let service_desk = demo_uuid(60);
    let _field_service = demo_uuid(61);

    // Every client's portal users see the Service Desk board. The exposure
    // flag gates portal ticket lists entirely, so a client without a row
    // signs in to an empty ticket list — which reads as a broken portal
    // rather than as a deliberate choice. Each client already carries
    // service-desk tickets, so all three lists are populated.
    for (id_offset, client) in [(52u64, acme), (55, northwind), (56, globex)] {
        sqlx::query(
            "INSERT INTO board_portal_exposure (id, org_id, board_id, client_id, exposed)
             VALUES ($1, $2, $3, $4, true)",
        )
        .bind(demo_uuid(id_offset))
        .bind(DEMO_ORG_ID)
        .bind(service_desk)
        .bind(client)
        .execute(&mut *tx)
        .await?;
    }

    // --- custom fields (2 defs + values on two tickets) -------------------
    let field_escalation = demo_uuid(90);
    sqlx::query(
        "INSERT INTO custom_field_defs (id, org_id, entity, name, kind, config, sort_order)
         VALUES ($1, $2, 'ticket', 'Escalation Tier', 'select', $3, 0)",
    )
    .bind(field_escalation)
    .bind(DEMO_ORG_ID)
    .bind(serde_json::json!({ "options": ["tier_1", "tier_2", "tier_3"] }))
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO custom_field_defs (id, org_id, entity, name, kind, config, sort_order)
         VALUES ($1, $2, 'ticket', 'RMA Number', 'text', '{}', 1)",
    )
    .bind(demo_uuid(91))
    .bind(DEMO_ORG_ID)
    .execute(&mut *tx)
    .await?;

    // --- tickets (15, spread across statuses/boards) ----------------------
    struct DemoTicket {
        index: u64,
        board: u64,
        status: u64,
        title: &'static str,
        body: &'static str,
        priority: &'static str,
        client: Uuid,
        assignee: Uuid,
        days_ago: i64,
        due_hours: Option<i64>,
        resolved: bool,
    }
    /// Row-table shorthand keeping the 15 entries readable.
    #[allow(clippy::too_many_arguments)]
    fn t(
        index: u64,
        board: u64,
        status: u64,
        title: &'static str,
        body: &'static str,
        priority: &'static str,
        client: Uuid,
        assignee: Uuid,
        days_ago: i64,
        due_hours: Option<i64>,
        resolved: bool,
    ) -> DemoTicket {
        DemoTicket {
            index,
            board,
            status,
            title,
            body,
            priority,
            client,
            assignee,
            days_ago,
            due_hours,
            resolved,
        }
    }
    let ticket_specs: [DemoTicket; 15] = [
        t(1, 60, 70, "VPN drops every hour", "Remote users lose the tunnel on the hour mark; logs point at the firewall's IKE rekey.", "p2", acme, owner, 0, Some(8), false),
        t(2, 60, 71, "License renewal question", "Asks whether the 25-seat license covers contractors.", "p3", acme, dispatcher, 1, None, false),
        t(3, 60, 72, "Mail server migration", "Move Northwind's on-prem exchange to the new host; cutover window pending.", "p2", northwind, tech, 2, Some(72), false),
        t(4, 60, 73, "Ransomware alert on file server", "EDR flagged mass encryption on SRV-FILE-01; isolated the host.", "p1", acme, owner, 0, Some(4), false),
        t(5, 60, 74, "Quote for office wifi", "Waiting on the customer's floor plan to size APs.", "p3", globex, dispatcher, 1, None, false),
        t(6, 60, 75, "Firewall rule change", "Requested outbound 3478/3480 for Teams; needs sign-off before applying.", "p2", acme, tech, 0, Some(24), false),
        t(7, 60, 76, "Password reset — vpn portal", "Standard reset, verified over the phone.", "p3", northwind, tech, 45, None, true),
        t(8, 60, 70, "Monitor flicker on dock", "External monitor flickers at 60Hz on the new dock.", "p4", acme, tech, 0, None, false),
        t(9, 60, 72, "Printer offline in warehouse", "Warehouse label printer drops off the network.", "p3", globex, tech, 1, Some(48), false),
        t(10, 61, 80, "Quarterly maintenance visit", "Scheduled quarterly walkthrough of Acme HQ.", "p2", acme, tech, 0, Some(96), false),
        t(11, 61, 81, "Switch replacement — comms room", "Replace the failed uplink switch; parts on the van.", "p2", northwind, tech, 1, Some(24), false),
        t(12, 61, 82, "Failed UPS unit", "UPS battery bank failed self-test; replacement ordered.", "p1", acme, tech, 2, Some(12), false),
        t(13, 61, 83, "AP installation — plant 2", "Two APs installed and validated; handover notes attached to the visit.", "p3", globex, tech, 15, None, true),
        t(14, 60, 70, "Email bounceback storm", "Bounces from one recipient domain; checking SPF/DKIM alignment.", "p1", globex, owner, 0, Some(4), false),
        t(15, 61, 80, "Camera firmware update", "Update the loading-dock cameras during the next visit.", "p3", acme, tech, 2, None, false),
    ];
    let mut ticket_ids: Vec<Uuid> = Vec::with_capacity(15);
    let mut ticket_created: Vec<chrono::DateTime<Utc>> = Vec::with_capacity(15);
    for spec in &ticket_specs {
        // next_ticket_number returns (prefix, sequence); the human ticket
        // number is the concatenation ("DEMO1001", ...).
        let (prefix, seq) = crate::orgs::next_ticket_number(&mut *tx, DEMO_ORG_ID).await?;
        // The hyphen is the app's ticket-number format everywhere else
        // (core-api's create/import paths, and every DB test fixture), so the
        // seeded org was the one place producing `DEMO1014` beside the API's
        // `DEMO-1065`. In a ledger that reads as two different numbering
        // schemes in a single column, and the seeded rows sit directly beside
        // API-created ones.
        let number = format!("{prefix}-{seq}");
        let id = demo_uuid(100 + spec.index);
        crate::tickets::insert_ticket(
            &mut tx,
            crate::tickets::NewTicket {
                id,
                org_id: DEMO_ORG_ID,
                number: &number,
                board_id: demo_uuid(spec.board),
                status_id: demo_uuid(spec.status),
                title: spec.title,
                body: Some(spec.body),
                priority: spec.priority.parse().expect("demo priority is valid"),
                client_id: Some(spec.client),
                site_id: None,
                contact_id: None,
                assignee_id: Some(spec.assignee),
                requester_id: None,
                due_at: spec.due_hours.map(|h| now + Duration::hours(h)),
            },
        )
        .await?;
        let created = now - Duration::days(spec.days_ago);
        sqlx::query(
            "UPDATE tickets SET created_at = $2, updated_at = $2,
                    resolved_at = CASE WHEN $3 THEN $4 ELSE NULL END
             WHERE id = $1",
        )
        .bind(id)
        .bind(created)
        .bind(spec.resolved)
        .bind(created + Duration::hours(6))
        .execute(&mut *tx)
        .await?;
        ticket_ids.push(id);
        ticket_created.push(created);
        trail_system_write(
            &mut tx,
            "ticket",
            id,
            serde_json::json!({ "number": number, "title": spec.title, "priority": spec.priority }),
        )
        .await?;
    }

    // Bulk demo load (the private build tracker DX volume): generated tickets so list views,
    // filters, pagination, and the reporting views have real data to chew
    // on. Deterministic — same rows on every seed.
    let bulk_clients = [acme, northwind, globex];
    let bulk_assignees = [owner, tech, dispatcher, owner, tech];
    let bulk_priorities = ["p1", "p2", "p3", "p4", "p2", "p3"];
    let bulk_statuses_60 = [70u64, 71, 72, 74, 75, 76];
    let bulk_statuses_61 = [80u64, 81, 82, 83];
    let bulk_titles = [
        "Onboarding checklist review",
        "VPN profile rollout",
        "Disk cleanup — file server",
        "Password expiry audit",
        "Patch cycle — January",
        "Shared drive permissions",
        "Laptop battery swelling",
        "Email quota increase",
        "Backup restore test",
        "MFA re-enrollment (lost device)",
        "WiFi dead zone — second floor",
        "Printer driver deployment",
        "Firewall firmware update",
        "Teams phone number port",
        "Asset audit — warehouse",
    ];
    let bulk_bodies = [
        "Generated demo ticket: walks the standard intake checklist end to end.",
        "Generated demo ticket: customer reports intermittent behaviour; monitoring before escalation.",
        "Generated demo ticket: routine maintenance item parked in the backlog.",
        "Generated demo ticket: needs a site visit to reproduce on premises.",
        "Generated demo ticket: waiting on vendor confirmation before work starts.",
    ];
    let mut bulk_ticket_count = 0usize;
    for i in 0u64..45 {
        let board = if i % 3 == 0 { 61 } else { 60 };
        let status = if board == 61 {
            bulk_statuses_61[(i % 4) as usize]
        } else {
            bulk_statuses_60[(i % 6) as usize]
        };
        let title = bulk_titles[(i % 15) as usize];
        let body = bulk_bodies[(i % 5) as usize];
        let priority = bulk_priorities[(i % 6) as usize];
        let client = bulk_clients[(i % 3) as usize];
        let assignee = bulk_assignees[(i % 5) as usize];
        let days_ago = (i % 21) as i64;
        let resolved = i % 9 == 0;
        let (prefix, seq) = crate::orgs::next_ticket_number(&mut *tx, DEMO_ORG_ID).await?;
        // The hyphen is the app's ticket-number format everywhere else
        // (core-api's create/import paths, and every DB test fixture), so the
        // seeded org was the one place producing `DEMO1014` beside the API's
        // `DEMO-1065`. In a ledger that reads as two different numbering
        // schemes in a single column, and the seeded rows sit directly beside
        // API-created ones.
        let number = format!("{prefix}-{seq}");
        let id = demo_uuid(200 + i);
        crate::tickets::insert_ticket(
            &mut tx,
            crate::tickets::NewTicket {
                id,
                org_id: DEMO_ORG_ID,
                number: &number,
                board_id: demo_uuid(board),
                status_id: demo_uuid(status),
                title,
                body: Some(body),
                priority: priority.parse().expect("demo priority is valid"),
                client_id: Some(client),
                site_id: None,
                contact_id: None,
                assignee_id: Some(assignee),
                requester_id: None,
                due_at: Some(now + Duration::hours(((i % 6) as i64 + 1) * 12)),
            },
        )
        .await?;
        let created = now - Duration::days(days_ago);
        sqlx::query(
            "UPDATE tickets SET created_at = $2, updated_at = $2,
                    resolved_at = CASE WHEN $3 THEN $4 ELSE NULL END
             WHERE id = $1",
        )
        .bind(id)
        .bind(created)
        .bind(resolved)
        .bind(created + Duration::hours(6))
        .execute(&mut *tx)
        .await?;
        trail_system_write(
            &mut tx,
            "ticket",
            id,
            serde_json::json!({ "number": number, "title": title, "priority": priority }),
        )
        .await?;
        bulk_ticket_count += 1;
    }

    // One staff comment per even bulk ticket (authors cycle) so conversation
    // views are not empty.
    let bulk_comment_bodies = [
        "Picked this up — initial triage done, gathering details.",
        "Spoke with the customer; they confirmed the reproduction steps.",
        "Change window requested for tomorrow evening.",
        "Parts ordered; ETA Thursday.",
        "Monitoring since the last patch — no recurrence so far.",
    ];
    for i in (0u64..45).step_by(2) {
        let author = bulk_assignees[(i % 5) as usize];
        let kind = if i % 4 == 0 {
            "client_visible"
        } else {
            "internal"
        };
        let body = bulk_comment_bodies[((i / 2) % 5) as usize];
        sqlx::query(
            "INSERT INTO ticket_comments (id, org_id, ticket_id, author_id, kind, body, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(demo_uuid(500 + i))
        .bind(DEMO_ORG_ID)
        .bind(demo_uuid(200 + i))
        .bind(author)
        .bind(kind)
        .bind(body)
        .bind(now - Duration::hours(((i % 10) + 1) as i64))
        .execute(&mut *tx)
        .await?;
    }

    // Time entries for every other bulk ticket so the gap report and
    // billing views have volume. Non-billable demo rows carry the typed
    // unbillable reason (Wave 39 V3) so the verification fixtures show
    // the workflow end to end.
    let bulk_work_types = ["remote", "onsite", "remote", "depot"];
    for i in (0u64..45).step_by(2) {
        let started = now - Duration::days((i % 14) as i64) - Duration::hours(3);
        let minutes = 30 + i32::try_from((i * 7) % 90).unwrap_or(45);
        let billable = i % 3 != 0;
        let work_type = bulk_work_types[(i % 4) as usize];
        // 600 is the entry the approval trail below walks through
        // submitted→approved, so its row has to AGREE with that trail:
        // before this, the history described an approval the queue never
        // saw and the ledger still called a draft.
        let decided = i == 0;
        sqlx::query(
            "INSERT INTO time_entries (id, org_id, ticket_id, user_id, started_at, ended_at,
                                       minutes, billable, notes, work_type, unbillable_reason,
                                       approval_status, approval_submitted_by, approval_submitted_at,
                                       approval_decided_by, approval_decided_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
        )
        .bind(demo_uuid(600 + i))
        .bind(DEMO_ORG_ID)
        .bind(demo_uuid(200 + i))
        .bind(bulk_assignees[(i % 5) as usize])
        .bind(started)
        .bind(started + Duration::minutes(i64::from(minutes)))
        .bind(minutes)
        .bind(billable)
        .bind("Generated demo work")
        .bind(work_type)
        .bind(if billable { None } else { Some("internal") })
        .bind(if decided { "approved" } else { "draft" })
        .bind(if decided { Some(tech) } else { None })
        .bind(if decided {
            Some(at(today - Duration::days(1), 9, 0))
        } else {
            None
        })
        .bind(if decided { Some(owner) } else { None })
        .bind(if decided {
            Some(at(today - Duration::days(1), 10, 0))
        } else {
            None
        })
        .execute(&mut *tx)
        .await?;
    }

    // Comments: mixed visibility kinds so portal + staff views both have
    // content.
    struct DemoComment {
        id_offset: u64,
        ticket_index: u64,
        author: Uuid,
        kind: &'static str,
        body: &'static str,
    }
    let comment_specs: [DemoComment; 6] = [
        DemoComment { id_offset: 120, ticket_index: 1, author: dispatcher, kind: "internal", body: "Rekey timer mismatch confirmed; staging a firmware change for Thursday." },
        DemoComment { id_offset: 121, ticket_index: 1, author: tech, kind: "client_visible", body: "We reproduced the hourly drop and have a fix staged; you will not need to be on the call." },
        DemoComment { id_offset: 122, ticket_index: 4, author: tech, kind: "internal", body: "Host isolated at the switch port. Forensics image requested before any restore." },
        DemoComment { id_offset: 123, ticket_index: 4, author: owner, kind: "in_house_update", body: "Acme leadership briefed; restore plan is image-from-golden, no ransom engaged." },
        DemoComment { id_offset: 124, ticket_index: 3, author: tech, kind: "internal", body: "MX cutover checklist drafted; waiting on Northwind's approval for Friday night." },
        DemoComment { id_offset: 125, ticket_index: 11, author: tech, kind: "client_visible", body: "Switch swapped and uplinks bonded; monitoring through Friday." },
    ];
    let mut comment_count = 0usize;
    for spec in &comment_specs {
        sqlx::query(
            "INSERT INTO ticket_comments (id, org_id, ticket_id, author_id, kind, body, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(demo_uuid(spec.id_offset))
        .bind(DEMO_ORG_ID)
        .bind(demo_uuid(100 + spec.ticket_index))
        .bind(spec.author)
        .bind(spec.kind)
        .bind(spec.body)
        .bind(now - Duration::hours(6))
        .execute(&mut *tx)
        .await?;
        comment_count += 1;
    }

    // Checklists: one mostly-done, one fresh.
    struct DemoChecklist {
        id_offset: u64,
        ticket_index: u64,
        text: &'static str,
        checked: bool,
        sort: i32,
    }
    let checklist_specs = [
        DemoChecklist {
            id_offset: 140,
            ticket_index: 3,
            text: "Freeze mailbox changes",
            checked: true,
            sort: 0,
        },
        DemoChecklist {
            id_offset: 141,
            ticket_index: 3,
            text: "Replicate databases to new host",
            checked: true,
            sort: 1,
        },
        DemoChecklist {
            id_offset: 142,
            ticket_index: 3,
            text: "Cut over MX records",
            checked: false,
            sort: 2,
        },
        DemoChecklist {
            id_offset: 143,
            ticket_index: 12,
            text: "RMA the failed battery bank",
            checked: true,
            sort: 0,
        },
        DemoChecklist {
            id_offset: 144,
            ticket_index: 12,
            text: "Schedule swap visit",
            checked: false,
            sort: 1,
        },
    ];
    for spec in &checklist_specs {
        sqlx::query(
            "INSERT INTO checklist_items (id, org_id, ticket_id, text, checked, sort_order, checked_by, checked_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(demo_uuid(spec.id_offset))
        .bind(DEMO_ORG_ID)
        .bind(demo_uuid(100 + spec.ticket_index))
        .bind(spec.text)
        .bind(spec.checked)
        .bind(spec.sort)
        .bind(if spec.checked { Some(tech) } else { None })
        .bind(if spec.checked { Some(now - Duration::hours(20)) } else { None })
        .execute(&mut *tx)
        .await?;
    }
    let checklist_count = checklist_specs.len();

    // Custom field values (select + text demos).
    sqlx::query(
        "INSERT INTO custom_field_values (field_id, entity_id, org_id, value)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(field_escalation)
    .bind(demo_uuid(104))
    .bind(DEMO_ORG_ID)
    .bind(serde_json::json!("tier_2"))
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO custom_field_values (field_id, entity_id, org_id, value)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(demo_uuid(91))
    .bind(demo_uuid(112))
    .bind(DEMO_ORG_ID)
    .bind(serde_json::json!("RMA-00042"))
    .execute(&mut *tx)
    .await?;

    // --- SLA state (non-closed tickets; time-travel shifts these) --------
    // The due dates come from the org's OWN policy targets rather than a flat
    // "four hours / three days". Two reasons, and the second is the one a
    // prospect notices: the same ladder is published on the SLA settings
    // page, so a P1 ticket counting down from a three-day budget while the
    // policy it belongs to promises eight hours is a contradiction on screen.
    // Deriving it also means each priority tier shows a DIFFERENT clock, which
    // is the whole point of having tiers.
    let sla_target_of = |priority: &str, field: &str| -> i64 {
        sla_targets[priority][field].as_i64().unwrap_or(60)
    };
    for (position, spec) in ticket_specs.iter().enumerate() {
        if spec.resolved {
            continue;
        }
        let created = ticket_created[position];
        sqlx::query(
            "INSERT INTO ticket_sla_state (ticket_id, org_id, policy_id,
                                           first_response_due, resolution_due,
                                           first_responded_at, breached)
             VALUES ($1, $2, $3, $4, $5, $6, false)",
        )
        .bind(demo_uuid(100 + spec.index))
        .bind(DEMO_ORG_ID)
        .bind(sla_policy_id)
        .bind(created + Duration::minutes(sla_target_of(spec.priority, "first_response_mins")))
        .bind(created + Duration::minutes(sla_target_of(spec.priority, "resolution_mins")))
        .bind(
            // Tickets with a staff comment have already had a first
            // response; the clocks the ladder reads stay realistic.
            if spec.index == 1 || spec.index == 4 {
                Some(created + Duration::hours(1))
            } else {
                None
            },
        )
        .execute(&mut *tx)
        .await?;
    }

    // --- SLA state for the BULK queue ------------------------------------
    // The generated tickets carry no SLA row, so every board row rendered
    // "no countdown". Walk the bulk queue and give the first OPEN tickets a
    // real spread: two already past due, two inside the hour, the rest
    // comfortable. Open-only because time_travel (and history) preserve
    // closed tickets, and `breached` stays false — it is the server's own
    // breach flag, and the app derives its red pill from the due date.
    /// breached, due inside the hour, comfortable, roomy.
    const SLA_PATTERN: [i64; 4] = [-1, 45, 96, 300];
    // Once per FIELD seat (the two people who open the app on site): whichever
    // one signs in sees a breached row, a due-soon row, and calm ones.
    for seat in [dispatcher, tech] {
        let mut sla_seeded = 0usize;
        for i in 0..24u64 {
            if sla_seeded == SLA_PATTERN.len() {
                break;
            }
            let due_minutes = SLA_PATTERN[sla_seeded];
            let inserted = sqlx::query(
                "INSERT INTO ticket_sla_state (ticket_id, org_id, policy_id,
                                           first_response_due, resolution_due,
                                           first_responded_at, breached)
             SELECT t.id, $1, $2,
                    $4 + make_interval(mins => $5::int - 60),
                    $4 + make_interval(mins => $5::int),
                    $4 + make_interval(mins => $5::int - 90),
                    false
             FROM tickets t
             JOIN board_statuses st ON st.id = t.status_id
             WHERE t.id = $3 AND st.category <> 'closed' AND t.assignee_id = $6
             ON CONFLICT (ticket_id) DO NOTHING",
            )
            .bind(DEMO_ORG_ID)
            .bind(sla_policy_id)
            .bind(demo_uuid(200 + i))
            .bind(now)
            .bind(due_minutes)
            .bind(seat)
            .execute(&mut *tx)
            .await?;
            if inserted.rows_affected() == 1 {
                sla_seeded += 1;
            }
        }
    }

    // --- one SLA row per PRIORITY TIER (Wave 47 B3) -----------------------
    // The per-seat spread above is deliberately arbitrary about priority, so
    // nothing guaranteed that a P1 row, a P2 row, a P3 row and a P4 row all
    // exist: a tier whose only open tickets happened to land on a seat that
    // already had its four, or on a closed status, rendered as a plain row
    // with no countdown. Walk the bulk queue by tier instead and guarantee
    // one open ticket per tier, anchored on the tier's OWN policy budget so
    // the numbers match the ladder everywhere else.
    //
    // `ON CONFLICT DO NOTHING` keeps the per-seat rows authoritative, and the
    // `NOT EXISTS` skips tickets that already carry a clock so this always
    // adds a row rather than quietly doing nothing.
    const TIER_TARGETS: [(&str, i64, i64); 4] = [
        ("p1", 15, 480),
        ("p2", 60, 1440),
        ("p3", 240, 4320),
        ("p4", 480, 10080),
    ];
    for (tier, first_response_mins, resolution_mins) in TIER_TARGETS {
        sqlx::query(
            "INSERT INTO ticket_sla_state (ticket_id, org_id, policy_id,
                                           first_response_due, resolution_due,
                                           first_responded_at, breached)
             SELECT t.id, $1, $2,
                    $3 + make_interval(mins => $4::int),
                    $3 + make_interval(mins => $5::int),
                    NULL,
                    false
             FROM tickets t
             JOIN board_statuses st ON st.id = t.status_id
             WHERE t.org_id = $1 AND t.priority = $6 AND st.category <> 'closed'
               AND NOT EXISTS (SELECT 1 FROM ticket_sla_state s WHERE s.ticket_id = t.id)
             ORDER BY t.created_at, t.id
             LIMIT 1
             ON CONFLICT (ticket_id) DO NOTHING",
        )
        .bind(DEMO_ORG_ID)
        .bind(sla_policy_id)
        .bind(now)
        .bind(first_response_mins)
        .bind(resolution_mins)
        .bind(tier)
        .execute(&mut *tx)
        .await?;
    }

    // --- stock: locations, items, serials, on-hand -------------------------
    // The field app's stock legs (lookup, item, location, serial) had nothing
    // to show. Seed a small, believable van: a warehouse, a van, a shelf,
    // two serialized tools, three consumables, and one LOW line so the blush
    // state is earned rather than described.
    const LOC_WAREHOUSE: u64 = 900;
    const LOC_VAN: u64 = 901;
    const LOC_SHELF: u64 = 902;
    // "Shelf B (returns)" deliberately repeats the SHELF-B code: a scan of
    // that code is genuinely ambiguous, and the picker exists for exactly it.
    const LOC_RETURNS: u64 = 903;
    for (id, name, kind, code, parent, assignee) in [
        (LOC_WAREHOUSE, "Warehouse", "warehouse", "WH-A", None, None),
        (LOC_VAN, "Van 2", "van", "VAN-2", None, Some(tech)),
        (
            LOC_SHELF,
            "Shelf B",
            "shelf",
            "SHELF-B",
            Some(LOC_WAREHOUSE),
            None,
        ),
        (
            LOC_RETURNS,
            "Shelf B (returns)",
            "shelf",
            "SHELF-B",
            Some(LOC_WAREHOUSE),
            None,
        ),
    ] {
        sqlx::query(
            "INSERT INTO stock_locations (id, org_id, name, kind, code, parent_id, assigned_user_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (org_id, name) DO NOTHING",
        )
        .bind(demo_uuid(id))
        .bind(DEMO_ORG_ID)
        .bind(name)
        .bind(kind)
        .bind(code)
        .bind(parent.map(demo_uuid))
        .bind(assignee)
        .execute(&mut *tx)
        .await?;
    }

    // (offset, name, sku, part, kind, unit, currency) — DOCK-01 and DOCK-02
    // carry the SAME part number on purpose: scanning the part resolves to two
    // candidates, which is the ambiguity picker's whole reason to exist.
    // is_tool marks the checkout-eligible items: the two kit-checkout carriers
    // below (the kit's own tool and the switch-firmware unit, one of them OVERDUE)
    // must be flagged or the tools board renders "No tool units yet" — the state
    // the fixture exists to draw would not exist.
    // Named row: an eight-field tuple written out in a binding reads as
    // unreadable, so it gets a name. The literals below stay positional.
    type StockSpecRow = (
        u64,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        bool,
    );
    let stock_specs: [StockSpecRow; 6] = [
        (
            910,
            "Torque wrench set",
            "TL-0142",
            "TL-0142",
            "serialized",
            "each",
            "USD",
            true,
        ),
        (
            911,
            "Cable tester",
            "TL-0088",
            "TL-0088",
            "serialized",
            "each",
            "USD",
            true,
        ),
        (
            912,
            "Thermal paste",
            "TP-220",
            "TP-220",
            "consumable",
            "each",
            "USD",
            false,
        ),
        (
            913,
            "Cat6 patch lead 2m",
            "NET-2M",
            "NET-2M",
            "consumable",
            "each",
            "USD",
            false,
        ),
        (
            914,
            "USB-C dock",
            "DOCK-01",
            "DOCK-01",
            "consumable",
            "each",
            "USD",
            false,
        ),
        (
            915,
            "USB-C dock (spare)",
            "DOCK-02",
            "DOCK-01",
            "consumable",
            "each",
            "USD",
            false,
        ),
    ];
    for (id, name, sku, part, kind, unit, currency, is_tool) in stock_specs {
        sqlx::query(
            "INSERT INTO stock_items (id, org_id, name, sku, part_number, kind, unit, currency, is_tool)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(demo_uuid(id))
        .bind(DEMO_ORG_ID)
        .bind(name)
        .bind(sku)
        .bind(part)
        .bind(kind)
        .bind(unit)
        .bind(currency)
        .bind(is_tool)
        .execute(&mut *tx)
        .await?;
    }

    for (id, item, number, location, status) in [
        (920u64, 910u64, "TL-0142", LOC_VAN, "checked_out"),
        (921, 911, "TL-0088", LOC_SHELF, "in_stock"),
        (922, 912, "SN-10004", LOC_WAREHOUSE, "in_stock"),
        (923, 911, "SN-10005", LOC_SHELF, "checked_out"),
    ] {
        sqlx::query(
            "INSERT INTO stock_serials (id, org_id, stock_item_id, serial_number,
                                        location_id, status)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(demo_uuid(id))
        .bind(DEMO_ORG_ID)
        .bind(demo_uuid(item))
        .bind(number)
        .bind(demo_uuid(location))
        .bind(status)
        .execute(&mut *tx)
        .await?;
    }

    // TP-220 in the van sits under its reorder point: the LOW pill is real.
    for (id, item, location, qty, min, reorder) in [
        (
            940u64,
            912u64,
            LOC_WAREHOUSE,
            24_000i64,
            10_000i64,
            10_000i64,
        ),
        (941, 912, LOC_VAN, 1_500, 2_000, 2_000),
        (942, 913, LOC_SHELF, 40_000, 10_000, 12_000),
        (943, 914, LOC_WAREHOUSE, 6_000, 2_000, 2_000),
        (944, 915, LOC_WAREHOUSE, 2_000, 2_000, 2_000),
        (945, 911, LOC_SHELF, 3_000, 1_000, 1_000),
    ] {
        sqlx::query(
            "INSERT INTO stock_levels (id, org_id, stock_item_id, stock_location_id,
                                      qty_milli, min_qty_milli, reorder_point_milli)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (org_id, stock_item_id, stock_location_id) DO NOTHING",
        )
        .bind(demo_uuid(id))
        .bind(DEMO_ORG_ID)
        .bind(demo_uuid(item))
        .bind(demo_uuid(location))
        .bind(qty)
        .bind(min)
        .bind(reorder)
        .execute(&mut *tx)
        .await?;
    }

    // A short movement history so the item screen's ledger is not empty.
    // Safe to reseed: migration 0119 gives the dev reset an explicit bypass
    // for the append-only guard, which the app itself never sets.
    for (id, item, kind, to, qty, ticket, note) in [
        (
            960u64,
            912u64,
            "receive",
            LOC_WAREHOUSE,
            24_000i64,
            None,
            "Opening stock",
        ),
        (
            961,
            912,
            "consume",
            LOC_WAREHOUSE,
            -1_500,
            Some(demo_uuid(202)),
            "Reseated two ports",
        ),
        (962, 913, "receive", LOC_SHELF, 40_000, None, "Van restock"),
    ] {
        sqlx::query(
            "INSERT INTO stock_movements (id, org_id, stock_item_id, kind, to_location_id,
                                          qty_milli, currency, cost_minor, ticket_id, note, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, 'USD', 1250, $7, $8, $9)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(demo_uuid(id))
        .bind(DEMO_ORG_ID)
        .bind(demo_uuid(item))
        .bind(kind)
        .bind(demo_uuid(to))
        .bind(qty)
        .bind(ticket)
        .bind(note)
        .bind(tech)
        .execute(&mut *tx)
        .await?;
    }

    // --- tools: a kit and two open checkouts (one overdue) -----------------
    sqlx::query(
        "INSERT INTO tool_kits (id, org_id, name, code, note)
         VALUES ($1, $2, 'Van toolbox', 'KIT-VAN-2', 'Everything in the box rides one transaction.')
         ON CONFLICT (org_id, name) DO NOTHING",
    )
    .bind(demo_uuid(970))
    .bind(DEMO_ORG_ID)
    .execute(&mut *tx)
    .await?;

    for (id, serial, item, location, due_hours, note) in [
        (
            971u64,
            920u64,
            910u64,
            LOC_VAN,
            26i64,
            "VPN troubleshooting",
        ),
        (
            972,
            923,
            911,
            LOC_SHELF,
            -20,
            "Switch firmware — still out, due back yesterday",
        ),
    ] {
        sqlx::query(
            "INSERT INTO tool_checkouts (id, org_id, holder_user_id, kit_id,
                                         from_location_id, due_at, notes, status, checked_out_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'open', $3)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(demo_uuid(id))
        .bind(DEMO_ORG_ID)
        .bind(tech)
        .bind(demo_uuid(970))
        .bind(demo_uuid(location))
        .bind(now + Duration::hours(due_hours))
        .bind(note)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO tool_checkout_lines (id, org_id, checkout_id, stock_item_id,
                                             serial_id, movement_out_id)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(demo_uuid(id + 20))
        .bind(DEMO_ORG_ID)
        .bind(demo_uuid(id))
        .bind(demo_uuid(item))
        .bind(demo_uuid(serial))
        .bind(demo_uuid(if serial == 920 { 960 } else { 962 }))
        .execute(&mut *tx)
        .await?;
    }

    // --- field purchasing + client loans -----------------------------------
    // Promoted from the assets-stock lane's capture fixtures
    // (scripts/seed-lane9-capture-fixtures.sh): the web stock cluster's
    // loans and purchasing legs, and the pack's overdue-red states, need
    // this data and seed-demo had none of it. Issued through the REAL write
    // paths (`issue_loan`, `insert_purchase_order` + `mark_ordered`) — the
    // same invariants and ledger legs tenant traffic exercises — rather
    // than hand-rolled rows. Deterministic shape (fixed ids, fixed offsets
    // from `now`); the write paths generate their own v7 ids, which the
    // reset-loop test tolerates by counting, not diffing.
    struct DemoVendor {
        id: u64,
        name: &'static str,
        currency: &'static str,
        email: Option<&'static str>,
        phone: Option<&'static str>,
    }
    let vendor_specs = [
        DemoVendor {
            id: 993,
            name: "Meridian Cable Co",
            currency: "USD",
            email: Some("orders@meridiancable.test"),
            phone: None,
        },
        DemoVendor {
            id: 994,
            name: "Kestrel Instrument",
            currency: "GBP",
            email: None,
            phone: Some("+44 20 7946 0102"),
        },
    ];
    let mut vendor_ids = Vec::with_capacity(vendor_specs.len());
    for spec in &vendor_specs {
        let vendor = core_domain::entities::purchasing::Vendor {
            id: demo_uuid(spec.id),
            org_id: DEMO_ORG_ID,
            name: spec.name.to_string(),
            code: None,
            email: spec.email.map(str::to_string),
            phone: spec.phone.map(str::to_string),
            website: None,
            account_number: None,
            default_currency: spec.currency.to_string(),
            notes: None,
            active: true,
            created_by: Some(owner),
            created_at: now,
            updated_at: now,
        };
        vendor
            .validate()
            .map_err(|e| DbError::Conflict(e.to_string()))?;
        crate::purchasing::insert_vendor(&mut *tx, &vendor).await?;
        trail_system_write(
            &mut tx,
            "vendor",
            vendor.id,
            serde_json::to_value(&vendor).unwrap_or_default(),
        )
        .await?;
        vendor_ids.push(vendor.id);
    }

    /// (vendor index, expected offset in days from today, ordered, lines).
    /// Two ordered orders — one inside its window, one past its expected
    /// date (the red state) — plus one draft for ledger variety.
    /// (vendor index, expected offset in days from today, ordered, lines).
    /// Two ordered orders — one inside its window, one past its expected
    /// date (the red state) — plus one draft for ledger variety.
    ///
    /// `stock_item` is the seeded `stock_items` row each line receives into.
    /// The write path allows None, but a PO whose lines name no stock item
    /// can never be received: there is nothing to post the ledger movement
    /// against. Naming them is what turns these from wish lists into stock.
    struct DemoOrder {
        vendor: usize,
        expected_days: i64,
        ordered: bool,
        lines: &'static [(&'static str, i64, i64, u64)],
    }
    let order_specs = [
        DemoOrder {
            vendor: 0,
            expected_days: 6,
            ordered: true,
            lines: &[
                ("Cat6a patch cable, 2m", 2_000, 480, 910),
                ("24-port patch panel", 500, 18_500, 911),
            ],
        },
        DemoOrder {
            vendor: 1,
            expected_days: -9,
            ordered: true,
            lines: &[
                ("Switch PSU 250W", 2_000, 4_200, 912),
                ("Thermal paste tube", 1_000, 1_250, 913),
            ],
        },
        DemoOrder {
            vendor: 0,
            expected_days: 21,
            ordered: false,
            lines: &[("USB-C dock, 10-pack", 300, 92_000, 915)],
        },
    ];
    let mut purchase_order_count = 0usize;
    for spec in &order_specs {
        let number = crate::purchasing::next_purchase_order_number(&mut *tx, DEMO_ORG_ID).await?;
        let order = core_domain::entities::purchasing::PurchaseOrder {
            id: Uuid::now_v7(),
            org_id: DEMO_ORG_ID,
            vendor_id: vendor_ids[spec.vendor],
            number,
            status: core_domain::entities::purchasing::PurchaseOrderStatus::Draft,
            currency: "USD".into(),
            vendor_reference: None,
            project_id: None,
            notes: None,
            ordered_at: None,
            expected_at: Some(today + Duration::days(spec.expected_days)),
            closed_at: None,
            close_kind: None,
            cancelled_at: None,
            cancel_reason: None,
            created_by: Some(owner),
            created_at: now,
            updated_at: now,
        };
        order
            .validate()
            .map_err(|e| DbError::Conflict(e.to_string()))?;
        crate::purchasing::insert_purchase_order(&mut *tx, &order).await?;
        if spec.ordered {
            crate::purchasing::mark_ordered(&mut *tx, DEMO_ORG_ID, order.id).await?;
        }
        for (position, (description, qty_milli, unit_cost_minor, stock_item)) in
            spec.lines.iter().enumerate()
        {
            let line = core_domain::entities::purchasing::PurchaseOrderLine {
                id: Uuid::now_v7(),
                org_id: DEMO_ORG_ID,
                purchase_order_id: order.id,
                stock_item_id: Some(demo_uuid(*stock_item)),
                catalog_item_id: None,
                description: description.to_string(),
                qty_ordered_milli: *qty_milli,
                qty_received_milli: 0,
                unit_cost_minor: *unit_cost_minor,
                receipt_state: core_domain::entities::purchasing::ReceiptState::Pending,
                position: position as i32,
                created_at: now,
                updated_at: now,
            };
            line.validate()
                .map_err(|e| DbError::Conflict(e.to_string()))?;
            crate::purchasing::insert_po_line(&mut *tx, &line).await?;
        }
        trail_system_write(
            &mut tx,
            "purchase_order",
            order.id,
            serde_json::to_value(&order).unwrap_or_default(),
        )
        .await?;
        purchase_order_count += 1;
    }

    // Client loans: one OVERDUE (red), one due in two days (blush), one
    // calm-but-open — the severity ladder the loans page and the pack draw.
    // Each loan issues a serialized unit, so the seed carries three loanable
    // items with one in-stock serial each; `issue_loan` flips the serial to
    // `loaned`, appends the `loan` ledger leg, and instantiates the client's
    // fleet asset exactly like the REST path does.
    let loanable_specs: [(u64, u64, &str, &str); 3] = [
        (930, 933, "Site survey kit", "SSK-2201"),
        (931, 934, "Signal tester", "SIG-3301"),
        (932, 935, "Borescope camera", "BOR-4402"),
    ];
    let mut loanable_ids = Vec::with_capacity(loanable_specs.len());
    for (item_id, serial_id, name, serial_number) in loanable_specs {
        let item = core_domain::entities::stock::StockItem {
            id: demo_uuid(item_id),
            org_id: DEMO_ORG_ID,
            name: name.to_string(),
            sku: Some(serial_number.to_string()),
            catalog_item_id: None,
            part_number: Some(serial_number.to_string()),
            kind: "serialized".into(),
            is_tool: false,
            unit: "each".into(),
            currency: "USD".into(),
            last_unit_cost_minor: None,
            active: true,
            created_by: Some(tech),
            created_at: now,
            updated_at: now,
        };
        item.validate()
            .map_err(|e| DbError::Conflict(e.to_string()))?;
        crate::stock::insert_stock_item(&mut *tx, &item).await?;
        let serial = core_domain::entities::stock::StockSerial {
            id: demo_uuid(serial_id),
            org_id: DEMO_ORG_ID,
            stock_item_id: item.id,
            serial_number: serial_number.to_string(),
            location_id: Some(demo_uuid(LOC_SHELF)),
            status: "in_stock".into(),
            ticket_id: None,
            client_id: None,
            created_by: Some(tech),
            created_at: now,
            updated_at: now,
        };
        serial
            .validate()
            .map_err(|e| DbError::Conflict(e.to_string()))?;
        crate::stock::upsert_serial(&mut tx, &serial).await?;
        // The receive that put the unit on the shelf: the loan's OUT leg
        // decrements stock_levels through the real ledger guard, so the
        // level row (and its receive movement) must exist first or the
        // loan fails with "insufficient stock".
        sqlx::query(
            "INSERT INTO stock_levels (id, org_id, stock_item_id, stock_location_id, qty_milli)
             VALUES ($1, $2, $3, $4, 1000)
             ON CONFLICT (org_id, stock_item_id, stock_location_id) DO NOTHING",
        )
        .bind(demo_uuid(946 + (item_id - 930)))
        .bind(DEMO_ORG_ID)
        .bind(item.id)
        .bind(demo_uuid(LOC_SHELF))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO stock_movements (id, org_id, stock_item_id, kind, to_location_id,
                                          qty_milli, serial_id, note, created_by)
             VALUES ($1, $2, $3, 'receive', $4, 1000, $5, 'Opening stock', $6)",
        )
        .bind(demo_uuid(963 + (item_id - 930)))
        .bind(DEMO_ORG_ID)
        .bind(item.id)
        .bind(demo_uuid(LOC_SHELF))
        .bind(serial.id)
        .bind(tech)
        .execute(&mut *tx)
        .await?;
        loanable_ids.push((item.id, serial.id));
    }
    let loan_specs: [(usize, i64, i64, &str); 3] = [
        // Acme borrowed the survey kit two weeks ago; it was due back
        // yesterday-plus — the loans page's red row.
        (0, -14, -9, "Loaner for the plant-2 wifi survey"),
        // Northwind's tester is back Thursday — the blush row.
        (1, -3, 2, "Bench tester for the comms-room audit"),
        // Globex keeps the borescope for a week — open, not urgent.
        (2, -1, 5, "Line-shutdown inspection at the cannery"),
    ];
    let client_ids = [demo_uuid(20), demo_uuid(21), demo_uuid(22)];
    let mut client_loan_count = 0usize;
    for (client_idx, out_days, due_days, note) in loan_specs {
        let (item_id, serial_id) = loanable_ids[client_idx];
        let request = crate::loans::LoanIssueRequest {
            client_id: client_ids[client_idx],
            contract_id: None,
            agreement_ref: None,
            out_at: Some(now + Duration::days(out_days)),
            expected_return_at: today + Duration::days(due_days),
            is_lease: false,
            lease_cycle: None,
            lease_cost_minor: None,
            lease_currency: None,
            notes: Some(note.to_string()),
            items: vec![crate::loans::LoanIssueItem {
                stock_item_id: Some(item_id),
                serial_id: Some(serial_id),
                device_asset_id: None,
            }],
        };
        let outcome = crate::loans::issue_loan(&mut tx, DEMO_ORG_ID, tech, &request).await?;
        trail_system_write(
            &mut tx,
            "client_loan",
            outcome.loan.id,
            serde_json::to_value(&outcome.loan).unwrap_or_default(),
        )
        .await?;
        client_loan_count += 1;
    }

    // --- notifications that point at tickets -------------------------------
    // The deep-linked open and the grouped list both need rows naming a
    // ticket; the desk's inbox is what the phone opens.
    for (id, ticket, kind, title, body) in [
        (
            980u64,
            202u64,
            "ticket_update",
            "Breach notice",
            "#18402 breached its SLA — the queue needs a new owner.",
        ),
        (
            981,
            207,
            "ticket_update",
            "Harbour Dental replied",
            "Printer offline on the second floor: we will swap the unit on Friday.",
        ),
        (
            982,
            201,
            "mention",
            "Marcus Bell on the backup job",
            "Restore finished at 02:10. Nothing has run since Friday.",
        ),
        (
            983,
            202,
            "missing_time",
            "Time not logged",
            "Yesterday's call with Kestrel Logistics has no time against it.",
        ),
        (
            984,
            204,
            "assignment",
            "Assigned to you",
            "Teams phone number port moved to your queue.",
        ),
    ] {
        for user in [dispatcher, tech] {
            sqlx::query(
                "INSERT INTO notifications (id, org_id, user_id, kind, ticket_id, title, body, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(demo_uuid(id + 100))
            .bind(DEMO_ORG_ID)
            .bind(user)
            .bind(kind)
            .bind(demo_uuid(200 + ticket - 200))
            .bind(title)
            .bind(body)
            .bind(now - Duration::minutes((id - 979) as i64 * 37))
            .execute(&mut *tx)
            .await?;
        }
    }

    // --- timers (one running for tech, one paused, one stopped) ----------
    let segments_running = serde_json::json!([
        { "started_at": now - Duration::minutes(45), "ended_at": null, "end_kind": null, "reason": null }
    ]);
    sqlx::query(
        "INSERT INTO timer_sessions (id, org_id, user_id, ticket_id, started_at, stopped_at, segments)
         VALUES ($1, $2, $3, $4, $5, NULL, $6)",
    )
    .bind(demo_uuid(150))
    .bind(DEMO_ORG_ID)
    .bind(tech)
    .bind(demo_uuid(101))
    .bind(now - Duration::minutes(45))
    .bind(&segments_running)
    .execute(&mut *tx)
    .await?;
    let segments_paused = serde_json::json!([
        { "started_at": now - Duration::hours(3), "ended_at": now - Duration::minutes(150), "end_kind": "pause", "reason": "vendor call" }
    ]);
    sqlx::query(
        "INSERT INTO timer_sessions (id, org_id, user_id, ticket_id, started_at, stopped_at, segments)
         VALUES ($1, $2, $3, $4, $5, NULL, $6)",
    )
    .bind(demo_uuid(151))
    .bind(DEMO_ORG_ID)
    .bind(tech)
    .bind(demo_uuid(103))
    .bind(now - Duration::hours(3))
    .bind(&segments_paused)
    .execute(&mut *tx)
    .await?;
    let segments_stopped = serde_json::json!([
        { "started_at": now - Duration::hours(26), "ended_at": now - Duration::hours(25), "end_kind": "stop", "reason": null }
    ]);
    sqlx::query(
        "INSERT INTO timer_sessions (id, org_id, user_id, ticket_id, started_at, stopped_at, segments)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(demo_uuid(152))
    .bind(DEMO_ORG_ID)
    .bind(dispatcher)
    .bind(demo_uuid(105))
    .bind(now - Duration::hours(26))
    .bind(now - Duration::hours(25))
    .bind(&segments_stopped)
    .execute(&mut *tx)
    .await?;

    // --- time entries (billed T&M trio lives inside the invoice window) --
    /// The approval lifecycle a seeded row starts in. `draft` is the column
    /// default (migration 0073), so a row that never gets an explicit state
    /// sits out of the manager queue entirely — which is what the seed used
    /// to do everywhere, leaving `/approvals` permanently empty.
    struct DemoEntry {
        id_offset: u64,
        ticket_index: u64,
        user: Uuid,
        started: chrono::DateTime<Utc>,
        minutes: i32,
        billable: bool,
        work_type: &'static str,
        notes: &'static str,
        /// `draft` | `submitted` | `approved`. A submitted row also carries
        /// who submitted it and when, so the queue's oldest-first ordering
        /// has something real to sort.
        approval: &'static str,
    }
    let (period_from, period_to) = last_month_window(now.date_naive());
    let entry_specs: [DemoEntry; 11] = [
        // Last month: the three entries the demo T&M invoice bills.
        DemoEntry {
            id_offset: 160,
            ticket_index: 3,
            user: tech,
            started: at(period_to - Duration::days(9), 10, 0),
            minutes: 120,
            billable: true,
            work_type: "remote",
            notes: "Mailbox replication run",
            approval: "draft",
        },
        DemoEntry {
            id_offset: 161,
            ticket_index: 3,
            user: tech,
            started: at(period_to - Duration::days(6), 14, 0),
            minutes: 60,
            billable: true,
            work_type: "remote",
            notes: "Cutover checklist review",
            approval: "draft",
        },
        DemoEntry {
            id_offset: 162,
            ticket_index: 11,
            user: tech,
            started: at(period_to - Duration::days(3), 9, 30),
            minutes: 90,
            billable: true,
            work_type: "on_site",
            notes: "Switch survey and staging",
            approval: "draft",
        },
        // This week: everyday demo traffic.
        // 163 is the manager-queue row (Wave 47 B3): logged this morning,
        // submitted for approval, still waiting. Approving or rejecting it
        // live is the /approvals demo.
        DemoEntry {
            id_offset: 163,
            ticket_index: 1,
            user: tech,
            started: now - Duration::hours(5),
            minutes: 45,
            billable: true,
            work_type: "remote",
            notes: "Rekey log analysis",
            approval: "submitted",
        },
        DemoEntry {
            id_offset: 175,
            ticket_index: 5,
            user: dispatcher,
            started: now - Duration::hours(6),
            minutes: 35,
            billable: true,
            work_type: "remote",
            notes: "Walked the client through the new portal and confirmed access",
            approval: "submitted",
        },
        DemoEntry {
            id_offset: 164,
            ticket_index: 2,
            user: dispatcher,
            started: now - Duration::hours(7),
            minutes: 25,
            billable: true,
            work_type: "remote",
            notes: "License matrix walk-through",
            approval: "draft",
        },
        DemoEntry {
            id_offset: 165,
            ticket_index: 4,
            user: tech,
            started: now - Duration::hours(3),
            minutes: 80,
            billable: true,
            work_type: "on_site",
            notes: "Isolation and forensics handoff",
            approval: "draft",
        },
        DemoEntry {
            id_offset: 166,
            ticket_index: 9,
            user: tech,
            started: now - Duration::hours(4),
            minutes: 20,
            billable: false,
            work_type: "remote",
            notes: "Printer ping checks",
            approval: "draft",
        },
        DemoEntry {
            id_offset: 167,
            ticket_index: 10,
            user: tech,
            started: now - Duration::hours(26),
            minutes: 60,
            billable: true,
            work_type: "on_site",
            notes: "Quarterly walkthrough prep",
            approval: "draft",
        },
        DemoEntry {
            id_offset: 168,
            ticket_index: 12,
            user: tech,
            started: now - Duration::hours(50),
            minutes: 15,
            billable: false,
            work_type: "on_site",
            notes: "UPS self-test capture",
            approval: "draft",
        },
        DemoEntry {
            id_offset: 169,
            ticket_index: 14,
            user: dispatcher,
            started: now - Duration::hours(2),
            minutes: 35,
            billable: true,
            work_type: "remote",
            notes: "SPF/DKIM alignment check",
            approval: "draft",
        },
    ];
    let mut billed_entry_ids: Vec<(Uuid, i64, Option<&'static str>)> = Vec::new();
    for spec in &entry_specs {
        let id = demo_uuid(spec.id_offset);
        sqlx::query(
            "INSERT INTO time_entries (id, org_id, ticket_id, user_id, started_at, ended_at,
                                       minutes, billable, notes, work_type, unbillable_reason,
                                       approval_status, approval_submitted_by, approval_submitted_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(id)
        .bind(DEMO_ORG_ID)
        .bind(demo_uuid(100 + spec.ticket_index))
        .bind(spec.user)
        .bind(spec.started)
        .bind(spec.started + Duration::minutes(i64::from(spec.minutes)))
        .bind(spec.minutes)
        .bind(spec.billable)
        .bind(spec.notes)
        .bind(spec.work_type)
        .bind(if spec.billable {
            None
        } else {
            Some("warranty")
        })
        .bind(spec.approval)
        .bind(if spec.approval == "submitted" {
            Some(spec.user)
        } else {
            None
        })
        .bind(if spec.approval == "submitted" {
            Some(now - Duration::hours(4))
        } else {
            None
        })
        .execute(&mut *tx)
        .await?;
        if spec.billable
            && spec.started.date_naive() >= period_from
            && spec.started.date_naive() <= period_to
        {
            billed_entry_ids.push((id, i64::from(spec.minutes), Some(spec.work_type)));
        }
    }
    let time_entry_count = entry_specs.len();

    // Search index backfill (the private build tracker search QA): the worker indexer is
    // event-driven, and the seeder writes rows directly without events —
    // so populate the index here. Set-based and idempotent; later real
    // events re-upsert the same (org, kind, entity) keys.
    // Search index backfill (the private build tracker search QA): the worker indexer is
    // event-driven, and the seeder writes rows directly without events —
    // so populate the index here. Set-based and idempotent; later real
    // events re-upsert the same (org, kind, entity) keys. One statement
    // per entity kind: a prepared statement cannot carry multiple
    // commands (sqlx 42601).
    const SEARCH_UPSERTS: &[&str] = &[
        r#"INSERT INTO search_index (id, org_id, entity_kind, entity_id, parent_id, title, body)
           SELECT md5('ticket' || t.id::text)::uuid, t.org_id, 'ticket', t.id,
                  t.board_id, t.title, coalesce(t.body, '')
           FROM tickets t WHERE t.org_id = $1
           ON CONFLICT (org_id, entity_kind, entity_id) DO UPDATE SET
             title = EXCLUDED.title, body = EXCLUDED.body, indexed_at = now()"#,
        r#"INSERT INTO search_index (id, org_id, entity_kind, entity_id, parent_id, title, body)
           SELECT md5('client' || c.id::text)::uuid, c.org_id, 'client', c.id,
                  NULL, c.name, ''
           FROM clients c WHERE c.org_id = $1
           ON CONFLICT (org_id, entity_kind, entity_id) DO UPDATE SET
             title = EXCLUDED.title, body = EXCLUDED.body, indexed_at = now()"#,
        r#"INSERT INTO search_index (id, org_id, entity_kind, entity_id, parent_id, title, body)
           SELECT md5('site' || s.id::text)::uuid, s.org_id, 'site', s.id,
                  s.client_id, s.name, coalesce(s.address, '')
           FROM sites s WHERE s.org_id = $1
           ON CONFLICT (org_id, entity_kind, entity_id) DO UPDATE SET
             title = EXCLUDED.title, body = EXCLUDED.body, indexed_at = now()"#,
        r#"INSERT INTO search_index (id, org_id, entity_kind, entity_id, parent_id, title, body)
           SELECT md5('contact' || c.id::text)::uuid, c.org_id, 'contact', c.id,
                  c.client_id, c.name, c.email
           FROM contacts c WHERE c.org_id = $1
           ON CONFLICT (org_id, entity_kind, entity_id) DO UPDATE SET
             title = EXCLUDED.title, body = EXCLUDED.body, indexed_at = now()"#,
        r#"INSERT INTO search_index (id, org_id, entity_kind, entity_id, parent_id, title, body)
           SELECT md5('kb' || k.id::text)::uuid, k.org_id, 'kb_article', k.id,
                  NULL, k.title, coalesce(k.body, '')
           FROM kb_articles k WHERE k.org_id = $1 AND k.status = 'published'
           ON CONFLICT (org_id, entity_kind, entity_id) DO UPDATE SET
             title = EXCLUDED.title, body = EXCLUDED.body, indexed_at = now()"#,
        r#"INSERT INTO search_index (id, org_id, entity_kind, entity_id, parent_id, title, body)
           SELECT md5('doc' || d.id::text)::uuid, d.org_id, 'document', d.id,
                  NULL, d.title, coalesce(v.body, '')
           FROM documents d
           LEFT JOIN document_versions v
             ON v.document_id = d.id AND v.version = d.current_version
           WHERE d.org_id = $1
           ON CONFLICT (org_id, entity_kind, entity_id) DO UPDATE SET
             title = EXCLUDED.title, body = EXCLUDED.body, indexed_at = now()"#,
        r#"INSERT INTO search_index (id, org_id, entity_kind, entity_id, parent_id, title, body)
           SELECT md5('comment' || c.id::text)::uuid, c.org_id, 'comment', c.id,
                  c.ticket_id, left(c.body, 60), c.body
           FROM ticket_comments c WHERE c.org_id = $1
           ON CONFLICT (org_id, entity_kind, entity_id) DO UPDATE SET
             title = EXCLUDED.title, body = EXCLUDED.body, indexed_at = now()"#,
    ];
    for upsert in SEARCH_UPSERTS {
        sqlx::query(upsert)
            .bind(DEMO_ORG_ID)
            .execute(&mut *tx)
            .await?;
    }

    // --- contracts: 1 recurring + 1 T&M with a rate card -----------------
    let contract_recurring = demo_uuid(170);
    sqlx::query(
        "INSERT INTO contracts (id, org_id, client_id, number, name, contract_type, status,
                                currency, effective_from, billing_frequency, billing_anchor,
                                amount_minor, notes)
         VALUES ($1, $2, $3, 'CT-1001', 'Acme Managed Support', 'recurring', 'active',
                 'USD', $4, 'monthly', $4, 50000,
                 'Demo recurring agreement: monthly base, 9x5 coverage.')",
    )
    .bind(contract_recurring)
    .bind(DEMO_ORG_ID)
    .bind(acme)
    .bind(anchor_two_months_back(now.date_naive()))
    .execute(&mut *tx)
    .await?;
    let contract_tm = demo_uuid(171);
    sqlx::query(
        "INSERT INTO contracts (id, org_id, client_id, number, name, contract_type, status,
                                currency, effective_from, hourly_rate_minor, notes)
         VALUES ($1, $2, $3, 'CT-1002', 'Northwind Break-Fix', 'time_and_materials', 'active',
                 'USD', $4, 15000, 'Demo T&M: rate card below, default 150/h.')",
    )
    .bind(contract_tm)
    .bind(DEMO_ORG_ID)
    .bind(northwind)
    .bind(period_from - Duration::days(30))
    .execute(&mut *tx)
    .await?;
    for (id_offset, work_type, rate) in [(180u64, "remote", 12_000i64), (181, "on_site", 18_000)] {
        sqlx::query(
            "INSERT INTO contract_rates (id, org_id, contract_id, work_type, rate_minor)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(demo_uuid(id_offset))
        .bind(DEMO_ORG_ID)
        .bind(contract_tm)
        .bind(work_type)
        .bind(rate)
        .execute(&mut *tx)
        .await?;
    }
    trail_system_write(
        &mut tx,
        "contract",
        contract_recurring,
        serde_json::json!({ "number": "CT-1001", "contract_type": "recurring" }),
    )
    .await?;
    trail_system_write(
        &mut tx,
        "contract",
        contract_tm,
        serde_json::json!({ "number": "CT-1002", "contract_type": "time_and_materials" }),
    )
    .await?;

    // --- Wave 39: a per-incident P1 band + a prepaid block ----------------
    let contract_incident = demo_uuid(172);
    sqlx::query(
        "INSERT INTO contracts (id, org_id, client_id, number, name, contract_type, status,
                                currency, effective_from, incident_price_minor,
                                incident_priority, po_number, notes)
         VALUES ($1, $2, $3, 'CT-1003', 'Globex P1 Coverage', 'per_incident', 'active',
                 'USD', $4, 45000, 'p1', 'PO-88231',
                 'Demo per-incident: 450.00 per closed P1 ticket.')",
    )
    .bind(contract_incident)
    .bind(DEMO_ORG_ID)
    .bind(globex)
    .bind(anchor_two_months_back(now.date_naive()))
    .execute(&mut *tx)
    .await?;
    trail_system_write(
        &mut tx,
        "contract",
        contract_incident,
        serde_json::json!({ "number": "CT-1003", "contract_type": "per_incident" }),
    )
    .await?;
    let contract_retainer = demo_uuid(173);
    sqlx::query(
        "INSERT INTO contracts (id, org_id, client_id, number, name, contract_type, status,
                                currency, effective_from, billing_frequency, billing_anchor,
                                amount_minor, included_minutes, overage_rate_minor, po_number, notes)
         VALUES ($1, $2, $3, 'CT-1004', 'Northwind Care Retainer', 'retainer', 'active',
                 'USD', $4, 'monthly', $4, 120000, 600, 16000, 'PO-55107',
                 'Demo retainer: 1200.00/month, 10 included hours, 160.00/h overage.')",
    )
    .bind(contract_retainer)
    .bind(DEMO_ORG_ID)
    .bind(northwind)
    .bind(anchor_two_months_back(now.date_naive()))
    .execute(&mut *tx)
    .await?;
    // A prepaid 10-hour block sold against the retainer (block rate
    // undercuts the overage card on purpose).
    sqlx::query(
        "INSERT INTO contract_blocks (id, org_id, contract_id, label, total_minutes,
                                      sale_price_minor, block_rate_minor, expires_on,
                                      rollover_policy)
         VALUES ($1, $2, $3, '10-hour prepaid block (this quarter)', 600, 100000,
                 12000, $4, 'expire')",
    )
    .bind(demo_uuid(174))
    .bind(DEMO_ORG_ID)
    .bind(contract_retainer)
    .bind(now.date_naive() + Duration::days(60))
    .execute(&mut *tx)
    .await?;
    trail_system_write(
        &mut tx,
        "contract",
        contract_retainer,
        serde_json::json!({ "number": "CT-1004", "contract_type": "retainer" }),
    )
    .await?;

    // --- invoice run over the past period → 2 invoices -------------------
    let run_number = crate::invoices::next_number(&mut *tx, DEMO_ORG_ID, "RUN-").await?;
    let run_id = demo_uuid(190);
    sqlx::query(
        "INSERT INTO invoice_runs (id, org_id, number, period_from, period_to, invoice_count,
                                   total_minor, currency, due_in_days, created_by)
         VALUES ($1, $2, $3, $4, $5, 2, 113000, 'USD', 30, $6)",
    )
    .bind(run_id)
    .bind(DEMO_ORG_ID)
    .bind(&run_number)
    .bind(period_from)
    .bind(period_to)
    .bind(owner)
    .execute(&mut *tx)
    .await?;

    // Invoice 1 — recurring base, fully paid.
    let invoice_paid_id = demo_uuid(200);
    let invoice_paid_number = crate::invoices::next_number(&mut *tx, DEMO_ORG_ID, "INV-").await?;
    let issued = period_to;
    let due_paid = issued + Duration::days(30);
    sqlx::query(
        "INSERT INTO invoices (id, org_id, run_id, client_id, contract_id, number, status,
                               currency, period_from, period_to, issued_at, due_on,
                               total_minor, total_paid_minor, dunning_state, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, 'paid', 'USD', $7, $8, $9, $10, 50000, 50000, 'none', $11)",
    )
    .bind(invoice_paid_id)
    .bind(DEMO_ORG_ID)
    .bind(run_id)
    .bind(acme)
    .bind(contract_recurring)
    .bind(&invoice_paid_number)
    .bind(period_from)
    .bind(period_to)
    .bind(issued)
    .bind(due_paid)
    .bind(owner)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO invoice_lines (id, org_id, invoice_id, contract_id, kind, description,
                                    period_start, period_end, quantity_minutes,
                                    unit_amount_minor, amount_minor, position)
         VALUES ($1, $2, $3, $4, 'recurring_base', $5, $6, $7, 0, 50000, 50000, 1)",
    )
    .bind(demo_uuid(210))
    .bind(DEMO_ORG_ID)
    .bind(invoice_paid_id)
    .bind(contract_recurring)
    .bind(format!(
        "Acme Managed Support — monthly base ({} … {})",
        period_from, period_to
    ))
    .bind(period_from)
    .bind(period_to)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO invoice_payments (id, org_id, invoice_id, amount_minor, currency,
                                       provider, provider_ref, notes, received_at, created_by)
         VALUES ($1, $2, $3, 50000, 'USD', 'none', 'demo-manual', 'Demo ACH payment', $4, $5)",
    )
    .bind(demo_uuid(230))
    .bind(DEMO_ORG_ID)
    .bind(invoice_paid_id)
    .bind(issued + Duration::days(2))
    .bind(bookkeeper)
    .execute(&mut *tx)
    .await?;

    // Invoice 2 — T&M labor, unpaid, reminder already sent (dunning step 1).
    let invoice_dunning_id = demo_uuid(201);
    let invoice_dunning_number =
        crate::invoices::next_number(&mut *tx, DEMO_ORG_ID, "INV-").await?;
    sqlx::query(
        "INSERT INTO invoices (id, org_id, run_id, client_id, contract_id, number, status,
                               currency, period_from, period_to, issued_at, due_on,
                               total_minor, total_paid_minor, dunning_state,
                               dunning_updated_at, dunning_note, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, 'issued', 'USD', $7, $8, $9, $10, 63000, 0,
                 'reminder_sent', $11, 'Reminder emailed (demo dunning step 1)', $12)",
    )
    .bind(invoice_dunning_id)
    .bind(DEMO_ORG_ID)
    .bind(run_id)
    .bind(northwind)
    .bind(contract_tm)
    .bind(&invoice_dunning_number)
    .bind(period_from)
    .bind(period_to)
    .bind(issued)
    .bind(due_paid)
    .bind(now - Duration::days(3))
    .bind(owner)
    .execute(&mut *tx)
    .await?;
    // T&M lines bucket by work_type at the contract rate (core-billing
    // `prorate`: (minutes × rate + 30) / 60), mirroring rules.rs exactly.
    let mut remote_minutes = 0i64;
    let mut on_site_minutes = 0i64;
    for (_, minutes, work_type) in &billed_entry_ids {
        match *work_type {
            Some("remote") => remote_minutes += minutes,
            Some("on_site") => on_site_minutes += minutes,
            _ => {}
        }
    }
    let mut position = 1i32;
    for (work_type, minutes, rate) in [
        ("remote", remote_minutes, 12_000i64),
        ("on_site", on_site_minutes, 18_000),
    ] {
        if minutes == 0 {
            continue;
        }
        let amount = (minutes * rate + 30) / 60;
        sqlx::query(
            "INSERT INTO invoice_lines (id, org_id, invoice_id, contract_id, kind, description,
                                        period_start, period_end, quantity_minutes,
                                        unit_amount_minor, amount_minor, position)
             VALUES ($1, $2, $3, $4, 'time_materials', $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(demo_uuid(211 + position as u64))
        .bind(DEMO_ORG_ID)
        .bind(invoice_dunning_id)
        .bind(contract_tm)
        .bind(format!(
            "Northwind Break-Fix labor: {minutes} min at {rate}/h ({work_type})"
        ))
        .bind(period_from)
        .bind(period_to)
        .bind(minutes)
        .bind(rate)
        .bind(amount)
        .bind(position)
        .execute(&mut *tx)
        .await?;
        position += 1;
    }
    for (entry_id, _, _) in &billed_entry_ids {
        sqlx::query(
            "INSERT INTO invoice_time_entries (id, org_id, invoice_id, time_entry_id)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(Uuid::now_v7())
        .bind(DEMO_ORG_ID)
        .bind(invoice_dunning_id)
        .bind(entry_id)
        .execute(&mut *tx)
        .await?;
    }
    trail_system_write(
        &mut tx,
        "invoice",
        invoice_paid_id,
        serde_json::json!({ "number": invoice_paid_number, "status": "paid" }),
    )
    .await?;
    trail_system_write(
        &mut tx,
        "invoice",
        invoice_dunning_id,
        serde_json::json!({ "number": invoice_dunning_number, "dunning_state": "reminder_sent" }),
    )
    .await?;

    // --- Wave 47 B3: the two invoice states the sales story needs ----------
    // The run above produces PAID and SENT. A demo that stops there never
    // shows the two states an MSP actually argues with a client about: a
    // draft still being checked, and money that is late. Both are reachable
    // today — `status` accepts 'draft' (0036) and the ledger derives overdue
    // from `due_on` plus an unpaid balance — so this is seed data, not a
    // schema change.
    //
    // Both rows sit OUTSIDE the run (run_id NULL): a held draft has not been
    // through it, and the late invoice belongs to an earlier month.
    let mut invoice_count = 2usize;

    // DRAFT — Acme's next recurring base, held before it goes out. Drafts
    // never reach the portal (`list_client_invoices` filters issued+paid),
    // which is the point: this is the bookkeeper's own worksheet.
    let invoice_draft_id = demo_uuid(405);
    let invoice_draft_number = crate::invoices::next_number(&mut *tx, DEMO_ORG_ID, "INV-").await?;
    sqlx::query(
        "INSERT INTO invoices (id, org_id, run_id, client_id, contract_id, number, status,
                               currency, period_from, period_to, issued_at, due_on,
                               total_minor, total_paid_minor, notes, created_by)
         VALUES ($1, $2, NULL, $3, $4, $5, 'draft',
                 'USD', $6, $7, $8, $9, 50000, 0, $10, $11)",
    )
    .bind(invoice_draft_id)
    .bind(DEMO_ORG_ID)
    .bind(acme)
    .bind(contract_recurring)
    .bind(&invoice_draft_number)
    .bind(today.with_day(1).unwrap_or(today))
    .bind(today)
    .bind(today)
    .bind(today + Duration::days(30))
    .bind("Held for review before the run issues it.")
    .bind(bookkeeper)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO invoice_lines (id, org_id, invoice_id, contract_id, kind, description,
                                    period_start, period_end, quantity_minutes,
                                    unit_amount_minor, amount_minor, position)
         VALUES ($1, $2, $3, $4, 'recurring_base', $5, $6, $7, 0, 50000, 50000, 1)",
    )
    .bind(demo_uuid(410))
    .bind(DEMO_ORG_ID)
    .bind(invoice_draft_id)
    .bind(contract_recurring)
    .bind(format!(
        "Acme Managed Support — monthly base ({} … {})",
        today.with_day(1).unwrap_or(today),
        today
    ))
    .bind(today.with_day(1).unwrap_or(today))
    .bind(today)
    .execute(&mut *tx)
    .await?;
    trail_system_write(
        &mut tx,
        "invoice",
        invoice_draft_id,
        serde_json::json!({ "number": invoice_draft_number, "status": "draft" }),
    )
    .await?;
    invoice_count += 1;

    // OVERDUE — Globex's per-incident P1 coverage from last month, three
    // weeks past its due date with no payment against it. `dunning_state`
    // is already 'overdue' so the collections ladder reads consistently,
    // and the portal shows it in the "what you owe" card.
    let invoice_overdue_id = demo_uuid(406);
    let invoice_overdue_number =
        crate::invoices::next_number(&mut *tx, DEMO_ORG_ID, "INV-").await?;
    let overdue_issued = today - Duration::days(38);
    let overdue_due = today - Duration::days(23);
    sqlx::query(
        "INSERT INTO invoices (id, org_id, run_id, client_id, contract_id, number, status,
                               currency, period_from, period_to, issued_at, due_on,
                               total_minor, total_paid_minor, dunning_state,
                               dunning_updated_at, dunning_note, created_by)
         VALUES ($1, $2, NULL, $3, $4, $5, 'issued',
                 'USD', $6, $7, $8, $9, 45000, 0,
                 'overdue', $10, 'Second notice emailed; account on hold', $11)",
    )
    .bind(invoice_overdue_id)
    .bind(DEMO_ORG_ID)
    .bind(globex)
    .bind(contract_incident)
    .bind(&invoice_overdue_number)
    .bind(overdue_issued)
    .bind(overdue_issued)
    .bind(overdue_issued)
    .bind(overdue_due)
    .bind(today - Duration::days(4))
    .bind(bookkeeper)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO invoice_lines (id, org_id, invoice_id, contract_id, kind, description,
                                    period_start, period_end, quantity_minutes,
                                    unit_amount_minor, amount_minor, position)
         VALUES ($1, $2, $3, $4, 'per_incident', $5, $6, $7, 0, 45000, 45000, 1)",
    )
    .bind(demo_uuid(411))
    .bind(DEMO_ORG_ID)
    .bind(invoice_overdue_id)
    .bind(contract_incident)
    .bind("Globex P1 coverage — two closed P1 incidents in the period".to_string())
    .bind(overdue_issued)
    .bind(overdue_issued)
    .execute(&mut *tx)
    .await?;
    trail_system_write(
        &mut tx,
        "invoice",
        invoice_overdue_id,
        serde_json::json!({ "number": invoice_overdue_number, "dunning_state": "overdue" }),
    )
    .await?;
    invoice_count += 1;

    // --- KB: category + client-facing articles ----------------------------
    let kb_category = demo_uuid(240);
    sqlx::query(
        "INSERT INTO kb_categories (id, org_id, name, slug, description, position, client_visible)
         VALUES ($1, $2, 'Onboarding', 'onboarding', 'Getting-started guides', 0, true)",
    )
    .bind(kb_category)
    .bind(DEMO_ORG_ID)
    .execute(&mut *tx)
    .await?;
    // (title, slug, body, client_visible, status).
    //
    // Wave 47 B3: this was three single-sentence stubs, which read as filler
    // in the one surface a prospect's own staff will judge the product by. A
    // real client's KB is the answers they would otherwise phone in, written
    // the way a technician would actually write them — so each one says what
    // to do, what not to do, and when to stop trying and call. Five are
    // published and client-visible (the portal search indexes them); the
    // escalation ladder stays internal.
    let kb_articles: [(&str, &str, &str, bool, &str); 6] = [
        (
            "How to submit a support ticket",
            "how-to-submit-a-support-ticket",
            "Open the portal and pick the service that is misbehaving. Tell us what breaks, when it started, \
and who it affects — a screenshot helps when the error is on screen. If the whole site is down, phone \
the number on your contract instead, because the portal is not the fastest route for that.",
            true,
            "published",
        ),
        (
            "What to have ready before you call",
            "what-to-have-ready-before-you-call",
            "Have the device's tag or serial number to hand, the error text exactly as it appears, and \
whether the problem follows one person or one machine. If anything changed recently, such as an update, \
a new laptop, or a cable that got moved, say so up front, because that is usually the answer.",
            true,
            "published",
        ),
        (
            "Wi-Fi keeps dropping or the network name has gone",
            "wi-fi-drops-or-missing-ssid",
            "Restart the access point first, not the switch, and give it a full two minutes to come back \
before you test anything. If the network name has disappeared from every device, leave the PoE \
injector alone and call us — that pattern almost always means the unit has failed, and we want to see \
it in the state it failed in.",
            true,
            "published",
        ),
        (
            "How to add or remove a user account",
            "how-to-add-or-remove-a-user-account",
            "Send us the full name, the start or end date, and which systems the person needs. We create \
the account with the same permissions the rest of the team has, and close the old one on the day we \
are told. Tell us about leavers on their last day rather than afterwards, so their access does not \
outlive their job.",
            true,
            "published",
        ),
        (
            "Changing your portal password and second factor",
            "changing-your-portal-password-and-mfa",
            "Sign in to the portal and open your profile to change your password. If you have lost the \
phone holding your second factor, use the recovery codes you saved when you set it up. If you never \
saved them, call us — we will reset it once we have verified who you are, and not before.",
            true,
            "published",
        ),
        (
            "Internal escalation path",
            "internal-escalation-path",
            "Tier 1 hands to Tier 2 after two touches with no progress. Anything marked p1 pages the \
on-call tech immediately; do not wait for the next working day. Log every touch against the ticket as \
you go, because the escalation record is what the SLA credit claim is read from later.",
            false,
            "draft",
        ),
    ];
    for (index, (title, slug, body, client_visible, status)) in kb_articles.iter().enumerate() {
        let id = demo_uuid(250 + index as u64);
        sqlx::query(
            "INSERT INTO kb_articles (id, org_id, category_id, title, slug, body, status,
                                      client_visible, created_by, updated_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)",
        )
        .bind(id)
        .bind(DEMO_ORG_ID)
        .bind(kb_category)
        .bind(title)
        .bind(slug)
        .bind(body)
        .bind(status)
        .bind(client_visible)
        .bind(owner)
        .execute(&mut *tx)
        .await?;
        trail_system_write(
            &mut tx,
            "kb_article",
            id,
            serde_json::json!({ "title": title, "slug": slug }),
        )
        .await?;
    }

    // --- document with 2 immutable versions -------------------------------
    let document_id = demo_uuid(260);
    sqlx::query(
        "INSERT INTO documents (id, org_id, title, current_version, created_by, updated_by)
         VALUES ($1, $2, 'Runbook — Client Onboarding', 2, $3, $3)",
    )
    .bind(document_id)
    .bind(DEMO_ORG_ID)
    .bind(owner)
    .execute(&mut *tx)
    .await?;
    for (version, body, summary, days_ago) in [
        (
            1i32,
            "Collect contacts, sites, and portal users. Schedule the discovery call.",
            "Initial draft",
            40,
        ),
        (
            2,
            "Collect contacts, sites, and portal users. Schedule the discovery call.\n\nAccess review: confirm the asset list before day 5.",
            "Added the access-review checklist",
            12,
        ),
    ] {
        sqlx::query(
            "INSERT INTO document_versions (id, org_id, document_id, version, title, body,
                                            change_summary, author_id, created_at)
             VALUES ($1, $2, $3, $4, 'Runbook — Client Onboarding', $5, $6, $7, $8)",
        )
        .bind(demo_uuid(270 + version as u64))
        .bind(DEMO_ORG_ID)
        .bind(document_id)
        .bind(version)
        .bind(body)
        .bind(summary)
        .bind(owner)
        .bind(now - Duration::days(days_ago))
        .execute(&mut *tx)
        .await?;
    }
    trail_system_write(
        &mut tx,
        "document",
        document_id,
        serde_json::json!({ "title": "Runbook — Client Onboarding", "current_version": 2 }),
    )
    .await?;

    // --- project: phases, tasks, one completed milestone ------------------
    // Wave 39 lane 4: the project bills THROUGH Acme's contract (CT-1001),
    // so phase T&M draws its rate card and released milestones reach the
    // run via that contract's candidates.
    let project_id = demo_uuid(280);
    sqlx::query(
        "INSERT INTO projects (id, org_id, client_id, name, description, status, billing_method,
                               budget_minor, currency, start_date, due_date, billing_contract_id)
         VALUES ($1, $2, $3, 'Acme Network Refresh', 'Core switching and wireless refresh across both sites.',
                 'active', 'milestone', 500000, 'USD', $4, $5, $6)",
    )
    .bind(project_id)
    .bind(DEMO_ORG_ID)
    .bind(acme)
    .bind(now.date_naive() - Duration::days(30))
    .bind(now.date_naive() + Duration::days(60))
    .bind(contract_recurring)
    .execute(&mut *tx)
    .await?;
    let phase_discovery = demo_uuid(290);
    sqlx::query(
        "INSERT INTO project_phases (id, org_id, project_id, name, sort_order, status, completed_at)
         VALUES ($1, $2, $3, 'Discovery', 0, 'completed', $4)",
    )
    .bind(phase_discovery)
    .bind(DEMO_ORG_ID)
    .bind(project_id)
    .bind(now - Duration::days(10))
    .execute(&mut *tx)
    .await?;
    crate::projects::insert_release(
        &mut *tx,
        &crate::projects::NewRelease {
            org_id: DEMO_ORG_ID,
            project_id,
            phase_id: Some(phase_discovery),
            task_id: None,
            label: "Discovery".into(),
            amount_minor: 120000,
            currency: "USD".into(),
            released_on: now.date_naive() - Duration::days(10),
            source_event_id: None,
        },
    )
    .await
    .expect("demo discovery release stages");
    let phase_implementation = demo_uuid(291);
    sqlx::query(
        "INSERT INTO project_phases (id, org_id, project_id, name, sort_order, status,
                                     is_milestone, milestone_amount_minor, milestone_currency)
         VALUES ($1, $2, $3, 'Implementation', 1, 'active', true, 250000, 'USD')",
    )
    .bind(phase_implementation)
    .bind(DEMO_ORG_ID)
    .bind(project_id)
    .execute(&mut *tx)
    .await?;
    // The active phase carries a productive burn story: a budget the T&M
    // entries draw against (warn band at 80%).
    crate::projects::upsert_phase_budget(
        &mut *tx,
        DEMO_ORG_ID,
        phase_implementation,
        crate::projects::NewPhaseBudget {
            budget_minor: Some(150000),
            currency: Some("USD"),
            fee_minor: None,
            fee_currency: None,
        },
    )
    .await
    .expect("demo implementation budget records");
    struct DemoTask {
        id_offset: u64,
        phase: Uuid,
        title: &'static str,
        status: &'static str,
        milestone: bool,
        completed_days_ago: Option<i64>,
    }
    let task_specs = [
        DemoTask {
            id_offset: 300,
            phase: phase_discovery,
            title: "Site survey — both sites",
            status: "completed",
            milestone: false,
            completed_days_ago: Some(12),
        },
        DemoTask {
            id_offset: 301,
            phase: phase_discovery,
            title: "Draft bill of materials",
            status: "completed",
            milestone: false,
            completed_days_ago: Some(11),
        },
        DemoTask {
            id_offset: 302,
            phase: phase_implementation,
            title: "Install core switch",
            status: "in_progress",
            milestone: false,
            completed_days_ago: None,
        },
        DemoTask {
            id_offset: 303,
            phase: phase_implementation,
            title: "Milestone: go-live sign-off",
            status: "pending",
            milestone: true,
            completed_days_ago: None,
        },
    ];
    for spec in &task_specs {
        sqlx::query(
            "INSERT INTO project_tasks (id, org_id, project_id, phase_id, title, status,
                                        assignee_id, is_milestone, milestone_amount_minor,
                                        milestone_currency, completed_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(demo_uuid(spec.id_offset))
        .bind(DEMO_ORG_ID)
        .bind(project_id)
        .bind(spec.phase)
        .bind(spec.title)
        .bind(spec.status)
        .bind(tech)
        .bind(spec.milestone)
        .bind(if spec.milestone {
            Some(250_000i64)
        } else {
            None
        })
        .bind(if spec.milestone { Some("USD") } else { None })
        .bind(spec.completed_days_ago.map(|d| now - Duration::days(d)))
        .execute(&mut *tx)
        .await?;
    }
    // Go-live sign-off depends on the core switch install.
    sqlx::query(
        "INSERT INTO project_task_deps (org_id, task_id, depends_on_task_id)
         VALUES ($1, $2, $3)",
    )
    .bind(DEMO_ORG_ID)
    .bind(demo_uuid(303))
    .bind(demo_uuid(302))
    .execute(&mut *tx)
    .await?;
    trail_system_write(
        &mut tx,
        "project",
        project_id,
        serde_json::json!({ "name": "Acme Network Refresh", "status": "active" }),
    )
    .await?;

    // --- 3 appointments on tech's calendar --------------------------------
    let appointment_specs: [(u64, i64, u32, u32, i64, i32, &str); 3] = [
        (310, 1, 9, 11, 1, 10, "Quarterly maintenance — Acme HQ"),
        (311, 3, 13, 15, 3, 0, "Mail migration cutover window"),
        (312, 7, 9, 12, 12, 30, "UPS battery bank swap"),
    ];
    for (id_offset, days_ahead, start_hour, end_hour, ticket_index, travel, title) in
        appointment_specs
    {
        let day = now.date_naive() + Duration::days(days_ahead);
        let starts = chrono::NaiveDateTime::new(
            day,
            NaiveTime::from_hms_opt(start_hour, 0, 0).unwrap_or_default(),
        )
        .and_utc();
        let ends = chrono::NaiveDateTime::new(
            day,
            NaiveTime::from_hms_opt(end_hour, 0, 0).unwrap_or_default(),
        )
        .and_utc();
        sqlx::query(
            "INSERT INTO appointments (id, org_id, user_id, ticket_id, client_id, title,
                                       starts_at, ends_at, kind, status, travel_minutes)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'on_site', 'scheduled', $9)",
        )
        .bind(demo_uuid(id_offset))
        .bind(DEMO_ORG_ID)
        .bind(tech)
        .bind(demo_uuid(100 + ticket_index as u64))
        .bind(acme)
        .bind(title)
        .bind(starts)
        .bind(ends)
        .bind(travel)
        .execute(&mut *tx)
        .await?;
    }

    // --- alert source + 2 incidents with storm collapse -------------------
    let alert_source_id = demo_uuid(320);
    let field_service_board = demo_uuid(61);
    sqlx::query(
        "INSERT INTO alert_sources (id, org_id, name, source_ref, secret_encrypted,
                                    board_id, client_id, correlation_window_secs, enabled, created_by)
         VALUES ($1, $2, 'NinjaOne Demo', 'demo-ninjaone', $3, $4, $5, 900, true, $6)",
    )
    .bind(alert_source_id)
    .bind(DEMO_ORG_ID)
    .bind(seal(&opts.seal_key, DEMO_ALERT_HMAC_SECRET.as_bytes())?)
    .bind(field_service_board)
    .bind(acme)
    .bind(owner)
    .execute(&mut *tx)
    .await?;
    trail_system_write(
        &mut tx,
        "alert_source",
        alert_source_id,
        serde_json::json!({ "name": "NinjaOne Demo", "source_ref": "demo-ninjaone" }),
    )
    .await?;

    // Incident A: three critical file-server alerts collapse into the
    // ransomware ticket (104).
    let storm_a = demo_uuid(104);
    for (id_offset, dedup, severity, minutes_ago) in [
        (340u64, "alt-file-001", "warning", 125i64),
        (341, "alt-file-002", "critical", 110),
        (342, "alt-file-003", "critical", 65),
    ] {
        sqlx::query(
            "INSERT INTO alerts (id, org_id, source_id, dedup_key, correlation_key, title,
                                 description, severity, occurred_at, incident_ticket_id, received_at)
             VALUES ($1, $2, $3, $4, 'srv-file-01', $5, $6, $7, $8, $9, $8)",
        )
        .bind(demo_uuid(id_offset))
        .bind(DEMO_ORG_ID)
        .bind(alert_source_id)
        .bind(dedup)
        .bind(format!("Mass file activity detected ({severity})"))
        .bind("EDR agent on SRV-FILE-01 reported sustained mass file modification.".to_string())
        .bind(severity)
        .bind(now - Duration::minutes(minutes_ago))
        .bind(storm_a)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "INSERT INTO alert_incidents (id, org_id, source_id, correlation_key, ticket_id,
                                      alert_count, first_alert_at, last_alert_at)
         VALUES ($1, $2, $3, 'srv-file-01', $4, 3, $5, $6)",
    )
    .bind(demo_uuid(330))
    .bind(DEMO_ORG_ID)
    .bind(alert_source_id)
    .bind(storm_a)
    .bind(now - Duration::minutes(125))
    .bind(now - Duration::minutes(65))
    .execute(&mut *tx)
    .await?;

    // Incident B: a two-alert UPS storm collapses into the UPS ticket (112).
    let storm_b = demo_uuid(112);
    for (id_offset, dedup, minutes_ago) in
        [(343u64, "alt-ups-001", 200i64), (344, "alt-ups-002", 150)]
    {
        sqlx::query(
            "INSERT INTO alerts (id, org_id, source_id, dedup_key, correlation_key, title,
                                 description, severity, occurred_at, incident_ticket_id, received_at)
             VALUES ($1, $2, $3, $4, 'ups-unit-7', 'UPS self-test failed',
                     'Battery bank 7 failed the weekly self-test twice in a row.',
                     'critical', $5, $6, $5)",
        )
        .bind(demo_uuid(id_offset))
        .bind(DEMO_ORG_ID)
        .bind(alert_source_id)
        .bind(dedup)
        .bind(now - Duration::minutes(minutes_ago))
        .bind(storm_b)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "INSERT INTO alert_incidents (id, org_id, source_id, correlation_key, ticket_id,
                                      alert_count, first_alert_at, last_alert_at)
         VALUES ($1, $2, $3, 'ups-unit-7', $4, 2, $5, $6)",
    )
    .bind(demo_uuid(331))
    .bind(DEMO_ORG_ID)
    .bind(alert_source_id)
    .bind(storm_b)
    .bind(now - Duration::minutes(200))
    .bind(now - Duration::minutes(150))
    .execute(&mut *tx)
    .await?;

    // --- report definition + 1 schedule -----------------------------------
    let report_definition_id = demo_uuid(350);
    sqlx::query(
        "INSERT INTO report_definitions (id, org_id, name, description, kind, spec, viz, scope, created_by)
         VALUES ($1, $2, 'Ticket volume by priority', 'Demo report: open ticket counts per priority bucket.',
                 'tickets', $3, 'bar', 'org', $4)",
    )
    .bind(report_definition_id)
    .bind(DEMO_ORG_ID)
    .bind(serde_json::json!({
        "filters": [],
        "group_by": ["priority"],
        "measures": [{ "agg": "count" }]
    }))
    .bind(owner)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO report_schedules (id, org_id, report_definition_id, name, recipients,
                                       cadence, time_of_day, day_of_week, timezone, window_days,
                                       enabled, next_run_at, created_by)
         VALUES ($1, $2, $3, 'Weekly ticket digest', $4, 'weekly', '08:00', 0, 'UTC', 30,
                 true, $5, $6)",
    )
    .bind(demo_uuid(351))
    .bind(DEMO_ORG_ID)
    .bind(report_definition_id)
    .bind(&[owner][..])
    .bind(next_monday_0800(now))
    .bind(owner)
    .execute(&mut *tx)
    .await?;
    trail_system_write(
        &mut tx,
        "report_schedule",
        demo_uuid(351),
        serde_json::json!({ "name": "Weekly ticket digest" }),
    )
    .await?;

    // --- webhook endpoint (test-fire ready) --------------------------------
    let webhook_id = demo_uuid(360);
    sqlx::query(
        "INSERT INTO webhooks (id, org_id, url, secret_encrypted, event_types, enabled, created_by)
         VALUES ($1, $2, 'http://localhost:9099/marula-demo-hook', $3, '{}', true, $4)",
    )
    .bind(webhook_id)
    .bind(DEMO_ORG_ID)
    .bind(seal(&opts.seal_key, DEMO_WEBHOOK_SECRET.as_bytes())?)
    .bind(owner)
    .execute(&mut *tx)
    .await?;
    trail_system_write(
        &mut tx,
        "webhook",
        webhook_id,
        serde_json::json!({ "url": "http://localhost:9099/marula-demo-hook" }),
    )
    .await?;

    // --- taxonomy: classify a slice of tickets with the org's standard
    // taxonomy (org creation seeds STANDARD_TYPES/CATEGORIES; classify by
    // name so the seed never duplicates the baseline) ------------------------
    for (ticket, type_name, category_name) in [
        ("VPN drops every hour", "Incident", "Network"),
        (
            "License renewal question",
            "Service Request",
            "Line-of-Business Applications",
        ),
        ("Ransomware alert on file server", "Incident", "Security"),
        ("Monitor flicker on dock", "Incident", "End-User Computing"),
        ("Quarterly maintenance visit", "Change", "Network"),
    ] {
        sqlx::query(
            "UPDATE tickets SET type_id = (SELECT id FROM ticket_types WHERE org_id = $2 AND name = $3),
                             category_id = (SELECT id FROM ticket_categories WHERE org_id = $2 AND name = $4)
             WHERE id = $1 AND org_id = $2",
        )
        .bind(demo_uuid(
            100 + ticket_specs
                .iter()
                .find(|spec| spec.title == ticket)
                .map(|spec| spec.index)
                .unwrap_or(1),
        ))
        .bind(DEMO_ORG_ID)
        .bind(type_name)
        .bind(category_name)
        .execute(&mut *tx)
        .await?;
    }

    // --- tags + applied ticket tags ----------------------------------------
    let tag_specs: [(&str, &str, u64); 4] = [
        ("urgent", "#d64545", 720),
        ("network", "#3b7dd8", 721),
        ("security", "#d8912b", 722),
        ("warranty", "#4c9a57", 723),
    ];
    for (name, color, off) in tag_specs {
        sqlx::query(
            "INSERT INTO tags (id, org_id, name, color, created_by) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(demo_uuid(off))
        .bind(DEMO_ORG_ID)
        .bind(name)
        .bind(color)
        .bind(owner)
        .execute(&mut *tx)
        .await?;
    }
    for (ticket, tag_off) in [
        (101u64, 720u64),
        (101, 722),
        (104, 720),
        (104, 722),
        (108, 721),
        (111, 721),
        (107, 723),
    ] {
        sqlx::query(
            "INSERT INTO ticket_tags (org_id, ticket_id, tag_id, applied_by)
             VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
        )
        .bind(DEMO_ORG_ID)
        .bind(demo_uuid(ticket))
        .bind(demo_uuid(tag_off))
        .bind(owner)
        .execute(&mut *tx)
        .await?;
    }

    // --- device assets (assets page) ----------------------------------------
    // (id offset, client, site, tag, name, kind, status, warranty, scope).
    // Warranty states are the three the page actually draws — covered,
    // ending, ended — and `scope` carries the MSP's own kit beside the
    // clients' (migration 0103: internal_org never carries a client).
    type AssetSpec = (
        u64,
        Option<Uuid>,
        Option<Uuid>,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        Option<&'static str>,
        &'static str,
    );
    let asset_specs: [AssetSpec; 8] = [
        (
            730,
            Some(acme),
            Some(demo_uuid(30)),
            "ACME-LT-0142",
            "Dell Latitude 5540 — Dana Reeve",
            "laptop",
            "active",
            // The mobile pack's scanned code. A 36-month horizon bought three
            // years back, so its warranty is genuinely ending — the found
            // sheet's third fact has real data instead of a null.
            Some("ending"),
            "client",
        ),
        (
            731,
            Some(acme),
            Some(demo_uuid(30)),
            "ACME-FW-01",
            "FortiGate 100F edge firewall",
            "network",
            "active",
            // Same horizon, bought a couple of months earlier: the lapsed
            // bucket, so both sides of the window exist in the demo.
            Some("ended"),
            "client",
        ),
        (
            732,
            Some(acme),
            None,
            "ACME-SW-RENT-03",
            "Rented 48-port PoE switch (comms room)",
            "network",
            "rented",
            None,
            "client",
        ),
        (
            733,
            Some(northwind),
            Some(demo_uuid(31)),
            "NW-SRV-EX-01",
            "Exchange migration target host",
            "server",
            "active",
            None,
            "client",
        ),
        (
            734,
            Some(northwind),
            None,
            "NW-LT-0031",
            "MacBook Pro 14 — Priya Natarajan",
            "laptop",
            "active",
            None,
            "client",
        ),
        (
            735,
            Some(globex),
            None,
            "GLX-PRN-WH-02",
            "Zebra warehouse label printer",
            "printer",
            "retired",
            None,
            "client",
        ),
        (
            // COVERED — the third side of the warranty story. Every other
            // demo asset was either inside the 90-day callout window or past
            // its horizon, so "no action needed" never appeared on the page.
            736,
            Some(acme),
            Some(demo_uuid(30)),
            "ACME-SRV-FS-01",
            "Synology RS1221+ primary file server",
            "server",
            "active",
            Some("active"),
            "client",
        ),
        (
            // The MSP's own kit: same lifecycle, same warranty rule, no
            // client. Rides `scope = 'internal_org'`, which is the only way
            // a client-less asset is legal (migration 0103's shape CHECK).
            737,
            None,
            None,
            "MARULA-FLD-01",
            "Field service laptop (internal)",
            "laptop",
            "active",
            Some("active"),
            "internal_org",
        ),
    ];
    for (off, client, site, tag, name, kind, status, warranty, scope) in asset_specs {
        sqlx::query(
            "INSERT INTO device_assets (id, org_id, client_id, site_id, tag, name, kind, status, scope,
                                        purchase_date, purchase_cost_minor, purchase_currency,
                                        useful_life_months, depreciation_method)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'USD', $12, $13)",
        )
        .bind(demo_uuid(off))
        .bind(DEMO_ORG_ID)
        .bind(client)
        .bind(site)
        .bind(tag)
        .bind(name)
        .bind(kind)
        .bind(status)
        .bind(scope)
        .bind(match warranty {
            // 36 months is a normal horizon; the purchase date is what puts
            // the asset on one side of it or the other.
            Some(_) => warranty_purchase_date(today, 36, warranty),
            None => last_month_window(today).0, // stable recent-ish purchase date
        })
        .bind(120_000_i64)
        .bind(warranty.map(|_| 36_i32))
        .bind("straight_line")
        .execute(&mut *tx)
        .await?;
        trail_system_write(
            &mut tx,
            "device_asset",
            demo_uuid(off),
            serde_json::json!({ "tag": tag, "scope": scope }),
        )
        .await?;
    }

    // --- sales catalog (sales/catalog) --------------------------------------
    let catalog_specs: [(&str, &str, &str, i64, i64, &str, &str); 6] = [
        (
            "Managed workstation",
            "MGD-WS",
            "Per-seat endpoint management + patching",
            4_500,
            1_200,
            "service",
            "Managed IT",
        ),
        (
            "Server monitoring",
            "MGD-SRV",
            "24/7 monitoring and alert triage",
            9_900,
            3_000,
            "service",
            "Managed IT",
        ),
        (
            "Firewall appliance (FortiGate 100F)",
            "HW-FG100F",
            "Shipped + racked + configured",
            320_000,
            210_000,
            "product",
            "Hardware",
        ),
        (
            "48-port PoE switch",
            "HW-SW48P",
            "Deployed switch with config backup",
            180_000,
            120_000,
            "product",
            "Hardware",
        ),
        (
            "Onsite labor",
            "SRV-ONSITE",
            "Technician hour, on-site",
            14_500,
            8_200,
            "service",
            "Labor",
        ),
        (
            "Offsite backup (per TB)",
            "SRV-BKP",
            "Encrypted offsite backup storage",
            5_000,
            1_800,
            "service",
            "Backup",
        ),
    ];
    for (i, (name, sku, description, price, cost, category, group_name)) in
        catalog_specs.iter().enumerate()
    {
        sqlx::query(
            "INSERT INTO catalog_items (id, org_id, name, sku, description, unit, currency,
                                        unit_price_minor, cost_minor, taxable, category, group_name, created_by)
             VALUES ($1, $2, $3, $4, $5, 'each', 'USD', $6, $7, true, $8, $9, $10)",
        )
        .bind(demo_uuid(740 + i as u64))
        .bind(DEMO_ORG_ID)
        .bind(name)
        .bind(sku)
        .bind(description)
        .bind(price)
        .bind(cost)
        .bind(category)
        .bind(group_name)
        .bind(owner)
        .execute(&mut *tx)
        .await?;
    }

    // --- quotes + lines (sales/quotes) --------------------------------------
    struct DemoQuote {
        off: u64,
        client: Uuid,
        status: &'static str,
        days_issued: i64,
        valid_days: i64,
        lines: &'static [(u64, i64, i64)], // (catalog off, qty_milli, unit_amount_minor)
        converted: Option<Uuid>,
        /// The portal quotes page uses `notes` as the row's title, so this is
        /// the first thing a client's decision-maker reads. "Demo quote
        /// generated by the seed" is not that.
        notes: &'static str,
    }
    let quote_specs: [DemoQuote; 3] = [
        DemoQuote {
            off: 750,
            client: acme,
            status: "sent",
            days_issued: 4,
            valid_days: 26,
            lines: &[(742, 1_000, 320_000), (744, 2_000, 180_000)],
            converted: None,
            notes: "Comms-room firewall and switch replacement — quoted after the plant-2 survey",
        },
        DemoQuote {
            off: 753,
            client: northwind,
            status: "accepted",
            days_issued: 21,
            valid_days: -1, // already decided
            lines: &[(740, 25_000, 4_500), (745, 3_000, 5_000)],
            converted: Some(contract_recurring),
            notes: "Managed workstations for 25 seats, plus the first month of offsite backup",
        },
        DemoQuote {
            off: 755,
            client: globex,
            status: "draft",
            days_issued: 0,
            valid_days: 30,
            lines: &[(741, 1_000, 9_900)],
            converted: None,
            notes: "Server monitoring for the warehouse hosts — draft, not sent yet",
        },
    ];
    for spec in &quote_specs {
        let subtotal: i64 = spec
            .lines
            .iter()
            .map(|(_, qty, unit)| (qty / 1_000) * unit)
            .sum();
        let tax = subtotal * 750 / 10_000; // 7.5%
        let sent_at = if spec.status == "draft" {
            None
        } else {
            Some(at(today - Duration::days(spec.days_issued), 10, 0))
        };
        let decided_at = if spec.status == "accepted" {
            Some(at(today - Duration::days(2), 15, 30))
        } else {
            None
        };
        sqlx::query(
            "INSERT INTO quotes (id, org_id, number, client_id, status, currency, issued_on,
                                 valid_until, notes, subtotal_minor, tax_rate_bps, tax_minor,
                                 total_minor, sent_at, decided_at, converted_contract_id)
             VALUES ($1, $2, $3, $4, $5, 'USD', $6, $7, $8, $9, 750, $10, $11, $12, $13, $14)",
        )
        .bind(demo_uuid(spec.off))
        .bind(DEMO_ORG_ID)
        .bind(format!("QT-{}", spec.off - 749))
        .bind(spec.client)
        .bind(spec.status)
        .bind(today - Duration::days(spec.days_issued))
        .bind(today + Duration::days(spec.valid_days))
        .bind(spec.notes)
        .bind(subtotal)
        .bind(tax)
        .bind(subtotal + tax)
        .bind(sent_at)
        .bind(decided_at)
        .bind(spec.converted)
        .execute(&mut *tx)
        .await?;
        for (pos, (catalog_off, qty, unit)) in spec.lines.iter().enumerate() {
            sqlx::query(
                "INSERT INTO quote_lines (id, org_id, quote_id, catalog_item_id, description,
                                          unit, quantity_milli, unit_amount_minor, taxable,
                                          amount_minor, position)
                 VALUES ($1, $2, $3, $4, $5, 'each', $6, $7, true, $8, $9)",
            )
            .bind(demo_uuid(spec.off + 1 + pos as u64))
            .bind(DEMO_ORG_ID)
            .bind(demo_uuid(spec.off))
            .bind(demo_uuid(*catalog_off))
            .bind(catalog_specs[(catalog_off - 740) as usize].0)
            .bind(qty)
            .bind(unit)
            .bind((qty / 1_000) * unit)
            .bind(pos as i32)
            .execute(&mut *tx)
            .await?;
        }
        trail_system_write(
            &mut tx,
            "quote",
            demo_uuid(spec.off),
            serde_json::json!({ "status": spec.status }),
        )
        .await?;
    }

    // --- pipeline stages + deals (sales/pipeline) ----------------------------
    let stage_specs: [(&str, i32, &str); 5] = [
        ("Lead", 0, "open"),
        ("Qualified", 1, "open"),
        ("Proposal Sent", 2, "open"),
        ("Won", 3, "won"),
        ("Lost", 4, "lost"),
    ];
    for (name, pos, kind) in stage_specs {
        sqlx::query(
            "INSERT INTO pipeline_stages (id, org_id, name, position, stage_type)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(demo_uuid(760 + pos as u64))
        .bind(DEMO_ORG_ID)
        .bind(name)
        .bind(pos)
        .bind(kind)
        .execute(&mut *tx)
        .await?;
    }
    let deal_specs: [(u64, Uuid, u64, &str, i64, &str, i64); 4] = [
        (
            765,
            acme,
            762,
            "Acme comms-room refresh",
            1_800_000,
            "open",
            30,
        ),
        (
            766,
            globex,
            761,
            "Globex backup consolidation",
            950_000,
            "open",
            45,
        ),
        (
            767,
            northwind,
            763,
            "Northwind managed workstations (25 seats)",
            2_400_000,
            "won",
            -3,
        ),
        (
            768,
            globex,
            764,
            "Globex printer fleet RFP",
            400_000,
            "lost",
            -10,
        ),
    ];
    for (off, client, stage, name, amount, status, close_days) in deal_specs {
        sqlx::query(
            "INSERT INTO pipeline_deals (id, org_id, client_id, stage_id, name, amount_minor,
                                         currency, status, expected_close_on, owner_user_id,
                                         closed_at, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, 'USD', $7, $8, $9, $10, $9)",
        )
        .bind(demo_uuid(off))
        .bind(DEMO_ORG_ID)
        .bind(client)
        .bind(demo_uuid(stage))
        .bind(name)
        .bind(amount)
        .bind(status)
        .bind(today + Duration::days(close_days))
        .bind(owner)
        .bind(if status == "open" {
            None
        } else {
            Some(at(today - Duration::days(close_days.abs()), 16, 0))
        })
        .execute(&mut *tx)
        .await?;
        trail_system_write(
            &mut tx,
            "pipeline_deal",
            demo_uuid(off),
            serde_json::json!({ "name": name }),
        )
        .await?;
    }
    // One transition each: created → current stage (won/lost deals get 2 hops).
    for (deal_off, from_off, to_off, days_ago) in [
        (765u64, 760u64, 762u64, 12i64),
        (766, 760, 761, 9),
        (767, 762, 763, 3),
        (768, 761, 764, 10),
    ] {
        sqlx::query(
            "INSERT INTO pipeline_deal_transitions (id, org_id, deal_id, from_stage_id, to_stage_id, actor_id, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(demo_uuid(770 + deal_off - 765))
        .bind(DEMO_ORG_ID)
        .bind(demo_uuid(deal_off))
        .bind(demo_uuid(from_off))
        .bind(demo_uuid(to_off))
        .bind(owner)
        .bind(at(today - Duration::days(days_ago), 11, 15))
        .execute(&mut *tx)
        .await?;
    }

    // --- credit note (billing/credit-notes) ----------------------------------
    let credit_note_id = demo_uuid(780);
    sqlx::query(
        "INSERT INTO credit_notes (id, org_id, number, client_id, issued_against_invoice_id,
                                   status, currency, total_minor, applied_minor, reason, created_by)
         VALUES ($1, $2, 'CN-0001', $3, $4, 'issued', 'USD', 50_000, 50_000,
                 'Duplicate seat billed on the dunning run', $5)",
    )
    .bind(credit_note_id)
    .bind(DEMO_ORG_ID)
    .bind(acme)
    .bind(invoice_dunning_id)
    .bind(bookkeeper)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO credit_note_applications (id, org_id, credit_note_id, invoice_id, amount_minor, created_by)
         VALUES ($1, $2, $3, $4, 50_000, $5)",
    )
    .bind(demo_uuid(781))
    .bind(DEMO_ORG_ID)
    .bind(credit_note_id)
    .bind(invoice_dunning_id)
    .bind(bookkeeper)
    .execute(&mut *tx)
    .await?;

    // NOTE: fx_rates is deliberately NOT seeded — migration 0066 makes the
    // table INSERT-only (UPDATE/DELETE raise, cascade included), so demo
    // rows could never be wiped by `reset`. FX surfaces still demo through
    // the ticket cost lines' fx_rate_used snapshots below.

    // --- password vault entries (passwords page; sealed at rest) --------------
    type VaultSpec = (
        u64,
        Option<Uuid>,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    );
    let vault_specs: [VaultSpec; 3] = [
        (
            790,
            Some(acme),
            "Acme firewall admin",
            "admin",
            "https://fw.acme.demo",
            "demo-vault-secret-acme-firewall",
        ),
        (
            791,
            Some(northwind),
            "Northwind DNS registrar",
            "dns-admin",
            "https://registrar.demo",
            "demo-vault-secret-nw-dns",
        ),
        (
            792,
            None,
            "Shared break-glass credential",
            "svc-breakglass",
            "",
            "demo-vault-secret-breakglass",
        ),
    ];
    for (off, client, title, username, url, secret) in vault_specs {
        sqlx::query(
            "INSERT INTO passwords (id, org_id, client_id, title, username, url, secret_ciphertext,
                                    expires_at, created_by, updated_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)",
        )
        .bind(demo_uuid(off))
        .bind(DEMO_ORG_ID)
        .bind(client)
        .bind(title)
        .bind(username)
        .bind(url)
        .bind(seal(&opts.seal_key, secret.as_bytes())?)
        .bind::<Option<chrono::DateTime<Utc>>>(None)
        .bind(owner)
        .execute(&mut *tx)
        .await?;
    }

    // --- org API key (settings/api-keys; hash keyed by the at-rest secret) ----
    let demo_api_raw = "mrl_demo0000000000000000000000000000000000000000000000000000key";
    use hmac::Mac;
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&opts.seal_key)
        .map_err(|e| DemoError::Crypto(e.to_string()))?;
    mac.update(demo_api_raw.as_bytes());
    let api_key_id = demo_uuid(795);
    sqlx::query(
        "INSERT INTO api_keys (id, org_id, name, key_hash, prefix, scopes, created_by)
         VALUES ($1, $2, 'Demo connector', $3, 'mrl_demo0000', $4, $5)",
    )
    .bind(api_key_id)
    .bind(DEMO_ORG_ID)
    .bind(hex::encode(mac.finalize().into_bytes()))
    .bind(serde_json::json!(["tickets:read", "clients:read"]))
    .bind(owner)
    .execute(&mut *tx)
    .await?;
    trail_system_write(
        &mut tx,
        "api_key",
        api_key_id,
        serde_json::json!({ "name": "Demo connector" }),
    )
    .await?;

    // --- notifications (bell menu) --------------------------------------------
    type NotifSpec = (
        u64,
        Uuid,
        &'static str,
        Option<u64>,
        &'static str,
        &'static str,
    );
    let notif_specs: [NotifSpec; 5] = [
        (
            800,
            owner,
            "mention",
            Some(101),
            "Dana mentioned this ticket",
            "Portal contact replied on VPN drops every hour",
        ),
        (
            801,
            owner,
            "sla_risk",
            Some(102),
            "SLA at risk",
            "License renewal question approaches its first-response target",
        ),
        (
            802,
            tech,
            "assignment",
            Some(108),
            "Assigned to you",
            "Monitor flicker on dock was assigned to you",
        ),
        (
            803,
            tech,
            "escalation",
            Some(104),
            "Escalated",
            "Ransomware alert on file server escalated to P1 bridge",
        ),
        (
            804,
            owner,
            "missing_time",
            None,
            "Missing time",
            "Yesterday has unlogged time for 2 technicians",
        ),
    ];
    for (off, user, kind, ticket, title, body) in notif_specs {
        sqlx::query(
            "INSERT INTO notifications (id, org_id, user_id, kind, ticket_id, title, body, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(demo_uuid(off))
        .bind(DEMO_ORG_ID)
        .bind(user)
        .bind(kind)
        .bind(ticket.map(demo_uuid))
        .bind(title)
        .bind(body)
        .bind(now - Duration::hours((off - 799) as i64))
        .execute(&mut *tx)
        .await?;
    }

    // --- availability windows (scheduling base grid) ---------------------------
    for user in [tech, dispatcher] {
        for weekday in 0..5 {
            sqlx::query(
                "INSERT INTO availability_windows (id, org_id, user_id, weekday, start_local, end_local, timezone)
                 VALUES ($1, $2, $3, $4, $5, $6, 'UTC')",
            )
            .bind(demo_uuid(810 + weekday as u64 + if user == tech { 0 } else { 5 }))
            .bind(DEMO_ORG_ID)
            .bind(user)
            .bind(weekday as i16)
            .bind(NaiveTime::from_hms_opt(9, 0, 0).expect("valid time"))
            .bind(NaiveTime::from_hms_opt(17, 0, 0).expect("valid time"))
            .execute(&mut *tx)
            .await?;
        }
    }

    // --- ticket costs + expenses (ticket cost rollups, FX-aware) ---------------
    let cost_specs: [(u64, u64, &str, &str, i64, bool, bool); 3] = [
        (
            820,
            101,
            "parts",
            "Replacement firewall power supply",
            42_000,
            false,
            false,
        ),
        (
            821,
            109,
            "other",
            "Warehouse LTE failover (monthly)",
            8_500,
            false,
            true,
        ),
        (
            822,
            101,
            "flat",
            "Firewall management retainer",
            12_000,
            false,
            true,
        ),
    ];
    for (off, ticket, kind, description, unit_cost, has_fx, billable) in cost_specs {
        sqlx::query(
            "INSERT INTO ticket_cost_lines (id, org_id, ticket_id, kind, description, quantity,
                                            unit_cost_minor, currency, billable, created_by,
                                            fx_rate_used, fx_rate_date, fx_source, base_amount_minor)
             VALUES ($1, $2, $3, $4, $5, 1, $6, $7, $8, $9, $10::numeric, $11, $12, $13)",
        )
        .bind(demo_uuid(off))
        .bind(DEMO_ORG_ID)
        .bind(demo_uuid(ticket))
        .bind(kind)
        .bind(description)
        .bind(unit_cost)
        .bind("EUR")
        .bind(billable)
        .bind(bookkeeper)
        .bind(has_fx.then_some("0.92"))
        .bind(has_fx.then_some(today - Duration::days(1)))
        .bind(has_fx.then_some("demo-static"))
        .bind(has_fx.then_some(unit_cost * 92 / 100))
        .execute(&mut *tx)
        .await?;
    }
    let expense_specs: [(u64, u64, &str, i64, bool, Uuid); 2] = [
        (
            825,
            110,
            "Mileage — quarterly walkthrough",
            4_800,
            true,
            tech,
        ),
        (
            826,
            112,
            "Courier — failed UPS unit return",
            3_200,
            false,
            tech,
        ),
    ];
    for (off, ticket, description, amount, billable, by) in expense_specs {
        sqlx::query(
            "INSERT INTO ticket_expenses (id, org_id, ticket_id, description, amount_minor, currency,
                                          billable, incurred_by, incurred_at)
             VALUES ($1, $2, $3, $4, $5, 'USD', $6, $7, $8)",
        )
        .bind(demo_uuid(off))
        .bind(DEMO_ORG_ID)
        .bind(demo_uuid(ticket))
        .bind(description)
        .bind(amount)
        .bind(billable)
        .bind(by)
        .bind(now - Duration::days(3))
        .execute(&mut *tx)
        .await?;
    }

    // --- PM schedules + runs (recurring preventive maintenance) ----------------
    sqlx::query(
        "INSERT INTO pm_schedules (id, org_id, name, board_id, title, body, priority, client_id,
                                   site_id, assignee_id, cadence, time_of_day, day_of_month,
                                   timezone, enabled, next_run_at, last_run_at, created_by)
         VALUES ($1, $2, 'Quarterly Acme walkthrough', $3, 'Quarterly maintenance visit',
                 'Scheduled walkthrough of Acme HQ.', 'p2', $4, $5, $6,
                 'monthly', $7, 1, 'UTC', true, $8, $9, $10)",
    )
    .bind(demo_uuid(830))
    .bind(DEMO_ORG_ID)
    .bind(demo_uuid(61)) // field-service board
    .bind(acme)
    .bind(demo_uuid(30))
    .bind(tech)
    .bind(NaiveTime::from_hms_opt(8, 0, 0).expect("valid time"))
    .bind(now + Duration::days(45))
    .bind(now - Duration::days(40))
    .bind(owner)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO pm_schedules (id, org_id, name, board_id, title, body, priority, client_id,
                                   assignee_id, cadence, time_of_day, day_of_week,
                                   timezone, enabled, next_run_at, created_by)
         VALUES ($1, $2, 'Weekly Globex backup check', $3, 'Backup restore spot-check',
                 'Verify the latest offsite backup completed and restore-test one file.', 'p3',
                 $4, $5, 'weekly', $6, 0, 'UTC', true, $7, $8)",
    )
    .bind(demo_uuid(831))
    .bind(DEMO_ORG_ID)
    .bind(demo_uuid(60)) // service desk board
    .bind(globex)
    .bind(tech)
    .bind(NaiveTime::from_hms_opt(7, 30, 0).expect("valid time"))
    .bind(now + Duration::days(6))
    .bind(owner)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO pm_runs (id, org_id, schedule_id, slot_at, status, ticket_id, detail)
         VALUES ($1, $2, $3, $4, 'generated', $5, '{}'),
                ($6, $2, $3, $7, 'skipped', NULL, $8)",
    )
    .bind(demo_uuid(835))
    .bind(DEMO_ORG_ID)
    .bind(demo_uuid(830))
    .bind(now - Duration::days(85))
    .bind(demo_uuid(110)) // the ticket the last run generated
    .bind(demo_uuid(836))
    .bind(now - Duration::days(10))
    .bind(serde_json::json!({ "reason": "no_enabled_schedules" }))
    .execute(&mut *tx)
    .await?;

    // --- meeting type + scheduling link (scheduling links; token sha256) ------
    sqlx::query(
        "INSERT INTO meeting_types (id, org_id, name, description, duration_minutes, buffer_minutes,
                                    booking_window_days, parameters, assignee_scope, assignee_user_id,
                                    auto_email_enabled)
         VALUES ($1, $2, 'Service triage call', 'Quick scoping call for a new issue.', 15, 5, 14,
                 '{}', 'user', $3, false)",
    )
    .bind(demo_uuid(840))
    .bind(DEMO_ORG_ID)
    .bind(tech)
    .execute(&mut *tx)
    .await?;
    let link_token = "demo-scheduling-link-token-not-a-real-secret";
    let link_token_hash: String = {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(link_token.as_bytes());
        digest.iter().map(|b| format!("{b:02x}")).collect()
    };
    sqlx::query(
        "INSERT INTO scheduling_links (id, org_id, meeting_type_id, token_hash, label, ticket_id,
                                       status, view_count, created_by, created_at)
         VALUES ($1, $2, $3, $4, 'Triage link — Acme VPN issue', $5, 'active', 2, $6, $7)",
    )
    .bind(demo_uuid(845))
    .bind(DEMO_ORG_ID)
    .bind(demo_uuid(840))
    .bind(link_token_hash)
    .bind(demo_uuid(101))
    .bind(dispatcher)
    .bind(now - Duration::days(2))
    .execute(&mut *tx)
    .await?;

    // --- automation rules (automations page) -----------------------------------
    sqlx::query(
        // `created_by` is not decoration: an automation effect runs as the
        // rule's author (p58, ruling A), so a seeded rule with no author is a
        // rule that refuses every firing and dead-letters.
        "INSERT INTO automation_rules (id, org_id, board_id, name, trigger_event, conditions, actions, enabled, created_by)
         VALUES ($1, $2, $3, 'Tag security alerts urgent', 'ticket.created',
                 $4, $5, true, $9),
                ($6, $2, $3, 'Notify dispatcher on P1', 'ticket.updated',
                 $7, $8, true, $9)",
    )
    .bind(demo_uuid(850))
    .bind(DEMO_ORG_ID)
    .bind(demo_uuid(60)) // service desk board
    .bind(serde_json::json!([{ "field": "category", "op": "eq", "value": "Security" }]))
    .bind(serde_json::json!([{ "type": "set_priority", "priority": "p1" }]))
    .bind(demo_uuid(851))
    .bind(serde_json::json!([{ "field": "priority", "op": "eq", "value": "p1" }]))
    .bind(serde_json::json!([{ "type": "notify_users", "user_ids": [demo_uuid(11).to_string()], "kind": "ticket_update", "title": "P1 ticket created" }]))
    .bind(owner)
    .execute(&mut *tx)
    .await?;

    // --- time-entry approval trail (approvals page) -----------------------------
    // A bulk time entry gets submitted→approved hops so the approvals
    // timeline has history to show.
    for (seq, action) in [(0i32, "submitted"), (1, "approved")] {
        sqlx::query(
            "INSERT INTO time_entry_approvals (id, org_id, time_entry_id, seq, action, actor_id, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(demo_uuid(860 + seq as u64))
        .bind(DEMO_ORG_ID)
        .bind(demo_uuid(600))
        .bind(seq)
        .bind(action)
        .bind(if action == "submitted" { tech } else { owner })
        .bind(at(today - Duration::days(1), 9 + seq as u32, 0))
        .execute(&mut *tx)
        .await?;
    }

    // --- plugin install (settings/plugins) --------------------------------------
    // plugin_catalog is GLOBAL (no org_id, no cascade): upsert by slug and
    // reuse the existing catalog row so reset→reseed converges.
    sqlx::query(
        "INSERT INTO plugin_catalog (id, slug, name, description, publisher, origin_org_id,
                                     latest_version, interface_version)
         VALUES ($1, 'sample-stock-ticker', 'Sample stock ticker', 'Demo plugin: ticker side panel.',
                 'marula-demo', $2, '1.0.0', 1)
         ON CONFLICT (slug) DO NOTHING",
    )
    .bind(demo_uuid(870))
    // The demo org owns the demo plugin, so the demo can still exercise the
    // revoke path; a NULL origin would make it operator-owned and immutable
    // through the tenant catalog routes.
    .bind(DEMO_ORG_ID)
    .execute(&mut *tx)
    .await?;
    let plugin_catalog_id: Uuid =
        sqlx::query_scalar("SELECT id FROM plugin_catalog WHERE slug = 'sample-stock-ticker'")
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query(
        "INSERT INTO plugin_installs (id, org_id, catalog_id, version, status, granted_capabilities, installed_by)
         VALUES ($1, $2, $3, '1.0.0', 'enabled', $4, $5)",
    )
    .bind(demo_uuid(875))
    .bind(DEMO_ORG_ID)
    .bind(plugin_catalog_id)
    .bind(serde_json::json!(["panel:stock"]))
    .bind(owner)
    .execute(&mut *tx)
    .await?;

    // =======================================================================
    // Attachments, documents, email and the remaining PSA surfaces.
    //
    // Everything above is the ticketing/billing core a sales demo needs. This
    // block covers the rest of the product so the same seeded instance can
    // show off the whole PSA: the attachment ledger (ticket, comment and
    // document owners), the email edge, CSAT, the secrets manager, contract
    // legs, the time taxonomy, purchasing/lending completion, board and
    // calendar metadata, reports and fx.
    //
    // Determinism: every id is `demo_uuid(<offset>)` in the 1000-1099 range,
    // which no other part of this file uses. Ids written by a REAL write path
    // (loans, purchase orders, plugin installs) are read back out in a
    // deterministic order rather than assumed.
    // =======================================================================

    let sha256_hex = |bytes: &[u8]| -> String {
        use sha2::Digest as _;
        let mut hasher = sha2::Sha256::new();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    };

    // The core-store adapter the api uses. `build(&StoreChoice::Local)`
    // resolves exactly the same path core-api reads from
    // `OBJECT_STORE_LOCAL_DIR`, so bytes land where the download endpoint
    // looks. `None` (tests, or a caller with no store configured) still
    // writes the metadata rows — only the byte fetch fails, which the route
    // reports as a missing object rather than corrupting anything.
    let attachment_store = opts.object_store_dir.as_deref().map(|dir| {
        core_store::build(&core_store::StoreChoice::Local(
            dir.to_string_lossy().into_owned(),
        ))
    });

    /// Writes one attachment row and its bytes, following the real upload
    /// path exactly: validate → sha256 → content-addressed key → dedup check
    /// → PUT (skipped when the org already holds these bytes) → row insert
    /// through `core_db::attachments::insert`, which is the write path tenant
    /// traffic uses, so the owner-shape constraints are genuinely exercised.
    struct DemoAttachment {
        id_offset: u64,
        kind: &'static str,
        ticket_index: Option<u64>,
        ticket_comment_id: Option<Uuid>,
        filename: &'static str,
        content_type: &'static str,
        bytes: Vec<u8>,
        uploader: Uuid,
    }
    let dashboard_png = || {
        demo_png(
            160,
            55,
            [30, 64, 96], // a flat "board screenshot" slate
        )
    };
    let mut attachment_count = 0usize;
    let mut attachment_store_bytes = 0u64;
    let attachment_specs = [
        // --- ticket-owned: the three states a screenshot can show ----------
        // Open P2 with a triage capture attached by the technician.
        DemoAttachment {
            id_offset: 1000,
            kind: "ticket",
            ticket_index: Some(1),
            ticket_comment_id: None,
            filename: "wifi-drop-capture.png",
            content_type: "image/png",
            bytes: dashboard_png(),
            uploader: tech,
        },
        // P1 ransomware thread, staff capture of the isolation command run.
        DemoAttachment {
            id_offset: 1001,
            kind: "ticket",
            ticket_index: Some(4),
            ticket_comment_id: None,
            filename: "isolation-run.txt",
            content_type: "text/plain",
            bytes: "port isolation run — comms room switch 10.20.0.9\n\
                    vlan 42 access port gi1/0/24 -> shutdown\n\
                    confirmed off by Mac table + ARP probe\n\
                    verified by second engineer before escalate\n"
                .as_bytes()
                .to_vec(),
            uploader: tech,
        },
        // Resolved ticket with the closing report still attached.
        DemoAttachment {
            id_offset: 1002,
            kind: "ticket",
            ticket_index: Some(7),
            ticket_comment_id: None,
            filename: "licence-usage-audit.csv",
            content_type: "text/csv",
            bytes: "seat,user,state,last_active\n\
                    1,j.okafor,northwind,2026-09-14\n\
                    2,a.patel,northwind,2026-09-15\n\
                    3,m.silva,globex,2026-08-30\n\
                    4,r.halim,acme,2026-09-18\n\
                    5,spare,-,-\n"
                .as_bytes()
                .to_vec(),
            uploader: dispatcher,
        },
        // --- comment-owned: the attachment thread surface -------------------
        // On the client-visible comment of ticket 1, so the portal shows a
        // file the client's own contact uploaded.
        DemoAttachment {
            id_offset: 1003,
            kind: "ticket_comment",
            ticket_index: Some(1),
            ticket_comment_id: Some(demo_uuid(121)),
            filename: "signal-strength-floor1.csv",
            content_type: "text/csv",
            bytes: "ap,channel,rssi_dbm,clients\n\
                    AP-W13-01,6,-71,14\n\
                    AP-W13-02,36,-63,9\n\
                    AP-W13-03,6,-78,11\n\
                    AP-W14-01,44,-58,4\n"
                .as_bytes()
                .to_vec(),
            uploader: owner,
        },
        // On the in-house update of the P1: a portal-visible evidence file.
        DemoAttachment {
            id_offset: 1004,
            kind: "ticket_comment",
            ticket_index: Some(4),
            ticket_comment_id: Some(demo_uuid(123)),
            filename: "restore-plan.pdf",
            content_type: "application/pdf",
            bytes: demo_pdf(&[
                "Acme Manufacturing — incident 2026-09-30 restore plan".into(),
                "".into(),
                "Method: image-from-golden on rebuilt host, then patch gap".into(),
                "identification. No ransom paid; no negotiation opened.".into(),
                "".into(),
                "Isolation: host removed from the switch port at 07:12,".into(),
                "forensic image captured and hashed before any restore.".into(),
                "".into(),
                "Estimated completion: 2h30m of work inside the agreed".into(),
                "window, with a staged file-level verification pass.".into(),
                "".into(),
                "Client sign-off required before the gate opens again.".into(),
                "".into(),
                "Approvals: owner (ops), bookkeeper (contract), Acme".into(),
                "leadership (business).".into(),
            ]),
            uploader: owner,
        },
        // --- document attachments (documents platform) ---------------------
    ];
    for spec in attachment_specs {
        let bytes = spec.bytes;
        // Same gate the upload route applies. A failure here means the
        // fixture itself drifted from the contract, which is a seed bug
        // worth failing the seed over.
        let (filename, content_type) = core_domain::attachments::validate_upload(
            spec.filename,
            spec.content_type,
            &bytes[..bytes.len().min(core_domain::attachments::MAGIC_HEAD_LEN)],
            bytes.len() as i64,
            core_domain::attachments::DEFAULT_MAX_UPLOAD_BYTES,
        )
        .map_err(|e| DbError::Conflict(e.to_string()))?;
        let sha256 = sha256_hex(&bytes);
        let storage_key = core_store::attachment_key(DEMO_ORG_ID, &sha256);

        // Content-addressed dedup, identical to the route: skip the PUT when
        // the org already holds these exact bytes.
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT storage_key FROM attachments WHERE org_id = $1 AND sha256 = $2",
        )
        .bind(DEMO_ORG_ID)
        .bind(&sha256)
        .fetch_optional(&mut *tx)
        .await?;
        if existing.is_none() {
            if let Some(store) = &attachment_store {
                store
                    .put(
                        &storage_key,
                        &content_type,
                        bytes::Bytes::copy_from_slice(&bytes),
                    )
                    .await
                    .map_err(|e| DemoError::Internal(format!("demo attachment put: {e}")))?;
            }
        }

        crate::attachments::insert(
            &mut tx,
            crate::attachments::NewAttachment {
                id: demo_uuid(spec.id_offset),
                org_id: DEMO_ORG_ID,
                kind: spec
                    .kind
                    .parse()
                    .map_err(|e: core_domain::DomainError| DbError::Conflict(e.to_string()))?,
                ticket_id: spec.ticket_index.map(|index| demo_uuid(100 + index)),
                ticket_comment_id: spec.ticket_comment_id,
                email_message_id: None,
                filename: &filename,
                content_type: &content_type,
                size_bytes: bytes.len() as i64,
                sha256: &sha256,
                storage_key: &storage_key,
                uploaded_by_user_id: Some(spec.uploader),
                uploaded_by_portal_user_id: None,
            },
        )
        .await?;
        attachment_count += 1;
        attachment_store_bytes += bytes.len() as u64;
    }

    // --- document attachments (documents platform) -------------------------
    // Same bytes, same content-addressing, different owner: the document the
    // seed already writes gets a PDF runbook and a CSV of the SLA matrix.
    struct DemoDocAttachment {
        id_offset: u64,
        filename: &'static str,
        content_type: &'static str,
        bytes: Vec<u8>,
    }
    let doc_attachment_specs = [
        DemoDocAttachment {
            id_offset: 1005,
            filename: "sla-matrix.pdf",
            content_type: "application/pdf",
            bytes: demo_pdf(&[
                "Marula support — response and resolution targets".into(),
                "".into(),
                "P1 (service down)          respond 15m   resolve 8h".into(),
                "P2 (degraded)              respond 60m   resolve 24h".into(),
                "P3 (single user affected)  respond 4h    resolve 72h".into(),
                "P4 (request / cosmetic)    respond 8h    resolve 7d".into(),
                "".into(),
                "Response is first staff reply, not acknowledgement.".into(),
                "Resolution is verified by the requester where practical.".into(),
                "".into(),
                "Escalation: p1 pages the on-call tech immediately and does".into(),
                "not wait for the next working day.".into(),
            ]),
        },
        DemoDocAttachment {
            id_offset: 1006,
            filename: "quarterly-hours.csv",
            content_type: "text/csv",
            bytes: "client,recurring,retainer,block,overage_minutes\n\
                    acme,38400,0,240,95\n\
                    northwind,0,57600,0,0\n\
                    globex,0,0,180,0\n"
                .as_bytes()
                .to_vec(),
        },
    ];
    let mut document_attachment_count = 0usize;
    for spec in doc_attachment_specs {
        let bytes = spec.bytes;
        let (filename, content_type) = core_domain::attachments::validate_upload(
            spec.filename,
            spec.content_type,
            &bytes[..bytes.len().min(core_domain::attachments::MAGIC_HEAD_LEN)],
            bytes.len() as i64,
            core_domain::attachments::DEFAULT_MAX_UPLOAD_BYTES,
        )
        .map_err(|e| DbError::Conflict(e.to_string()))?;
        let sha256 = sha256_hex(&bytes);
        let storage_key = core_store::attachment_key(DEMO_ORG_ID, &sha256);
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT storage_key FROM attachments WHERE org_id = $1 AND sha256 = $2",
        )
        .bind(DEMO_ORG_ID)
        .bind(&sha256)
        .fetch_optional(&mut *tx)
        .await?;
        if existing.is_none() {
            if let Some(store) = &attachment_store {
                store
                    .put(
                        &storage_key,
                        &content_type,
                        bytes::Bytes::copy_from_slice(&bytes),
                    )
                    .await
                    .map_err(|e| DemoError::Internal(format!("demo attachment put: {e}")))?;
            }
        }
        sqlx::query(
            "INSERT INTO documents_attachments (id, org_id, document_id, filename, content_type,
                                                size_bytes, sha256, storage_key, uploaded_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(demo_uuid(spec.id_offset))
        .bind(DEMO_ORG_ID)
        .bind(document_id)
        .bind(&filename)
        .bind(&content_type)
        .bind(bytes.len() as i64)
        .bind(&sha256)
        .bind(&storage_key)
        .bind(owner)
        .execute(&mut *tx)
        .await?;
        trail_system_write(
            &mut tx,
            "document_attachment",
            demo_uuid(spec.id_offset),
            serde_json::json!({ "filename": filename, "sha256": sha256 }),
        )
        .await?;
        document_attachment_count += 1;
        attachment_store_bytes += bytes.len() as u64;
    }

    // --- email edge (email inbox / outbox) -------------------------------
    // The web composer's outbound queue, and the inbound side that actually
    // files mail against a ticket. Without these the mail surfaces are
    // permanently empty however the API behaves.
    struct DemoEmail {
        id_offset: u64,
        direction: &'static str,
        rfc_message_id: &'static str,
        subject: &'static str,
        from_address: &'static str,
        from_name: &'static str,
        to_addresses: &'static [&'static str],
        body_text: &'static str,
        ticket_index: Option<u64>,
        comment_id: Option<Uuid>,
        in_reply_to: Option<&'static str>,
        scan_status: &'static str,
    }
    let email_specs: [DemoEmail; 6] = [
        // Inbound that created the licence ticket — the "email to ticket"
        // path a prospect will ask about.
        DemoEmail {
            id_offset: 1030,
            direction: "inbound",
            rfc_message_id: "<licence-export-decline@acme.example>",
            subject: "Licence export keeps failing for two users",
            from_address: "d.reeve@acme.example",
            from_name: "Dana Reeve",
            to_addresses: &["support@marula.example"],
            body_text: "Since Friday two of our people cannot export the licence \
                        report — it spins then errors. Both are on the same laptop \
                        build. Restarting did not help. Is this something you can \
                        look at today?",
            ticket_index: Some(2),
            comment_id: None,
            in_reply_to: None,
            scan_status: "clean",
        },
        DemoEmail {
            id_offset: 1031,
            direction: "inbound",
            rfc_message_id: "<printer-jamming-again@acme.example>",
            subject: "Printer jammed again — third time this month",
            from_address: "j.okafor@acme.example",
            from_name: "Jonah Okafor",
            to_addresses: &["support@marula.example"],
            body_text: "The floor printer jammed twice more since the last visit. \
                        Same tray each time, and it now prints one page then stops. \
                        Parts on the way?",
            ticket_index: Some(6),
            comment_id: None,
            in_reply_to: Some("<printer-spooler-timeout@acme.example>"),
            scan_status: "clean",
        },
        // A clean reply inside the P1 thread, so threading by in_reply_to
        // has something to walk.
        DemoEmail {
            id_offset: 1032,
            direction: "inbound",
            rfc_message_id: "<ransomware-qa@acme.example>",
            subject: "Re: Malware alert on the file server",
            from_address: "it-director@acme.example",
            from_name: "Acme IT",
            to_addresses: &["support@marula.example"],
            body_text: "Understood on isolation. We can take the file server down \
                        until 16:00 today if that is what you need — please \
                        confirm the restore window in writing.",
            ticket_index: Some(4),
            comment_id: None,
            in_reply_to: Some("<ransomware-file-server@acme.example>"),
            scan_status: "clean",
        },
        // Outbound: delivered, dead and queued, so the outbox shows every
        // state including a failure worth explaining.
        DemoEmail {
            id_offset: 1033,
            direction: "outbound",
            rfc_message_id: "<dunning-second-follow-up@marula.example>",
            subject: "Second notice — invoice past its due date",
            from_address: "support@marula.example",
            from_name: "Marula Support",
            to_addresses: &["accounts@globex.example"],
            body_text: "Following up on the invoice that passed its due date. The \
                        account is on hold until it is settled — let us know if \
                        the PO number has changed and we will re-send it.",
            ticket_index: None,
            comment_id: None,
            in_reply_to: None,
            scan_status: "clean",
        },
        DemoEmail {
            id_offset: 1034,
            direction: "outbound",
            rfc_message_id: "<quarterly-report-northwind@marula.example>",
            subject: "Your Q3 support report",
            from_address: "support@marula.example",
            from_name: "Marula Support",
            to_addresses: &["ops@northwind.example"],
            body_text: "Attached is your Q3 report: 41 tickets, 3 SLA credits, \
                        response inside target on 38 of them. Anything that looks \
                        wrong, reply here and we will correct it.",
            ticket_index: None,
            comment_id: None,
            in_reply_to: None,
            scan_status: "clean",
        },
        DemoEmail {
            id_offset: 1035,
            direction: "outbound",
            rfc_message_id: "<appointment-confirmed@marula.example>",
            subject: "Confirmed: comms-room visit Thursday",
            from_address: "support@marula.example",
            from_name: "Marula Support",
            to_addresses: &["reception@acme.example"],
            body_text: "Confirmed for Thursday 09:00. We will need access to the \
                        comms room for about two hours and the switch admin \
                        password held in your vault.",
            ticket_index: Some(3),
            comment_id: None,
            in_reply_to: None,
            scan_status: "clean",
        },
    ];
    let mut email_count = 0usize;
    let mut email_outbound_count = 0usize;
    for spec in &email_specs {
        let email_id = demo_uuid(spec.id_offset);
        sqlx::query(
            "INSERT INTO email_messages (id, org_id, direction, rfc_message_id, in_reply_to,
                                         subject, from_address, from_name, to_addresses,
                                         body_text, ticket_id, comment_id, scan_status,
                                         transport)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(email_id)
        .bind(DEMO_ORG_ID)
        .bind(spec.direction)
        .bind(spec.rfc_message_id)
        .bind(spec.in_reply_to)
        .bind(spec.subject)
        .bind(spec.from_address)
        .bind(spec.from_name)
        .bind(spec.to_addresses)
        .bind(spec.body_text)
        .bind(spec.ticket_index.map(|index| demo_uuid(100 + index)))
        .bind(spec.comment_id)
        .bind(spec.scan_status)
        .bind("smtp")
        .execute(&mut *tx)
        .await?;
        trail_system_write(
            &mut tx,
            "email_message",
            email_id,
            serde_json::json!({
                "direction": spec.direction,
                "subject": spec.subject,
                "ticket_index": spec.ticket_index,
            }),
        )
        .await?;

        // The outbound queue only exists for messages going one way, and
        // each state needs a plausible reason: delivered, a dead one whose
        // provider rejected the address, and one still waiting to send.
        if spec.direction == "outbound" {
            let (status, attempts, next_offset, last_error, provider_message_id) =
                match spec.id_offset {
                    // Delivery succeeded.
                    1033 => ("sent", 1, 0, None, Some("msg-7f21c4")),
                    // Provider rejected the recipient address twice, then gave up.
                    1034 => ("dead", 8, 0, Some("550 recipient domain not found"), None),
                    // Still in the queue, next attempt in a few minutes.
                    _ => ("queued", 0, 300, None, None),
                };
            sqlx::query(
                "INSERT INTO email_outbound (id, org_id, email_message_id, status, attempts,
                                             max_attempts, next_attempt_at, last_error,
                                             provider_message_id)
                 VALUES ($1, $2, $3, $4, $5, 8, $6, $7, $8)",
            )
            .bind(demo_uuid(spec.id_offset + 200))
            .bind(DEMO_ORG_ID)
            .bind(email_id)
            .bind(status)
            .bind(attempts)
            .bind(now + Duration::seconds(next_offset))
            .bind(last_error)
            .bind(provider_message_id)
            .execute(&mut *tx)
            .await?;
            email_outbound_count += 1;
        }
        email_count += 1;
    }

    // --- CSAT surveys (portal satisfaction) -------------------------------
    // Two on resolved tickets, one still awaiting a response, so the CSAT
    // page shows the aggregate, the individual responses and the pending
    // request. `token_hash` is opaque (the raw token lives only in the
    // email), so a digest of a demo string is honest about it.
    let csat_specs: [(u64, u64, i16, Option<&str>); 3] = [
        // Responded. Ticket 7 is Acme's licence export, resolved.
        (
            1260,
            7,
            5,
            Some("Spot on — backup verified inside the hour."),
        ),
        // Responded. Ticket 13 is Acme's laptop fan, resolved.
        (
            1261,
            13,
            4,
            Some("Fixed next day. Would have liked a heads-up that the box was running hot."),
        ),
        // Sent, awaiting a response. Ticket 9 is Acme's Wi-Fi P3.
        (1262, 9, 0, None),
    ];
    let mut csat_count = 0usize;
    for (id_offset, ticket_index, score, comment) in csat_specs {
        let responded = comment.is_some();
        let client_id = match ticket_index {
            7 | 13 | 9 => acme,
            _ => acme,
        };
        let contact = match ticket_index {
            7 => "d.reeve@acme.example",
            13 => "j.okafor@acme.example",
            _ => "a.patel@northwind.example",
        };
        sqlx::query(
            "INSERT INTO csat_surveys (id, org_id, ticket_id, client_id, contact_email,
                                       token_hash, token_expires_at, status, skip_reason,
                                       scheduled_for, sent_at, score, comment, responded_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
                     CASE WHEN $11::timestamptz IS NULL THEN NULL ELSE $12 END, $13,
                     CASE WHEN $11::timestamptz IS NULL THEN NULL ELSE $11 END)",
        )
        .bind(demo_uuid(id_offset))
        .bind(DEMO_ORG_ID)
        .bind(demo_uuid(100 + ticket_index))
        .bind(client_id)
        .bind(contact)
        // A 64-char SHA-256 of a demo string: the column is opaque and the
        // real single-use token lives only in the sent email.
        .bind(sha256_hex(
            format!("csat-demo-token-{id_offset}").as_bytes(),
        ))
        .bind(today + Duration::days(6))
        .bind(if responded { "responded" } else { "sent" })
        .bind(None::<&str>)
        .bind(at(today - Duration::days(2), 9, 0))
        // sent_at doubles as the "was actually delivered" signal.
        .bind(if responded {
            Some(at(today - Duration::days(1), 10, 0))
        } else {
            Some(at(today - Duration::days(2), 9, 5))
        })
        .bind((score > 0).then_some(score))
        .bind(comment)
        .execute(&mut *tx)
        .await?;
        csat_count += 1;
    }

    // --- secrets manager ---------------------------------------------------
    // The five immutable default categories, planted through the real write
    // path (which also seeds the ADR-0006 owner/admin grants so the org is
    // never locked out of its own vault), then client-visible records
    // beneath them.
    //
    // `secret_records_check` requires exactly one of org_id / client_id to
    // be non-null, mirroring the folder it hangs under. The default
    // categories are ORG-scope folders (`create_folder` is called with
    // `client_id = None`), so the records that live in them are org-scope
    // too; a client-scoped record needs a forked client folder.
    //
    // Seeding this way means the category tree, the restricted defaults and
    // the grants are all exercised rather than hand-written.
    crate::secret_vault::seed_default_secret_folders(&mut tx, DEMO_ORG_ID, owner).await?;
    let secret_folders = crate::secret_vault::list_all_org_folders(&mut tx, DEMO_ORG_ID).await?;
    let folder_of = |slug: &str| -> Uuid {
        secret_folders
            .iter()
            .find(|f| f.slug == slug && f.client_id.is_none())
            .map(|f| f.id)
            .unwrap_or_else(|| demo_uuid(1))
    };

    struct DemoSecretRecord {
        id_offset: u64,
        folder_slug: &'static str,
        kind: &'static str,
        title: &'static str,
        fields: &'static [(&'static str, &'static str)],
        expires_days: Option<i64>,
    }
    // The records live under org-scope folders, so they are org-scope too.
    // `client` here is only documentation of who the credential belongs to;
    // the row's `client_id` is NULL and the folder is the tree position.
    let secret_specs: [DemoSecretRecord; 4] = [
        DemoSecretRecord {
            id_offset: 1270,
            folder_slug: "passwords",
            kind: "password",
            title: "Acme firewall admin",
            fields: &[
                ("username", "admin"),
                ("password", "9xk-commroom-local"),
                ("url", "https://10.20.0.1"),
            ],
            expires_days: Some(60),
        },
        DemoSecretRecord {
            id_offset: 1271,
            folder_slug: "api-keys",
            kind: "api-key",
            title: "Ticketing connector key",
            fields: &[
                ("api_key", "tk_live_demo_northwind_4f2a"),
                ("scopes", "tickets:read tickets:write"),
            ],
            expires_days: None,
        },
        DemoSecretRecord {
            id_offset: 1272,
            folder_slug: "certificates",
            kind: "certificate",
            title: "Wildcard TLS certificate",
            fields: &[
                ("common_name", "*.globex.example"),
                ("issuer", "Let's Encrypt R10"),
                ("expires", "2027-03-14T23:59:59Z"),
            ],
            expires_days: Some(150),
        },
        DemoSecretRecord {
            id_offset: 1273,
            folder_slug: "recovery-codes",
            kind: "recovery-code",
            title: "Tenant admin recovery codes",
            fields: &[("code_1", "2415-8890"), ("code_2", "7731-9044")],
            expires_days: None,
        },
    ];
    let mut secret_count = 0usize;
    for spec in &secret_specs {
        let record_id = demo_uuid(spec.id_offset);
        sqlx::query(
            "INSERT INTO secret_records (id, org_id, client_id, folder_id, kind, title, expires_at,
                                         created_by)
             VALUES ($1, $2, NULL, $3, $4::text::secret_record_kind, $5, $6, $7)",
        )
        .bind(record_id)
        .bind(DEMO_ORG_ID)
        .bind(folder_of(spec.folder_slug))
        .bind(spec.kind)
        .bind(spec.title)
        .bind(
            spec.expires_days
                .map(|days| at(today + Duration::days(days), 0, 0)),
        )
        .bind(owner)
        .execute(&mut *tx)
        .await?;
        for (field_index, (field_key, value)) in spec.fields.iter().enumerate() {
            sqlx::query(
                "INSERT INTO secret_fields (id, record_id, org_id, client_id, field_key,
                                            value_ciphertext)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(demo_uuid(
                1290u64 + (spec.id_offset - 1270) * 10 + field_index as u64,
            ))
            .bind(record_id)
            .bind(DEMO_ORG_ID)
            .bind(None::<Uuid>)
            .bind(field_key)
            .bind(secret_field_ciphertext(&opts.seal_key, spec.title, value)?)
            .execute(&mut *tx)
            .await?;
        }
        trail_system_write(
            &mut tx,
            "secret_record",
            record_id,
            serde_json::json!({ "title": spec.title, "kind": spec.kind }),
        )
        .await?;
        secret_count += 1;
    }

    // --- contract legs (Wave 39 contract types) ----------------------------
    // Fixed fees, a monthly minimum, an annual escalator, a cap with an
    // overage rate, a service credit that was actually applied, two change
    // orders in different states, and draws against the prepaid block.
    // These are what make the contract detail page a real page rather than
    // a rate card and a note.
    struct DemoFixedFee {
        id_offset: u64,
        contract: Uuid,
        label: &'static str,
        amount_minor: i64,
        due_days: i64,
    }
    let fixed_fee_specs: [DemoFixedFee; 2] = [
        DemoFixedFee {
            id_offset: 1280,
            contract: contract_tm,
            label: "Comms-room survey and report",
            amount_minor: 24_000,
            due_days: 12,
        },
        DemoFixedFee {
            id_offset: 1281,
            contract: contract_incident,
            label: "Quarterly restore rehearsal",
            amount_minor: 18_500,
            due_days: 40,
        },
    ];
    for spec in &fixed_fee_specs {
        sqlx::query(
            "INSERT INTO contract_fixed_fees (id, org_id, contract_id, label, amount_minor,
                                              due_on, notes)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(demo_uuid(spec.id_offset))
        .bind(DEMO_ORG_ID)
        .bind(spec.contract)
        .bind(spec.label)
        .bind(spec.amount_minor)
        .bind(today + Duration::days(spec.due_days))
        .bind(if spec.id_offset == 1280 {
            Some("Billed on completion; mobilisation is included.")
        } else {
            None
        })
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "INSERT INTO contract_minimums (id, org_id, contract_id, minimum_commitment_minor)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(demo_uuid(1282))
    .bind(DEMO_ORG_ID)
    .bind(contract_tm)
    .bind(180_000_i64)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO contract_escalators (id, org_id, contract_id, uplift_bps, first_uplift_on)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(demo_uuid(1283))
    .bind(DEMO_ORG_ID)
    .bind(contract_recurring)
    .bind(400_i32) // 4%
    .bind(today + Duration::days(60))
    .execute(&mut *tx)
    .await?;

    // A cap: warn on approach, then bill overage at the card rate rather
    // than stopping work. That is the behaviour a client wants to see
    // spelled out.
    sqlx::query(
        "INSERT INTO contract_caps (id, org_id, contract_id, scope, cap_minor, reached_behavior,
                                    overage_rate_minor, work_type)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(demo_uuid(1284))
    .bind(DEMO_ORG_ID)
    .bind(contract_tm)
    .bind("period")
    .bind(120_000_i64)
    .bind("overage_billable")
    .bind(11_000_i64)
    .bind("remote")
    .execute(&mut *tx)
    .await?;

    // A credit that has already been applied to the dunning invoice — the
    // money side of a missed SLA.
    sqlx::query(
        "INSERT INTO contract_sla_credits (id, org_id, contract_id, ticket_id, reason,
                                           amount_minor, currency, applies_on, status,
                                           applied_invoice_id, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, 'USD', $7, 'applied', $8, $9)",
    )
    .bind(demo_uuid(1285))
    .bind(DEMO_ORG_ID)
    .bind(contract_recurring)
    // Ticket 11 is the UPS P2 ticket that breached its resolution target.
    .bind(demo_uuid(111))
    .bind("P2 resolution target missed by 11 hours during the power cut")
    .bind(7_500_i64)
    .bind(today - Duration::days(2))
    .bind(invoice_dunning_id)
    .bind(owner)
    .execute(&mut *tx)
    .await?;

    // Two change orders, one approved and one still in draft — the
    // difference between "agreed" and "proposed".
    struct DemoChangeOrder {
        id_offset: u64,
        contract: Uuid,
        number: &'static str,
        title: &'static str,
        amount_minor: i64,
        status: &'static str,
        approved_by: Option<Uuid>,
        approved_days_ago: Option<i64>,
    }
    let change_order_specs: [DemoChangeOrder; 2] = [
        DemoChangeOrder {
            id_offset: 1286,
            contract: contract_recurring,
            number: "CO-2026-001",
            title: "Add two comms-room APs and 25 client licences",
            amount_minor: 42_000,
            status: "approved",
            approved_by: Some(owner),
            approved_days_ago: Some(9),
        },
        DemoChangeOrder {
            id_offset: 1287,
            contract: contract_incident,
            number: "CO-2026-002",
            title: "24/7 out-of-hours escalation for the cannery line",
            amount_minor: 16_800,
            status: "draft",
            approved_by: None,
            approved_days_ago: None,
        },
    ];
    let mut change_order_count = 0usize;
    for spec in &change_order_specs {
        sqlx::query(
            "INSERT INTO contract_change_orders (id, org_id, contract_id, number, title,
                                                 description, amount_minor, status,
                                                 approved_by, approved_at, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(demo_uuid(spec.id_offset))
        .bind(DEMO_ORG_ID)
        .bind(spec.contract)
        .bind(spec.number)
        .bind(spec.title)
        .bind(if spec.id_offset == 1286 {
            Some("Hardware supplied by us; 12-month warranty from install.")
        } else {
            None
        })
        .bind(spec.amount_minor)
        .bind(spec.status)
        .bind(spec.approved_by)
        .bind(
            spec.approved_days_ago
                .map(|days| at(today - Duration::days(days), 14, 0)),
        )
        .bind(owner)
        .execute(&mut *tx)
        .await?;
        change_order_count += 1;
    }

    // Two draws against the prepaid block: one already billed onto the run,
    // one not. This is the "is the block running out" number a client asks
    // for mid-demo.
    for (id_offset, entry_offset, minutes, days, invoice_id) in [
        (1288u64, 160u64, 90_i32, -2_i64, Some(invoice_dunning_id)),
        (1289, 163, 45, -1, None),
    ] {
        sqlx::query(
            "INSERT INTO contract_block_draws (id, org_id, block_id, time_entry_id, invoice_id,
                                               minutes, drawn_on)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(demo_uuid(id_offset))
        .bind(DEMO_ORG_ID)
        .bind(demo_uuid(174))
        .bind(demo_uuid(entry_offset))
        .bind(invoice_id)
        .bind(minutes)
        .bind(today + Duration::days(days))
        .execute(&mut *tx)
        .await?;
    }

    // --- time taxonomy (Wave 39 time categories) --------------------------
    // Every NEW org gets the ten semantic categories from migration 0110,
    // but that migration only seeded the orgs that existed when it ran, so a
    // freshly reset demo org has NOTHING in this table. Seed the same ten
    // here (identical slugs, labels, flags and positions as the migration),
    // idempotent on (org_id, slug) so a rerun cannot diverge.
    let time_categories: [(&str, &str, &str, bool, bool, bool, i32); 10] = [
        (
            "remote",
            "Remote",
            "Work done remotely (phone, email, remote tools)",
            true,
            false,
            true,
            0,
        ),
        (
            "on_site",
            "On-site",
            "Work at the client location",
            true,
            false,
            true,
            10,
        ),
        (
            "depot",
            "Depot",
            "Bench / workshop repair work",
            true,
            false,
            true,
            20,
        ),
        (
            "travel",
            "Travel",
            "Travel time to or from client sites",
            true,
            false,
            false,
            30,
        ),
        (
            "incident",
            "Incident",
            "Time against incident response or emergencies",
            true,
            false,
            true,
            40,
        ),
        (
            "project",
            "Project",
            "Time against project phases rather than tickets",
            true,
            false,
            true,
            50,
        ),
        (
            "internal",
            "Internal",
            "Internal admin, meetings, and own-system work",
            false,
            false,
            false,
            60,
        ),
        (
            "warranty",
            "Warranty",
            "Work covered by warranty — not billable to the client",
            false,
            true,
            false,
            70,
        ),
        (
            "training",
            "Training",
            "Training and enablement time",
            false,
            false,
            false,
            80,
        ),
        (
            "admin",
            "Admin",
            "Administrative time not attributable to client work",
            false,
            false,
            false,
            90,
        ),
    ];
    let mut time_category_count = 0usize;
    for (
        slug,
        label,
        description,
        default_billable,
        requires_unbillable_reason,
        rate_card_affinity,
        position,
    ) in time_categories
    {
        sqlx::query(
            "INSERT INTO time_categories (id, org_id, slug, label, description, default_billable,
                                          requires_unbillable_reason, rate_card_affinity, position,
                                          retired, created_by)
             VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $6, $7, $8, false, $9)
             ON CONFLICT (org_id, slug) DO NOTHING",
        )
        .bind(DEMO_ORG_ID)
        .bind(slug)
        .bind(label)
        .bind(description)
        .bind(default_billable)
        .bind(requires_unbillable_reason)
        .bind(rate_card_affinity)
        .bind(position)
        .bind(owner)
        .execute(&mut *tx)
        .await?;
        time_category_count += 1;
    }

    // The legacy map keeps the older work_type column readable by the new
    // taxonomy, so historical entries still resolve to a category. Idempotent
    // on (org_id, lower(legacy_value)) as the table's unique requires.
    for (legacy_value, category_slug) in [
        ("remote", "remote"),
        ("on_site", "on_site"),
        ("onsite", "on_site"),
        ("depot", "depot"),
        ("travel", "travel"),
    ] {
        sqlx::query(
            "INSERT INTO time_category_work_type_map (id, org_id, legacy_value, category_slug,
                                                      source)
             VALUES (gen_random_uuid(), $1, $2, $3, 'backfill')
             ON CONFLICT (org_id, legacy_value) DO NOTHING",
        )
        .bind(DEMO_ORG_ID)
        .bind(legacy_value)
        .bind(category_slug)
        .execute(&mut *tx)
        .await?;
    }

    // Dispositions on the three lines that went into the T&M invoice, plus
    // one on a non-billable line marked as such — what a reviewer signs off
    // against.
    struct DemoDisposition {
        id_offset: u64,
        time_entry_id: Uuid,
        status: &'static str,
        reason: Option<&'static str>,
        note: &'static str,
        current: bool,
    }
    let disposition_specs: [DemoDisposition; 3] = [
        DemoDisposition {
            id_offset: 1320,
            time_entry_id: demo_uuid(161),
            status: "approved",
            reason: None,
            note: "Matches the cutover log; no change.",
            current: true,
        },
        DemoDisposition {
            id_offset: 1321,
            time_entry_id: demo_uuid(163),
            status: "approved",
            reason: None,
            note: "45 min agreed after the split with ticket 164.",
            current: true,
        },
        DemoDisposition {
            id_offset: 1322,
            time_entry_id: demo_uuid(168),
            status: "non_billable",
            reason: Some("warranty"),
            note: "Covered by the 12-month install warranty.",
            current: true,
        },
    ];
    for spec in &disposition_specs {
        sqlx::query(
            "INSERT INTO time_entry_dispositions (id, org_id, time_entry_id, status,
                                                  unbillable_reason, note, created_by, current)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(demo_uuid(spec.id_offset))
        .bind(DEMO_ORG_ID)
        .bind(spec.time_entry_id)
        .bind(spec.status)
        .bind(spec.reason)
        .bind(spec.note)
        .bind(owner)
        .bind(spec.current)
        .execute(&mut *tx)
        .await?;
    }

    // One adjustment on a billed line: 60 min down to 45, on the run, with
    // the reason a credit note would later need.
    sqlx::query(
        "INSERT INTO time_entry_adjustments (id, org_id, time_entry_id, run_id,
                                             original_minutes, delta_minutes, reason, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(demo_uuid(1323))
    .bind(DEMO_ORG_ID)
    .bind(demo_uuid(163))
    .bind(run_id)
    .bind(60_i32)
    .bind(-15_i32)
    .bind("15 min of the rekey was spent on the unrelated Wi-Fi ticket and moved to ticket 2")
    .bind(owner)
    .execute(&mut *tx)
    .await?;

    // --- purchasing completion: receipts and bills -----------------------
    // The three POs, their lines and the three loans ALREADY exist here
    // (written through the real `insert_purchase_order` / `insert_po_line` /
    // `issue_loan` paths). What was missing was the leg that turns a PO from
    // a wish list into stock: a receipt, the ledger movement it posts, and
    // the vendor's bill.
    let ordered_po_ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM purchase_orders ORDER BY expected_at ASC")
            .fetch_all(&mut *tx)
            .await?;

    let mut receipt_count = 0usize;
    for (index, po_id) in ordered_po_ids.iter().enumerate() {
        let po_lines: Vec<(Uuid, i64)> = sqlx::query_as(
            "SELECT id, qty_ordered_milli FROM purchase_order_lines
              WHERE purchase_order_id = $1 ORDER BY position",
        )
        .bind(po_id)
        .fetch_all(&mut *tx)
        .await?;
        let Some((line_id, qty_milli)) = po_lines.first().copied() else {
            continue;
        };

        let receipt_id = demo_uuid(1330 + index as u64);
        let movement_id = demo_uuid(1350 + index as u64);
        let received_at = now - Duration::days(1);
        let carrier = match index {
            0 => "Metro Freight",
            1 => "Royal Courier Group",
            _ => "Local drop",
        };
        let tracking_ref = match index {
            0 => "MF-4421180",
            1 => "RCG-8870432",
            _ => "",
        };

        crate::purchasing::insert_receipt(
            &mut *tx,
            &core_domain::entities::PurchaseReceipt {
                id: receipt_id,
                org_id: DEMO_ORG_ID,
                purchase_order_id: *po_id,
                location_id: demo_uuid(900),
                carrier: Some(carrier.to_string()),
                tracking_ref: (!tracking_ref.is_empty()).then(|| tracking_ref.to_string()),
                note: Some("First line received in full; second line to follow.".to_string()),
                received_at,
                created_by: Some(tech),
                created_at: now,
            },
        )
        .await?;

        // The ledger leg first: `purchase_receipt_lines.movement_id` has a
        // FK to it, so the movement must exist before the line can point at
        // it. Appended in the same transaction, so stock levels move and the
        // receipt shows a real movement against it.
        sqlx::query(
            "INSERT INTO stock_movements (id, org_id, stock_item_id, kind, to_location_id,
                                          qty_milli, currency, cost_minor, carrier, tracking_ref,
                                          note, created_by)
             SELECT $1, $2, l.stock_item_id, 'receive', $3, l.qty_ordered_milli,
                    'USD', l.unit_cost_minor * l.qty_ordered_milli / 1000, $4, $5, 'Received against PO line', $6
               FROM purchase_order_lines l WHERE l.id = $7",
        )
        .bind(movement_id)
        .bind(DEMO_ORG_ID)
        .bind(demo_uuid(900))
        .bind(carrier)
        .bind(tracking_ref)
        .bind(tech)
        .bind(line_id)
        .execute(&mut *tx)
        .await?;

        crate::purchasing::insert_receipt_line(
            &mut *tx,
            &core_domain::entities::PurchaseReceiptLine {
                id: demo_uuid(1340 + index as u64),
                org_id: DEMO_ORG_ID,
                receipt_id,
                po_line_id: line_id,
                qty_milli,
                movement_id: Some(movement_id),
                created_at: now,
            },
        )
        .await?;

        crate::purchasing::bump_line_received(&mut tx, DEMO_ORG_ID, line_id, qty_milli, false)
            .await?;
        crate::purchasing::advance_order_receipt_status(&mut *tx, DEMO_ORG_ID, *po_id).await?;
        trail_system_write(
            &mut tx,
            "purchase_receipt",
            receipt_id,
            serde_json::json!({ "po": po_id.to_string(), "movement": movement_id.to_string() }),
        )
        .await?;
        receipt_count += 1;
    }

    // One bill per PO, with a full FX snapshot on the GBP one so the bill
    // shows a foreign-currency leg rather than a flat total. The
    // `purchase_bills_fx_snapshot_atomic` CHECK makes an FX row
    // all-or-nothing, so the USD rows stay entirely NULL.
    let mut bill_count = 0usize;
    for (index, po_id) in ordered_po_ids.iter().enumerate() {
        let amount: i64 = sqlx::query_scalar(
            "SELECT coalesce(sum(l.qty_ordered_milli * l.unit_cost_minor / 1000), 0)::bigint
               FROM purchase_order_lines l WHERE l.purchase_order_id = $1",
        )
        .bind(po_id)
        .fetch_one(&mut *tx)
        .await?;
        let is_fx = index == 1;
        sqlx::query(
            "INSERT INTO purchase_bills (id, org_id, purchase_order_id, bill_number, amount_minor,
                                         currency, fx_rate_used, fx_rate_date, fx_source,
                                         base_amount_minor, posted_at, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(demo_uuid(1360 + index as u64))
        .bind(DEMO_ORG_ID)
        .bind(po_id)
        .bind(format!("KER-{index:06}"))
        .bind(amount)
        .bind("GBP")
        .bind(if is_fx { Some(1.26_f64) } else { None })
        .bind(if is_fx {
            Some(today - Duration::days(1))
        } else {
            None
        })
        .bind(if is_fx { Some("ecb-daily") } else { None })
        .bind(if is_fx {
            Some((amount as f64 * 1.26).round() as i64)
        } else {
            None
        })
        .bind(now - Duration::days(1))
        .bind(bookkeeper)
        .execute(&mut *tx)
        .await?;
        bill_count += 1;
    }

    // Vendor contacts: each vendor gets a named primary contact, so the
    // vendor detail page has a person rather than an email alias.
    for (index, name) in [(0usize, "Ana Ferreira"), (1, "Tom Whitlock")] {
        sqlx::query(
            "INSERT INTO vendor_contacts (id, org_id, vendor_id, name, email, phone, role,
                                          is_primary)
             VALUES ($1, $2, $3, $4, $5, $6, $7, true)",
        )
        .bind(demo_uuid(1380 + index as u64))
        .bind(DEMO_ORG_ID)
        .bind(vendor_ids[index])
        .bind(name)
        .bind(if index == 0 {
            Some("a.ferreira@meridiancable.test")
        } else {
            Some("t.whitlock@kestrelinstrument.test")
        })
        .bind(if index == 0 {
            Some("+1 415 555 0142")
        } else {
            Some("+44 20 7946 0998")
        })
        .bind(if index == 0 {
            Some("Account manager")
        } else {
            Some("Sales engineer")
        })
        .execute(&mut *tx)
        .await?;
    }

    // Three client loans and their lines already exist above, written through
    // the real `issue_loan` path. Each line's movement leg was posted too.
    let loan_line_count = usize::try_from(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM client_loan_lines")
            .fetch_one(&mut *tx)
            .await?,
    )
    .unwrap_or(0);

    // --- audit trail: the seed itself, as the system actor -----------------
    crate::audit::insert(
        &mut *tx,
        &core_domain::entities::AuditEvent {
            id: Uuid::now_v7(),
            org_id: DEMO_ORG_ID,
            actor: core_domain::entities::Actor::System,
            action: "dev.seed".into(),
            entity_type: Some("org".into()),
            entity_id: Some(DEMO_ORG_ID.to_string()),
            detail: serde_json::json!({ "surface": "dev_tools", "tickets": 15 }),
            ip: None,
            occurred_at: now,
        },
    )
    .await?;

    tx.commit().await?;

    Ok(DemoSeedReport {
        org_id: DEMO_ORG_ID,
        org_name: DEMO_ORG_NAME,
        slug: DEMO_ORG_SLUG,
        users: users.len(),
        clients: client_specs.len(),
        portal_users: portal_user_count,
        boards: board_specs.len(),
        tickets: ticket_ids.len() + bulk_ticket_count,
        comments: comment_count,
        checklist_items: checklist_count,
        timer_sessions: 3,
        time_entries: time_entry_count,
        contracts: 4,
        invoices: invoice_count,
        kb_articles: kb_articles.len(),
        documents: 1,
        projects: 1,
        appointments: appointment_specs.len(),
        alert_sources: 1,
        alerts: 5,
        alert_incidents: 2,
        report_schedules: 1,
        webhooks: 1,
        assets: asset_specs.len(),
        catalog_items: catalog_specs.len(),
        quotes: quote_specs.len(),
        pipeline_deals: deal_specs.len(),
        credit_notes: 1,
        tags: tag_specs.len(),
        api_keys: 1,
        vault_entries: vault_specs.len(),
        notifications: notif_specs.len(),
        pm_schedules: 2,
        automation_rules: 2,
        plugin_installs: 1,
        vendors: vendor_specs.len(),
        purchase_orders: purchase_order_count,
        client_loans: client_loan_count,
        attachments: attachment_count,
        document_attachments: document_attachment_count,
        attachment_store_bytes,
        email_messages: email_count,
        email_outbound: email_outbound_count,
        csat_surveys: csat_count,
        secret_records: secret_count,
        contract_change_orders: change_order_count,
        time_categories: time_category_count,
        purchase_receipts: receipt_count,
        purchase_bills: bill_count,
        client_loan_lines: loan_line_count,
        logins: vec![
            ("owner@demo.local", "owner"),
            ("dispatcher@demo.local", "dispatcher"),
            ("tech@demo.local", "technician"),
            ("bookkeeper@demo.local", "admin (bookkeeping seat)"),
        ],
        // One per client, in the same order as `clients`. A prospect who
        // wants to see the portal does not need a staff seat to do it.
        portal_logins: portal_user_specs
            .iter()
            .map(|spec| (spec.email, spec.client_name))
            .collect(),
        password: DEMO_PASSWORD,
        alert_hmac_secret: DEMO_ALERT_HMAC_SECRET,
        webhook_signing_secret: DEMO_WEBHOOK_SECRET,
    })
}

/// Appends one `entity_writes` row as the SYSTEM actor (the IF9 seam's
/// storage substrate) with a matching `entity_write_seqs` allocation, so
/// demo data answers provenance questions without pretending a human wrote
/// it. `before` is always NULL: a seed is a create.
async fn trail_system_write(
    tx: &mut Transaction<'_, Postgres>,
    entity: &str,
    entity_id: Uuid,
    after: serde_json::Value,
) -> DbResult<()> {
    let seq: i64 = sqlx::query_scalar(
        "INSERT INTO entity_write_seqs (org_id, entity, entity_id, seq)
         VALUES ($1, $2, $3, 1)
         ON CONFLICT (org_id, entity, entity_id)
           DO UPDATE SET seq = entity_write_seqs.seq + 1, updated_at = now()
         RETURNING seq",
    )
    .bind(DEMO_ORG_ID)
    .bind(entity)
    .bind(entity_id)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO entity_writes (id, org_id, entity, entity_id, seq, op, actor_id,
                                    actor_kind, before, after)
         VALUES ($1, $2, $3, $4, $5, 'dev_seed', $6, 'system', NULL, $7)",
    )
    .bind(Uuid::now_v7())
    .bind(DEMO_ORG_ID)
    .bind(entity)
    .bind(entity_id)
    .bind(seq)
    .bind(SYSTEM_ACTOR_ID)
    .bind(&after)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Demo attachment fixtures — real bytes, not placeholder blobs
//
// `validate_upload` (core_domain::attachments) gates every upload on
// *magic bytes*, so a demo attachment cannot be an arbitrary buffer with a
// plausible extension: the PNG must actually start `\x89PNG`, the PDF with
// `%PDF-`, and the text family must be clean UTF-8 with no C0 controls.
// Anything else is a row the API would never have accepted, and the first
// person to click "download" gets a file their reader refuses to open.
//
// These builders emit GENUINELY valid artifacts — correct CRC/Adler32 chunks,
// correct xref offsets — from fixed inputs, so the sha256, and therefore the
// content-addressed storage key, is byte-stable across reseeds.
// ---------------------------------------------------------------------------

/// CRC-32 (reflected, polynomial 0xEDB8_8320) — the variant PNG chunk
/// checksums use.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Adler-32 — the zlib trailer checksum.
fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

/// A zlib stream built from *stored* (uncompressed) DEFLATE blocks. No
/// compression library exists in the workspace, and the format permits stored
/// blocks, so a real encoder would produce a smaller but equivalent file.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01]; // deflate, 32K window, no dictionary
    if data.is_empty() {
        out.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]); // final empty block
    } else {
        let mut i = 0;
        while i < data.len() {
            let n = (data.len() - i).min(65_535);
            let last = u8::from(i + n >= data.len());
            out.push(last);
            out.extend_from_slice(&(n as u16).to_le_bytes());
            out.extend_from_slice(&(!(n as u16)).to_le_bytes());
            out.extend_from_slice(&data[i..i + n]);
            i += n;
        }
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// One PNG chunk: length, type, data, CRC-32 over type+data.
fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// A real, valid PNG: `width` × `height` of a single flat `rgb` colour.
/// Wide enough that it is a few kilobytes rather than a one-pixel token, so
/// the attachment list shows a believable size badge.
fn demo_png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let row_len = 1 + width as usize * 3;
    // One scanline: the filter byte (type 0, None) then `width` flat pixels.
    let mut row = Vec::with_capacity(row_len);
    row.push(0);
    for _ in 0..width {
        row.extend_from_slice(&rgb);
    }
    // A flat-colour image is that same scanline repeated `height` times.
    let raw = row.repeat(height as usize);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit, colour type 2 (truecolour)

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png_chunk(&mut out, b"IHDR", &ihdr);
    png_chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    png_chunk(&mut out, b"IEND", &[]);
    out
}

/// A real, valid PDF 1.7 with one Helvetica page of `lines`. Every offset in
/// the cross-reference table is read from the live byte position, so a reader
/// parses it without repair.
fn demo_pdf(lines: &[String]) -> Vec<u8> {
    let mut content = String::from("BT /F1 9 Tf 14 TL 56 780 Td\n");
    for line in lines {
        content.push_str(&pdf_string(line));
        content.push_str(" Tj T*\n");
    }
    content.push_str("ET\n");
    let stream = content.into_bytes();

    let mut out = Vec::new();
    let mut offset_of: Vec<usize> = Vec::with_capacity(6);
    let push = |out: &mut Vec<u8>, text: &str| {
        out.extend_from_slice(text.as_bytes());
    };

    push(&mut out, "%PDF-1.7\n%\u{E2}\u{E3}\u{CF}\u{D3}\n");
    for (index, body) in [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources \
         << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding \
         /WinAnsiEncoding >>",
    ]
    .iter()
    .enumerate()
    {
        offset_of.push(out.len());
        push(
            &mut out,
            &format!(
                "{index_plus_one} 0 obj\n{body}\nendobj\n",
                index_plus_one = index + 1
            ),
        );
    }

    offset_of.push(out.len());
    push(
        &mut out,
        &format!("5 0 obj\n<< /Length {} >>\nstream\n", stream.len()),
    );
    out.extend_from_slice(&stream);
    push(&mut out, "endstream\nendobj\n");

    let xref_at = out.len();
    let mut xref = format!("xref\n0 {}\n", offset_of.len() + 1);
    xref.push_str("0000000000 65535 f \n");
    for off in &offset_of {
        xref.push_str(&format!("{off:010} 00000 n \n"));
    }
    push(&mut out, &xref);
    push(
        &mut out,
        &format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
            offset_of.len() + 1
        ),
    );
    out
}

/// Escapes one text line for a PDF literal string, skipping characters
/// WinAnsiEncoding cannot represent so the content stream stays valid.
fn pdf_string(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 2);
    out.push('(');
    for c in line.chars() {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0xFF && !c.is_control() => out.push(c),
            _ => out.push('?'),
        }
    }
    out.push(')');
    out
}

/// AES-256-GCM seal in the exact `crypto::seal` format (12-byte nonce
/// prefixed to the ciphertext) so core-api's `crypto::open` can read these
/// demo secrets back on the ingest/test-fire paths.
fn seal(key: &[u8; 32], plaintext: &[u8]) -> DemoResult<Vec<u8>> {
    use aes_gcm::aead::AeadCore;
    use aes_gcm::aead::{Aead, KeyInit, OsRng};
    use aes_gcm::Aes256Gcm;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| DemoError::Crypto(e.to_string()))?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ct = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| DemoError::Crypto(e.to_string()))?;
    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(nonce.as_slice());
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Argon2id hash of the shared demo password (fresh salt per run is fine —
/// the PHC string is self-describing and verification is per-login).
fn hash_demo_password() -> DemoResult<String> {
    use argon2::password_hash::{rand_core::OsRng, SaltString};
    use argon2::{Argon2, PasswordHasher};
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(DEMO_PASSWORD.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| DemoError::Crypto(e.to_string()))
}

/// (first day of last month, last day of last month) for `today`.
fn last_month_window(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let first_this_month = today.with_day(1).unwrap_or(today);
    let last_month_end = first_this_month - Duration::days(1);
    let last_month_start = last_month_end.with_day(1).unwrap_or(last_month_end);
    (last_month_start, last_month_end)
}

/// The recurring contract's anchor: the first day of the month before last,
/// so last month is a full billing period the demo run covers.
fn anchor_two_months_back(today: NaiveDate) -> NaiveDate {
    let (last_month_start, _) = last_month_window(today);
    (last_month_start - Duration::days(1))
        .with_day(1)
        .unwrap_or(last_month_start)
}

/// A purchase date that puts an asset's warranty horizon on a known side of
/// today, so the demo carries an asset whose warranty is covered, one whose is
/// ending, and one whose has lapsed instead of a null every surface reads as
/// "not set". Months are counted as 30 days: this is demo data, and a few days
/// of calendar drift cannot move an asset across the 90-day callout window by
/// accident.
///
/// The three offsets below are chosen against `WARRANTY_ENDING_WINDOW_DAYS`
/// (90) in `core_domain::entities::client`: `active` sits well outside the
/// callout window, `ending` just inside it, `ended` past the horizon.
fn warranty_purchase_date(today: NaiveDate, life_months: i64, which: Option<&str>) -> NaiveDate {
    let horizon = Duration::days(life_months * 30);
    let owned = match which {
        Some("ended") => horizon + Duration::days(45),
        Some("active") => horizon - Duration::days(400),
        _ => horizon - Duration::days(31),
    };
    today - owned
}

/// Local helper: UTC datetime at `day` + hour:minute.
fn at(day: NaiveDate, hour: u32, minute: u32) -> chrono::DateTime<Utc> {
    chrono::NaiveDateTime::new(
        day,
        NaiveTime::from_hms_opt(hour, minute, 0).unwrap_or_default(),
    )
    .and_utc()
}

/// The next Monday 08:00 UTC strictly in the future (report schedule claim
/// state; due-when-enabled reads this column directly).
fn next_monday_0800(now: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
    let today = now.date_naive();
    let offset_days = (8 - today.weekday().num_days_from_monday()) % 7;
    let days = if offset_days == 0 { 7 } else { offset_days };
    chrono::NaiveDateTime::new(
        today + Duration::days(i64::from(days)),
        NaiveTime::from_hms_opt(8, 0, 0).unwrap_or_default(),
    )
    .and_utc()
}

/// AES-256-GCM seal whose nonce is derived from the plaintext rather than
/// the RNG, so the demo's sealed columns are byte-stable across reseeds.
/// The nonce is still prefixed to the ciphertext, so `crypto::open` reads
/// these normally — only the randomness is gone.
///
/// Use for values that are compared across runs (demo data). The
/// production-shaped `seal` above keeps its random nonce for real secrets.
fn seal_stable(key: &[u8; 32], plaintext: &[u8]) -> DemoResult<Vec<u8>> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::Aes256Gcm;
    use aes_gcm::Nonce;
    use sha2::{Digest as _, Sha256};

    // 12 bytes of digest, matching the real nonce width.
    let mut nonce_bytes = [0u8; 12];
    Sha256::new()
        .chain_update(key)
        .chain_update(plaintext)
        .finalize()
        .into_iter()
        .take(12)
        .enumerate()
        .for_each(|(i, b)| nonce_bytes[i] = b);

    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| DemoError::Crypto(e.to_string()))?;
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
        .map_err(|e| DemoError::Crypto(e.to_string()))?;
    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Sealed bytes for one demo secret-field value. The plaintext the app opens
/// back is the value with its record title appended, so a reveal shows
/// something recognisable rather than a bare random string.
fn secret_field_ciphertext(key: &[u8; 32], record_title: &str, value: &str) -> DemoResult<Vec<u8>> {
    let plaintext = format!("{value} [{record_title}]");
    seal_stable(key, plaintext.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every demo fixture must survive the upload gate — magic bytes,
    /// extension and declared content type all consistent. If one of them
    /// drifts (a renamed PNG, a wrong extension), the seed fails here
    /// instead of shipping an attachment the API would have rejected.
    #[test]
    fn demo_fixtures_pass_the_upload_gate() {
        let cases: [(&str, &str, Vec<u8>); 3] = [
            (
                "ap-monitoring.png",
                "image/png",
                demo_png(160, 55, [30, 64, 96]),
            ),
            (
                "restore-plan.pdf",
                "application/pdf",
                demo_pdf(&["Acme — restore plan".into(), "".into(), "line three".into()]),
            ),
            (
                "signal-strength.csv",
                "text/csv",
                "ap,channel\nAP-01,6\n".as_bytes().to_vec(),
            ),
        ];
        for (filename, content_type, bytes) in cases {
            let (name, ct) = core_domain::attachments::validate_upload(
                filename,
                content_type,
                &bytes[..bytes.len().min(core_domain::attachments::MAGIC_HEAD_LEN)],
                bytes.len() as i64,
                core_domain::attachments::DEFAULT_MAX_UPLOAD_BYTES,
            )
            .expect("fixture must pass the upload gate");
            assert_eq!(name, filename);
            assert_eq!(ct, content_type);
        }
    }

    /// The fixtures are a few kilobytes each, so the attachment list shows a
    /// believable size badge rather than a 1-byte token.
    #[test]
    fn demo_fixtures_are_demo_plausible_sizes() {
        let png = demo_png(160, 55, [30, 64, 96]);
        assert!(png.len() > 1_500, "png was {} bytes", png.len());
        let pdf = demo_pdf(&["a".repeat(0), "b".repeat(20)]);
        assert!(pdf.len() > 400, "pdf was {} bytes", pdf.len());
    }

    /// PNG chunk CRCs are correct, so a real decoder renders the image.
    #[test]
    fn demo_png_has_valid_chunk_crcs() {
        let png = demo_png(4, 2, [1, 2, 3]);
        assert!(
            png.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
            "PNG signature missing"
        );
        // IHDR (13 bytes of data) must come first, IEND (none) closes it.
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
        let mut offset = 8;
        while offset + 8 <= png.len() {
            let len = u32::from_be_bytes(png[offset..offset + 4].try_into().unwrap()) as usize;
            let kind = &png[offset + 4..offset + 8];
            let data_end = offset + 8 + len;
            let stored_crc = u32::from_be_bytes(png[data_end..data_end + 4].try_into().unwrap());
            let mut crc_input = kind.to_vec();
            crc_input.extend_from_slice(&png[offset + 8..data_end]);
            assert_eq!(stored_crc, crc32(&crc_input), "bad CRC on {kind:?}");
            offset = data_end + 4;
        }
        assert_eq!(offset, png.len(), "trailing bytes after the last chunk");
    }

    /// The xref table points at the real object offsets, so a reader opens
    /// the file without repair.
    #[test]
    fn demo_pdf_xref_offsets_are_correct() {
        let pdf = demo_pdf(&["one".into(), "two".into()]);
        let text = String::from_utf8_lossy(&pdf);
        let startxref = text
            .find("startxref")
            .and_then(|i| {
                // `startxref\n<digits>\n%%EOF` — skip the leading newline,
                // then take everything up to the next one.
                let after = &text[i + "startxref".len()..];
                let digits = after.strip_prefix('\n')?;
                let line = digits.get(..digits.find('\n').unwrap_or(digits.len()))?;
                line.trim().parse::<usize>().ok()
            })
            .expect("a startxref offset");
        assert_eq!(&pdf[startxref..startxref + 4], b"xref");
        assert!(pdf.ends_with(b"%%EOF\n"));
    }
}
