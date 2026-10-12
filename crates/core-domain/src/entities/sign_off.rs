//! Sign-off entity: the witnessed record that a customer signed off on a
//! ticket (Wave 43 track 2 lane 12, migration 0127).
//!
//! Purpose: the first-class evidence row behind the drawn mark. A signature
//! is not "an attachment that happens to be a scribble" — the image bytes
//! ride the content-addressed store as this entity's PAYLOAD, while this row
//! carries the facts an audit needs: who signed, when, who witnessed it, and
//! exactly which consent wording they saw. Bytes never live here either: the
//! row points at a `core_store::attachment_key` through `attachment_id`, and
//! that pointer is `None` only for a typed agreement, which carries no ink.
//!
//! Non-purpose: no byte movement (callers go through `core_store::ObjectStore`),
//! no admission rules for the PNG (core-domain's attachment validator), no
//! authorization, and no mutable state — revocation is a separate append-only
//! trail (`SignOffRevocation`), never a flag flipped on this row.
//!
//! A signer who cannot draw is not a lesser signer: the typed-agreement path
//! produces the same row with `attachment_id: None`, and the migration's
//! capture-shape CHECK keeps that the only reason the payload may be absent.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Who drew the mark. `PortalClient` is RESERVED: the API rejects it today so
/// a future portal-signing lane needs no schema change, mirroring the reserved
/// `Attachment::uploaded_by_portal_user_id` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SignOffSignerKind {
    /// The customer signed on the tech's device at the door.
    Customer,
    /// A client signed inside the portal. Reserved — not yet accepted.
    PortalClient,
}

impl SignOffSignerKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            SignOffSignerKind::Customer => "customer",
            SignOffSignerKind::PortalClient => "portal_client",
        }
    }

    /// Whether the API may accept this signer kind right now.
    #[must_use]
    pub const fn is_supported(self) -> bool {
        matches!(self, SignOffSignerKind::Customer)
    }
}

impl std::str::FromStr for SignOffSignerKind {
    type Err = crate::DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "customer" => Ok(SignOffSignerKind::Customer),
            "portal_client" => Ok(SignOffSignerKind::PortalClient),
            other => Err(crate::DomainError::Validation(format!(
                "invalid sign-off signer kind: {other}"
            ))),
        }
    }
}

/// The evidence row. Immutable by design: there is no UPDATE path, so a
/// correction is a second row linked by `supersedes_sign_off_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SignOff {
    pub id: Uuid,
    pub org_id: Uuid,
    /// The ticket being signed off. A sign-off always has one subject.
    pub ticket_id: Uuid,
    /// The PNG payload in the content-addressed store (`kind = sign_off`).
    /// `None` exactly when the signer agreed by typing their name instead of
    /// drawing, which is the first-class alternative path rather than a
    /// degenerate capture.
    pub attachment_id: Option<Uuid>,
    /// True when the signer agreed by typing their name. An auditor needs to
    /// know which captures carried ink, because the two are not equally
    /// strong evidence and only one can be compared against a sample.
    pub typed_agreement: bool,
    /// Who signed, as they typed it. A drawn mark identifies nobody on its own.
    pub signer_name: String,
    pub signer_kind: SignOffSignerKind,
    /// The staff member who presented the pad and watched the signature.
    pub witnessed_by_user_id: Option<Uuid>,
    /// Device-clock reading when the stroke finished, recorded verbatim —
    /// offline capture is the normal case, so this is evidence about the
    /// device, not a trusted server timestamp.
    pub captured_at: DateTime<Utc>,
    /// Consent wording version the signer saw.
    pub consent_version: i32,
    /// Verbatim text the signer agreed to. Snapshot, not a live join, so a
    /// later consent revision can never change what an earlier signer agreed.
    pub consent_text_snapshot: String,
    /// Set only on a correction row; points backward at the original.
    pub supersedes_sign_off_id: Option<Uuid>,
    /// Client-minted, server-enforced unique per org (offline replay lands once).
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
}

/// Append-only consent wording per org. Revision N+1 never rewrites revision N.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SignOffConsent {
    pub id: Uuid,
    pub org_id: Uuid,
    pub version: i32,
    pub body: String,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// One entry in a sign-off's revocation trail. The row and its bytes survive;
/// "is revoked" is answered by the existence of one of these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SignOffRevocation {
    pub id: Uuid,
    pub org_id: Uuid,
    pub sign_off_id: Uuid,
    /// 1-based per sign-off; a second revocation of the same sign-off is a
    /// data error, not an overwrite.
    pub seq: i32,
    pub reason: String,
    pub revoked_by_user_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// A sign-off plus the facts a client renders on the timeline: the payload's
/// storage facts, the current revocation state, and the correction relation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SignOffView {
    #[serde(flatten)]
    pub sign_off: SignOff,
    /// Payload facts (filename/content type/size/sha256), never the bytes.
    /// All `None` on a typed agreement, which has no mark to describe.
    pub attachment_filename: Option<String>,
    pub attachment_content_type: Option<String>,
    pub attachment_size_bytes: Option<i64>,
    pub attachment_sha256: Option<String>,
    /// The most recent revocation, when the sign-off has been revoked.
    pub revocation: Option<SignOffRevocation>,
    /// The id of a correction that replaced this one, when one exists.
    pub superseded_by: Option<Uuid>,
}
