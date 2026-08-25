//! Org-scoped transaction helper: every mutation runs inside a transaction
//! bound to the caller's org (RLS) with its domain event in the same commit.

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use serde_json::Value;
use sqlx::Transaction;
use std::future::Future;
use uuid::Uuid;

pub struct OrgTx<'a> {
    pub tx: Transaction<'static, sqlx::Postgres>,
    pub org_id: Uuid,
    pub user_id: Uuid,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl OrgTx<'_> {
    /// Inserts a domain event into the outbox within this transaction.
    pub async fn emit(
        &mut self,
        event_type: &str,
        entity: &str,
        entity_id: Option<&str>,
        payload: &Value,
    ) -> ApiResult<()> {
        core_db::outbox::insert(
            &mut self.tx,
            Uuid::now_v7(),
            self.org_id,
            event_type,
            entity,
            entity_id,
            payload,
        )
        .await?;
        Ok(())
    }

    pub async fn commit(self) -> ApiResult<()> {
        self.tx.commit().await?;
        Ok(())
    }
}

/// Begins an org-bound transaction for the principal.
pub async fn begin(state: &AppState, org_id: Uuid, user_id: Uuid) -> ApiResult<OrgTx<'static>> {
    let mut tx = state.pool.begin().await.map_err(|e| ApiError::Internal(e.into()))?;
    // Bind THIS transaction to the org. `set_config(..., true)` is
    // transaction-local: it auto-resets on commit/rollback, so a pooled
    // connection can never leak org scope across requests.
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    Ok(OrgTx {
        tx,
        org_id,
        user_id,
        _marker: std::marker::PhantomData,
    })
}

/// Convenience wrapper: run `f` with an org-bound transaction and commit.
pub async fn with_org_tx<T, F, Fut>(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    f: F,
) -> ApiResult<T>
where
    F: FnOnce(OrgTx<'static>) -> Fut,
    Fut: Future<Output = ApiResult<(OrgTx<'static>, T)>>,
{
    let ctx = begin(state, org_id, user_id).await?;
    let (ctx, out) = f(ctx).await?;
    ctx.commit().await?;
    Ok(out)
}
