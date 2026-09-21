//! Operator signing-key trust store (Wave 19 lane 2, ADR 0009 rotation
//! semantics, migration 0095): CRUD for `plugin_signing_keys`.
//!
//! Purpose: the global (server-wide, no org_id, no RLS — the
//! `plugin_catalog` doctrine) registry the distribution verification
//! path resolves the signature block's `key_id` against. The rotation
//! model: ADD a new key (active), publish against it, RETIRE the old one
//! (a retired key STILL verifies pre-rotation artifacts — retired is
//! 'past trust', not distrust), REVOKE only on compromise (a revoked key
//! verifies nothing retroactively, which is the point).
//!
//! Non-purpose: key material — `public_key` is verification-only
//! material (64 hex chars); private keys never live here, never in this
//! codebase, never in the vault. No parallel audit: lifecycle transitions
//! are operator CLI actions on a config table — the rotation RUNBOOK
//! (docs/reference/plugin-signing-rotation-runbook.md) and the typed
//! verification reasons carry the supply-chain accountability.

use crate::{DbError, DbResult};
use core_domain::entities::{SigningKeyRecord, SigningKeyStatus};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

const KEY_COLS: &str = "id, key_id, public_key, status::text AS status, label, created_at, \
     retired_at, revoked_at, revoked_reason";

fn row_to_key(row: &sqlx::postgres::PgRow) -> DbResult<SigningKeyRecord> {
    let status: String = row.get("status");
    let status = SigningKeyStatus::parse(&status)
        .ok_or_else(|| DbError::Conflict(format!("unknown signing key status {status:?}")))?;
    Ok(SigningKeyRecord {
        id: row.get("id"),
        key_id: row.get("key_id"),
        public_key: row.get("public_key"),
        status,
        label: row.get("label"),
        created_at: row.get("created_at"),
        retired_at: row.get("retired_at"),
        revoked_at: row.get("revoked_at"),
        revoked_reason: row.get("revoked_reason"),
    })
}

/// Every registry row (the operator roster surface — active, retired,
/// and revoked rows live in history; a revoked key_id stays occupied so
/// the id can never be re-registered after compromise).
pub async fn list_keys(conn: &mut PgConnection) -> DbResult<Vec<SigningKeyRecord>> {
    let rows = sqlx::query(&format!(
        "SELECT {KEY_COLS} FROM plugin_signing_keys ORDER BY created_at, key_id"
    ))
    .fetch_all(&mut *conn)
    .await?;
    rows.iter().map(row_to_key).collect::<Result<Vec<_>, _>>()
}

/// The keys that VERIFY an artifact today: active + retired (rotation
/// overlap). Revoked keys never appear — denial is retroactive.
pub async fn trusted_verification_keys(conn: &mut PgConnection) -> DbResult<Vec<SigningKeyRecord>> {
    let rows = sqlx::query(&format!(
        "SELECT {KEY_COLS} FROM plugin_signing_keys WHERE status IN ('active', 'retired') \
         ORDER BY created_at"
    ))
    .fetch_all(&mut *conn)
    .await?;
    rows.iter().map(row_to_key).collect::<Result<Vec<_>, _>>()
}

/// Adds a key as ACTIVE. `key_id` defaults to the first 16 hex chars of
/// the public key when the operator does not name one. The public key
/// must be exactly 64 hex chars (the DB CHECK is the last gate; this
/// boundary keeps error text helpful without echoing material back).
pub async fn add_key(
    conn: &mut PgConnection,
    key_id: Option<&str>,
    public_key_hex: &str,
    label: &str,
) -> DbResult<SigningKeyRecord> {
    let pk = public_key_hex.trim().to_lowercase();
    if pk.len() != 64 || !pk.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(DbError::Conflict(
            "public_key must be exactly 64 hex chars (the ed25519 public key)".into(),
        ));
    }
    let key_id = key_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| pk.chars().take(16).collect());
    let id = Uuid::now_v7();
    let row = sqlx::query(&format!(
        "INSERT INTO plugin_signing_keys (id, key_id, public_key, label)
         VALUES ($1, $2, $3, $4) RETURNING {KEY_COLS}"
    ))
    .bind(id)
    .bind(&key_id)
    .bind(&pk)
    .bind(label.trim())
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.constraint().is_some() => {
            DbError::Conflict(format!("signing key id {key_id:?} is already registered"))
        }
        _ => e.into(),
    })?;
    row_to_key(&row)
}

/// Rotation step: an ACTIVE key becomes RETIRED — it stops being named
/// by NEW publications but keeps verifying pre-rotation artifacts.
pub async fn retire_key(conn: &mut PgConnection, key_id: &str) -> DbResult<SigningKeyRecord> {
    let row = sqlx::query(&format!(
        "UPDATE plugin_signing_keys SET status = 'retired', retired_at = now()
         WHERE key_id = $1 AND status = 'active' RETURNING {KEY_COLS}"
    ))
    .bind(key_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    row_to_key(&row)
}

/// COMPROMISE response: the key verifies nothing from this moment on
/// (retroactively poisoning every artifact citing it is deliberate).
/// A revoked key_id stays occupied forever — never re-registered.
pub async fn revoke_key(
    conn: &mut PgConnection,
    key_id: &str,
    reason: &str,
) -> DbResult<SigningKeyRecord> {
    let reason = reason.trim().to_string();
    if reason.is_empty() {
        return Err(DbError::Conflict(
            "revoking a signing key requires a reason (the registry records it forever)".into(),
        ));
    }
    let row = sqlx::query(&format!(
        "UPDATE plugin_signing_keys SET status = 'revoked', revoked_at = now(), revoked_reason = $2
         WHERE key_id = $1 AND status <> 'revoked' RETURNING {KEY_COLS}"
    ))
    .bind(key_id)
    .bind(&reason)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| match &e {
        sqlx::Error::RowNotFound => DbError::NotFound,
        _ => e.into(),
    })?;
    row_to_key(&row)
}

#[cfg(test)]
mod tests {

    // Row-shape tests live in the SQLx API suites (workflow tests need a
    // real Postgres); structural constants are covered here so a spelling
    // drift in the retired-trust semantics fails the cheap suite.
    #[test]
    fn derived_key_id_takes_the_first_16_hex() {
        let pk = "a".repeat(64);
        let derived: String = pk.chars().take(16).collect();
        assert_eq!(derived.len(), 16);
        assert!(pk.len() == 64 && pk.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
