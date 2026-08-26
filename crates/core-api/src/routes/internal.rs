//! Internal machine-to-machine effect surface (§4.1): the ONLY writer of
//! the search index. The worker's event-driven indexer calls these routes;
//! core-api performs the writes inside org-scoped transactions so RLS
//! applies exactly as it does for every other mutation.
//!
//! Auth: `X-Internal-Token` compared in constant time against
//! `MERIDIAN_INTERNAL_TOKEN`. Unset token ⇒ 503 fail-closed; wrong token ⇒
//! 401.
//!
//! Non-purpose: no user-facing endpoints and NO OpenAPI entries — this is
//! a machine-to-machine surface that must never leak into the public spec
//! or the generated SDK.

use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{extract::State, http::HeaderMap, Json};
use core_db::search::SearchKind;
use serde::Deserialize;
use uuid::Uuid;

/// Constant-time equality over raw bytes; length differences still leak
/// only through the early return (lengths are not secret).
fn token_matches(expected: &str, provided: &str) -> bool {
    use subtle::ConstantTimeEq;
    let a = expected.as_bytes();
    let b = provided.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

/// Extracts and verifies the internal token; disabled when unset.
fn guard(state: &AppState, headers: &HeaderMap) -> ApiResult<()> {
    let Some(expected) = state.config.internal_token.as_deref() else {
        return Err(ApiError::Unavailable(
            "internal effect surface disabled: MERIDIAN_INTERNAL_TOKEN not configured".into(),
        ));
    };
    let provided = headers
        .get("x-internal-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !token_matches(expected, provided) {
        return Err(ApiError::Unauthorized("invalid internal token".into()));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct UpsertEntry {
    pub entity_kind: String,
    pub entity_id: Uuid,
    pub parent_id: Option<Uuid>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteEntry {
    pub entity_kind: String,
    pub entity_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct IndexRequest<E> {
    /// Tenant the rows belong to; bound into the RLS transaction below.
    pub org_id: Uuid,
    pub entries: Vec<E>,
}

#[derive(Debug, serde::Serialize)]
pub struct IndexResult {
    pub applied: usize,
}

/// POST /internal/search/upsert — idempotent row upserts (replays safe).
pub async fn upsert(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<IndexRequest<UpsertEntry>>,
) -> ApiResult<Json<IndexResult>> {
    guard(&state, &headers)?;
    if req.entries.len() > 500 {
        return Err(ApiError::Validation("too many entries".into()));
    }
    let mut kinds = Vec::with_capacity(req.entries.len());
    for entry in &req.entries {
        kinds.push(
            SearchKind::parse(&entry.entity_kind)
                .ok_or_else(|| ApiError::Validation("unknown entity_kind".into()))?,
        );
    }
    // The caller is a process, not a user; the org binding is what matters
    // for RLS, so org_id doubles as the principal id here.
    let applied = org_tx::with_org_tx(&state, req.org_id, req.org_id, |mut ctx| async move {
        let mut applied = 0_usize;
        for (entry, kind) in req.entries.iter().zip(kinds) {
            core_db::search::upsert_row(
                &mut *ctx.tx,
                ctx.org_id,
                kind,
                entry.entity_id,
                entry.parent_id,
                &entry.title,
                &entry.body,
            )
            .await?;
            applied += 1;
        }
        Ok((ctx, applied))
    })
    .await?;
    Ok(Json(IndexResult { applied }))
}

/// POST /internal/search/delete — removals when source entities vanish.
pub async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<IndexRequest<DeleteEntry>>,
) -> ApiResult<Json<IndexResult>> {
    guard(&state, &headers)?;
    if req.entries.len() > 500 {
        return Err(ApiError::Validation("too many entries".into()));
    }
    let mut kinds = Vec::with_capacity(req.entries.len());
    for entry in &req.entries {
        kinds.push(
            SearchKind::parse(&entry.entity_kind)
                .ok_or_else(|| ApiError::Validation("unknown entity_kind".into()))?,
        );
    }
    let applied = org_tx::with_org_tx(&state, req.org_id, req.org_id, |mut ctx| async move {
        let mut applied = 0_usize;
        for (entry, kind) in req.entries.iter().zip(kinds) {
            core_db::search::delete_row(&mut *ctx.tx, ctx.org_id, kind, entry.entity_id).await?;
            applied += 1;
        }
        Ok((ctx, applied))
    })
    .await?;
    Ok(Json(IndexResult { applied }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_comparison_is_exact_and_length_safe() {
        assert!(token_matches("secret", "secret"));
        assert!(!token_matches("secret", "secreT"));
        assert!(!token_matches("secret", "secrets"));
        assert!(!token_matches("secret", ""));
        assert!(token_matches("", ""));
    }
}
