//! Reproducible perf dataset generator (deliverable 2a).
//!
//! Seeds one org at a requested scale with deterministic ids:
//! `md5('<tag>')::uuid` computed IN the database, so the SQL generator can
//! reference parent ids (ticket → client/status/assignee) without any
//! lookups, and re-running the seed at the same scale reproduces the exact
//! same ids and distribution.
//!
//! All inserts are set-based (`generate_series` joins) — no row-by-row
//! round-trips; a 100k-ticket org seeds in seconds.
//!
//! Scope note: the dataset covers exactly what the ticket-list budget path
//! reads — `tickets` (with realistic priority/category/due_at/created_at
//! distributions), the batched custom-field values enrichment
//! (`list_values_many`), and the auth rows the HTTP load runner needs
//! (users, builtin roles). Comments/checklists/tags/search are NOT in the
//! list read path and are deliberately omitted; the seed is honest about
//! that in `SeedReport`.

use anyhow::Context;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{SeedScale, PERF_ORG_ID, PERF_PASSWORD, PERF_USER_EMAIL};

/// What the seeder created (for the report + smoke assertions).
#[derive(Debug, Clone, serde::Serialize)]
pub struct SeedReport {
    pub org_id: Uuid,
    pub tickets: i64,
    pub clients: i64,
    pub users: i64,
    pub custom_field_values: i64,
    pub seed_ms: u128,
}

/// Deletes the perf org (cascades the dataset) and re-seeds it at `scale`.
///
/// Runs on a PRIVILEGED pool (the migrations owner): the perf data is
/// fixture data written outside RLS, then read back through the real
/// `marula_app` + org-GUC path during measurement — same split the
/// core-db test harness uses.
pub async fn seed(admin: &PgPool, scale: &SeedScale) -> anyhow::Result<SeedReport> {
    let started = std::time::Instant::now();
    let mut tx = admin.begin().await.context("begin seed tx")?;

    // Prior seed (any shape) is wiped by the org-level cascade. Users'
    // auth_sessions cascade with users; nothing dangles.
    sqlx::query("DELETE FROM orgs WHERE id = $1")
        .bind(PERF_ORG_ID)
        .execute(&mut *tx)
        .await?;

    sqlx::query("INSERT INTO orgs (id, name, slug, ticket_prefix) VALUES ($1, $2, $3, $4)")
        .bind(PERF_ORG_ID)
        .bind("Perf Harness")
        .bind("perf-harness")
        .bind("PERF")
        .execute(&mut *tx)
        .await?;

    // One shared argon2 hash (fixture-only; login verifies it like any user).
    let password_hash = {
        use argon2::PasswordHasher;
        argon2::Argon2::default()
            .hash_password(PERF_PASSWORD.as_bytes())
            .map_err(|e| anyhow::anyhow!("hash perf password: {e}"))?
            .to_string()
    };

    sqlx::query(
        r#"INSERT INTO users (id, org_id, email, display_name, password_hash)
           SELECT md5('perf-user-' || g.i)::uuid, $1,
                  'perf-user-' || g.i || '@perf.local',
                  'Perf User ' || g.i, $2
           FROM generate_series(1, $3) AS g(i)"#,
    )
    .bind(PERF_ORG_ID)
    .bind(&password_hash)
    .bind(scale.users)
    .execute(&mut *tx)
    .await?;

    // Builtin roles (ids are time-based; they never enter measured queries),
    // then the primary harness user is an Admin so the load runner's bearer
    // token satisfies tickets:view.
    let role_ids = core_db::orgs::seed_default_roles(&mut tx, PERF_ORG_ID)
        .await
        .context("seed perf roles")?;
    let admin_role = role_ids
        .get("admin")
        .copied()
        .context("builtin admin role missing")?;
    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)")
        .bind(md5_uuid("perf-user-1"))
        .bind(admin_role)
        .execute(&mut *tx)
        .await?;

    // Board + the four semantic statuses with deterministic ids.
    sqlx::query(
        r#"INSERT INTO boards (id, org_id, name, description, is_default)
           VALUES ($1, $2, 'Perf Service Desk', 'perf harness board', true)"#,
    )
    .bind(md5_uuid("perf-board"))
    .bind(PERF_ORG_ID)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO board_statuses (id, board_id, org_id, name, category, sort_order, is_entry)
           SELECT md5('perf-status-' || g.cat)::uuid, $1, $2, g.cat,
                  g.cat, g.ord, g.ord = 1
           FROM unnest(ARRAY['open','in_progress','waiting','closed']) WITH ORDINALITY
                AS g(cat, ord)"#,
    )
    .bind(md5_uuid("perf-board"))
    .bind(PERF_ORG_ID)
    .execute(&mut *tx)
    .await?;

    // Clients → sites → contacts, one chain each, deterministic ids.
    sqlx::query(
        r#"INSERT INTO clients (id, org_id, name)
           SELECT md5('perf-client-' || g.i)::uuid, $1, 'Perf Client ' || g.i
           FROM generate_series(1, $2) AS g(i)"#,
    )
    .bind(PERF_ORG_ID)
    .bind(scale.clients)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO sites (id, org_id, client_id, name)
           SELECT md5('perf-site-' || g.i)::uuid, $1,
                  md5('perf-client-' || g.i)::uuid, 'Perf Site ' || g.i
           FROM generate_series(1, $2) AS g(i)"#,
    )
    .bind(PERF_ORG_ID)
    .bind(scale.clients)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO contacts (id, org_id, client_id, site_id, name, is_primary)
           SELECT md5('perf-contact-' || g.i)::uuid, $1,
                  md5('perf-client-' || g.i)::uuid,
                  md5('perf-site-' || g.i)::uuid,
                  'Perf Contact ' || g.i, true
           FROM generate_series(1, $2) AS g(i)"#,
    )
    .bind(PERF_ORG_ID)
    .bind(scale.clients)
    .execute(&mut *tx)
    .await?;

    // Tickets. Distributions mirror a working MSP queue:
    // - priority: 10% p1, 20% p2, 60% p3, 10% p4 (i % 10)
    // - category: ~40% closed, 35% open, 20% in_progress, 5% waiting (i % 20)
    // - assignee: NULL 30% of the time, else round-robin over users
    // - created_at spread over ~2 years so page 1 sorts over the full range
    // - due_at: priority-shaped horizons, NULL for p3/p4 older tickets
    // The ORDER BY the list endpoint uses is
    //   (priority CASE, due_at NULLS LAST, created_at) — the seed keeps all
    // three columns varied so the sort does real work.
    sqlx::query(
        r#"INSERT INTO tickets (id, org_id, number, board_id, status_id, title, body,
                                priority, client_id, site_id, contact_id, assignee_id,
                                category, due_at, created_at, updated_at)
           SELECT
             md5('perf-ticket-' || g.i)::uuid,
             $1,
             'P' || g.i,
             md5('perf-board')::uuid,
             md5('perf-status-' || (CASE WHEN g.i % 20 < 8 THEN 'closed'
                                         WHEN g.i % 20 < 15 THEN 'open'
                                         WHEN g.i % 20 < 19 THEN 'in_progress'
                                         ELSE 'waiting' END))::uuid,
             'Perf ticket ' || g.i || ' ' || md5('t' || g.i::text),
             'Seeded load body for ticket ' || g.i,
             CASE WHEN g.i % 10 = 0 THEN 'p1'
                  WHEN g.i % 10 < 3 THEN 'p2'
                  WHEN g.i % 10 < 9 THEN 'p3'
                  ELSE 'p4' END,
             md5('perf-client-' || (1 + g.i % $3)::text)::uuid,
             md5('perf-site-' || (1 + g.i % $3)::text)::uuid,
             md5('perf-contact-' || (1 + g.i % $3)::text)::uuid,
             CASE WHEN g.i % 10 < 3 THEN NULL
                  ELSE md5('perf-user-' || (1 + (g.i / 10) % $4)::text)::uuid END,
             CASE WHEN g.i % 20 < 8 THEN 'closed'
                  WHEN g.i % 20 < 15 THEN 'open'
                  WHEN g.i % 20 < 19 THEN 'in_progress'
                  ELSE 'waiting' END,
             CASE WHEN g.i % 10 = 0 THEN now() + interval '4 hours'
                  WHEN g.i % 10 < 3 THEN now() + interval '1 day'
                  WHEN g.i % 4 = 0 THEN now() + interval '7 days'
                  ELSE NULL END,
             now() - ((g.i % 730) || ' days')::interval,
             now() - ((g.i % 730) || ' days')::interval
           FROM generate_series(1, $2) AS g(i)"#,
    )
    .bind(PERF_ORG_ID) // $1
    .bind(scale.tickets) // $2
    .bind(scale.clients) // $3
    .bind(scale.users) // $4
    .execute(&mut *tx)
    .await?;

    // Search corpus (Wave 52 lane P5): one indexed row per ticket, the shape
    // the indexer writes. "network" is in every title, so a deep page is a
    // real deep page; `repeat` varies the body so scores are not all equal,
    // and the rows that do tie exercise the cursor's `(indexed_at,
    // entity_id)` tail.
    sqlx::query(
        r#"INSERT INTO search_index (id, org_id, entity_kind, entity_id, title, body, indexed_at)
           SELECT md5('perf-search-' || g.i)::uuid,
                  $1,
                  'ticket',
                  md5('perf-ticket-' || g.i)::uuid,
                  'Perf ticket ' || g.i || ' network ' || md5('t' || g.i::text),
                  repeat('context ', (1 + (g.i % 5))::int) ||
                    'the network printer on floor ' || (g.i % 9) || ' keeps dropping',
                  now() - (g.i || ' seconds')::interval
             FROM generate_series(1, $2) AS g(i)"#,
    )
    .bind(PERF_ORG_ID)
    .bind(scale.tickets)
    .execute(&mut *tx)
    .await?;

    // Ticket custom fields: three definitions, values on ~50% of tickets —
    // exactly the batched-enrichment shape the list endpoint enriches with.
    for (i, (name, kind, config)) in [
        (
            "Environment",
            "select",
            r#"{"options":["prod","staging","dev"]}"#,
        ),
        ("Impact", "number", "{}"),
        ("Escalation note", "text", "{}"),
    ]
    .into_iter()
    .enumerate()
    {
        let def_id = md5_uuid(&format!("perf-cf-def-{i}"));
        sqlx::query(
            r#"INSERT INTO custom_field_defs (id, org_id, entity, name, kind, config, sort_order)
               VALUES ($1, $2, 'ticket', $3, $4, $5::jsonb, $6)"#,
        )
        .bind(def_id)
        .bind(PERF_ORG_ID)
        .bind(name)
        .bind(kind)
        .bind(config)
        .bind(i as i32)
        .execute(&mut *tx)
        .await?;
        // One value per (even-index) ticket per definition. PK is
        // (field_id, entity_id) so this never collides across defs.
        sqlx::query(
            r#"INSERT INTO custom_field_values (field_id, entity_id, org_id, value)
               SELECT $1, md5('perf-ticket-' || g.i)::uuid, $2,
                      CASE $4
                        WHEN 0 THEN to_jsonb(CASE WHEN g.i % 3 = 0 THEN 'prod'
                                                  WHEN g.i % 3 = 1 THEN 'staging'
                                                  ELSE 'dev' END)
                        WHEN 1 THEN to_jsonb((g.i % 100)::int)
                        ELSE to_jsonb('note ' || g.i)
                      END
               FROM generate_series(1, $3) AS g(i)
               WHERE g.i % 2 = 0"#,
        )
        .bind(def_id)
        .bind(PERF_ORG_ID)
        .bind(scale.tickets)
        .bind(i as i32)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await.context("commit seed")?;

    let report = SeedReport {
        org_id: PERF_ORG_ID,
        tickets: scale.tickets,
        clients: scale.clients,
        users: scale.users,
        // ceil(tickets/2) values per definition × 3 definitions.
        custom_field_values: 3 * ((scale.tickets + 1) / 2),
        seed_ms: started.elapsed().as_millis(),
    };
    Ok(report)
}

/// The md5-derived uuid convention, mirrored in Rust for the ids the
/// harness needs client-side (the login user). Must stay in lockstep with
/// the SQL `md5('<tag>')::uuid` expressions above.
#[must_use]
pub fn md5_uuid(tag: &str) -> Uuid {
    use md5::{Digest, Md5};
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&Md5::digest(tag.as_bytes()));
    Uuid::from_bytes(bytes)
}

/// Convenience: the id of `PERF_USER_EMAIL` under the md5 convention.
#[must_use]
pub fn perf_user_id() -> Uuid {
    md5_uuid("perf-user-1")
}

/// Verifies the seeded org's headline counts (cheap smoke assertions after
/// seeding on an existing DB).
pub async fn verify_counts(admin: &PgPool, scale: &SeedScale) -> anyhow::Result<()> {
    let tickets: i64 = sqlx::query_scalar("SELECT count(*) FROM tickets WHERE org_id = $1")
        .bind(PERF_ORG_ID)
        .fetch_one(admin)
        .await?;
    anyhow::ensure!(
        tickets == scale.tickets,
        "perf org ticket count {tickets} != expected {}",
        scale.tickets
    );
    let indexed: i64 = sqlx::query_scalar("SELECT count(*) FROM search_index WHERE org_id = $1")
        .bind(PERF_ORG_ID)
        .fetch_one(admin)
        .await?;
    anyhow::ensure!(
        indexed == scale.tickets,
        "perf org search_index count {indexed} != expected {}",
        scale.tickets
    );
    Ok(())
}

/// The email of harness user index 1, exposed for the load runner CLI.
#[must_use]
pub fn perf_user_email() -> &'static str {
    PERF_USER_EMAIL
}
