//! Reply-snippet persistence over the settings domain (Wave 34 lane 3).
//!
//! Purpose: each snippet is one org-scope settings row keyed
//! `reply_snippet:<id>` carrying the typed JSON value. The settings model
//! holds per-board scoping because scoping is a field on the snippet
//! (`board_ids`, empty = all boards) — no table, no migration. Reads go
//! through the org-scoped prefix list (RLS additionally constrains); RLS
//! for org scope applies as for every settings row.
//!
//! Non-purpose: validation and the search predicate — core-domain owns
//! both; this layer only moves typed values.

use crate::settings;
use crate::DbResult;
use core_domain::reply_snippets::{self, ReplySnippet};
use uuid::Uuid;

/// Inserts or updates one snippet row.
pub async fn upsert(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    snippet: &ReplySnippet,
) -> DbResult<()> {
    let key = format!("{}{}", reply_snippets::SNIPPET_KEY_PREFIX, snippet.id);
    let value = reply_snippets::to_settings_value(snippet)
        .map_err(|e| crate::DbError::Conflict(e.to_string()))?;
    settings::upsert_org_setting(tx, org_id, &key, &value, snippet.updated_by).await
}

/// Every snippet in the org's library, newest edit first.
pub async fn list(
    org: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
) -> DbResult<Vec<ReplySnippet>> {
    let rows =
        settings::list_org_values_with_prefix(org, org_id, reply_snippets::SNIPPET_KEY_PREFIX)
            .await?;
    let mut out: Vec<ReplySnippet> = rows
        .into_iter()
        .filter_map(|r| reply_snippets::from_settings_value(&r.key, &r.value))
        .collect();
    out.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
    Ok(out)
}

/// One snippet by id, if present.
pub async fn get(
    org: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<ReplySnippet> {
    let key = format!("{}{}", reply_snippets::SNIPPET_KEY_PREFIX, id);
    let rows = settings::list_org_values_with_prefix(org, org_id, &key).await?;
    rows.into_iter()
        .find_map(|r| reply_snippets::from_settings_value(&r.key, &r.value))
        .ok_or(crate::DbError::NotFound)
}

/// Removes one snippet row; absent rows stay absent (idempotent delete).
pub async fn delete(tx: &mut sqlx::PgConnection, org_id: Uuid, id: Uuid) -> DbResult<()> {
    let key = format!("{}{}", reply_snippets::SNIPPET_KEY_PREFIX, id);
    settings::delete_org_setting(tx, org_id, &key).await
}
