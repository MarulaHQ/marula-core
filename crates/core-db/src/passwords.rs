//! Password vault queries (docs-platform v1, migration 0053). The
//! database stores ONLY AES-256-GCM ciphertext (`crypto::seal` output
//! as bytea); plaintext never crosses this module — callers hand sealed
//! bytes in and receive sealed bytes out. Reveal is the API layer's
//! audited decrypt path, not a query here.
//!
//! Every secret change appends the previous ciphertext to
//! `password_history` (audit posture: history rows are never updated
//! or deleted by app code — cascade delete with the credential only).

use crate::{DbError, DbResult};
use core_domain::entities::{PasswordEntry, PasswordHistoryEntry};
use sqlx::{Postgres, Row};
use uuid::Uuid;

fn row(r: sqlx::postgres::PgRow) -> PasswordEntry {
    use sqlx::Row;
    PasswordEntry {
        id: r.get("id"),
        org_id: r.get("org_id"),
        client_id: r.get("client_id"),
        title: r.get("title"),
        username: r.get("username"),
        url: r.get("url"),
        expires_at: r.get("expires_at"),
        rotate_at: r.get("rotate_at"),
        last_rotated_at: r.get("last_rotated_at"),
        has_notes: r.get::<Option<Vec<u8>>, _>("notes_ciphertext").is_some(),
        created_by: r.get("created_by"),
        updated_by: r.get("updated_by"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

const COLS: &str = "id, org_id, client_id, title, username, url, expires_at, rotate_at, last_rotated_at, notes_ciphertext, created_by, updated_by, created_at, updated_at";

#[derive(Debug, Default, Clone)]
pub struct PasswordFilter {
    /// A client id, or None for org-level entries + all scopes.
    pub client_id: Option<Uuid>,
    /// `true` filters to org-global entries (client_id IS NULL).
    pub global_only: bool,
}

pub async fn list_passwords<'e, E>(
    executor: E,
    org_id: Uuid,
    filter: &PasswordFilter,
) -> DbResult<Vec<PasswordEntry>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM passwords
         WHERE org_id = $1
           AND ($2::uuid IS NULL OR client_id = $2)
           AND (NOT $3::bool OR client_id IS NULL)
         ORDER BY updated_at DESC, id"
    ))
    .bind(org_id)
    .bind(filter.client_id)
    .bind(filter.global_only)
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(row).collect())
}

pub async fn get_password<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<PasswordEntry>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(&format!(
        "SELECT {COLS} FROM passwords WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .map(row)
    .ok_or(DbError::NotFound)
}

/// The sealed secret + notes ciphertext for a credential (reveal path).
pub struct SealedSecret {
    pub secret: Vec<u8>,
    pub notes: Option<Vec<u8>>,
}

pub async fn get_sealed<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<SealedSecret>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let r = sqlx::query(
        "SELECT secret_ciphertext, notes_ciphertext FROM passwords WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(SealedSecret {
        secret: r.get("secret_ciphertext"),
        notes: r.get("notes_ciphertext"),
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn create_password<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    client_id: Option<Uuid>,
    title: &str,
    username: &str,
    url: &str,
    secret_ciphertext: &[u8],
    notes_ciphertext: Option<&[u8]>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    rotate_at: Option<chrono::DateTime<chrono::Utc>>,
    actor: Uuid,
) -> DbResult<PasswordEntry>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let r = sqlx::query(&format!(
        "INSERT INTO passwords (id, org_id, client_id, title, username, url, secret_ciphertext, notes_ciphertext, expires_at, rotate_at, created_by, updated_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11)
         RETURNING {COLS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(client_id)
    .bind(title)
    .bind(username)
    .bind(url)
    .bind(secret_ciphertext)
    .bind(notes_ciphertext)
    .bind(expires_at)
    .bind(rotate_at)
    .bind(actor)
    .fetch_one(executor)
    .await?;
    Ok(row(r))
}

/// Updates the metadata and (when `new_secret`/`new_notes` carry sealed
/// bytes) rotates the ciphertext. Any prior ciphertext goes to history
/// FIRST, inside the caller's transaction.
#[allow(clippy::too_many_arguments)]
pub async fn update_password(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    title: &str,
    username: &str,
    url: &str,
    new_secret: Option<&[u8]>,
    new_notes: Option<Option<&[u8]>>,
    expires_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    rotate_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    actor: Uuid,
) -> DbResult<PasswordEntry> {
    // Archive the outgoing ciphertext whenever the secret changes.
    if new_secret.is_some() {
        sqlx::query(
            "INSERT INTO password_history (id, org_id, password_id, secret_ciphertext, changed_by, created_at)
             SELECT gen_random_uuid(), org_id, id, secret_ciphertext, $3, now()
             FROM passwords WHERE org_id = $1 AND id = $2",
        )
        .bind(org_id)
        .bind(id)
        .bind(actor)
        .execute(&mut *executor)
        .await?;
    }
    let r = sqlx::query(&format!(
        "UPDATE passwords SET
           title = $3,
           username = $4,
           url = $5,
           secret_ciphertext = COALESCE($6, secret_ciphertext),
           notes_ciphertext = CASE WHEN $7::bool THEN $8 ELSE notes_ciphertext END,
           expires_at = CASE WHEN $9::bool THEN $10 ELSE expires_at END,
           rotate_at = CASE WHEN $12::bool THEN $13 ELSE rotate_at END,
           -- Moving the schedule stamps the rotation ledger (Wave 19
           -- lane 3): a fresh rotate_at IS a rotation of the entry; the
           -- sealed bytes rotate through the normal history path when
           -- new_secret rides along.
           last_rotated_at = CASE WHEN $12::bool THEN now() ELSE last_rotated_at END,
           updated_by = $11,
           updated_at = now()
         WHERE org_id = $1 AND id = $2
         RETURNING {COLS}"
    ))
    .bind(org_id)
    .bind(id)
    .bind(title)
    .bind(username)
    .bind(url)
    .bind(new_secret)
    .bind(new_notes.is_some())
    .bind(new_notes.and_then(|n| n))
    .bind(expires_at.is_some())
    .bind(expires_at.flatten())
    .bind(actor)
    .bind(rotate_at.is_some())
    .bind(rotate_at.flatten())
    .fetch_optional(executor)
    .await?
    .map(row)
    .ok_or(DbError::NotFound)?;
    Ok(r)
}

pub async fn delete_password<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let r = sqlx::query("DELETE FROM passwords WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(executor)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

pub async fn list_history<'e, E>(
    executor: E,
    org_id: Uuid,
    password_id: Uuid,
) -> DbResult<Vec<PasswordHistoryEntry>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        "SELECT id, org_id, password_id, changed_by, created_at
         FROM password_history
         WHERE org_id = $1 AND password_id = $2
         ORDER BY created_at DESC",
    )
    .bind(org_id)
    .bind(password_id)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| PasswordHistoryEntry {
            id: r.get("id"),
            org_id: r.get("org_id"),
            password_id: r.get("password_id"),
            changed_by: r.get("changed_by"),
            created_at: r.get("created_at"),
        })
        .collect())
}
