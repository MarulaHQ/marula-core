//! Core-side entitlement artifact adoption (RFC 0023).
//!
//! Purpose: the one operation Core publishes for entitlements — adopt a
//! SIGNED, revision-stamped artifact and report the revision held.
//!
//! Why this shape. HQ cannot push entitlements into Core: a live push endpoint
//! would be commerce-shaped, every self-hosted install would carry it with no
//! caller, and it would make Core a second source of truth for what a tenant
//! bought. Instead HQ mints one signed artifact and the operator presents it
//! here. Core verifies it against the same signing trust set it already uses
//! for plugin artifacts, stores it verbatim, and answers with the revision.
//! An air-gapped install works, because verification needs no network.
//!
//! What Core does NOT do with it. It does not read the payload. The artifact
//! describes a commercial fact that belongs to whoever issued it; core verifies
//! that the fact is authentic and reports it back. No feature in core gates on
//! this row — that is the entitlement-gating law, and this surface deliberately
//! gives it nothing to gate on.
//!
//! Refusals are typed and happen before the write:
//! * a signature that does not verify against the trust set (an empty trust set
//!   fails closed rather than degrading to "unverified"),
//! * an artifact minted for a different org (an artifact for tenant B must never
//!   be adoptable by tenant A, even with a valid signature),
//! * a payload that does not match the digest the signer stamped,
//! * a stale revision (strictly increasing per org; a replay of the revision
//!   already held with identical content is answered `unchanged`, not applied
//!   twice, and the same revision with different content is a conflict).
//!
//! Non-purpose: no push, no entitlement interpretation, no gating, no pricing.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use chrono::{DateTime, Utc};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::{self, types};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// The only artifact schema this build accepts. An artifact naming another
/// schema is refused rather than guessed at: a document the verifier cannot
/// read is not one it can adopt.
pub const ENTITLEMENT_ARTIFACT_SCHEMA: &str = "marula.entitlement/v1";

/// Safety cap on the artifact document. A real bundle is a few KiB; anything
/// larger is a malformed write, not an entitlement.
const MAX_ARTIFACT_BYTES: usize = 64 * 1024;

fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

/// Why an artifact was refused. The code is stable and appears in the error
/// body; the prose is for the operator holding the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactRefusal {
    /// Not an object, or a field of the wrong type.
    Malformed,
    /// A schema this build does not verify.
    UnsupportedSchema,
    /// Larger than [`MAX_ARTIFACT_BYTES`].
    TooLarge,
    /// Minted for a different org than the one in the path.
    OrgMismatch,
    /// The payload does not hash to the digest the signer stamped.
    PayloadDigestMismatch,
    /// A revision that is missing, non-integral or not positive.
    InvalidRevision,
    /// No signing keys are trusted, so nothing can be verified (fails closed).
    NoTrustedKeys,
    /// The signature does not verify against any trusted key.
    BadSignature,
}

impl ArtifactRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            ArtifactRefusal::Malformed => "malformed_artifact",
            ArtifactRefusal::UnsupportedSchema => "unsupported_schema",
            ArtifactRefusal::TooLarge => "artifact_too_large",
            ArtifactRefusal::OrgMismatch => "org_mismatch",
            ArtifactRefusal::PayloadDigestMismatch => "payload_digest_mismatch",
            ArtifactRefusal::InvalidRevision => "invalid_revision",
            ArtifactRefusal::NoTrustedKeys => "no_trusted_keys",
            ArtifactRefusal::BadSignature => "bad_signature",
        }
    }
}

/// An artifact that verified. Every field is read from the *verified* document,
/// not from the request envelope, so the response cannot report a value the
/// signature did not cover.
#[derive(Debug, Clone, PartialEq)]
pub struct VerifiedArtifact {
    pub org_id: Uuid,
    pub revision: i64,
    pub issued_at: Option<DateTime<Utc>>,
    pub payload: serde_json::Value,
    /// sha256 of the canonical payload (the digest inside the signed document).
    pub payload_sha256: String,
    /// sha256 of the canonical artifact object — what the signature covers.
    pub artifact_sha256: String,
    /// The trust identity whose public key verified.
    pub key_id: String,
}

/// The signature block. `key_id` is advisory — it says which identity the
/// issuer believes signed — while verification runs against the whole trust
/// set. `key_id` in the response is the key that actually verified.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EntitlementSignature {
    /// The trust identity the issuer says signed it. Advisory only.
    pub key_id: Option<String>,
    /// 128 hex characters: ed25519 over sha256(canonical artifact).
    pub sig: String,
}

/// The adoption request: the signed document, verbatim, plus its signature.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdoptEntitlementRequest {
    /// The artifact object the signature covers. Stored verbatim once verified.
    #[schema(value_type = Object)]
    pub artifact: serde_json::Value,
    pub signature: EntitlementSignature,
}

/// Verifies an artifact and, on success, returns what may be stored.
///
/// Pure by design: it takes the trust set as a parameter, touches no database,
/// and is therefore the unit-testable core of the surface. Order matters — the
/// cheap structural checks run before hashing, and the org binding is checked
/// before the signature so a valid signature over another tenant's artifact
/// cannot reach the write path.
pub fn verify_artifact(
    artifact: &serde_json::Value,
    signature: &EntitlementSignature,
    trusted: &[plugin_host::distribution::TrustedSigningKey],
    expected_org: Uuid,
) -> Result<VerifiedArtifact, ArtifactRefusal> {
    let bytes = serde_json::to_vec(artifact).map_err(|_| ArtifactRefusal::Malformed)?;
    if bytes.len() > MAX_ARTIFACT_BYTES {
        return Err(ArtifactRefusal::TooLarge);
    }
    let object = artifact.as_object().ok_or(ArtifactRefusal::Malformed)?;

    let schema = object
        .get("schema")
        .and_then(|v| v.as_str())
        .ok_or(ArtifactRefusal::Malformed)?;
    if schema != ENTITLEMENT_ARTIFACT_SCHEMA {
        return Err(ArtifactRefusal::UnsupportedSchema);
    }

    let artifact_org = object
        .get("org_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or(ArtifactRefusal::Malformed)?;
    if artifact_org != expected_org {
        return Err(ArtifactRefusal::OrgMismatch);
    }

    let revision = object
        .get("revision")
        .and_then(|v| v.as_i64())
        .filter(|revision| *revision > 0)
        .ok_or(ArtifactRefusal::InvalidRevision)?;

    let payload = object
        .get("payload")
        .filter(|v| v.is_object())
        .cloned()
        .ok_or(ArtifactRefusal::Malformed)?;
    // Canonical JSON refuses fractional numbers, so a payload that cannot be
    // reproduced by the signer's canonicaliser is malformed rather than hashed
    // into a digest only this server could produce.
    let payload_sha256 = plugin_host::distribution::manifest_sha256(&payload)
        .map_err(|_| ArtifactRefusal::Malformed)?;
    let claimed = object
        .get("payload_sha256")
        .and_then(|v| v.as_str())
        .ok_or(ArtifactRefusal::Malformed)?;
    if !claimed.eq_ignore_ascii_case(&payload_sha256) {
        return Err(ArtifactRefusal::PayloadDigestMismatch);
    }

    let issued_at = match object.get("issued_at") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            value
                .as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.with_timezone(&Utc))
                .ok_or(ArtifactRefusal::Malformed)?,
        ),
    };

    let artifact_sha256 = plugin_host::distribution::manifest_sha256(artifact)
        .map_err(|_| ArtifactRefusal::Malformed)?;

    let key = verify_against_trust_set(
        trusted,
        &artifact_sha256,
        &signature.sig,
        signature.key_id.as_deref(),
    )?;

    Ok(VerifiedArtifact {
        org_id: artifact_org,
        revision,
        issued_at,
        payload,
        payload_sha256,
        artifact_sha256,
        key_id: key.key_id.clone(),
    })
}

/// Tries the trusted keys and names the one that verified.
///
/// The identity the issuer names is tried FIRST, and only orders the
/// candidates: it is never trusted. An operator rotating keys wants the
/// answered identity to be the one the issuer said, not whichever row the
/// registry happens to return first, and a claim that does not verify simply
/// falls through to the rest of the set.
///
/// An empty trust set is [`ArtifactRefusal::NoTrustedKeys`], never a pass: an
/// install that has configured no key is an install that cannot verify, and
/// accepting the artifact anyway would make the signature decorative.
fn verify_against_trust_set<'a>(
    trusted: &'a [plugin_host::distribution::TrustedSigningKey],
    artifact_sha256: &str,
    signature_hex: &str,
    claimed_key_id: Option<&str>,
) -> Result<&'a plugin_host::distribution::TrustedSigningKey, ArtifactRefusal> {
    if trusted.is_empty() {
        return Err(ArtifactRefusal::NoTrustedKeys);
    }
    let verifies = |key: &plugin_host::distribution::TrustedSigningKey| {
        plugin_host::distribution::verify_artifact_signature(
            &key.public_key,
            artifact_sha256,
            signature_hex,
        )
        .is_ok()
    };
    if let Some(claimed) = claimed_key_id {
        if let Some(key) = trusted.iter().find(|key| key.key_id == claimed) {
            if verifies(key) {
                return Ok(key);
            }
        }
    }
    trusted
        .iter()
        .find(|key| verifies(key))
        .ok_or(ArtifactRefusal::BadSignature)
}

fn artifact_refusal(refusal: ArtifactRefusal) -> ApiError {
    let message = format!("entitlement artifact refused ({})", refusal.as_str());
    match refusal {
        // Nothing to read is a client-shaped problem.
        ArtifactRefusal::Malformed
        | ArtifactRefusal::UnsupportedSchema
        | ArtifactRefusal::TooLarge
        | ArtifactRefusal::OrgMismatch
        | ArtifactRefusal::PayloadDigestMismatch
        | ArtifactRefusal::InvalidRevision
        | ArtifactRefusal::BadSignature => ApiError::Validation(message),
        // A trust set with no keys is a state conflict the operator must fix
        // before any artifact can be adopted, not a malformed document.
        ArtifactRefusal::NoTrustedKeys => ApiError::Conflict(message),
    }
}

/// What an adoption did. `Unchanged` means the artifact presented is the one
/// already held (same revision, same payload digest): a retry that must not be
/// recorded as a second adoption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntitlementOutcome {
    Adopted,
    Unchanged,
}

/// The artifact an org holds.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EntitlementHeld {
    /// The issuing side's monotonic revision.
    pub revision: i64,
    /// The signed payload, verbatim. Core does not interpret it.
    #[schema(value_type = Object)]
    pub payload: serde_json::Value,
    pub payload_sha256: String,
    /// The trust identity that verified the artifact when it was adopted.
    pub key_id: Option<String>,
    #[schema(value_type = Option<String>, format = "date-time")]
    pub issued_at: Option<DateTime<Utc>>,
    #[schema(value_type = String, format = "date-time")]
    pub adopted_at: DateTime<Utc>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub adopted_by: Option<Uuid>,
}

/// The read answer. `entitlement` is null when the org never adopted one,
/// which is different from "adopted zero" — there is no zero revision.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EntitlementView {
    pub org_id: Uuid,
    pub entitlement: Option<EntitlementHeld>,
}

/// The write answer: what the org now holds, and whether this call changed it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AdoptEntitlementView {
    pub org_id: Uuid,
    pub outcome: EntitlementOutcome,
    pub entitlement: EntitlementHeld,
}

fn held_from(row: core_db::entitlements::EntitlementRow) -> EntitlementHeld {
    EntitlementHeld {
        revision: row.revision,
        payload: row.payload,
        payload_sha256: row.payload_sha256,
        key_id: row.key_id,
        issued_at: row.issued_at,
        adopted_at: row.adopted_at,
        adopted_by: row.adopted_by,
    }
}

/// Whether the held row and the verified artifact are the same fact. Used only
/// after the monotonic guard refused, to answer a retry `unchanged` instead of
/// failing it.
fn is_same_fact(held: &core_db::entitlements::EntitlementRow, verified: &VerifiedArtifact) -> bool {
    held.revision == verified.revision && held.payload_sha256 == verified.payload_sha256
}

fn authorize_org(authed: &crate::auth::AuthedUser, org_id: Uuid, action: Action) -> ApiResult<()> {
    require_permission(&authed.principal, admin(action))?;
    if authed.principal.user.org_id != org_id {
        // The three reasons an org is not visible to the caller (deleted,
        // someone else's, never existed) stay indistinguishable.
        return Err(ApiError::NotFound);
    }
    Ok(())
}

/// GET /api/v1/orgs/{org_id}/entitlements — the artifact the org holds.
///
/// Reads require the Admin:read grant: the payload is the org's commercial
/// facts, and every member does not need to know what was bought.
#[utoipa::path(operation_id = "org_entitlement_get", get,
    path = "/api/orgs/{org_id}/entitlements", tag = "org", security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "the org to report on; must be the caller's own org")),
    responses((status = 200, body = EntitlementView),
              (status = 401, description = "not authenticated"),
              (status = 403, description = "caller lacks the Admin:read grant"),
              (status = 404, description = "no such org for this caller")))]
pub async fn get_entitlement(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(org_id): Path<Uuid>,
) -> ApiResult<Json<EntitlementView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    authorize_org(&authed, org_id, Action::View)?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let row = core_db::entitlements::get(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(EntitlementView {
        org_id,
        entitlement: row.map(held_from),
    }))
}

/// PUT /api/v1/orgs/{org_id}/entitlements — adopt a signed artifact.
///
/// Verifies against the signing trust set, refuses a stale revision and an
/// artifact minted for another org, stores the verified document, and answers
/// with the revision now held so the issuer can mark its snapshot acked.
#[utoipa::path(operation_id = "org_entitlement_put", put,
    path = "/api/orgs/{org_id}/entitlements", tag = "org", security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "the org adopting the artifact; must be the caller's own org")),
    request_body = AdoptEntitlementRequest,
    responses((status = 200, body = AdoptEntitlementView),
              (status = 401, description = "not authenticated"),
              (status = 403, description = "caller lacks the Admin:edit grant"),
              (status = 404, description = "no such org for this caller"),
              (status = 409, description = "stale revision, or no signing key is trusted"),
              (status = 422, description = "the artifact or its signature did not verify")))]
pub async fn adopt_entitlement(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(org_id): Path<Uuid>,
    Json(req): Json<AdoptEntitlementRequest>,
) -> ApiResult<Json<AdoptEntitlementView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    authorize_org(&authed, org_id, Action::Edit)?;
    let user_id = authed.principal.user.id;
    let state_ref: &AppState = &state;

    let (outcome, held) = crate::org_tx::with_org_tx(state_ref, org_id, user_id, |mut ctx| {
        async move {
            let trusted =
                super::plugin_distribution::trusted_signing_keys(state_ref, &mut ctx.tx).await;
            let verified = verify_artifact(&req.artifact, &req.signature, &trusted, org_id)
                .map_err(artifact_refusal)?;

            let candidate = core_db::entitlements::AdoptCandidate {
                revision: verified.revision,
                payload: &verified.payload,
                payload_sha256: &verified.payload_sha256,
                artifact: &req.artifact,
                artifact_sha256: &verified.artifact_sha256,
                key_id: Some(verified.key_id.as_str()),
                issued_at: verified.issued_at,
                adopted_by: user_id,
            };

            if let Some(row) = core_db::entitlements::adopt(&mut ctx.tx, org_id, &candidate).await?
            {
                ctx.emit(
                    types::ENTITLEMENT_ADOPTED,
                    "org",
                    Some(org_id.to_string().as_str()),
                    &serde_json::to_value(events::EntitlementAdopted {
                        revision: row.revision,
                        payload_sha256: row.payload_sha256.clone(),
                        key_id: row.key_id.clone(),
                    })
                    .unwrap_or_default(),
                )
                .await?;
                core_db::audit::insert(
                    &mut *ctx.tx,
                    &AuditEvent {
                        id: Uuid::now_v7(),
                        org_id,
                        actor: Actor::Human(user_id),
                        action: "org.entitlement_adopted".to_string(),
                        entity_type: Some("org".to_string()),
                        entity_id: Some(org_id.to_string()),
                        detail: serde_json::json!({
                            "revision": row.revision,
                            "payload_sha256": row.payload_sha256,
                            "key_id": row.key_id,
                            "artifact_sha256": row.artifact_sha256,
                        }),
                        ip: None,
                        occurred_at: Utc::now(),
                    },
                )
                .await?;
                return Ok((ctx, (EntitlementOutcome::Adopted, row)));
            }

            // The guard refused: a row exists at the same or a higher revision.
            let existing = core_db::entitlements::get(&mut *ctx.tx, org_id)
                .await?
                .ok_or_else(|| {
                    // Unreachable: the guard only refuses by finding a row.
                    // Treated as a conflict rather than a 500 because the
                    // honest answer is "something is held, read it again".
                    ApiError::Conflict("an entitlement is held but could not be read".into())
                })?;
            if is_same_fact(&existing, &verified) {
                return Ok((ctx, (EntitlementOutcome::Unchanged, existing)));
            }
            Err(ApiError::Conflict(format!(
                "stale entitlement revision {}: the org already holds revision {}",
                verified.revision, existing.revision
            )))
        }
    })
    .await?;

    Ok(Json(AdoptEntitlementView {
        org_id,
        outcome,
        entitlement: held_from(held),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer as _;

    const ORG: Uuid = Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0);

    fn signing_key(seed: u8) -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&[seed; 32])
    }

    fn trusted(
        key_id: &str,
        key: &ed25519_dalek::SigningKey,
    ) -> plugin_host::distribution::TrustedSigningKey {
        plugin_host::distribution::TrustedSigningKey {
            key_id: key_id.to_string(),
            public_key: hex::encode(key.verifying_key().as_bytes()),
        }
    }

    /// Builds the artifact the issuer would mint, signs it exactly as the
    /// issuer does (ed25519 over sha256 of the canonical artifact), and
    /// returns the request envelope.
    fn signed_request(
        org: Uuid,
        revision: i64,
        payload: serde_json::Value,
        key: &ed25519_dalek::SigningKey,
    ) -> AdoptEntitlementRequest {
        let payload_sha256 =
            plugin_host::distribution::manifest_sha256(&payload).expect("payload is canonical");
        let artifact = serde_json::json!({
            "schema": ENTITLEMENT_ARTIFACT_SCHEMA,
            "org_id": org.to_string(),
            "revision": revision,
            "issued_at": "2026-10-27T00:00:00Z",
            "payload": payload,
            "payload_sha256": payload_sha256,
        });
        let digest_hex =
            plugin_host::distribution::manifest_sha256(&artifact).expect("artifact is canonical");
        let digest: [u8; 32] = hex::decode(&digest_hex).unwrap().try_into().unwrap();
        let sig = hex::encode(key.sign(&digest).to_bytes());
        AdoptEntitlementRequest {
            artifact,
            signature: EntitlementSignature {
                key_id: Some("advisory".into()),
                sig,
            },
        }
    }

    fn verify(
        request: &AdoptEntitlementRequest,
        keys: &[plugin_host::distribution::TrustedSigningKey],
        org: Uuid,
    ) -> Result<VerifiedArtifact, ArtifactRefusal> {
        verify_artifact(&request.artifact, &request.signature, keys, org)
    }

    #[test]
    fn accepts_a_real_signature_and_names_the_key_that_verified() {
        let key = signing_key(7);
        let request = signed_request(ORG, 7, serde_json::json!({ "seats": 5 }), &key);
        let verified = verify(&request, &[trusted("ops-1", &key)], ORG).expect("accepted");
        assert_eq!(verified.revision, 7);
        assert_eq!(verified.org_id, ORG);
        // The answered key is the one that verified, not the one the issuer
        // named in the signature block.
        assert_eq!(verified.key_id, "ops-1");
        assert_eq!(
            verified.payload_sha256,
            request.artifact["payload_sha256"].as_str().unwrap()
        );
    }

    #[test]
    fn refuses_a_signature_from_an_untrusted_key() {
        let signer = signing_key(7);
        let stranger = signing_key(9);
        let request = signed_request(ORG, 1, serde_json::json!({ "seats": 1 }), &stranger);
        // Trusting a DIFFERENT key is the case that matters: a signature that
        // is cryptographically valid but not ours must not adopt.
        assert_eq!(
            verify(&request, &[trusted("ops-1", &signer)], ORG),
            Err(ArtifactRefusal::BadSignature)
        );
    }

    #[test]
    fn refuses_an_artifact_minted_for_another_org() {
        let key = signing_key(7);
        let other_org = Uuid::from_u128(0xffff);
        let request = signed_request(other_org, 1, serde_json::json!({ "seats": 1 }), &key);
        // The signature is valid over the other tenant's artifact; the org
        // binding is checked before the signature for exactly this reason.
        assert_eq!(
            verify(&request, &[trusted("ops-1", &key)], ORG),
            Err(ArtifactRefusal::OrgMismatch)
        );
    }

    #[test]
    fn refuses_a_payload_that_was_edited_after_signing() {
        let key = signing_key(7);
        let mut request = signed_request(ORG, 1, serde_json::json!({ "seats": 1 }), &key);
        request.artifact["payload"] = serde_json::json!({ "seats": 5_000 });
        assert_eq!(
            verify(&request, &[trusted("ops-1", &key)], ORG),
            Err(ArtifactRefusal::PayloadDigestMismatch)
        );
    }

    #[test]
    fn refuses_an_artifact_edited_after_signing() {
        let key = signing_key(7);
        let mut request = signed_request(ORG, 1, serde_json::json!({ "seats": 1 }), &key);
        request.artifact["revision"] = serde_json::json!(2);
        assert_eq!(
            verify(&request, &[trusted("ops-1", &key)], ORG),
            Err(ArtifactRefusal::BadSignature)
        );
    }

    #[test]
    fn refuses_an_unknown_schema() {
        let key = signing_key(7);
        let mut request = signed_request(ORG, 1, serde_json::json!({ "seats": 1 }), &key);
        request.artifact["schema"] = serde_json::json!("marula.entitlement/v99");
        assert_eq!(
            verify(&request, &[trusted("ops-1", &key)], ORG),
            Err(ArtifactRefusal::UnsupportedSchema)
        );
    }

    #[test]
    fn fails_closed_when_no_key_is_trusted() {
        let key = signing_key(7);
        let request = signed_request(ORG, 1, serde_json::json!({ "seats": 1 }), &key);
        assert_eq!(
            verify(&request, &[], ORG),
            Err(ArtifactRefusal::NoTrustedKeys)
        );
    }

    #[test]
    fn refuses_a_revision_that_is_not_positive() {
        let key = signing_key(7);
        for revision in [0_i64, -1] {
            let request = signed_request(ORG, revision, serde_json::json!({ "seats": 1 }), &key);
            assert_eq!(
                verify(&request, &[trusted("ops-1", &key)], ORG),
                Err(ArtifactRefusal::InvalidRevision),
                "revision {revision} must be refused"
            );
        }
    }

    #[test]
    fn no_trusted_keys_maps_to_a_conflict_not_a_malformed_request() {
        // The document is fine; the INSTALL cannot verify anything. That is a
        // state the operator fixes, so it cannot read as a client error.
        assert!(matches!(
            artifact_refusal(ArtifactRefusal::NoTrustedKeys),
            ApiError::Conflict(_)
        ));
        for refusal in [
            ArtifactRefusal::Malformed,
            ArtifactRefusal::UnsupportedSchema,
            ArtifactRefusal::TooLarge,
            ArtifactRefusal::OrgMismatch,
            ArtifactRefusal::PayloadDigestMismatch,
            ArtifactRefusal::InvalidRevision,
            ArtifactRefusal::BadSignature,
        ] {
            assert!(matches!(artifact_refusal(refusal), ApiError::Validation(_)));
        }
    }

    fn held_at(revision: i64, payload_sha256: &str) -> core_db::entitlements::EntitlementRow {
        core_db::entitlements::EntitlementRow {
            org_id: ORG,
            revision,
            payload: serde_json::json!({}),
            payload_sha256: payload_sha256.to_string(),
            artifact_sha256: "b".repeat(64),
            key_id: Some("ops-1".into()),
            issued_at: None,
            adopted_by: None,
            adopted_at: Utc::now(),
        }
    }

    fn verified_at(revision: i64, payload_sha256: &str) -> VerifiedArtifact {
        VerifiedArtifact {
            org_id: ORG,
            revision,
            issued_at: None,
            payload: serde_json::json!({}),
            payload_sha256: payload_sha256.to_string(),
            artifact_sha256: "c".repeat(64),
            key_id: "ops-1".into(),
        }
    }

    #[test]
    fn the_claimed_identity_is_tried_first_but_never_trusted() {
        // Two trust identities share one public key, which is what an operator
        // naming a key does when they import the same material under a label.
        // The issuer names the second; the answer must be the second, even
        // though the first would also verify.
        let key = signing_key(11);
        let keys = vec![trusted("first", &key), trusted("second", &key)];
        let mut request = signed_request(ORG, 1, serde_json::json!({ "seats": 1 }), &key);
        request.signature.key_id = Some("second".into());
        assert_eq!(
            verify(&request, &keys, ORG).expect("accepted").key_id,
            "second"
        );

        // A claim that names nothing trusted changes nothing: the set still
        // decides, and a claim cannot make an unverifiable key verify.
        request.signature.key_id = Some("absent".into());
        assert_eq!(
            verify(&request, &keys, ORG).expect("accepted").key_id,
            "first"
        );

        // And naming a trusted identity whose key does not match the signature
        // refuses rather than falling back to a pass.
        let stranger = signing_key(13);
        let mut wrong = signed_request(ORG, 1, serde_json::json!({ "seats": 1 }), &stranger);
        wrong.signature.key_id = Some("first".into());
        assert_eq!(
            verify(&wrong, &keys, ORG),
            Err(ArtifactRefusal::BadSignature)
        );
    }

    #[test]
    fn a_retry_of_the_held_revision_is_the_same_fact_only_with_the_same_digest() {
        let digest = "a".repeat(64);
        assert!(is_same_fact(&held_at(3, &digest), &verified_at(3, &digest)));
        // Same revision, different content: the signer contradicted itself.
        assert!(!is_same_fact(
            &held_at(3, &digest),
            &verified_at(3, &"d".repeat(64))
        ));
        // A different revision is never the same fact.
        assert!(!is_same_fact(
            &held_at(4, &digest),
            &verified_at(3, &digest)
        ));
    }
}
