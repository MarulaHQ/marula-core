//! Secret category trees + SecretRecord generalization (native vault,
//! ADR 0006, ADR 0004 template doctrine, migration 0078).
//!
//! Scope: exactly one of org/client — org-scope rows carry `org_id`,
//! client-scope rows carry `client_id` (their RLS tenancy spine rides
//! the owning client's org). Every material field of a record is
//! SEALED (`crypto::seal` ciphertexts in `secret_fields`); this module
//! never touches plaintext — callers hand sealed bytes in and receive
//! sealed bytes out. Reveal is the API layer's audited decrypt path.
//!
//! Doctrine encoded here:
//! * Seeds are IMMUTABLE (`is_default_seed = true`): update/delete are
//!   refused — customization forks (ADR 0004 template doctrine).
//! * A folder whose subtree still holds records refuses deletion at the
//!   DB level (the `secret_records.folder_id` FK default block);
//!   moving contents out is a deliberate act through the record routes.
//! * Forks create a new row graph (never edit-in-place): `fork_folder`
//!   deep-copies a folder subtree into client scope with `forked_from`
//!   provenance on the root.
//! * Category RBAC rides `secret_category_grants` (role × folder):
//!   `folder_access` walks the ancestor chain — unrestricted folders
//!   allow everything the matrix already granted; restricted folders
//!   deny unless a grant row for one of the caller's roles allows it.

use core_domain::entities::{
    SecretCategoryGrant, SecretFieldMeta, SecretFolder, SecretFolderKind, SecretRecord,
    SecretRecordKind,
};
use sqlx::{PgConnection, Postgres, Row};
use std::collections::HashMap;
use uuid::Uuid;

use crate::{DbError, DbResult};

/// The 5 immutable seeded defaults as data: (slug, label, kind).
pub const DEFAULT_FOLDERS: [(&str, &str, SecretFolderKind); 5] = [
    ("passwords", "Passwords", SecretFolderKind::Passwords),
    ("api-keys", "API Keys", SecretFolderKind::ApiKeys),
    (
        "certificates",
        "Certificates",
        SecretFolderKind::Certificates,
    ),
    (
        "recovery-codes",
        "Recovery Codes",
        SecretFolderKind::RecoveryCodes,
    ),
    ("documents", "Documents", SecretFolderKind::Documents),
];

/// Plants the immutable default category tree for a NEW org (the same 5
/// folders migration 0078 backfilled for orgs that already existed).
pub async fn seed_default_secret_folders(
    conn: &mut PgConnection,
    org_id: Uuid,
    actor: Uuid,
) -> DbResult<()> {
    for (slug, label, kind) in DEFAULT_FOLDERS {
        create_folder(
            conn,
            org_id,
            None,
            None,
            Uuid::now_v7(),
            slug,
            label,
            kind,
            true,
            false,
            actor,
        )
        .await?;
    }
    Ok(())
}

fn folder_row(r: sqlx::postgres::PgRow) -> SecretFolder {
    SecretFolder {
        id: r.get("id"),
        org_id: r.get("org_id"),
        client_id: r.get("client_id"),
        parent_id: r.get("parent_id"),
        slug: r.get("slug"),
        label: r.get("label"),
        kind: SecretFolderKind::parse(r.get::<String, _>("kind").as_str())
            .unwrap_or(SecretFolderKind::Passwords),
        is_default_seed: r.get("is_default_seed"),
        restricted: r.get("restricted"),
        forked_from: r.get("forked_from"),
        created_by: r.get("created_by"),
        updated_by: r.get("updated_by"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn record_row(r: sqlx::postgres::PgRow) -> SecretRecord {
    SecretRecord {
        id: r.get("id"),
        org_id: r.get("org_id"),
        client_id: r.get("client_id"),
        folder_id: r.get("folder_id"),
        kind: SecretRecordKind::parse(r.get::<String, _>("kind").as_str())
            .unwrap_or(SecretRecordKind::Password),
        title: r.get("title"),
        expires_at: r.get("expires_at"),
        object_key: r.get("object_key"),
        file_sha256: r.get("file_sha256"),
        file_bytes: r.get("file_bytes"),
        fields: Vec::new(),
        created_by: r.get("created_by"),
        updated_by: r.get("updated_by"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn field_meta_row(r: sqlx::postgres::PgRow) -> SecretFieldMeta {
    SecretFieldMeta {
        record_id: r.get("record_id"),
        field_key: r.get("field_key"),
        has_value: r.get::<Option<Vec<u8>>, _>("value_ciphertext").is_some(),
        updated_at: r.get("updated_at"),
    }
}

fn grant_row(r: sqlx::postgres::PgRow) -> SecretCategoryGrant {
    SecretCategoryGrant {
        id: r.get("id"),
        org_id: r.get("org_id"),
        role_id: r.get("role_id"),
        folder_id: r.get("folder_id"),
        can_read: r.get("can_read"),
        can_edit: r.get("can_edit"),
        created_by: r.get("created_by"),
        updated_by: r.get("updated_by"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

/// DB-level refusals typed as `Conflict`; unique (`23505`) violations
/// ride the raw sqlx error so the API layer's constraint-name registry
/// can type them (`folder_slug_taken`, `record_title_taken`, …).
fn maybe_conflict(e: sqlx::Error, msg: &str) -> DbError {
    if let sqlx::Error::Database(db) = &e {
        match db.code().as_deref() {
            Some("23503") => return DbError::Conflict(msg.to_string()),
            Some("23505") => return DbError::Sql(e),
            _ => {}
        }
    }
    e.into()
}

/// Org anchor: the rows' org must be the caller's org, either directly
/// (org-scope rows carry `org_id`) or through their owning client
/// (exactly-one-of scope pairs from migration 0078).
fn in_org_scope(alias: &str, org_p: usize) -> String {
    format!(
        "({alias}.org_id = ${org_p} OR ({alias}.org_id IS NULL AND {alias}.client_id IN \
         (SELECT id FROM clients WHERE org_id = ${org_p})))"
    )
}

// ---------------------------------------------------------------------------
// Folders
// ---------------------------------------------------------------------------

/// Org-scope folder browse (the org-level vault tree).
pub async fn list_org_folders(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<SecretFolder>> {
    let sql = "SELECT id, org_id, client_id, parent_id, slug, label, kind, is_default_seed, restricted, forked_from, created_by, updated_by, created_at, updated_at
               FROM secret_folders sf
               WHERE sf.org_id = $1 AND sf.client_id IS NULL
               ORDER BY sf.parent_id NULLS FIRST, sf.created_at";
    let rows = sqlx::query(sql).bind(org_id).fetch_all(&mut *conn).await?;
    Ok(rows.into_iter().map(folder_row).collect())
}

/// Client-scope folder browse (a client's forked vault tree).
pub async fn list_client_folders(
    conn: &mut PgConnection,
    client_id: Uuid,
) -> DbResult<Vec<SecretFolder>> {
    let sql = "SELECT id, org_id, client_id, parent_id, slug, label, kind, is_default_seed, restricted, forked_from, created_by, updated_by, created_at, updated_at
               FROM secret_folders sf
               WHERE sf.client_id = $1
               ORDER BY sf.parent_id NULLS FIRST, sf.created_at";
    let rows = sqlx::query(sql)
        .bind(client_id)
        .fetch_all(&mut *conn)
        .await?;
    Ok(rows.into_iter().map(folder_row).collect())
}

/// Folders anywhere inside the org (the audit/export surface only).
pub async fn list_all_org_folders(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<SecretFolder>> {
    let scope = in_org_scope("sf", 1);
    let sql = "SELECT id, org_id, client_id, parent_id, slug, label, kind, is_default_seed, restricted, forked_from, created_by, updated_by, created_at, updated_at
               FROM secret_folders sf
               WHERE {SCOPE}
               ORDER BY sf.parent_id NULLS FIRST, sf.created_at"
        .replace("{SCOPE}", &scope);
    let rows = sqlx::query(&sql).bind(org_id).fetch_all(&mut *conn).await?;
    Ok(rows.into_iter().map(folder_row).collect())
}

/// One folder; only inside the org (org-scope direct, client-scope
/// through the owning client).
pub async fn get_folder<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<SecretFolder>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let scope = in_org_scope("sf", 1);
    let sql = "SELECT id, org_id, client_id, parent_id, slug, label, kind, is_default_seed, restricted, forked_from, created_by, updated_by, created_at, updated_at
               FROM secret_folders sf
               WHERE sf.id = $2 AND ".to_string() + &scope;
    let r = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(folder_row(r))
}

/// Creates a folder in exactly one scope. Org scope: `client_id = None`.
/// Client scope must ride an existing client of the calling org.
#[allow(clippy::too_many_arguments)]
pub async fn create_folder(
    conn: &mut PgConnection,
    org_id: Uuid,
    client_id: Option<Uuid>,
    parent_id: Option<Uuid>,
    id: Uuid,
    slug: &str,
    label: &str,
    kind: SecretFolderKind,
    is_default_seed: bool,
    restricted: bool,
    actor: Uuid,
) -> DbResult<SecretFolder> {
    // Parent must exist in the SAME scope: org-scope parent for an
    // org-scope folder, same-client parent for a client-scope folder.
    if let Some(parent) = parent_id {
        let ok = sqlx::query(
            "SELECT 1 FROM secret_folders sf
             WHERE sf.id = $2 AND sf.org_id = $1 AND sf.client_id IS NULL",
        )
        .bind(org_id)
        .bind(parent)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|e| maybe_conflict(e, "parent folder lookup rejected"))?;
        match (ok.is_some(), client_id) {
            (true, None) => {}
            (true, Some(_)) => {
                return Err(DbError::Conflict(
                    "a client-scope folder cannot hang off an org-scope parent".into(),
                ))
            }
            (false, _) => return Err(DbError::NotFound),
        }
    }
    // Client-scope rows must ride an existing client of the calling org.
    if let Some(client) = client_id {
        let ok = sqlx::query("SELECT 1 FROM clients WHERE id = $2 AND org_id = $1")
            .bind(org_id)
            .bind(client)
            .fetch_optional(&mut *conn)
            .await
            .map_err(|e| maybe_conflict(e, "client scope lookup rejected"))?;
        if ok.is_none() {
            return Err(DbError::NotFound);
        }
    }
    sqlx::query(
        "INSERT INTO secret_folders
           (id, org_id, client_id, parent_id, slug, label, kind,
            is_default_seed, restricted, created_by, updated_by)
         VALUES ($2, $1, $3, $4, $5, $6, $7, $8, $9, $10, $10)",
    )
    .bind(org_id)
    .bind(id)
    .bind(client_id)
    .bind(parent_id)
    .bind(slug.trim().to_lowercase())
    .bind(label)
    .bind(kind.as_str())
    .bind(is_default_seed)
    .bind(restricted)
    .bind(actor)
    .execute(&mut *conn)
    .await
    .map_err(|e| maybe_conflict(e, "secret folder create rejected"))?;
    get_folder(&mut *conn, org_id, id).await
}

/// Mutates label / slug / restricted. IMMUTABLE seed refusal (ADR 0004
/// template doctrine): customization forks, never edit-in-place.
pub async fn update_folder(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    label: Option<&str>,
    slug: Option<&str>,
    restricted: Option<bool>,
    actor: Uuid,
) -> DbResult<SecretFolder> {
    let seed = sqlx::query_scalar::<_, bool>(
        "SELECT is_default_seed FROM secret_folders WHERE id = $2 AND org_id = $1",
    )
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    if seed {
        return Err(DbError::Conflict(
            "the shipped default folder is immutable as data; fork it instead (ADR 0004)".into(),
        ));
    }
    let r = sqlx::query(
        "UPDATE secret_folders SET
           label = COALESCE($3, label),
           slug = COALESCE(lower($4), slug),
           restricted = COALESCE($5, restricted),
           updated_by = $6,
           updated_at = now()
         WHERE id = $2 AND org_id = $1",
    )
    .bind(org_id)
    .bind(id)
    .bind(label.map(|l| l.trim().to_string()))
    .bind(slug.map(|s| s.trim().to_lowercase()))
    .bind(restricted)
    .bind(actor)
    .execute(&mut *conn)
    .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    get_folder(&mut *conn, org_id, id).await
}

/// Folders with non-empty subtrees refuse deletion at the FK (records
/// never cascade-eaten by the folder write path); empty subtrees
/// cascade-delete (children only).
pub async fn delete_folder(conn: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<()> {
    let seed = sqlx::query_scalar::<_, bool>(
        "SELECT is_default_seed FROM secret_folders WHERE id = $2 AND org_id = $1",
    )
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    if seed {
        return Err(DbError::Conflict(
            "the shipped default folder is immutable as data; fork it instead (ADR 0004)".into(),
        ));
    }
    let r = sqlx::query("DELETE FROM secret_folders WHERE id = $2 AND org_id = $1")
        .bind(org_id)
        .bind(id)
        .execute(&mut *conn)
        .await
        .map_err(|e| maybe_conflict(e, "folder subtree still holds secret records"))?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// FORK-THE-SEED (ADR 0004 template doctrine, not edit-in-place):
/// deep-copies a folder subtree into `client_id` scope as a new row
/// graph. The root row carries `forked_from` provenance pointing at the
/// source; descendants are plain copies inside the new graph. Returned
/// rows: the newly created client-scope tree, root first.
pub async fn fork_folder(
    conn: &mut PgConnection,
    org_id: Uuid,
    source_id: Uuid,
    client_id: Uuid,
    actor: Uuid,
) -> DbResult<Vec<SecretFolder>> {
    // The client must belong to the calling org.
    let anchor = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM clients WHERE id = $2 AND org_id = $1)",
    )
    .bind(org_id)
    .bind(client_id)
    .fetch_one(&mut *conn)
    .await?;
    if !anchor {
        return Err(DbError::NotFound);
    }
    // Pull the source subtree parent-first (deterministic depth walk);
    // the shipped seed lives in org scope.
    let rows = sqlx::query(
        "WITH RECURSIVE tree AS (
           SELECT sf.*, 0 AS depth FROM secret_folders sf WHERE sf.id = $2
           UNION ALL
           SELECT c.*, t.depth + 1 FROM secret_folders c JOIN tree t ON c.parent_id = t.id
         )
         SELECT id, parent_id, slug, label, kind, is_default_seed, restricted
         FROM tree ORDER BY depth",
    )
    .bind(org_id)
    .bind(source_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(|e| maybe_conflict(e, "source subtree lookup rejected"))?;
    if rows.is_empty() {
        return Err(DbError::NotFound);
    }

    // Old id -> new id remap; walk parent-first so parents exist before
    // their children reference them.
    let mut map: HashMap<Uuid, Uuid> = HashMap::new();
    let mut created: Vec<SecretFolder> = Vec::with_capacity(rows.len());
    for (i, r) in rows.iter().enumerate() {
        let old_id: Uuid = r.get("id");
        let old_parent: Option<Uuid> = r.get("parent_id");
        let new_id = Uuid::now_v7();
        let new_parent = old_parent.and_then(|p| map.get(&p)).copied();
        // The forked ROOT carries the provenance.
        let forked_from: Option<Uuid> = if i == 0 { Some(source_id) } else { None };
        sqlx::query(
            "INSERT INTO secret_folders
               (id, org_id, client_id, parent_id, slug, label, kind,
                is_default_seed, restricted, forked_from, created_by, updated_by)
             VALUES ($2, NULL, $3, $4, $5, $6, $7, false, $8, $9, $10, $10)",
        )
        .bind(org_id)
        .bind(new_id)
        .bind(client_id)
        .bind(new_parent)
        .bind(r.get::<String, _>("slug"))
        .bind(r.get::<String, _>("label"))
        .bind(r.get::<String, _>("kind"))
        .bind(r.get::<bool, _>("restricted"))
        .bind(forked_from)
        .bind(actor)
        .execute(&mut *conn)
        .await
        .map_err(|e| maybe_conflict(e, "fork insert rejected"))?;
        map.insert(old_id, new_id);
        created.push(get_folder(&mut *conn, org_id, new_id).await?);
    }
    Ok(created)
}

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

/// Org-scope record list (the org-level vault browser).
pub async fn list_org_records(
    conn: &mut PgConnection,
    org_id: Uuid,
    folder_id: Option<Uuid>,
) -> DbResult<Vec<SecretRecord>> {
    let sql = "SELECT id, org_id, client_id, folder_id, kind::text AS kind, title, expires_at, object_key, file_sha256, file_bytes, created_by, updated_by, created_at, updated_at
               FROM secret_records sr
               WHERE sr.org_id = $1
                 AND ($2::uuid IS NULL OR sr.folder_id = $2)
               ORDER BY sr.updated_at DESC";
    let rows = sqlx::query(sql)
        .bind(org_id)
        .bind(folder_id)
        .fetch_all(&mut *conn)
        .await?;
    let ids: Vec<Uuid> = rows.iter().map(|r| r.get("id")).collect();
    let mut out: Vec<SecretRecord> = rows.into_iter().map(record_row).collect();
    attach_fields(conn, &ids, &mut out).await?;
    Ok(out)
}

/// Client-scope record list (a client's vault browser).
pub async fn list_client_records(
    conn: &mut PgConnection,
    client_id: Uuid,
    folder_id: Option<Uuid>,
) -> DbResult<Vec<SecretRecord>> {
    let sql = "SELECT id, org_id, client_id, folder_id, kind::text AS kind, title, expires_at, object_key, file_sha256, file_bytes, created_by, updated_by, created_at, updated_at
               FROM secret_records sr
               WHERE sr.client_id = $1
                 AND ($2::uuid IS NULL OR sr.folder_id = $2)
               ORDER BY sr.updated_at DESC";
    let rows = sqlx::query(sql)
        .bind(client_id)
        .bind(folder_id)
        .fetch_all(&mut *conn)
        .await?;
    let ids: Vec<Uuid> = rows.iter().map(|r| r.get("id")).collect();
    let mut out: Vec<SecretRecord> = rows.into_iter().map(record_row).collect();
    attach_fields(conn, &ids, &mut out).await?;
    Ok(out)
}

/// Metadata-only field views (never ciphertext, never plaintext).
async fn attach_fields(
    conn: &mut PgConnection,
    _record_ids: &[Uuid],
    out: &mut [SecretRecord],
) -> DbResult<()> {
    for rec in out.iter_mut() {
        let sql = "SELECT record_id, field_key, value_ciphertext, updated_at
                   FROM secret_fields f
                   WHERE f.record_id = $1
                   ORDER BY f.field_key";
        let rows = sqlx::query(sql).bind(rec.id).fetch_all(&mut *conn).await?;
        rec.fields = rows.into_iter().map(field_meta_row).collect();
    }
    Ok(())
}

/// One record + metadata-only field views; only inside the org.
pub async fn get_record(conn: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<SecretRecord> {
    let scope = in_org_scope("sr", 1);
    let sql = "SELECT id, org_id, client_id, folder_id, kind::text AS kind, title, expires_at, object_key, file_sha256, file_bytes, created_by, updated_by, created_at, updated_at
               FROM secret_records sr
               WHERE sr.id = $2 AND ".to_string() + &scope;
    let r = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(DbError::NotFound)?;
    let mut rec = record_row(r);
    let sql = "SELECT record_id, field_key, value_ciphertext, updated_at
               FROM secret_fields f
               WHERE f.record_id = $1
               ORDER BY f.field_key";
    let metas = sqlx::query(sql).bind(rec.id).fetch_all(&mut *conn).await?;
    rec.fields = metas.into_iter().map(field_meta_row).collect();
    Ok(rec)
}

/// Creates a record inside a folder whose scope matches the record's
/// scope (org vs the same client); sealed field bytes hand in. Returns
/// the record row with metadata-only field views.
#[allow(clippy::too_many_arguments)]
pub async fn create_record(
    conn: &mut PgConnection,
    org_id: Uuid,
    client_id: Option<Uuid>,
    folder_id: Uuid,
    id: Uuid,
    kind: SecretRecordKind,
    title: &str,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    object_key: Option<&str>,
    file_sha256: Option<&str>,
    file_bytes: Option<i64>,
    sealed_fields: &[(String, Vec<u8>)],
    actor: Uuid,
) -> DbResult<SecretRecord> {
    // The owning folder must exist inside the same org AND carry the
    // SAME scope as the record (the six-scope invariant from 0078).
    let folder =
        sqlx::query("SELECT client_id FROM secret_folders sf WHERE sf.id = $2 AND sf.org_id = $1")
            .bind(org_id)
            .bind(folder_id)
            .fetch_optional(&mut *conn)
            .await?;
    let folder_client: Option<Uuid> = folder.ok_or(DbError::NotFound)?.get("client_id");
    if folder_client != client_id {
        return Err(DbError::Conflict(
            "folder scope does not match the record's scope".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO secret_records
           (id, org_id, client_id, folder_id, kind, title, expires_at,
            object_key, file_sha256, file_bytes, created_by, updated_by)
         VALUES ($2, $1, $3, $4, $5::secret_record_kind, $6, $7, $8, $9, $10, $11, $11)",
    )
    .bind(org_id)
    .bind(id)
    .bind(client_id)
    .bind(folder_id)
    .bind(kind.as_str())
    .bind(title)
    .bind(expires_at)
    .bind(object_key)
    .bind(file_sha256)
    .bind(file_bytes)
    .bind(actor)
    .execute(&mut *conn)
    .await
    .map_err(|e| maybe_conflict(e, "secret record create rejected"))?;
    for (key, ciphertext) in sealed_fields {
        sqlx::query(
            "INSERT INTO secret_fields (id, record_id, org_id, client_id, field_key, value_ciphertext)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(Uuid::now_v7())
        .bind(id)
        .bind(org_id)
        .bind(client_id)
        .bind(key)
        .bind(ciphertext)
        .execute(&mut *conn)
        .await
        .map_err(|e| maybe_conflict(e, "secret field seal rejected"))?;
    }
    get_record(conn, org_id, id).await
}

/// Updates metadata + sealed fields. Field semantics mirror the shipped
/// password surface: absent key ignored; `Some(None)` clears; `Some`
/// bytes replaces (rotate). Returns the fresh record.
#[allow(clippy::type_complexity)]
pub async fn update_record(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    title: &str,
    expires_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    field_updates: &[(String, Option<Vec<u8>>)],
    actor: Uuid,
) -> DbResult<SecretRecord> {
    let r = sqlx::query(
        "UPDATE secret_records SET
           title = $3,
           expires_at = CASE WHEN $4::bool THEN $5 ELSE expires_at END,
           updated_by = $6,
           updated_at = now()
         WHERE id = $2 AND org_id = $1",
    )
    .bind(org_id)
    .bind(id)
    .bind(title)
    .bind(expires_at.is_some())
    .bind(expires_at.flatten())
    .bind(actor)
    .execute(&mut *conn)
    .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    for (key, value) in field_updates {
        if let Some(v) = value {
            sqlx::query(
                "INSERT INTO secret_fields (id, record_id, org_id, client_id, field_key, value_ciphertext)
                 SELECT $1, sr.id, sr.org_id, sr.client_id, $2, $3 FROM secret_records sr
                 WHERE sr.id = $4 AND sr.org_id = $5
                 ON CONFLICT (record_id, field_key)
                 DO UPDATE SET value_ciphertext = $3, updated_at = now()",
            )
            .bind(Uuid::now_v7())
            .bind(key)
            .bind(v)
            .bind(id)
            .bind(org_id)
            .execute(&mut *conn)
            .await
            .map_err(|e| maybe_conflict(e, "sealed field update rejected"))?;
        } else {
            sqlx::query(
                "DELETE FROM secret_fields
                 WHERE record_id = (SELECT x.id FROM secret_records x WHERE x.id = $2 AND x.org_id = $1)
                   AND field_key = $3",
            )
            .bind(org_id)
            .bind(id)
            .bind(key)
            .execute(&mut *conn)
            .await
            .map_err(|e| maybe_conflict(e, "sealed field clear rejected"))?;
        }
    }
    get_record(conn, org_id, id).await
}

/// Sets the document-kind ciphertext-file pointer columns (A4, Wave 16
/// lane 9): the record row carries ONLY the object-storage key + the
/// sha256 + byte length of the SEALED payload — never the bytes
/// themselves, which live in the object store behind [`core_store`].
/// Refused for non-document records (kind gate).
pub async fn set_file_pointer(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    object_key: &str,
    file_sha256: &str,
    file_bytes: i64,
    actor: Uuid,
) -> DbResult<SecretRecord> {
    let r = sqlx::query(
        "UPDATE secret_records SET
           object_key = $3, file_sha256 = $4, file_bytes = $5,
           updated_by = $6, updated_at = now()
         WHERE id = $2 AND org_id = $1 AND kind = 'document'",
    )
    .bind(org_id)
    .bind(id)
    .bind(object_key)
    .bind(file_sha256)
    .bind(file_bytes)
    .bind(actor)
    .execute(&mut *conn)
    .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    get_record(conn, org_id, id).await
}

/// Counts other document records in the org pointing at the same storage
/// key (content-addressed ciphertext dedup — safe object deletion check).
pub async fn count_other_file_refs(
    conn: &mut PgConnection,
    org_id: Uuid,
    object_key: &str,
    exclude_record: Uuid,
) -> DbResult<i64> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM secret_records
         WHERE org_id = $1 AND object_key = $2 AND id <> $3",
    )
    .bind(org_id)
    .bind(object_key)
    .bind(exclude_record)
    .fetch_one(&mut *conn)
    .await?;
    Ok(n)
}

pub async fn delete_record(conn: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<()> {
    let r = sqlx::query("DELETE FROM secret_records WHERE id = $2 AND org_id = $1")
        .bind(org_id)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// The ONLY ciphertext read path in this module (the API's audited
/// reveal endpoint decrypts on top of this). Sealed field bytes only.
pub async fn list_sealed_fields(
    conn: &mut PgConnection,
    org_id: Uuid,
    record_id: Uuid,
) -> DbResult<Vec<(String, Vec<u8>)>> {
    let sql = "SELECT f.field_key, f.value_ciphertext
               FROM secret_fields f
               JOIN secret_records sr ON sr.id = f.record_id AND sr.org_id = $1
               WHERE f.record_id = $2
               ORDER BY f.field_key";
    let rows = sqlx::query(sql)
        .bind(org_id)
        .bind(record_id)
        .fetch_all(&mut *conn)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.get("field_key"), r.get("value_ciphertext")))
        .collect())
}

// ---------------------------------------------------------------------------
// Category grants (role × folder)
// ---------------------------------------------------------------------------

/// Upserts ONE grant row (deny-by-default restricted categories grant
/// read/edit per ADR 0004-adjacent data doctrine). `can_read` and
/// `can_edit` may not BOTH be false (CHECK can_read OR can_edit).
#[allow(clippy::too_many_arguments)]
pub async fn upsert_grant(
    conn: &mut PgConnection,
    org_id: Uuid,
    role_id: Uuid,
    folder_id: Uuid,
    can_read: bool,
    can_edit: bool,
    actor: Uuid,
) -> DbResult<SecretCategoryGrant> {
    // Grant semantics: read implies at least read; edit implies read.
    let (cr, ce) = if can_edit {
        (true, true)
    } else {
        (can_read, false)
    };
    if !(cr || ce) {
        return Err(DbError::Conflict(
            "a category grant must carry at least one action".into(),
        ));
    }
    let scope = in_org_scope("sf", 1);
    let sql = "SELECT sf.id FROM secret_folders sf
               WHERE sf.id = $2 AND "
        .to_string()
        + &scope;
    sqlx::query(&sql)
        .bind(org_id)
        .bind(folder_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(DbError::NotFound)?;
    let r = sqlx::query(
        "INSERT INTO secret_category_grants
           (id, org_id, role_id, folder_id, can_read, can_edit, created_by, updated_by)
         VALUES ($2, $1, $3, $4, $5, $6, $7, $7)
         ON CONFLICT (role_id, folder_id)
         DO UPDATE SET can_read = $5, can_edit = $6, updated_by = $7, updated_at = now()
         RETURNING id, org_id, role_id, folder_id, can_read, can_edit, created_by, updated_by, created_at, updated_at",
    )
    .bind(org_id)
    .bind(Uuid::now_v7())
    .bind(role_id)
    .bind(folder_id)
    .bind(cr)
    .bind(ce)
    .bind(actor)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| maybe_conflict(e, "category grant upsert rejected"))?;
    Ok(grant_row(r))
}

pub async fn list_grants(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<SecretCategoryGrant>> {
    let sql = "SELECT id, org_id, role_id, folder_id, can_read, can_edit, created_by, updated_by, created_at, updated_at
               FROM secret_category_grants g
               WHERE g.org_id = $1
               ORDER BY g.folder_id, g.role_id";
    let rows = sqlx::query(sql).bind(org_id).fetch_all(&mut *conn).await?;
    Ok(rows.into_iter().map(grant_row).collect())
}

pub async fn delete_grant(
    conn: &mut PgConnection,
    org_id: Uuid,
    role_id: Uuid,
    folder_id: Uuid,
) -> DbResult<()> {
    let r = sqlx::query(
        "DELETE FROM secret_category_grants WHERE org_id = $1 AND role_id = $2 AND folder_id = $3",
    )
    .bind(org_id)
    .bind(role_id)
    .bind(folder_id)
    .execute(&mut *conn)
    .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// The CATEGORY-level RBAC check (the financial:read gate shape):
/// walks the ancestor chain of `folder_id` (the folder itself
/// included). Unrestricted chain ⇒ allowed (the matrix already gated).
/// Restricted ⇒ allowed only when one of `role_ids` holds a grant row
/// that permits the wanted action across the chain.
pub async fn folder_access(
    conn: &mut PgConnection,
    org_id: Uuid,
    role_ids: &[Uuid],
    folder_id: Uuid,
    need_edit: bool,
) -> DbResult<bool> {
    let scope = in_org_scope("sf", 1);
    let sql = "
        WITH RECURSIVE chain AS (
          SELECT sf.id, sf.parent_id, sf.restricted FROM secret_folders sf
          WHERE sf.id = $2 AND "
        .to_string()
        + &scope
        + "
          UNION ALL
          SELECT p.id, p.parent_id, p.restricted
          FROM secret_folders p JOIN chain c ON c.parent_id = p.id
        )
        SELECT CASE
          WHEN EXISTS (SELECT 1 FROM chain WHERE restricted) THEN
            EXISTS (
              SELECT 1 FROM secret_category_grants g
              WHERE g.org_id = $1
                AND g.folder_id IN (SELECT id FROM chain)
                AND g.role_id = ANY($3)
                AND (CASE WHEN $4::bool THEN g.can_edit ELSE g.can_read END)
            )
          ELSE TRUE
        END";
    let r = sqlx::query_scalar::<_, bool>(&sql)
        .bind(org_id)
        .bind(folder_id)
        .bind(role_ids.to_vec())
        .bind(need_edit)
        .fetch_one(&mut *conn)
        .await?;
    Ok(r)
}
