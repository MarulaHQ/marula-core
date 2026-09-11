//! Per-client workflow overrides (Wave 12 lane 5, migration 0063):
//! persistence + org-default resolution.
//!
//! Purpose: store validated client override rows and resolve each client's
//! effective workflow against the ORG defaults — the org's default board, its
//! entry status, its default SLA policy, and the org-scope settings-registry
//! toggles. The precedence rule itself is pure domain logic
//! ([`core_domain::workflow_overrides::resolve`]); this layer gathers the
//! inputs, validates reference values against the org's constructs, and
//! assembles the resolved document with renderable details (names + semantic
//! status categories) and typed drift issues.
//!
//! Drift handling: overrides reference org constructs by id (jsonb payload, no
//! FK), so a construct deleted after the override was written leaves a stale
//! row. Resolution never hard-fails on drift — it records a typed issue and
//! falls back to the org default, and write-time validation refuses references
//! that do not exist or do not fit the construct's semantic contract (an
//! intake status must be an entry status ON the client's effective intake
//! board).
//!
//! Non-purpose: no permission decisions (routes gate), no event emission
//! (routes own the outbox + audit trail), no behavior *enforcement* — callers
//! consult the resolved document when they need the effective workflow.

use crate::{DbError, DbResult};
use core_domain::status::TicketStatusCategory;
use core_domain::workflow_overrides as registry;
use core_domain::workflow_overrides::{
    ClientWorkflowResolution, OverrideEntry, OverrideState, OverrideValue, Resolution,
    ResolutionSource, ResolvedDetail,
};
use sqlx::PgConnection;
use std::str::FromStr;
use uuid::Uuid;

/// One stored override row (parsed further against the registry).
#[derive(Debug, Clone)]
pub struct OverrideRow {
    pub id: Uuid,
    pub override_key: String,
    pub state: String,
    pub value: Option<serde_json::Value>,
    pub updated_by: Option<Uuid>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

// --- persistence ---------------------------------------------------------------

pub async fn client_exists(
    conn: &mut PgConnection,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<bool> {
    let found: Option<Uuid> = sqlx::query_scalar!(
        r#"SELECT id FROM clients WHERE org_id = $1 AND id = $2"#,
        org_id,
        client_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(found.is_some())
}

pub async fn get_override(
    conn: &mut PgConnection,
    org_id: Uuid,
    client_id: Uuid,
    key: &str,
) -> DbResult<Option<OverrideRow>> {
    let row = sqlx::query!(
        r#"SELECT id, override_key, state, value, updated_by, updated_at
           FROM client_workflow_overrides
           WHERE org_id = $1 AND client_id = $2 AND override_key = $3"#,
        org_id,
        client_id,
        key
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|r| OverrideRow {
        id: r.id,
        override_key: r.override_key,
        state: r.state,
        value: r.value,
        updated_by: r.updated_by,
        updated_at: r.updated_at,
    }))
}

pub async fn list_overrides(
    conn: &mut PgConnection,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<Vec<OverrideRow>> {
    let rows = sqlx::query!(
        r#"SELECT id, override_key, state, value, updated_by, updated_at
           FROM client_workflow_overrides
           WHERE org_id = $1 AND client_id = $2
           ORDER BY override_key"#,
        org_id,
        client_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| OverrideRow {
            id: r.id,
            override_key: r.override_key,
            state: r.state,
            value: r.value,
            updated_by: r.updated_by,
            updated_at: r.updated_at,
        })
        .collect())
}

/// Inserts or updates one override row; returns the row id (stable across
/// updates, so the write-trail seq accumulates per construct).
pub async fn upsert_override(
    conn: &mut PgConnection,
    org_id: Uuid,
    client_id: Uuid,
    key: &str,
    state: &str,
    value: Option<&serde_json::Value>,
    updated_by: Uuid,
) -> DbResult<Uuid> {
    let id = sqlx::query_scalar!(
        r#"INSERT INTO client_workflow_overrides
             (id, org_id, client_id, override_key, state, value, updated_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           ON CONFLICT (org_id, client_id, override_key)
           DO UPDATE SET state = EXCLUDED.state,
                         value = EXCLUDED.value,
                         updated_by = EXCLUDED.updated_by,
                         updated_at = now()
           RETURNING id"#,
        Uuid::now_v7(),
        org_id,
        client_id,
        key,
        state,
        value,
        updated_by,
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(id)
}

/// Removes one override row. Returns whether a row existed (clearing an
/// absent row is a no-op — inherit is the default, so nothing to remove).
pub async fn delete_override(
    conn: &mut PgConnection,
    org_id: Uuid,
    client_id: Uuid,
    key: &str,
) -> DbResult<bool> {
    let res = sqlx::query!(
        r#"DELETE FROM client_workflow_overrides
           WHERE org_id = $1 AND client_id = $2 AND override_key = $3"#,
        org_id,
        client_id,
        key
    )
    .execute(&mut *conn)
    .await?;
    Ok(res.rows_affected() > 0)
}

// --- org-default lookups ---------------------------------------------------------

async fn board_row(
    conn: &mut PgConnection,
    org_id: Uuid,
    board_id: Uuid,
) -> DbResult<Option<(Uuid, String)>> {
    let row = sqlx::query!(
        r#"SELECT id, name FROM boards WHERE org_id = $1 AND id = $2"#,
        org_id,
        board_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|b| (b.id, b.name)))
}

pub async fn default_board(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Option<(Uuid, String)>> {
    let row = sqlx::query!(
        r#"SELECT id, name FROM boards WHERE org_id = $1 AND is_default
           ORDER BY created_at LIMIT 1"#,
        org_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|b| (b.id, b.name)))
}

/// One board status with its semantic category — the binding behavior rides.
#[derive(Debug, Clone)]
pub struct StatusRef {
    pub id: Uuid,
    pub board_id: Uuid,
    pub name: String,
    pub category: TicketStatusCategory,
    pub is_entry: bool,
}

fn status_from_row(r: (Uuid, Uuid, String, String, bool)) -> DbResult<StatusRef> {
    let (id, board_id, name, category, is_entry) = r;
    let category = TicketStatusCategory::from_str(&category).map_err(|_| {
        DbError::Sql(sqlx::Error::ColumnDecode {
            index: "category".into(),
            source: format!("unknown status category {category:?}").into(),
        })
    })?;
    Ok(StatusRef {
        id,
        board_id,
        name,
        category,
        is_entry,
    })
}

pub async fn get_status(
    conn: &mut PgConnection,
    org_id: Uuid,
    status_id: Uuid,
) -> DbResult<Option<StatusRef>> {
    let row = sqlx::query!(
        r#"SELECT id, board_id, name, category, is_entry
           FROM board_statuses WHERE org_id = $1 AND id = $2"#,
        org_id,
        status_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    match row {
        Some(r) => Ok(Some(status_from_row((
            r.id, r.board_id, r.name, r.category, r.is_entry,
        ))?)),
        None => Ok(None),
    }
}

/// The board's lowest-order admissible entry status — the org default for
/// `workflow.intake_status`.
pub async fn default_entry_status(
    conn: &mut PgConnection,
    org_id: Uuid,
    board_id: Uuid,
) -> DbResult<Option<StatusRef>> {
    let row = sqlx::query!(
        r#"SELECT id, board_id, name, category, is_entry
           FROM board_statuses
           WHERE org_id = $1 AND board_id = $2 AND is_entry
           ORDER BY sort_order, name LIMIT 1"#,
        org_id,
        board_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    match row {
        Some(r) => Ok(Some(status_from_row((
            r.id, r.board_id, r.name, r.category, r.is_entry,
        ))?)),
        None => Ok(None),
    }
}

pub async fn get_sla_policy(
    conn: &mut PgConnection,
    org_id: Uuid,
    policy_id: Uuid,
) -> DbResult<Option<(Uuid, String)>> {
    let row = sqlx::query!(
        r#"SELECT id, name FROM sla_policies WHERE org_id = $1 AND id = $2"#,
        org_id,
        policy_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|p| (p.id, p.name)))
}

pub async fn default_sla_policy(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Option<(Uuid, String)>> {
    let row = sqlx::query!(
        r#"SELECT id, name FROM sla_policies WHERE org_id = $1 AND is_default
           ORDER BY created_at LIMIT 1"#,
        org_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|p| (p.id, p.name)))
}

/// The stored org-scope value for a toggle's org default, `None` when unset
/// (the caller falls back to the declaration default).
async fn org_setting_bool(
    conn: &mut PgConnection,
    org_id: Uuid,
    key: &str,
) -> DbResult<Option<bool>> {
    let row = sqlx::query!(
        r#"SELECT value FROM settings
           WHERE org_id = $1 AND scope = 'org' AND key = $2"#,
        org_id,
        key
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.and_then(|r| r.value.as_bool()))
}

// --- write-time validation --------------------------------------------------------

/// Validates a reference value against the org's constructs. Returns the human
/// reason on failure (routes map it to a typed 422). Toggle values need no
/// database check.
pub async fn validate_value(
    conn: &mut PgConnection,
    org_id: Uuid,
    client_id: Uuid,
    decl: &registry::OverrideDeclaration,
    value: &OverrideValue,
) -> Result<(), String> {
    match value {
        OverrideValue::Board(id) => {
            if board_row(conn, org_id, *id)
                .await
                .map_err(db_str)?
                .is_none()
            {
                return Err(format!("{}: board {id} is not in this org", decl.key));
            }
            Ok(())
        }
        OverrideValue::SlaPolicy(id) => {
            if get_sla_policy(conn, org_id, *id)
                .await
                .map_err(db_str)?
                .is_none()
            {
                return Err(format!("{}: SLA policy {id} is not in this org", decl.key));
            }
            Ok(())
        }
        OverrideValue::Status(id) => {
            let status = get_status(conn, org_id, *id)
                .await
                .map_err(db_str)?
                .ok_or_else(|| format!("{}: status {id} is not in this org", decl.key))?;
            if !status.is_entry {
                return Err(format!(
                    "{}: '{}' is not an entry status (tickets cannot open in it)",
                    decl.key, status.name
                ));
            }
            // Membership: the intake status must live on the client's
            // EFFECTIVE intake board (its own board override, else the org
            // default board) — the semantic-category mapping is per-board.
            let board_override = get_override(
                &mut *conn,
                org_id,
                client_id,
                registry::WORKFLOW_INTAKE_BOARD,
            )
            .await
            .map_err(db_str)?;
            let board_decl =
                registry::declaration(registry::WORKFLOW_INTAKE_BOARD).expect("board decl");
            let overridden_board: Option<Uuid> = match board_override {
                Some(row) => {
                    match registry::parse_state(board_decl, &row.state, row.value.as_ref()) {
                        Ok(OverrideState::Set(OverrideValue::Board(b))) => Some(b),
                        _ => None,
                    }
                }
                None => None,
            };
            let effective_board = match overridden_board {
                Some(b) => b,
                None => default_board(conn, org_id)
                    .await
                    .map_err(db_str)?
                    .map(|(id, _)| id)
                    .ok_or_else(|| {
                        format!(
                            "{}: the org has no default board to resolve against",
                            decl.key
                        )
                    })?,
            };
            if status.board_id != effective_board {
                return Err(format!(
                    "{}: '{}' belongs to a different board than the client's \
                     effective intake board",
                    decl.key, status.name
                ));
            }
            Ok(())
        }
        OverrideValue::Toggle(_) => Ok(()),
    }
}

// --- resolution ---------------------------------------------------------------------

/// Resolves the client's full workflow document: one entry per registry
/// declaration, registry order preserved, with per-construct precedence,
/// provenance, renderable details, and typed drift issues.
pub async fn resolve_client_workflow(
    conn: &mut PgConnection,
    org_id: Uuid,
    client_id: Uuid,
) -> DbResult<ClientWorkflowResolution> {
    let rows = list_overrides(&mut *conn, org_id, client_id).await?;
    let default_board = default_board(&mut *conn, org_id).await?;
    let default_policy = default_sla_policy(&mut *conn, org_id).await?;

    // The board entry resolves first: it anchors where the STATUS default
    // comes from (the effective board).
    let board_decl = registry::declaration(registry::WORKFLOW_INTAKE_BOARD).expect("board decl");
    let board_row_match = rows.iter().find(|r| r.override_key == board_decl.key);
    let mut board_issues: Vec<String> = Vec::new();
    let board_client_state = match board_row_match {
        Some(r) => match registry::parse_state(board_decl, &r.state, r.value.as_ref()) {
            Ok(s) => Some(s),
            Err(e) => {
                board_issues.push(format!("invalid_override: {e}"));
                None
            }
        },
        None => None,
    };
    let org_default_board = default_board
        .as_ref()
        .map(|(id, _)| OverrideValue::Board(*id));
    let (board_resolution, board_source) = registry::resolve(board_client_state, org_default_board);

    // Effective intake board: the resolved board while it still exists, else
    // the org default (a stale override is drift — issue + fallback).
    let effective_board_id: Option<Uuid> = match resolution_board_id(&board_resolution) {
        Some(id) => {
            if board_row(&mut *conn, org_id, id).await?.is_some() {
                Some(id)
            } else {
                board_issues.push("referenced_board_not_found".into());
                default_board.as_ref().map(|(id, _)| *id)
            }
        }
        None => default_board.as_ref().map(|(id, _)| *id),
    };
    if effective_board_id.is_none() {
        board_issues.push("org_has_no_default_board".into());
    }

    let mut entries = Vec::with_capacity(registry::declarations().len());
    for decl in registry::declarations() {
        let row = rows.iter().find(|r| r.override_key == decl.key);
        let mut issues: Vec<String> = Vec::new();
        let client_state = match row {
            Some(r) => match registry::parse_state(decl, &r.state, r.value.as_ref()) {
                Ok(s) => Some(s),
                Err(e) => {
                    issues.push(format!("invalid_override: {e}"));
                    None
                }
            },
            None => None,
        };

        let (org_default_value, resolution, source, detail) = if decl.key == board_decl.key {
            let detail = match effective_board_id {
                Some(id) => board_row(&mut *conn, org_id, id)
                    .await?
                    .map(|(_, name)| ResolvedDetail::Board { board_id: id, name }),
                None => None,
            };
            issues.extend(board_issues.iter().cloned());
            (
                org_default_board,
                board_resolution.clone(),
                board_source,
                detail,
            )
        } else {
            match decl.value_kind {
                registry::OverrideValueKind::StatusRef => {
                    resolve_status_entry(
                        &mut *conn,
                        org_id,
                        client_state,
                        effective_board_id,
                        &mut issues,
                    )
                    .await?
                }
                registry::OverrideValueKind::SlaPolicyRef => {
                    let org_default = default_policy
                        .as_ref()
                        .map(|(id, _)| OverrideValue::SlaPolicy(*id));
                    let (resolution, source) = registry::resolve(client_state, org_default);
                    let detail = match resolution_board_id(&resolution) {
                        Some(id) => match get_sla_policy(&mut *conn, org_id, id).await? {
                            Some((policy_id, name)) => {
                                Some(ResolvedDetail::SlaPolicy { policy_id, name })
                            }
                            None => {
                                issues.push("referenced_policy_not_found".into());
                                None
                            }
                        },
                        None => None,
                    };
                    (org_default, resolution, source, detail)
                }
                registry::OverrideValueKind::Toggle => {
                    let default_bool = match decl.org_default_setting {
                        Some(setting_key) => org_setting_bool(&mut *conn, org_id, setting_key)
                            .await?
                            .unwrap_or(true),
                        None => true,
                    };
                    let org_default = Some(OverrideValue::Toggle(default_bool));
                    let (resolution, source) = registry::resolve(client_state, org_default);
                    let enabled = match &resolution {
                        registry::Resolution::Set { value } => {
                            value.as_bool().unwrap_or(default_bool)
                        }
                        registry::Resolution::Disabled => false,
                        registry::Resolution::Unset => default_bool,
                    };
                    (
                        org_default,
                        resolution,
                        source,
                        Some(ResolvedDetail::Toggle { enabled }),
                    )
                }
                registry::OverrideValueKind::BoardRef => {
                    unreachable!("only the intake-board declaration uses BoardRef")
                }
            }
        };

        entries.push(OverrideEntry {
            key: decl.key.to_string(),
            label: decl.label.to_string(),
            description: decl.description.to_string(),
            value_kind: decl.value_kind,
            allow_disable: decl.allow_disable,
            resolution,
            source,
            org_default: org_default_value.map(|v| v.to_json()),
            client_override: row.map(row_provenance),
            detail,
            issues,
        });
    }

    Ok(ClientWorkflowResolution { client_id, entries })
}

fn db_str(e: crate::DbError) -> String {
    e.to_string()
}

fn resolution_board_id(res: &Resolution) -> Option<Uuid> {
    match res {
        Resolution::Set { value } => value.as_str().and_then(|s| Uuid::parse_str(s).ok()),
        _ => None,
    }
}

fn row_provenance(r: &OverrideRow) -> registry::ClientOverrideRow {
    registry::ClientOverrideRow {
        state: r.state.clone(),
        value: r.value.clone(),
        changed_by: r.updated_by,
        updated_at: r.updated_at,
    }
}

/// Resolves `workflow.intake_status`: a client override wins only while it
/// still names an entry status ON the effective intake board; otherwise the
/// effective board's default entry status applies (documented fallback).
async fn resolve_status_entry(
    conn: &mut PgConnection,
    org_id: Uuid,
    client_state: Option<OverrideState>,
    effective_board_id: Option<Uuid>,
    issues: &mut Vec<String>,
) -> DbResult<(
    Option<OverrideValue>,
    Resolution,
    ResolutionSource,
    Option<ResolvedDetail>,
)> {
    let Some(board_id) = effective_board_id else {
        issues.push("no_effective_board".into());
        return Ok((None, Resolution::Unset, ResolutionSource::Unset, None));
    };

    // A client status override is honored only while it still names a valid
    // entry status on the effective board; otherwise it falls back with an
    // issue (drift-safety, never silently misroutes intake).
    if let Some(OverrideState::Set(OverrideValue::Status(id))) = client_state {
        match get_status(conn, org_id, id).await? {
            Some(status) if status.board_id == board_id && status.is_entry => {
                let (resolution, source) =
                    registry::resolve(Some(OverrideState::Set(OverrideValue::Status(id))), None);
                return Ok((
                    None,
                    resolution,
                    source,
                    Some(ResolvedDetail::Status {
                        status_id: status.id,
                        name: status.name,
                        category: status.category,
                        board_id: status.board_id,
                    }),
                ));
            }
            Some(status) if status.board_id != board_id => {
                issues.push(format!("status_not_on_board: {}", status.name));
            }
            Some(status) => issues.push(format!("status_not_entry: {}", status.name)),
            None => issues.push("referenced_status_not_found".into()),
        }
    }

    // Inherit path: the effective board's default entry status.
    let org_default = default_entry_status(conn, org_id, board_id)
        .await?
        .map(|s| OverrideValue::Status(s.id));
    let (resolution, source) = registry::resolve(None, org_default);
    let detail = match (&resolution, org_default.as_ref().copied()) {
        (Resolution::Set { .. }, Some(OverrideValue::Status(id))) => {
            let status = get_status(conn, org_id, id).await?;
            status.map(|s| ResolvedDetail::Status {
                status_id: s.id,
                name: s.name,
                category: s.category,
                board_id: s.board_id,
            })
        }
        _ => None,
    };
    Ok((org_default, resolution, source, detail))
}
