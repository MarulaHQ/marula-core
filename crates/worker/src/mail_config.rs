//! Per-org mail transport resolution (Wave 34 lane 8).
//!
//! Purpose: at send time, resolve what actually carries this org's mail —
//! the org's transport selection (`email.transport`), its non-secret
//! parameters (SMTP host/port/username, Postmark base URL), and its SEALED
//! credential (Postmark server token / SMTP password, decrypted in-process
//! with the shared at-rest key). The process env transport remains the
//! fallback for deployments configured the old way.
//!
//! Non-purpose: no writes (the resolver only reads), no caching of secrets
//! beyond the returned adapter, and no policy — the worker consumes; core-api
//! owns configuration through the admin surface.
//!
//! Failure mode: an unreadable selection / missing credential returns a
//! typed error, the message is reported `ok=false` and the ledger's backoff
//! retries it — mail is never silently dropped.

use crate::config::AtRestKey;
use core_domain::settings as registry;
use core_mail::{MailTransport, TransportKind};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// Sealed provider ids (must match core-api's reserved ids).
const SEALED_POSTMARK_PROVIDER: &str = "mail.postmark";
const SEALED_SMTP_PROVIDER: &str = "mail.smtp";

/// Resolves the transport adapter for one org at send time.
#[async_trait::async_trait]
pub trait OrgMailResolver: Send + Sync {
    /// The adapter for this org. `Err` carries a retryable explanation.
    async fn transport_for(&self, org_id: Uuid) -> Result<Arc<dyn MailTransport>, String>;
}

/// Database-backed resolver: org settings + the sealed slot, with the env
/// transport as the fallback when the org has no selection.
pub struct DbOrgMailResolver {
    pool: PgPool,
    key: Option<AtRestKey>,
    env_fallback: TransportKind,
}

impl DbOrgMailResolver {
    #[must_use]
    pub fn new(pool: PgPool, key: Option<AtRestKey>, env_fallback: TransportKind) -> Self {
        Self {
            pool,
            key,
            env_fallback,
        }
    }

    /// Decrypts one sealed credential into its JSON document.
    async fn sealed_json(
        &self,
        conn: &mut sqlx::PgConnection,
        org_id: Uuid,
        provider: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        let Some(sealed) = core_db::byok::get_secret(&mut *conn, org_id, provider)
            .await
            .map_err(|e| format!("reading sealed credential {provider}: {e}"))?
        else {
            return Ok(None);
        };
        let Some(key) = self.key else {
            return Err(format!(
                "credential {provider} is sealed but MERIDIAN_SECRET_KEY is unset"
            ));
        };
        let plaintext = key
            .decrypt(&sealed)
            .map_err(|e| format!("decrypting credential {provider}: {e}"))?;
        serde_json::from_slice(&plaintext)
            .map(Some)
            .map_err(|e| format!("credential {provider} is not valid JSON: {e}"))
    }

    /// The org's configured transport, falling back to env when unset.
    async fn kind_for(&self, org_id: Uuid) -> Result<TransportKind, String> {
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        core_db::set_org(&mut tx, org_id)
            .await
            .map_err(|e| e.to_string())?;
        let stored = core_db::settings::get_org_value(
            &mut *tx,
            org_id,
            registry::EMAIL_TRANSPORT,
        )
        .await
        .map_err(|e| e.to_string())?
        .and_then(|v| v.as_str().map(str::to_string));
        let kind = match stored.as_deref() {
            Some("postmark") => match self.sealed_json(&mut tx, org_id, SEALED_POSTMARK_PROVIDER).await? {
                Some(secret) => {
                    let token = secret
                        .get("server_token")
                        .and_then(|v| v.as_str())
                        .filter(|v| !v.trim().is_empty())
                        .ok_or_else(|| {
                            "the sealed Postmark credential has no server_token".to_string()
                        })?;
                    let base_url = org_text(&mut tx, org_id, registry::EMAIL_POSTMARK_BASE_URL)
                        .await?
                        .unwrap_or_else(|| "https://api.postmarkapp.com".to_string());
                    TransportKind::Postmark {
                        server_token: token.to_string(),
                        base_url,
                    }
                }
                None => {
                    return Err(
                        "org transport is postmark but no server token is sealed".to_string()
                    )
                }
            },
            Some("smtp") => {
                let host = org_text(&mut tx, org_id, registry::EMAIL_SMTP_HOST)
                    .await?
                    .ok_or_else(|| {
                        "org transport is smtp but email.smtp_host is empty".to_string()
                    })?;
                let port = core_db::settings::get_org_value(
                    &mut *tx,
                    org_id,
                    registry::EMAIL_SMTP_PORT,
                )
                .await
                .map_err(|e| e.to_string())?
                .and_then(|v| v.as_i64())
                .unwrap_or(587);
                let username = org_text(&mut tx, org_id, registry::EMAIL_SMTP_USERNAME).await?;
                let password = match username {
                    Some(_) => match self.sealed_json(&mut tx, org_id, SEALED_SMTP_PROVIDER).await? {
                        Some(secret) => secret
                            .get("password")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        None => None,
                    },
                    // Unauthenticated relay: trust comes from the network.
                    None => None,
                };
                if username.is_some() && password.is_none() {
                    return Err(
                        "org transport is smtp with a username but no sealed password".to_string()
                    );
                }
                TransportKind::Smtp {
                    host,
                    port: u16::try_from(port).unwrap_or(587),
                    username,
                    password,
                }
            }
            // Explicit none keeps mail queued; an absent selection falls
            // back to the process transport so legacy env-configured
            // deployments keep sending unchanged.
            Some("none") => TransportKind::None,
            None | Some("") => self.env_fallback.clone(),
            Some(other) => return Err(format!("unknown org transport selection '{other}'")),
        };
        tx.rollback().await.ok();
        Ok(kind)
    }
}

/// One stored org setting as trimmed text; `None` when unset or blank.
async fn org_text(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    key: &str,
) -> Result<Option<String>, String> {
    let value = core_db::settings::get_org_value(&mut *tx, org_id, key)
        .await
        .map_err(|e| e.to_string())?;
    Ok(value
        .and_then(|v| v.as_str().map(str::to_string))
        .filter(|s| !s.trim().is_empty()))
}

#[async_trait::async_trait]
impl OrgMailResolver for DbOrgMailResolver {
    async fn transport_for(&self, org_id: Uuid) -> Result<Arc<dyn MailTransport>, String> {
        let kind = self.kind_for(org_id).await?;
        // A "none" org selection keeps mail queued: report a retryable error
        // so the ledger's backoff holds the message instead of dead-lettering
        // it (the operator may configure a transport any day).
        if matches!(kind, TransportKind::None) {
            return Err("no outbound transport is configured for this organization".to_string());
        }
        Ok(core_mail::build_transport(&kind))
    }
}

/// Test/stub resolver: one static transport for every org.
pub struct StaticResolver(pub Arc<dyn MailTransport>);

#[async_trait::async_trait]
impl OrgMailResolver for StaticResolver {
    async fn transport_for(&self, _org_id: Uuid) -> Result<Arc<dyn MailTransport>, String> {
        Ok(Arc::clone(&self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn static_resolver_hands_back_the_same_transport() {
        let recording = Arc::new(core_mail::RecordingTransport::new());
        let resolver = StaticResolver(Arc::clone(&recording) as Arc<dyn MailTransport>);
        let first = resolver.transport_for(Uuid::now_v7()).await.unwrap();
        let second = resolver.transport_for(Uuid::now_v7()).await.unwrap();
        assert_eq!(first.name(), "recording");
        assert_eq!(second.name(), "recording");
    }
}
