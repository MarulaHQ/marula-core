//! Org-data export endpoints (TASKLIST D1, portability/DR).
//!
//! Purpose: build the org's canonical [`ExportBundle`] — the U5
//! customization manifest generalized to the full logical data plane —
//! stream it into the ObjectStore under an org-scoped key, and hand back a
//! SHORT-LIVED SIGNED DOWNLOAD URL. The bundle itself is versioned and
//! deterministic (see core-db `export` / core-domain `entities::export`);
//! the URL is self-contained so DR scripts can `curl` it without a bearer
//! token, exactly like an object-store presigned link — but issued by the
//! API, because the local/memory store backends cannot presign.
//!
//! Signature scheme: hex HMAC-SHA256 over the message
//! `export-download:{org_id}:{export_id}:{expires}` under the at-rest
//! secret key, verified constant-time. The org id binds the token to THIS
//! org's bundle, so a valid-looking token for another org's export never
//! verifies.
//!
//! Gating: only owner/admin may CREATE an export (same bar as the U5
//! manifest — the bundle contains every ticket, comment, and custom-field
//! value in the org); the download endpoint authenticates purely by
//! signature + expiry.
//!
//! Non-purpose: no import/apply path, no scheduled exports, no partial
//! entity filters (an org's export is one canonical bundle — the same
//! determinism stance as the manifest), no user/credential material (the
//! loader structurally never reads the users table).

use crate::auth::middleware::authed;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::Extensions,
    response::Response,
    Json,
};
use core_domain::entities::export::ExportBundle;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

/// How long a signed download link stays valid. Long enough for a slow
/// DR script to start pulling; short enough that a leaked link expires on
/// its own without any revocation machinery.
const DOWNLOAD_TTL_SECS: i64 = 15 * 60;

/// The canonical ObjectStore key for an org's export bundle. Content is
/// identified by the export id, namespaced per org — keys stay opaque.
#[must_use]
fn export_key(org_id: Uuid, export_id: Uuid) -> String {
    format!("org/{org_id}/exports/{export_id}/bundle.json")
}

/// The exact signing message for a download token. Keep in ONE place so
/// sign and verify can never drift.
fn signing_message(org_id: Uuid, export_id: Uuid, expires: i64) -> Vec<u8> {
    format!("export-download:{org_id}:{export_id}:{expires}").into_bytes()
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExportCreatedResponse {
    #[schema(value_type = uuid::Uuid)]
    pub export_id: Uuid,
    /// Self-contained relative URL carrying the signature + expiry; the
    /// client prefixes its API origin. Works without a bearer token.
    pub download_url: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// Exact bundle byte size as stored.
    pub size_bytes: i64,
    /// sha256 of the stored bytes (download integrity check).
    pub sha256: String,
    /// Envelope version of the produced bundle (mirrors the body).
    pub bundle_version: u32,
}

/// POST /api/org/exports — builds the versioned bundle, streams it into
/// the ObjectStore, and returns the signed download link.
#[utoipa::path(operation_id = "org_export_create", post, path = "/api/org/exports", tag = "org",
    security(("bearer" = [])),
    responses((status = 200, body = ExportCreatedResponse)))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<ExportCreatedResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    if !authed.principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "org data export is limited to org admins".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let export_id = Uuid::now_v7();
    let generated_at = chrono::Utc::now();

    let bundle = crate::org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut ctx| async move {
            let bundle: ExportBundle =
                core_db::export::load_export_bundle(&mut ctx.tx, org_id, generated_at).await?;
            Ok((ctx, bundle))
        },
    )
    .await?;

    let body = serde_json::to_vec_pretty(&bundle)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("bundle serialize: {e}")))?;
    let sha256 = hex::encode(Sha256::digest(&body));
    let key = export_key(org_id, export_id);
    let meta = state
        .store
        .put(&key, "application/json", bytes::Bytes::from(body))
        .await
        .map_err(crate::routes::attachments::store_error)?;

    let expires = chrono::Utc::now().timestamp() + DOWNLOAD_TTL_SECS;
    let signature = core_mail::hmac::sign(
        &state.config.secret_key.0,
        &signing_message(org_id, export_id, expires),
    );
    let download_url = format!(
        "/api/v1/org/exports/{org_id}/{export_id}/download?expires={expires}&signature={signature}"
    );
    Ok(Json(ExportCreatedResponse {
        export_id,
        download_url,
        expires_at: chrono::DateTime::from_timestamp(expires, 0).unwrap_or_else(chrono::Utc::now),
        size_bytes: meta.size_bytes as i64,
        sha256,
        bundle_version: bundle.bundle_version,
    }))
}

/// Query parameters of the signed download link.
#[derive(Debug, Deserialize)]
pub struct DownloadQuery {
    pub expires: i64,
    pub signature: String,
}

/// GET /api/org/exports/{org_id}/{export_id}/download — streams the stored
/// bundle. Authenticated by the signed link alone (no JWT): an expired or
/// forged token is an unauthorized request, indistinguishable between the
/// two. The org id is IN the path (and inside the signature) so a token
/// authorizes exactly one (org, export, expiry) triple.
#[utoipa::path(operation_id = "org_export_download", get, path = "/api/org/exports/{org_id}/{export_id}/download", tag = "org",
    params(("org_id" = Uuid, Path), ("export_id" = Uuid, Path)),
    responses((status = 200, body = ExportBundle, content_type = "application/json")))]
pub async fn download(
    State(state): State<AppState>,
    Path((org_id, export_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<DownloadQuery>,
) -> ApiResult<Response> {
    // The org id is part of the PATH: a token can only ever authorize the
    // exact (org, export, expiry) triple it was signed for.
    if query.expires < chrono::Utc::now().timestamp() {
        return Err(ApiError::Unauthorized("export link expired".into()));
    }
    let ok = core_mail::hmac::verify(
        &state.config.secret_key.0,
        &signing_message(org_id, export_id, query.expires),
        &query.signature,
    );
    if !ok {
        return Err(ApiError::Unauthorized("invalid export signature".into()));
    }

    let obj = state
        .store
        .get(&export_key(org_id, export_id))
        .await
        .map_err(crate::routes::attachments::store_error)?;
    Response::builder()
        .status(axum::http::StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, obj.content_type)
        .header(
            axum::http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"export-{export_id}.json\""),
        )
        .header(axum::http::header::CONTENT_LENGTH, obj.size_bytes)
        .body(Body::from_stream(obj.stream))
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))
}
