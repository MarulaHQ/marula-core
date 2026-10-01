//! Client-portal v1 persistence: portal users, board×client exposure,
//! portal sessions, and the client-scoped ticket reads the portal API
//! serves.
//!
//! Purpose: every portal query scopes by `client_id` IN ADDITION to the
//! org RLS GUC — a correctly-org-scoped transaction still cannot see
//! another client's tickets through these functions. Comment reads are
//! NOT defined here: they reuse `tickets::list_comments` with the kind
//! whitelist from `core_domain::notes::visible_kinds`, so visibility can
//! never drift between staff and portal surfaces.
//!
//! Non-purpose: no password hashing/verification (core-api owns argon2),
//! no JWT handling (core-api), no authorization-tier decisions
//! (core-domain policy).

use crate::tickets::row_to_ticket;
use crate::DbResult;
use core_domain::entities::{BoardPortalExposure, PortalRole, PortalUser, PortalUserStatus};
use core_domain::status::TicketStatusCategory;
use sqlx::Postgres;
use uuid::Uuid;

// --- portal users -------------------------------------------------------------

pub struct NewPortalUser<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub client_id: Uuid,
    pub email: &'a str,
    pub display_name: &'a str,
    /// Argon2id hash — computed by the caller (core-api), never a plaintext.
    pub password_hash: &'a str,
    pub role: PortalRole,
}

const PORTAL_USER_COLS: &str =
    "id, org_id, client_id, email, display_name, role, status, created_at, updated_at";

/// Provisions a portal identity for one client. Email is unique per org.
pub async fn create_portal_user<'e, E>(executor: E, u: NewPortalUser<'_>) -> DbResult<PortalUser>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        r#"INSERT INTO portal_users (id, org_id, client_id, email, display_name, password_hash, role, status)
           VALUES ($1, $2, $3, $4, $5, $6, $7, 'invited')
           RETURNING {PORTAL_USER_COLS}"#
    ))
    .bind(u.id)
    .bind(u.org_id)
    .bind(u.client_id)
    .bind(u.email)
    .bind(u.display_name)
    .bind(u.password_hash)
    .bind(u.role.as_str())
    .fetch_one(executor)
    .await?;
    Ok(portal_user_from_row(row))
}

pub async fn get_portal_user<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<PortalUser>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        "SELECT {PORTAL_USER_COLS} FROM portal_users WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or(crate::DbError::NotFound)?;
    Ok(portal_user_from_row(row))
}

pub async fn list_portal_users<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Option<Uuid>,
) -> DbResult<Vec<PortalUser>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(&format!(
        r#"SELECT {PORTAL_USER_COLS} FROM portal_users
           WHERE org_id = $1 AND ($2::uuid IS NULL OR client_id = $2)
           ORDER BY created_at"#
    ))
    .bind(org_id)
    .bind(client_id)
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(portal_user_from_row).collect())
}

/// Admin-side mutation surface: rename, re-tier, enable/disable, reset
/// password. `None` fields are left unchanged.
pub struct PortalUserPatch {
    pub display_name: Option<String>,
    pub role: Option<PortalRole>,
    pub status: Option<PortalUserStatus>,
    /// Pre-hashed replacement credential (argon2id), or None.
    pub password_hash: Option<String>,
}

pub async fn update_portal_user<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    patch: PortalUserPatch,
) -> DbResult<PortalUser>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        r#"UPDATE portal_users SET
             display_name = COALESCE($3, display_name),
             role         = COALESCE($4, role),
             status       = COALESCE($5, status),
             password_hash = COALESCE($6, password_hash),
             updated_at   = now()
           WHERE org_id = $1 AND id = $2
           RETURNING {PORTAL_USER_COLS}"#
    ))
    .bind(org_id)
    .bind(id)
    .bind(patch.display_name)
    .bind(patch.role.map(|r| r.as_str().to_string()))
    .bind(patch.status.map(|s| s.as_str().to_string()))
    .bind(patch.password_hash)
    .fetch_optional(executor)
    .await?
    .ok_or(crate::DbError::NotFound)?;
    Ok(portal_user_from_row(row))
}

/// Hard delete; sessions cascade. Admins usually prefer `disabled` status
/// (history preserved) — this exists for GDPR-style removal requests.
pub async fn delete_portal_user<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        "DELETE FROM portal_users WHERE org_id = $1 AND id = $2",
        org_id,
        id
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(crate::DbError::NotFound);
    }
    Ok(())
}

/// Login-time credential read under an already-bound org scope. Returns
/// the hash + lifecycle state; verification happens in core-api.
pub struct PortalCredentials {
    pub id: Uuid,
    pub org_id: Uuid,
    pub client_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub password_hash: String,
    pub role: PortalRole,
    pub status: PortalUserStatus,
}

pub async fn find_portal_credentials_by_email<'e, E>(
    executor: E,
    org_id: Uuid,
    email: &str,
) -> DbResult<Option<PortalCredentials>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, client_id, email, display_name, password_hash, role, status
           FROM portal_users WHERE org_id = $1 AND lower(email) = lower($2)"#,
        org_id,
        email
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|r| PortalCredentials {
        id: r.id,
        org_id: r.org_id,
        client_id: r.client_id,
        email: r.email,
        display_name: r.display_name,
        password_hash: r.password_hash,
        role: r.role.parse().unwrap_or(PortalRole::Contact),
        status: r.status.parse().unwrap_or(PortalUserStatus::Invited),
    }))
}

/// First successful login flips `invited` → `active` (idempotent for
/// users who are already active). Disabled users never reach login.
pub async fn activate_on_first_login<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"UPDATE portal_users SET status = 'active', updated_at = now()
           WHERE org_id = $1 AND id = $2 AND status = 'invited'"#,
        org_id,
        id
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Maps a raw portal_users row into the domain entity.
fn portal_user_from_row(row: sqlx::postgres::PgRow) -> PortalUser {
    use sqlx::Row;
    PortalUser {
        id: row.get("id"),
        org_id: row.get("org_id"),
        client_id: row.get("client_id"),
        email: row.get("email"),
        display_name: row.get("display_name"),
        role: row
            .get::<String, _>("role")
            .parse()
            .unwrap_or(PortalRole::Contact),
        status: row
            .get::<String, _>("status")
            .parse()
            .unwrap_or(PortalUserStatus::Invited),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

// --- board×client exposure -----------------------------------------------------

pub struct NewExposure {
    pub org_id: Uuid,
    pub board_id: Uuid,
    pub client_id: Uuid,
    pub exposed: bool,
    /// Empty = ALL statuses visible (v1 default). Non-empty narrows to
    /// exactly those status ids of that board.
    pub visible_status_ids: Vec<Uuid>,
}

/// Upsert keyed on (board, client): toggling exposure never creates
/// duplicate rows.
pub async fn upsert_exposure<'e, E>(executor: E, x: NewExposure) -> DbResult<BoardPortalExposure>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO board_portal_exposure (id, org_id, board_id, client_id, exposed, visible_status_ids)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (board_id, client_id) DO UPDATE SET
             exposed = EXCLUDED.exposed,
             visible_status_ids = EXCLUDED.visible_status_ids,
             updated_at = now()
           RETURNING id, org_id, board_id, client_id, exposed, visible_status_ids, created_at, updated_at"#,
        Uuid::now_v7(),
        x.org_id,
        x.board_id,
        x.client_id,
        x.exposed,
        &x.visible_status_ids,
    )
    .fetch_one(executor)
    .await?;
    Ok(BoardPortalExposure {
        id: row.id,
        org_id: row.org_id,
        board_id: row.board_id,
        client_id: row.client_id,
        exposed: row.exposed,
        visible_status_ids: row.visible_status_ids,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

pub async fn list_exposures<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Option<Uuid>,
) -> DbResult<Vec<BoardPortalExposure>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, board_id, client_id, exposed, visible_status_ids, created_at, updated_at
           FROM board_portal_exposure
           WHERE org_id = $1 AND ($2::uuid IS NULL OR client_id = $2)
           ORDER BY created_at"#,
        org_id,
        client_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| BoardPortalExposure {
            id: row.id,
            org_id: row.org_id,
            board_id: row.board_id,
            client_id: row.client_id,
            exposed: row.exposed,
            visible_status_ids: row.visible_status_ids,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
        .collect())
}

// --- client-scoped ticket reads -------------------------------------------------
//
// The JOIN is the substrate: a ticket surfaces only when its own
// client_id matches AND its board has an `exposed` row for THAT client,
// honoring any per-board status narrowing. Org RLS applies underneath.

pub struct PortalTicketFilter {
    pub board_id: Option<Uuid>,
    pub category: Option<TicketStatusCategory>,
    pub limit: i64,
    pub offset: i64,
}

const PORTAL_TICKET_FROM: &str = r#"FROM tickets t
       JOIN board_portal_exposure e
         ON e.org_id = t.org_id
        AND e.board_id = t.board_id
        AND e.client_id = $2
        AND e.exposed
        AND (cardinality(e.visible_status_ids) = 0 OR t.status_id = ANY(e.visible_status_ids))
      "#;

async fn portal_tickets_query<'e, E>(
    executor: E,
    where_extra: &str,
    bind_ticket_id: Option<Uuid>,
    org_id: Uuid,
    client_id: Uuid,
    f: PortalTicketFilter,
) -> DbResult<Vec<core_domain::entities::Ticket>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    // Column order mirrors tickets::TICKET_COLS so row_to_ticket decodes it.
    let sql = format!(
        r#"SELECT t.id, t.org_id, t.number, t.board_id, t.status_id, t.title, t.body, t.priority,
                  t.client_id, t.site_id, t.contact_id, t.assignee_id, t.requester_id, t.category,
                  t.due_at, t.resolved_at, t.created_at, t.updated_at
           {PORTAL_TICKET_FROM}
           WHERE t.org_id = $1 AND t.client_id = $2
             AND ($3::uuid IS NULL OR t.id = $3)
             AND ($4::uuid IS NULL OR t.board_id = $4)
             AND ($5::text IS NULL OR t.category = $5)
           {where_extra}
           ORDER BY t.updated_at DESC
           LIMIT $6 OFFSET $7"#,
        PORTAL_TICKET_FROM = PORTAL_TICKET_FROM,
        where_extra = where_extra,
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(client_id)
        .bind(bind_ticket_id)
        .bind(f.board_id)
        .bind(f.category.map(|c| c.as_str().to_string()))
        .bind(f.limit)
        .bind(f.offset)
        .fetch_all(executor)
        .await?;
    Ok(rows
        .into_iter()
        .map(row_to_ticket)
        .collect::<Result<_, _>>()?)
}

/// Tickets of ONE client across its EXPOSED boards, newest activity first.
pub async fn list_client_tickets<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
    f: PortalTicketFilter,
) -> DbResult<Vec<core_domain::entities::Ticket>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    portal_tickets_query(executor, "", None, org_id, client_id, f).await
}

/// One ticket under the same exposure+client contract; `DbError::NotFound`
/// when the ticket belongs to a different client or sits on an unexposed
/// board — indistinguishable from missing by design.
pub async fn get_client_ticket<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<core_domain::entities::Ticket>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let mut rows = portal_tickets_query(
        executor,
        "",
        Some(ticket_id),
        org_id,
        client_id,
        PortalTicketFilter {
            board_id: None,
            category: None,
            limit: 1,
            offset: 0,
        },
    )
    .await?;
    rows.pop().ok_or(crate::DbError::NotFound)
}

// --- portal sessions --------------------------------------------------------------

pub async fn create_session<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    portal_user_id: Uuid,
    refresh_hash: &str,
    user_agent: Option<&str>,
    expires_at: chrono::DateTime<chrono::Utc>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(r#"SELECT * FROM app_create_portal_session($1, $2, $3, $4, $5, $6)"#)
        .bind(id)
        .bind(org_id)
        .bind(portal_user_id)
        .bind(refresh_hash)
        .bind(user_agent)
        .bind(expires_at)
        .execute(executor)
        .await?;
    Ok(())
}

/// Revokes one session (used during refresh rotation: presenting a valid
/// refresh token retires that session row permanently).
pub async fn revoke_session<'e, E>(executor: E, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        "UPDATE portal_sessions SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL",
        id
    )
    .execute(executor)
    .await?;
    Ok(())
}

// --- portal magic-link tokens (migration 0119) --------------------------------
//
// The portal's own one-use token table. It is SEPARATE from the staff
// `auth_one_use_tokens` because that table's `user_id` is a hard foreign key to
// `users(id)` and a portal identity lives in `portal_users`; see the migration
// header for why that FK is not loosened instead.
//
// The claim goes through a SECURITY DEFINER bridge rather than a plain
// UPDATE-then-SELECT, because the claim must be race-proof (the UPDATE is the
// claim) and because the caller has no org scope set on a pre-auth request.

/// What a claimed portal link resolves to: the identity, its org, and the
/// client whose data the session may then read.
#[derive(Debug, Clone, Copy)]
pub struct PortalTokenResolution {
    pub portal_user_id: Uuid,
    pub org_id: Uuid,
    pub client_id: Uuid,
}

/// A new one-use token row. `token_hash` is sha256 hex — the raw token never
/// persists, matching the refresh-token and staff-link posture.
pub struct PutPortalToken<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub portal_user_id: Uuid,
    pub kind: &'a str,
    pub token_hash: &'a str,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

pub async fn put_portal_one_use_token<'e, E>(executor: E, t: PutPortalToken<'_>) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    // Plain INSERT: the caller sets the org GUC first, so RLS admits the write
    // without the row bypassing the policy.
    sqlx::query(
        r#"INSERT INTO portal_auth_one_use_tokens
             (id, org_id, portal_user_id, kind, token_hash, expires_at)
           VALUES ($1, $2, $3, $4, $5, $6)"#,
    )
    .bind(t.id)
    .bind(t.org_id)
    .bind(t.portal_user_id)
    .bind(t.kind)
    .bind(t.token_hash)
    .bind(t.expires_at)
    .execute(executor)
    .await?;
    Ok(())
}

/// Spends a token. Returns `None` for an unknown, already-used, expired, or
/// disabled-identity token — the four cases are deliberately indistinguishable
/// to the caller, so a probed token learns nothing.
pub async fn consume_portal_one_use_token(
    pool: &sqlx::PgPool,
    kind: &str,
    token_hash: &str,
) -> DbResult<Option<PortalTokenResolution>> {
    let row: Option<(Uuid, Uuid, Uuid)> = sqlx::query_as(
        "SELECT portal_user_id, org_id, client_id FROM app_consume_portal_one_use_token($1, $2)",
    )
    .bind(kind)
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(
        |(portal_user_id, org_id, client_id)| PortalTokenResolution {
            portal_user_id,
            org_id,
            client_id,
        },
    ))
}

/// Retires every live token of one kind for one identity. Asking for a fresh
/// link invalidates the previous one, so an older link in a forwarded thread
/// stops working — the same rule the staff magic-link flow uses.
pub async fn invalidate_portal_one_use_tokens<'e, E>(
    executor: E,
    portal_user_id: Uuid,
    kind: &str,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"UPDATE portal_auth_one_use_tokens
              SET used_at = now()
            WHERE portal_user_id = $1 AND kind = $2 AND used_at IS NULL"#,
    )
    .bind(portal_user_id)
    .bind(kind)
    .execute(executor)
    .await?;
    Ok(())
}

// --- portal v2 appointment requests ---------------------------------------------
//
// The intent row between a client portal user and a staff calendar (see
// `entities::appointment_request` for the doctrine: requests are NOT
// bookings; only staff approval creates the real appointment). Status
// transitions are forward-only and enforced by the `AND status =
// 'pending'` predicate on every decision UPDATE — the row count is the
// concurrency gate, so a racing approve/decline/cancel resolves exactly
// once.

fn row_to_request(row: &sqlx::postgres::PgRow) -> core_domain::entities::AppointmentRequest {
    use sqlx::Row;
    core_domain::entities::AppointmentRequest {
        id: row.get("id"),
        org_id: row.get("org_id"),
        client_id: row.get("client_id"),
        portal_user_id: row.get("portal_user_id"),
        user_id: row.get("user_id"),
        title: row.get("title"),
        starts_at: row.get("starts_at"),
        ends_at: row.get("ends_at"),
        kind: core_domain::entities::AppointmentKind::parse(&row.get::<String, _>("kind"))
            .unwrap_or(core_domain::entities::AppointmentKind::Remote),
        notes: row.get("notes"),
        status: core_domain::entities::AppointmentRequestStatus::parse(
            &row.get::<String, _>("status"),
        )
        .unwrap_or(core_domain::entities::AppointmentRequestStatus::Pending),
        decision_note: row.get("decision_note"),
        decided_by: row.get("decided_by"),
        decided_at: row.get("decided_at"),
        appointment_id: row.get("appointment_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

const REQUEST_COLS: &str =
    "id, org_id, client_id, portal_user_id, user_id, title, starts_at, ends_at, kind::text AS kind, notes, \
     status, decision_note, decided_by, decided_at, appointment_id, created_at, updated_at";

pub async fn create_appointment_request<'e, E>(
    executor: E,
    r: &core_domain::entities::AppointmentRequest,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO portal_appointment_requests
             (id, org_id, client_id, portal_user_id, user_id, title, starts_at, ends_at,
              kind, notes, status)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::text::appointment_kind, $10, 'pending')"#,
        r.id,
        r.org_id,
        r.client_id,
        r.portal_user_id,
        r.user_id,
        r.title,
        r.starts_at,
        r.ends_at,
        r.kind.as_str(),
        r.notes,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// One request by id within the org. Callers add their own client /
/// portal-user scope on top for the portal surface.
pub async fn get_appointment_request<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<core_domain::entities::AppointmentRequest>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {REQUEST_COLS} FROM portal_appointment_requests WHERE org_id = $1 AND id = $2"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_all(executor)
        .await?;
    rows.first()
        .map(row_to_request)
        .ok_or(crate::DbError::NotFound)
}

/// The caller's OWN requests (client + portal-user scoped), newest first.
pub async fn list_own_appointment_requests<'e, E>(
    executor: E,
    org_id: Uuid,
    client_id: Uuid,
    portal_user_id: Uuid,
    limit: i64,
) -> DbResult<Vec<core_domain::entities::AppointmentRequest>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {REQUEST_COLS} FROM portal_appointment_requests \
         WHERE org_id = $1 AND client_id = $2 AND portal_user_id = $3 \
         ORDER BY created_at DESC LIMIT $4"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(client_id)
        .bind(portal_user_id)
        .bind(limit)
        .fetch_all(executor)
        .await?;
    Ok(rows.iter().map(row_to_request).collect())
}

/// Staff view: every request of the org, optionally narrowed by status.
pub async fn list_appointment_requests<'e, E>(
    executor: E,
    org_id: Uuid,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> DbResult<Vec<core_domain::entities::AppointmentRequest>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {REQUEST_COLS} FROM portal_appointment_requests \
         WHERE org_id = $1 AND ($2::text IS NULL OR status = $2) \
         ORDER BY created_at DESC LIMIT $3 OFFSET $4"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(status)
        .bind(limit)
        .bind(offset)
        .fetch_all(executor)
        .await?;
    Ok(rows.iter().map(row_to_request).collect())
}

pub struct AppointmentRequestDecision<'a> {
    pub status: &'a str,
    pub decision_note: Option<&'a str>,
    pub decided_by: Uuid,
    pub appointment_id: Option<Uuid>,
}

/// Staff decision (approve/decline): moves a PENDING request to the
/// target state, stamping decided_by/at. Returns None when the request
/// is not pending anymore (already decided/cancelled) or missing — the
/// caller maps that to a typed conflict.
pub async fn decide_appointment_request<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    decision: AppointmentRequestDecision<'_>,
) -> DbResult<Option<core_domain::entities::AppointmentRequest>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE portal_appointment_requests SET \
           status = $3, decision_note = $4, decided_by = $5, decided_at = now(), \
           appointment_id = $6, updated_at = now() \
         WHERE org_id = $1 AND id = $2 AND status = 'pending' \
         RETURNING {REQUEST_COLS}"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .bind(decision.status)
        .bind(decision.decision_note)
        .bind(decision.decided_by)
        .bind(decision.appointment_id)
        .fetch_all(executor)
        .await?;
    Ok(rows.first().map(row_to_request))
}

/// The requesting portal user withdraws their own still-pending request.
pub async fn cancel_own_appointment_request<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    client_id: Uuid,
    portal_user_id: Uuid,
) -> DbResult<Option<core_domain::entities::AppointmentRequest>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE portal_appointment_requests SET status = 'cancelled', decided_at = now(), updated_at = now() \
         WHERE org_id = $1 AND id = $2 AND client_id = $3 AND portal_user_id = $4 AND status = 'pending' \
         RETURNING {REQUEST_COLS}"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .bind(client_id)
        .bind(portal_user_id)
        .fetch_all(executor)
        .await?;
    Ok(rows.first().map(row_to_request))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_domain::notes::{visible_kinds, NoteAudience};

    /// The exposure join is the only ticket path portal handlers may call;
    /// comment whitelists come from the pure domain layer. This test pins
    /// the wiring assumption: the IT-manager audience must include in-house
    /// updates while excluding internal notes.
    #[test]
    fn portal_comment_whitelists_come_from_the_domain_layer() {
        assert_eq!(
            visible_kinds(NoteAudience::Portal),
            [core_domain::status::NoteKind::ClientVisible]
        );
        assert_eq!(
            visible_kinds(NoteAudience::PortalItManager),
            [
                core_domain::status::NoteKind::ClientVisible,
                core_domain::status::NoteKind::InHouseUpdate
            ]
        );
    }

    #[test]
    fn exposure_filter_defaults_are_type_safe() {
        // Compile-time pin of the filter shape used by both reads.
        let f = PortalTicketFilter {
            board_id: None,
            category: None,
            limit: 50,
            offset: 0,
        };
        assert!(f.board_id.is_none());
        assert!(f.category.is_none());
    }
}
