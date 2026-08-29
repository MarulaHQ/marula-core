//! Instant-first write-path primitives (TASKLIST IF1–IF3, binding contract
//! in `docs/instant-first.md`).
//!
//! ONE shared layer that every autosave-style mutation routes through, so
//! endpoints added by later lanes converge to the contract for free:
//!
//! 1. [`claim`] — the idempotency gate (IF1). A create/update carrying an
//!    idempotency key claims its key row inside the SAME transaction that
//!    performs the write. Same key + same payload ⇒ [`InstantClaim::Replay`]
//!    with the byte-identical stored response (debounce/retry safe, never
//!    double-applies). Same key + different payload ⇒ typed 409
//!    `idempotency_key_reused`. Only a fully resolved entity persists: the
//!    claim, the entity write, the trail row, and the stored response all
//!    commit — or roll back — atomically.
//! 2. [`entity_seq`] — the If-Match-style version gate + monotonic seq
//!    allocation (IF1/IF2). When the caller presents a `version`, it must
//!    match the entity's current seq or the write is rejected with a typed
//!    409 `version_conflict` before anything changes; the allocated seq
//!    doubles as the next version clients echo back.
//! 3. [`record`] — the read-your-write substrate (IF2). Appends the
//!    write-trail row (prior value + new value, the undo substrate) and
//!    stores the exact response under the idempotency key, then returns the
//!    [`MutationMeta`] every mutation response carries.
//!
//! Usage pattern for a handler (see `routes/tickets.rs` for live examples):
//!
//! ```ignore
//! let fingerprint = serde_json::to_value(&req)?;
//! let key = req.instant.idempotency_key.clone();
//! let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
//!     if let InstantClaim::Replay(value) = instant::claim(&mut ctx, ClaimSpec {
//!         entity: "ticket", entity_id: Some(id), key: key.as_deref(),
//!         fingerprint: &fingerprint,
//!     }).await? {
//!         return Ok((ctx, MutationOutcome::Replayed(value)));
//!     }
//!     let before = load_entity(/* NotFound inside a mutation ⇒ RlsSparse */).await?;
//!     let seq = instant::entity_seq(&mut ctx, "ticket", id, req.instant.version).await?;
//!     // ... apply the write, emit events ...
//!     let meta = instant::record(&mut ctx, RecordSpec { /* before/after/payload */ }).await?;
//!     Ok((ctx, MutationOutcome::Applied(ResponseStruct { entity, mutation: meta })))
//! }).await?;
//! Ok(Json(outcome.into_response()?))
//! ```

use crate::error::{ApiError, ApiResult};
use crate::org_tx::OrgTx;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

/// The autosave mutation parameters every gated create/update body carries
/// (flattened into the request struct — purely additive fields).
#[derive(Debug, Clone, Default, serde::Deserialize, Serialize, ToSchema)]
pub struct MutationParams {
    /// Idempotency key: the same key with the same payload replays the
    /// SAME stored result (never double-applies); the same key with a
    /// different payload is a typed 409. Generate one per user edit
    /// debounce/retry episode (UUID works).
    #[serde(default)]
    pub idempotency_key: Option<String>,
    /// If-Match-style version: the entity `version` the caller last saw
    /// (the `mutation.version` of the previous response). A mismatch
    /// rejects the write with a typed 409 `version_conflict` carrying
    /// `expected`/`provided`; omit for creates and for legacy callers.
    #[serde(default)]
    pub version: Option<i64>,
}

/// What the idempotency gate decided.
#[derive(Debug)]
pub enum InstantClaim {
    /// No key, or this request just claimed the key row: proceed with the
    /// write inside this transaction.
    Fresh,
    /// The key was already committed with an identical payload: replay the
    /// stored response verbatim and touch nothing.
    Replay(Value),
}

/// What a gated mutation body produced: the freshly applied typed response,
/// or the byte-identical replay of an earlier identical request.
#[derive(Debug)]
pub enum MutationOutcome<T> {
    Applied(T),
    Replayed(Value),
}

impl<T: DeserializeOwned> MutationOutcome<T> {
    /// Collapses the outcome into the typed response the handler returns.
    /// A replay deserializes into the same concrete shape it was stored as,
    /// so the client receives the identical result on every attempt.
    pub fn into_response(self) -> ApiResult<T> {
        match self {
            MutationOutcome::Applied(t) => Ok(t),
            MutationOutcome::Replayed(value) => serde_json::from_value(value)
                .map_err(|e| ApiError::Internal(anyhow::anyhow!("stored replay mismatch: {e}"))),
        }
    }
}

/// Input to [`claim`].
#[derive(Debug)]
pub struct ClaimSpec<'a> {
    /// Entity family the write targets (e.g. `"ticket"`). A key claimed
    /// against one family cannot replay on another.
    pub entity: &'a str,
    /// Known target id; `None` for creates (the id exists only after the
    /// insert) and for flows that resolve it mid-transaction.
    pub entity_id: Option<Uuid>,
    pub key: Option<&'a str>,
    /// The canonical request body; hashed into the replay fingerprint.
    pub fingerprint: &'a Value,
}

/// Input to [`entity_seq`].
#[derive(Debug)]
pub struct SeqSpec<'a> {
    pub entity: &'a str,
    pub entity_id: Uuid,
    /// The caller-presented version to enforce, when any.
    pub version: Option<i64>,
}

/// Input to [`record`].
#[derive(Debug)]
pub struct RecordSpec<'a, T: Serialize> {
    pub entity: &'a str,
    pub entity_id: Uuid,
    /// Machine-readable operation name for the trail (`create`, `update`,
    /// `status_change`, `comment_add`, `timer_start`, `timer_stop`, ...).
    pub op: &'a str,
    /// The seq allocated by [`entity_seq`].
    pub seq: i64,
    /// Prior entity snapshot; `None` for creates.
    pub before: Option<Value>,
    /// Applied entity snapshot.
    pub after: Value,
    /// The entity's own `updated_at` (or creation time) surfaced in the
    /// mutation metadata.
    pub updated_at: DateTime<Utc>,
    /// The idempotency key this request claimed, if any.
    pub key: Option<&'a str>,
    /// The typed response body (entity fields at the top level). Stored
    /// under the key with the mutation metadata merged in so replays are
    /// byte-identical.
    pub payload: &'a T,
}

/// Read-your-write metadata every mutation response carries (IF2), merged
/// as the additive `mutation` object alongside the entity fields.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MutationMeta {
    /// Monotonic per-entity sequence: strictly increases by one per
    /// applied write, so clients order reconciliations against the event
    /// stream.
    pub seq: i64,
    /// Version token for If-Match-style writes; mirrors `seq`. Echo it in
    /// the next write's `version` field.
    pub version: i64,
    /// The applied entity's `updated_at`.
    #[schema(value_type = String)]
    pub updated_at: DateTime<Utc>,
    /// Prior entity snapshot (the undo substrate). `null` on creates.
    #[schema(value_type = Object, nullable = true)]
    pub before: Option<Value>,
    /// Applied entity snapshot.
    #[schema(value_type = Object)]
    pub after: Value,
    /// The idempotency key that claimed this result, when the caller
    /// supplied one.
    #[schema(nullable = true)]
    pub idempotency_key: Option<String>,
}

fn request_hash(entity: &str, fingerprint: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(entity.as_bytes());
    hasher.update([0]);
    hasher.update(
        serde_json::to_string(fingerprint)
            .unwrap_or_default()
            .as_bytes(),
    );
    hex::encode(hasher.finalize())
}

fn validate_key(key: &str) -> ApiResult<()> {
    if key.is_empty() {
        return Err(ApiError::Validation(
            "idempotency_key must not be empty".into(),
        ));
    }
    if key.len() > 200 {
        return Err(ApiError::Validation(
            "idempotency_key must be at most 200 characters".into(),
        ));
    }
    Ok(())
}

/// Maps a read miss inside a MUTATION to the typed `rls_sparse` envelope
/// code: under row-level security the target is invisible whether it was
/// deleted, moved out of scope, or never existed — mutations surface that
/// as `rls_sparse` (reads keep `not_found`).
pub(crate) fn mutation_scope_err(e: ApiError) -> ApiError {
    match e {
        ApiError::NotFound => ApiError::RlsSparse,
        other => other,
    }
}

/// The idempotency gate. Must be the FIRST statement sequence of the
/// mutation transaction: a replay returns before any state changes.
pub async fn claim(ctx: &mut OrgTx<'_>, spec: ClaimSpec<'_>) -> ApiResult<InstantClaim> {
    let Some(key) = spec.key else {
        return Ok(InstantClaim::Fresh);
    };
    validate_key(key)?;
    let hash = request_hash(spec.entity, spec.fingerprint);
    let claimed: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO idempotency_keys (id, org_id, key, entity, entity_id, request_hash, response, actor_id) \
         VALUES ($1, $2, $3, $4, $5, $6, NULL, $7) \
         ON CONFLICT (org_id, key) DO NOTHING RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(ctx.org_id)
    .bind(key)
    .bind(spec.entity)
    .bind(spec.entity_id)
    .bind(&hash)
    .bind(ctx.user_id)
    .fetch_optional(&mut *ctx.tx)
    .await?;
    if claimed.is_some() {
        return Ok(InstantClaim::Fresh);
    }

    // The key row exists: wait on its row lock so an in-flight first
    // request finishes committing, then judge by its stored fingerprint.
    let existing: Option<(String, String, Option<Value>)> = sqlx::query_as(
        "SELECT entity, request_hash, response FROM idempotency_keys \
         WHERE org_id = $1 AND key = $2 FOR UPDATE",
    )
    .bind(ctx.org_id)
    .bind(key)
    .fetch_optional(&mut *ctx.tx)
    .await?;
    match existing {
        Some((entity, request_hash, response)) => {
            if entity != spec.entity || request_hash != hash {
                return Err(ApiError::IdempotencyKeyReused);
            }
            match response {
                Some(stored) => Ok(InstantClaim::Replay(stored)),
                // Unreachable through the API (claim + stored response
                // commit atomically), but fail closed rather than writing.
                None => Err(ApiError::Conflict(
                    "idempotency claim has no stored response".into(),
                )),
            }
        }
        // The original claim rolled back; the key is free again — retry
        // once inside this transaction.
        None => {
            let reclaimed: Option<Uuid> = sqlx::query_scalar(
                "INSERT INTO idempotency_keys (id, org_id, key, entity, entity_id, request_hash, response, actor_id) \
                 VALUES ($1, $2, $3, $4, $5, $6, NULL, $7) \
                 ON CONFLICT (org_id, key) DO NOTHING RETURNING id",
            )
            .bind(Uuid::now_v7())
            .bind(ctx.org_id)
            .bind(key)
            .bind(spec.entity)
            .bind(spec.entity_id)
            .bind(&hash)
            .bind(ctx.user_id)
            .fetch_optional(&mut *ctx.tx)
            .await?;
            if reclaimed.is_some() {
                Ok(InstantClaim::Fresh)
            } else {
                Err(ApiError::Conflict(
                    "idempotency key contention; retry with a fresh key".into(),
                ))
            }
        }
    }
}

/// Enforces the If-Match-style version gate and allocates the entity's
/// next monotonic seq. Call once per write, at the point the entity id is
/// known; the row lock on the counter serializes concurrent writers to the
/// same entity only.
pub async fn entity_seq(ctx: &mut OrgTx<'_>, spec: SeqSpec<'_>) -> ApiResult<i64> {
    for _ in 0..3 {
        let current: Option<i64> = sqlx::query_scalar(
            "SELECT seq FROM entity_write_seqs \
             WHERE org_id = $1 AND entity = $2 AND entity_id = $3 FOR UPDATE",
        )
        .bind(ctx.org_id)
        .bind(spec.entity)
        .bind(spec.entity_id)
        .fetch_optional(&mut *ctx.tx)
        .await?;
        if let Some(cur) = current {
            if let Some(provided) = spec.version {
                if provided != cur {
                    return Err(ApiError::VersionConflict {
                        expected: cur,
                        provided,
                    });
                }
            }
            let next = cur + 1;
            sqlx::query(
                "UPDATE entity_write_seqs SET seq = $4, updated_at = now() \
                 WHERE org_id = $1 AND entity = $2 AND entity_id = $3",
            )
            .bind(ctx.org_id)
            .bind(spec.entity)
            .bind(spec.entity_id)
            .bind(next)
            .execute(&mut *ctx.tx)
            .await?;
            return Ok(next);
        }
        // Fresh entity: there is nothing to match; a presented version
        // other than 0 cannot ever have been observed.
        if let Some(provided) = spec.version {
            if provided != 0 {
                return Err(ApiError::VersionConflict {
                    expected: 0,
                    provided,
                });
            }
        }
        let inserted = sqlx::query(
            "INSERT INTO entity_write_seqs (org_id, entity, entity_id, seq) \
             VALUES ($1, $2, $3, 1)",
        )
        .bind(ctx.org_id)
        .bind(spec.entity)
        .bind(spec.entity_id)
        .execute(&mut *ctx.tx)
        .await;
        match inserted {
            Ok(_) => return Ok(1),
            // A concurrent writer created the counter row between our
            // locked read and insert; retry on the locked path.
            Err(sqlx::Error::Database(db)) if db.code().as_deref() == Some("23505") => continue,
            Err(e) => return Err(ApiError::from(e)),
        }
    }
    Err(ApiError::Conflict("entity write contention; retry".into()))
}

/// Appends the write-trail row and stores the exact response under the
/// idempotency key, then returns the metadata the handler merges into its
/// response. Runs last, inside the same transaction as the write.
pub async fn record<T: Serialize>(
    ctx: &mut OrgTx<'_>,
    spec: RecordSpec<'_, T>,
) -> ApiResult<MutationMeta> {
    sqlx::query(
        "INSERT INTO entity_writes (id, org_id, entity, entity_id, seq, op, actor_id, actor_kind, idempotency_key, before, after) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'human', $8, $9, $10)",
    )
    .bind(Uuid::now_v7())
    .bind(ctx.org_id)
    .bind(spec.entity)
    .bind(spec.entity_id)
    .bind(spec.seq)
    .bind(spec.op)
    .bind(ctx.user_id)
    .bind(spec.key)
    .bind(spec.before.clone())
    .bind(&spec.after)
    .execute(&mut *ctx.tx)
    .await?;

    let meta = MutationMeta {
        seq: spec.seq,
        version: spec.seq,
        updated_at: spec.updated_at,
        before: spec.before.clone(),
        after: spec.after.clone(),
        idempotency_key: spec.key.map(str::to_string),
    };

    if let Some(key) = spec.key {
        // Persist the exact response so a replay with the same key and
        // payload returns byte-identical output. The claim row already
        // exists (this transaction created it).
        let mut wrapped = serde_json::to_value(spec.payload)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("response serialize: {e}")))?;
        if let Some(map) = wrapped.as_object_mut() {
            map.insert(
                "mutation".to_string(),
                serde_json::to_value(&meta)
                    .map_err(|e| ApiError::Internal(anyhow::anyhow!("meta serialize: {e}")))?,
            );
        }
        let updated = sqlx::query(
            "UPDATE idempotency_keys SET response = $3, entity_id = COALESCE(entity_id, $2) \
             WHERE org_id = $1 AND key = $4",
        )
        .bind(ctx.org_id)
        .bind(spec.entity_id)
        .bind(&wrapped)
        .bind(key)
        .execute(&mut *ctx.tx)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(ApiError::Conflict(
                "idempotency claim row vanished; retry".into(),
            ));
        }
    }

    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::MutationParams;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct Probe {
        #[allow(dead_code)]
        pub title: String,
        #[serde(flatten)]
        pub instant: MutationParams,
    }

    #[derive(Debug, Deserialize)]
    struct ProbeEmbedded {
        #[allow(dead_code)]
        pub title: String,
        #[serde(default)]
        pub instant: MutationParams,
    }

    #[test]
    fn flattened_params_capture_top_level_keys() {
        let req: Probe = serde_json::from_value(serde_json::json!({
            "title": "x",
            "idempotency_key": "KEY",
            "version": 3,
        }))
        .expect("deserialize");
        assert_eq!(req.instant.idempotency_key.as_deref(), Some("KEY"));
        assert_eq!(req.instant.version, Some(3));

        // WITHOUT flatten (embedded object): must also work.
        let emb: ProbeEmbedded = serde_json::from_value(serde_json::json!({
            "title": "x",
            "instant": { "idempotency_key": "KEY2" },
        }))
        .expect("deserialize");
        assert_eq!(emb.instant.idempotency_key.as_deref(), Some("KEY2"));
    }
}
