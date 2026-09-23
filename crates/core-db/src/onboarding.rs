//! Wave 26 onboarding state machine support (org wizard + user-level flows).
//!
//! Purpose: server-gated step satisfaction checks (artifacts verified in
//! the DB, never trusted from the client) plus typed reset helpers for the
//! re-run / re-trigger controls under Settings → Onboarding.
//!
//! Non-purpose: persistence of the step flags themselves (the typed
//! settings registry owns those — `core_db::settings`) and the HTTP shape
//! (core-api's onboarding route owns that).

use crate::DbResult;
use uuid::Uuid;

/// The org wizard's steps in display order (mirrors the domain list; the
/// route module validates step ids against the domain constants).
pub const ORG_STEPS: &[&str] = &["clients", "board", "tickets", "invites", "feeder", "ai"];

/// Server-verified readiness of every org wizard step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OrgStepReadiness {
    pub clients: bool,
    pub board: bool,
    pub tickets: bool,
    pub invites: bool,
    pub feeder: bool,
    pub ai: bool,
}

impl OrgStepReadiness {
    /// Whether `step`'s completion gate is currently satisfied.
    pub fn satisfied(&self, step: &str) -> bool {
        match step {
            "clients" => self.clients,
            "board" => self.board,
            "tickets" => self.tickets,
            "invites" => self.invites,
            "feeder" => self.feeder,
            "ai" => self.ai,
            _ => false,
        }
    }
}

/// Reads the gate artifacts for every org step in ONE round trip (the
/// caller's org GUC scopes every table via RLS).
pub async fn org_step_readiness(
    exec: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
) -> DbResult<OrgStepReadiness> {
    let row = sqlx::query!(
        r#"
        SELECT
          EXISTS (SELECT 1 FROM clients
                  WHERE org_id = $1)                             AS clients,
          EXISTS (SELECT 1 FROM boards WHERE org_id = $1)        AS board,
          EXISTS (SELECT 1 FROM tickets WHERE org_id = $1)       AS tickets,
          EXISTS (SELECT 1 FROM user_invites WHERE org_id = $1)  AS invites,
          -- A feeder is "connected" once a feed actually wrote: any config
          -- source link proves a real feeder upsert happened (issued keys
          -- with no writes do NOT satisfy the gate).
          EXISTS (SELECT 1 FROM config_source_links
                  WHERE org_id = $1)                             AS feeder,
          -- AI gate mirrors the AI settings master switch
          -- (`ai.features_enabled`). No stored override = default off.
          EXISTS (
            SELECT 1 FROM settings
            WHERE scope = 'org' AND owner_id = $1
              AND key = 'ai.features_enabled'
              AND (value #>> '{}') IN ('true', '1')
          )                                              AS ai
        "#,
        org_id
    )
    .fetch_one(exec)
    .await?;
    Ok(OrgStepReadiness {
        clients: row.clients.unwrap_or(false),
        board: row.board.unwrap_or(false),
        tickets: row.tickets.unwrap_or(false),
        invites: row.invites.unwrap_or(false),
        feeder: row.feeder.unwrap_or(false),
        ai: row.ai.unwrap_or(false),
    })
}

/// Whether the user has ANY confirmed second factor (TOTP or WebAuthn).
pub async fn user_mfa_ready(
    exec: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    user_id: Uuid,
) -> DbResult<bool> {
    let row = sqlx::query!(
        r#"
        SELECT
          EXISTS (SELECT 1 FROM totp_secrets
                  WHERE user_id = $1 AND confirmed = true)
          OR EXISTS (SELECT 1 FROM webauthn_credentials WHERE user_id = $1)
        AS ready
        "#,
        user_id
    )
    .fetch_one(exec)
    .await?;
    Ok(row.ready.unwrap_or(false))
}

/// Deletes every org-scope onboarding step override. Used by the Settings →
/// Onboarding re-run control; the flows_disabled master switch is NOT
/// touched (re-running a disabled flow family stays disabled — deliberate,
/// and the toggle lives on the same page).
pub async fn reset_org_flags(
    exec: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
) -> DbResult<u64> {
    let res = sqlx::query!(
        r#"DELETE FROM settings
           WHERE org_id = $1
             AND scope = 'org'
             AND (key LIKE 'onboarding.org\_%\_done' OR key LIKE 'onboarding.org\_%\_dismissed')"#,
        org_id
    )
    .execute(exec)
    .await?;
    Ok(res.rows_affected())
}

/// Deletes one member's user-scope onboarding overrides. Used by the admin
/// re-trigger control (Settings → Onboarding → member row).
pub async fn reset_user_flags(
    exec: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    org_id: Uuid,
    user_id: Uuid,
) -> DbResult<u64> {
    let res = sqlx::query!(
        r#"DELETE FROM settings
           WHERE org_id = $1
             AND scope = 'user'
             AND owner_id = $2
             AND (key LIKE 'onboarding.user\_%\_done' OR key LIKE 'onboarding.user\_%\_dismissed')"#,
        org_id,
        user_id
    )
    .execute(exec)
    .await?;
    Ok(res.rows_affected())
}
