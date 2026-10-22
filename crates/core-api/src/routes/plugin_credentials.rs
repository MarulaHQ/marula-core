//! The consent surface for credentialed egress (Wave 53 lane 12): the request
//! and view shapes an install's binding list is read and written through.
//!
//! The binding itself lives in `core_db::plugin_credentials`; this module is
//! what an operator sees and what a request body may name. Three rules are
//! enforced here rather than in the database, because they are questions about
//! the MANIFEST and about a header, not about a row:
//!
//! * a profile name must be declared by the install's pinned manifest, so a
//!   binding cannot point at authority the artifact never asked for;
//! * a header name must be a lowercase HTTP token the transport could apply,
//!   and must not be one the transport owns (framing and hop-by-hop headers);
//! * the same profile may appear once. One profile, one credential, one
//!   header — a list whose order decided which secret went out is exactly the
//!   ambiguity this feature cannot afford.
//!
//! [`CredentialBindingView`] is the type that carries the negative property:
//! it names the profile, the header, the record id and the record's TITLE, and
//! it has no member that could hold a field name, a field value, ciphertext or
//! a key. [`tests::the_consent_view_cannot_carry_a_secret`] pins the serialized
//! key set, so adding one later is a test failure rather than a review miss.

use chrono::{DateTime, Utc};
use core_domain::entities::{PluginInstall, PluginManifestV2};
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

/// One binding an operator asks for, in the replace-set body of
/// `PATCH /api/plugins/installed/{id}`.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct CredentialBindingInput {
    /// A key of the pinned manifest's `connector_profile.outbound_scopes`.
    pub profile: String,
    /// The vault record whose `secret` field is injected. The record is named
    /// by id here because that is the only handle a screen has; the id is
    /// resolved through the caller's org, so an id from another tenant is a
    /// `404` rather than a binding.
    #[schema(value_type = uuid::Uuid)]
    pub secret_record_id: Uuid,
    /// The header the value is injected as, e.g. `x-api-key`. Lowercased
    /// before it is stored.
    pub header_name: String,
}

/// What the consent screen shows for one binding: the profile, the header, and
/// the vault record's NAME. Never a field, a value, ciphertext or a key.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct CredentialBindingView {
    pub profile: String,
    pub header_name: String,
    #[schema(value_type = uuid::Uuid)]
    pub secret_record_id: Uuid,
    /// The vault record's title. This is the whole of what the screen may say
    /// about the record's contents.
    pub record_title: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub bound_by: Option<Uuid>,
    #[schema(value_type = String, format = "date-time")]
    pub bound_at: DateTime<Utc>,
}

impl From<core_db::plugin_credentials::CredentialBinding> for CredentialBindingView {
    fn from(b: core_db::plugin_credentials::CredentialBinding) -> Self {
        Self {
            profile: b.profile,
            header_name: b.header_name,
            secret_record_id: b.secret_record_id,
            record_title: b.record_title,
            bound_by: b.bound_by,
            bound_at: b.created_at,
        }
    }
}

/// Headers the transport owns. A credential bound to one of these would be
/// refused at the first call (`Refusal::FrameInvalid`), so the bind refuses it
/// while an operator is looking at the form instead.
///
/// The authority is `HOST_OWNED_HEADERS` in `crates/plugin-host/src/http.rs`;
/// that constant is private to the transport and this list is a courtesy copy,
/// which is acceptable only because it can reject a bind and never allow one.
/// The follow-up that removes the copy is making the transport's constant
/// public, which this lane may not do while a sibling lane holds the file.
const TRANSPORT_OWNED_HEADERS: [&str; 8] = [
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "upgrade",
    "proxy-connection",
    "te",
    "trailer",
];

/// The manifest caps a profile name at `[a-z][a-z0-9_-]{0,63}`; the column
/// CHECK repeats it so a hand-written insert cannot park another spelling.
fn valid_profile_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && name.len() <= 64
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
}

/// An RFC 9110 token, lowercase, at most 64 bytes — the same ceiling the
/// transport's `MAX_HEADER_NAME_BYTES` applies.
fn valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.bytes().all(|b| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || matches!(
                    b,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

/// Everything [`apply`] needs that is not the desired set itself.
pub struct BindContext<'a> {
    pub org_id: Uuid,
    pub install: &'a PluginInstall,
    pub manifest: &'a PluginManifestV2,
    pub actor: Uuid,
    /// The roles the acting operator holds — the authority the vault grant is
    /// checked against, recorded on the row for the resolve to re-check.
    pub role_ids: Vec<Uuid>,
}

/// Replaces the install's binding set with `desired`, in the caller's
/// transaction.
///
/// Replace-set, like `granted_capabilities`: what the body names is what the
/// install ends up holding, so a caller cannot leave a stale credential behind
/// by forgetting to mention it. An empty list removes every binding, which is
/// the deliberate way to withdraw consent.
///
/// Each change writes its own audit row. A credential appearing or
/// disappearing on an install is exactly the kind of change that must be
/// answerable later, and the row names the profile, the header and the record
/// — never a value.
pub async fn apply(
    conn: &mut PgConnection,
    ctx: &BindContext<'_>,
    desired: &[CredentialBindingInput],
) -> ApiResult<()> {
    // Validation first, so a bad entry cannot leave the set half-applied. The
    // transaction would roll it back anyway; this way the refusal names the
    // entry.
    let mut wanted: Vec<(String, Uuid, String)> = Vec::with_capacity(desired.len());
    for entry in desired {
        let profile = entry.profile.trim().to_string();
        if !valid_profile_name(&profile) {
            return Err(ApiError::Validation(format!(
                "egress profile {profile:?} must match [a-z][a-z0-9_-]{{0,63}}"
            )));
        }
        if !ctx
            .manifest
            .connector_profile
            .as_ref()
            .is_some_and(|p| p.outbound_scopes.contains_key(&profile))
        {
            return Err(ApiError::Validation(format!(
                "egress profile {profile:?} is not declared by manifest version {}; a binding \
                 names a profile the artifact asked for",
                ctx.manifest.version
            )));
        }
        let header = entry.header_name.trim().to_ascii_lowercase();
        if !valid_header_name(&header) {
            return Err(ApiError::Validation(format!(
                "header name {header:?} is not a lowercase HTTP token"
            )));
        }
        if TRANSPORT_OWNED_HEADERS.contains(&header.as_str()) {
            return Err(ApiError::Validation(format!(
                "header {header:?} belongs to the transport; a credential may not set it"
            )));
        }
        if wanted.iter().any(|(existing, ..)| existing == &profile) {
            return Err(ApiError::Validation(format!(
                "egress profile {profile:?} appears twice; one profile carries one credential"
            )));
        }
        wanted.push((profile, entry.secret_record_id, header));
    }

    let existing = core_db::plugin_credentials::list(conn, ctx.org_id, ctx.install.id).await?;

    for (profile, record_id, header) in &wanted {
        let outcome = core_db::plugin_credentials::bind(
            conn,
            ctx.org_id,
            ctx.install.id,
            &core_db::plugin_credentials::NewBinding {
                profile,
                secret_record_id: *record_id,
                header_name: header,
                bound_by: Some(ctx.actor),
                bound_role_ids: ctx.role_ids.clone(),
            },
        )
        .await?;
        let bound = match outcome {
            core_db::plugin_credentials::BindOutcome::Bound(bound) => bound,
            // One answer for another tenant's record and for a record that is
            // not there, so the route cannot be used to probe the vault.
            core_db::plugin_credentials::BindOutcome::NoSuchRecord => {
                return Err(ApiError::NotFound)
            }
            core_db::plugin_credentials::BindOutcome::CategoryDenied => {
                return Err(ApiError::Forbidden(
                    "this record's category is restricted for your roles; a credential can \
                     only be bound by an operator who can read it"
                        .into(),
                ))
            }
        };
        let unchanged = existing.iter().any(|b| {
            b.profile == bound.profile
                && b.secret_record_id == bound.secret_record_id
                && b.header_name == bound.header_name
        });
        if unchanged {
            continue;
        }
        core_db::audit::insert(
            &mut *conn,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id: ctx.org_id,
                actor: core_domain::entities::Actor::Human(ctx.actor),
                action: "plugin.credential_bound".into(),
                entity_type: Some("plugin_install".into()),
                entity_id: Some(ctx.install.id.to_string()),
                detail: serde_json::json!({
                    "slug": ctx.install.slug,
                    "profile": bound.profile,
                    "header_name": bound.header_name,
                    "secret_record_id": bound.secret_record_id,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
    }

    for old in &existing {
        if wanted.iter().any(|(profile, ..)| profile == &old.profile) {
            continue;
        }
        core_db::plugin_credentials::unbind(conn, ctx.org_id, ctx.install.id, old.profile.as_str())
            .await?;
        core_db::audit::insert(
            &mut *conn,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id: ctx.org_id,
                actor: core_domain::entities::Actor::Human(ctx.actor),
                action: "plugin.credential_unbound".into(),
                entity_type: Some("plugin_install".into()),
                entity_id: Some(ctx.install.id.to_string()),
                detail: serde_json::json!({
                    "slug": ctx.install.slug,
                    "profile": old.profile,
                    "secret_record_id": old.secret_record_id,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_db::plugin_credentials::CredentialBinding;

    fn binding() -> CredentialBinding {
        CredentialBinding {
            id: Uuid::nil(),
            org_id: Uuid::nil(),
            install_id: Uuid::nil(),
            profile: "acme".into(),
            secret_record_id: Uuid::nil(),
            header_name: "x-api-key".into(),
            bound_by: Some(Uuid::nil()),
            bound_role_ids: vec![Uuid::nil()],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            record_title: "Acme tenant token".into(),
        }
    }

    /// THE NEGATIVE PROPERTY, as a test rather than a review note: the consent
    /// view's serialized shape is exactly the fields an operator is allowed to
    /// see. A member that could carry a field name, a field value, ciphertext
    /// or a key fails here, because the key set moves.
    #[test]
    fn the_consent_view_cannot_carry_a_secret() {
        let json = serde_json::to_value(CredentialBindingView::from(binding()))
            .expect("the consent view serializes");
        let mut keys: Vec<&str> = json
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "bound_at",
                "bound_by",
                "header_name",
                "profile",
                "record_title",
                "secret_record_id",
            ],
            "the consent view may name the profile, the header, the record and who bound it — \
             nothing that could carry a secret"
        );
    }

    /// The record's title is the only thing said about the record's contents:
    /// no field name, no ciphertext, no key material anywhere in the payload.
    #[test]
    fn the_consent_view_names_no_field_and_no_value() {
        let encoded = serde_json::to_string(&CredentialBindingView::from(binding()))
            .expect("the consent view serializes");
        for forbidden in ["bound_role_ids", "value", "ciphertext", "field", "sealed"] {
            assert!(
                !encoded.contains(forbidden),
                "consent payload must not mention {forbidden:?}: {encoded}"
            );
        }
    }

    #[test]
    fn profile_names_follow_the_manifest_vocabulary() {
        assert!(valid_profile_name("acme"));
        assert!(valid_profile_name("acme_corp-2"));
        assert!(!valid_profile_name("Acme"));
        assert!(!valid_profile_name("2acme"));
        assert!(!valid_profile_name(""));
        assert!(!valid_profile_name(&"a".repeat(65)));
    }

    #[test]
    fn header_names_are_lowercase_tokens_the_transport_could_apply() {
        assert!(valid_header_name("x-api-key"));
        assert!(valid_header_name("x_vendor.token"));
        assert!(!valid_header_name("Authorization"));
        assert!(!valid_header_name("x api key"));
        assert!(!valid_header_name("x-api-key\r\nx-evil"));
        assert!(!valid_header_name(""));
        assert!(!valid_header_name(&"a".repeat(65)));
        for owned in TRANSPORT_OWNED_HEADERS {
            assert!(
                valid_header_name(owned),
                "{owned} is a valid token but must be refused by name, not by shape"
            );
            assert!(TRANSPORT_OWNED_HEADERS.contains(&owned));
        }
    }
}
