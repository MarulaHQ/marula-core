//! Customization-manifest loader (TASKLIST U5, upgrade-compatibility-policy
//! .md §5).
//!
//! Purpose: assemble the org's canonical [`CustomizationManifest`] — every
//! customization section in one read — with DETERMINISTIC ordering so two
//! consecutive dumps of an unchanged org are byte-identical apart from
//! `generated_at` (golden stability contract, verified by
//! `core-db/tests/manifest_queries.rs`).
//!
//! Ordering rules: every array is sorted by stable SEMANTIC keys (names,
//! entity kinds, sort orders, category positions); row ids serve only as
//! tiebreakers, so a manifest re-imported into a fresh org (new ids) still
//! compares equal section-for-section.
//!
//! Secret discipline: the queries here deliberately CANNOT represent secret
//! material — `webhooks.secret_encrypted` and `api_keys.key_hash` are never
//! selected, and the section types (core-domain `entities::manifest`) have
//! no field that could carry them. "No secrets ever in output" is enforced
//! by construction, not by filtering at the edge.
//!
//! Non-purpose: no operational data (ticket bodies, field VALUES, delivery
//! ledgers — D1 owns the data plane), no user-scope settings (per-user
//! preferences are not org customization), no writes (read-only by design;
//! this module never needs a migration).

use crate::{DbError, DbResult};
use core_domain::entities::manifest::{
    CustomizationManifest, ManifestApiKey, ManifestAutomationRule, ManifestBoard,
    ManifestBoardStatus, ManifestClientWorkflowOverride, ManifestCustomField,
    ManifestEscalationStep, ManifestReportDefinition, ManifestSections, ManifestSetting,
    ManifestSla, ManifestSlaCalendar, ManifestSlaPolicy, ManifestRole, ManifestTaxonomy,
    ManifestTaxonomyCategory, ManifestTaxonomyType, ManifestTransition, ManifestWebhook,
};
use core_domain::status::TicketStatusCategory;
use sqlx::PgConnection;
use std::str::FromStr;
use uuid::Uuid;

/// Envelope version. Bump ONLY for a breaking section reshape; additive
/// section growth keeps this at 1 (documented evolution contract).
pub const MANIFEST_VERSION: u32 = 1;

/// Loads the org's full customization manifest. Run inside a transaction
/// with `set_org` applied so RLS scopes every query to the tenant (the
/// explicit `org_id` predicates are belt-and-suspenders, matching the rest
/// of the query layer).
///
/// `generated_at` is injected by the caller so the API layer and future
/// offline tools (CLI preflight, fixture capture) control the stamp.
pub async fn load_customization_manifest(
    conn: &mut PgConnection,
    org_id: Uuid,
    generated_at: chrono::DateTime<chrono::Utc>,
) -> DbResult<CustomizationManifest> {
    let sections = ManifestSections {
        custom_fields: load_custom_fields(&mut *conn, org_id).await?,
        boards: load_boards(&mut *conn, org_id).await?,
        automation_rules: load_automation_rules(&mut *conn, org_id).await?,
        webhooks: load_webhooks(&mut *conn, org_id).await?,
        api_keys: load_api_keys(&mut *conn, org_id).await?,
        settings: load_org_settings(&mut *conn, org_id).await?,
        sla: load_sla(&mut *conn, org_id).await?,
        report_definitions: load_report_definitions(&mut *conn, org_id).await?,
        taxonomy: load_taxonomy(&mut *conn, org_id).await?,
        client_workflow_overrides: load_client_workflow_overrides(&mut *conn, org_id).await?,
        roles: load_roles(&mut *conn, org_id).await?,
    };
    Ok(CustomizationManifest {
        manifest_version: MANIFEST_VERSION,
        org_id,
        generated_at,
        sections,
    })
}

fn json_err(field: &str, e: serde_json::Error) -> DbError {
    DbError::Sql(sqlx::Error::ColumnDecode {
        index: field.into(),
        source: Box::new(e),
    })
}

// --- custom fields -------------------------------------------------------------

/// Definitions only — the VALUES rows are operational data (D1), not
/// customization. Archived definitions are included: a preflight must see
/// constructs whose shaped values may still exist.
async fn load_custom_fields(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ManifestCustomField>> {
    let rows = sqlx::query!(
        r#"SELECT id, entity, name, kind, config, required, sort_order, archived_at
           FROM custom_field_defs WHERE org_id = $1"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut out: Vec<ManifestCustomField> = rows
        .into_iter()
        .map(|r| ManifestCustomField {
            id: r.id,
            entity: r.entity,
            name: r.name,
            kind: r.kind,
            config: r.config,
            required: r.required,
            sort_order: r.sort_order,
            archived_at: r.archived_at,
        })
        .collect();
    out.sort_by(|a, b| (&a.entity, &a.name, a.id).cmp(&(&b.entity, &b.name, b.id)));
    Ok(out)
}

// --- boards / statuses / transitions --------------------------------------------

/// One row per board with its statuses and transitions grouped in memory.
/// An EMPTY transitions array is meaningful: the semantic-category floor
/// applies (workflow semantics contract, migration 0011).
async fn load_boards(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<ManifestBoard>> {
    let boards = sqlx::query!(
        r#"SELECT id, name, is_default, require_checklist_close
           FROM boards WHERE org_id = $1"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;

    let statuses = sqlx::query!(
        r#"SELECT id, board_id, name, category, sort_order, is_entry, color
           FROM board_statuses WHERE org_id = $1"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;

    let transitions = sqlx::query!(
        r#"SELECT board_id, from_status_id, to_status_id
           FROM board_status_transitions WHERE org_id = $1
           ORDER BY from_status_id, to_status_id"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;

    let mut out: Vec<ManifestBoard> = boards
        .into_iter()
        .map(|b| ManifestBoard {
            id: b.id,
            name: b.name,
            is_default: b.is_default,
            require_checklist_close: b.require_checklist_close,
            statuses: Vec::new(),
            transitions: Vec::new(),
        })
        .collect();
    // (name, id): names are unique per org, ids only break exotic ties.
    out.sort_by(|a, b| (&a.name, a.id).cmp(&(&b.name, b.id)));

    let mut by_board: std::collections::HashMap<Uuid, Vec<ManifestBoardStatus>> =
        std::collections::HashMap::new();
    for s in statuses {
        let category = TicketStatusCategory::from_str(&s.category).map_err(|_| {
            DbError::Sql(sqlx::Error::ColumnDecode {
                index: "category".into(),
                source: format!("unknown status category {s:?}").into(),
            })
        })?;
        by_board
            .entry(s.board_id)
            .or_default()
            .push(ManifestBoardStatus {
                id: s.id,
                name: s.name,
                category,
                sort_order: s.sort_order,
                is_entry: s.is_entry,
                color: s.color,
            });
    }
    for list in by_board.values_mut() {
        list.sort_by(|a, b| (a.sort_order, &a.name, a.id).cmp(&(b.sort_order, &b.name, b.id)));
    }
    for board in &mut out {
        if let Some(list) = by_board.remove(&board.id) {
            board.statuses = list;
        }
    }

    let mut transitions_by_board: std::collections::HashMap<Uuid, Vec<ManifestTransition>> =
        std::collections::HashMap::new();
    for t in transitions {
        transitions_by_board
            .entry(t.board_id)
            .or_default()
            .push(ManifestTransition {
                from_status_id: t.from_status_id,
                to_status_id: t.to_status_id,
            });
    }
    // SQL pre-ordered by (from_status_id, to_status_id); keep that canonical
    // id order per board — the adjacency is a SET, ids are its stable keys.
    for board in &mut out {
        if let Some(list) = transitions_by_board.remove(&board.id) {
            board.transitions = list;
        }
    }
    Ok(out)
}

// --- automations -----------------------------------------------------------------

async fn load_automation_rules(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ManifestAutomationRule>> {
    let rows = sqlx::query!(
        r#"SELECT id, board_id, name, trigger_event, conditions, actions, enabled
           FROM automation_rules WHERE org_id = $1"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut out: Vec<ManifestAutomationRule> = rows
        .into_iter()
        .map(|r| ManifestAutomationRule {
            id: r.id,
            board_id: r.board_id,
            name: r.name,
            trigger_event: r.trigger_event,
            conditions: r.conditions,
            actions: r.actions,
            enabled: r.enabled,
        })
        .collect();
    out.sort_by(|a, b| (&a.name, a.id).cmp(&(&b.name, b.id)));
    Ok(out)
}

// --- webhooks / api keys -----------------------------------------------------------

/// Deliberately does NOT select `webhooks.secret_encrypted`: the sealed
/// signing secret must never even enter the manifest assembly path.
async fn load_webhooks(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<ManifestWebhook>> {
    let rows = sqlx::query!(
        r#"SELECT id, url, event_types, enabled, created_by
           FROM webhooks WHERE org_id = $1"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut out: Vec<ManifestWebhook> = rows
        .into_iter()
        .map(|r| ManifestWebhook {
            id: r.id,
            url: r.url,
            event_types: r.event_types,
            enabled: r.enabled,
            created_by: r.created_by,
        })
        .collect();
    out.sort_by(|a, b| (&a.url, a.id).cmp(&(&b.url, b.id)));
    Ok(out)
}

/// Deliberately does NOT select `api_keys.key_hash`: the credential
/// verifier is secret-equivalent and has no place in the manifest.
async fn load_api_keys(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<ManifestApiKey>> {
    let rows = sqlx::query!(
        r#"SELECT id, name, prefix, scopes, created_by, created_at, last_used_at, revoked_at
           FROM api_keys WHERE org_id = $1"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    // `scopes` is JSONB (same widening the api_keys listing uses): it is a
    // JSON array of scope strings, NOT a Postgres text[] column.
    let mut out: Vec<ManifestApiKey> = rows
        .into_iter()
        .map(|r| ManifestApiKey {
            id: r.id,
            name: r.name,
            prefix: r.prefix,
            scopes: r
                .scopes
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            created_by: r.created_by,
            created_at: r.created_at,
            last_used_at: r.last_used_at,
            revoked_at: r.revoked_at,
        })
        .collect();
    out.sort_by(|a, b| (&a.name, a.id).cmp(&(&b.name, b.id)));
    Ok(out)
}

// --- settings -----------------------------------------------------------------------

/// Org-scope registry overrides only. User-scope rows are per-user
/// preferences, not org customization; `updated_by` is dropped (who last
/// touched a value is audit history, not the customization itself).
async fn load_org_settings(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ManifestSetting>> {
    let rows = sqlx::query!(
        r#"SELECT key, value FROM settings
           WHERE org_id = $1 AND scope = 'org'"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut out: Vec<ManifestSetting> = rows
        .into_iter()
        .map(|r| ManifestSetting {
            key: r.key,
            value: r.value,
        })
        .collect();
    out.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(out)
}

// --- SLA ------------------------------------------------------------------------------

async fn load_sla(conn: &mut PgConnection, org_id: Uuid) -> DbResult<ManifestSla> {
    let cal_rows = sqlx::query!(
        r#"SELECT c.id, c.name,
                  jsonb_build_object('name', c.name, 'timezone', c.timezone,
                                     'windows', c.windows, 'holidays', c.holidays,
                                     'maintenance', c.maintenance) AS "calendar: serde_json::Value"
           FROM sla_calendars c WHERE c.org_id = $1"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut calendars: Vec<ManifestSlaCalendar> = cal_rows
        .into_iter()
        .map(|r| {
            // sqlx cannot prove jsonb_build_object non-null; NULL is
            // impossible (built from NOT NULL columns) and would surface
            // as a clean decode error if it ever were.
            let calendar_doc = r.calendar.unwrap_or(serde_json::Value::Null);
            Ok(ManifestSlaCalendar {
                id: r.id,
                name: r.name,
                calendar: serde_json::from_value(calendar_doc)
                    .map_err(|e| json_err("calendar", e))?,
            })
        })
        .collect::<DbResult<_>>()?;
    calendars.sort_by(|a, b| (&a.name, a.id).cmp(&(&b.name, b.id)));

    let policy_rows = sqlx::query!(
        r#"SELECT p.id, p.name, p.is_default, p.targets,
                  p.calendar AS "calendar: serde_json::Value", p.calendar_id
           FROM sla_policies p WHERE p.org_id = $1"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;

    let step_rows = sqlx::query!(
        r#"SELECT policy_id, step_index, trigger_kind, at_pct, notify_user_ids, enabled
           FROM sla_escalation_steps WHERE org_id = $1
           ORDER BY policy_id, step_index"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut steps_by_policy: std::collections::HashMap<Uuid, Vec<ManifestEscalationStep>> =
        std::collections::HashMap::new();
    for s in step_rows {
        steps_by_policy
            .entry(s.policy_id)
            .or_default()
            .push(ManifestEscalationStep {
                step_index: s.step_index,
                trigger_kind: s.trigger_kind,
                at_pct: s.at_pct,
                notify_user_ids: s.notify_user_ids,
                enabled: s.enabled,
            });
    }

    let mut policies: Vec<ManifestSlaPolicy> = policy_rows
        .into_iter()
        .map(|p| {
            Ok(ManifestSlaPolicy {
                id: p.id,
                name: p.name,
                is_default: p.is_default,
                targets: p.targets,
                calendar: serde_json::from_value(p.calendar)
                    .map_err(|e| json_err("calendar", e))?,
                calendar_id: p.calendar_id,
                escalation_steps: steps_by_policy.remove(&p.id).unwrap_or_default(),
            })
        })
        .collect::<DbResult<_>>()?;
    policies.sort_by(|a, b| (&a.name, a.id).cmp(&(&b.name, b.id)));

    Ok(ManifestSla {
        calendars,
        policies,
    })
}

// --- report definitions (TASKLIST R3) -------------------------------------------------

/// Stored report specs are customizations-as-data: the U6 preflight must
/// see every definition's view/viz/scope slugs and spec body so it can
/// flag deprecated-construct usage (policy §5). The body rides as the raw
/// jsonb document exactly like custom-field `config`; the slugs travel as
/// their own columns. Nothing secret-bearing exists on this table.
async fn load_report_definitions(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ManifestReportDefinition>> {
    let rows = sqlx::query!(
        r#"SELECT id, name, description, kind, spec, viz, scope, created_by, created_at, updated_at
           FROM report_definitions WHERE org_id = $1"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut out: Vec<ManifestReportDefinition> = rows
        .into_iter()
        .map(|r| ManifestReportDefinition {
            id: r.id,
            name: r.name,
            description: r.description,
            kind: r.kind,
            spec: r.spec,
            viz: r.viz,
            scope: r.scope,
            created_by: r.created_by,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect();
    out.sort_by(|a, b| (&a.name, a.id).cmp(&(&b.name, b.id)));
    Ok(out)
}

// --- ticket taxonomy (Wave 11 lane 3) ---------------------------------------------

/// Both classification trees, retired rows INCLUDED: tickets keep their
/// references after retirement, so the preflight must see the full
/// surface. Deterministic order: types by (name, id); categories by
/// (parent name, sort_order, name, id) — the parent's NAME is the stable
/// semantic group key, so a re-import into a fresh org (new ids) still
/// compares equal; ids only break ties.
async fn load_taxonomy(conn: &mut PgConnection, org_id: Uuid) -> DbResult<ManifestTaxonomy> {
    let types = sqlx::query!(
        r#"SELECT id, name, description, is_retired, sort_order
           FROM ticket_types WHERE org_id = $1"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut types: Vec<ManifestTaxonomyType> = types
        .into_iter()
        .map(|r| ManifestTaxonomyType {
            id: r.id,
            name: r.name,
            description: r.description,
            is_retired: r.is_retired,
            sort_order: r.sort_order,
        })
        .collect();
    types.sort_by(|a, b| (&a.name, a.id).cmp(&(&b.name, b.id)));

    let categories = sqlx::query!(
        r#"SELECT c.id, c.parent_id, c.name, c.description, c.is_retired, c.sort_order,
                  p.name AS "parent_name?"
           FROM ticket_categories c
           LEFT JOIN ticket_categories p ON p.id = c.parent_id
           WHERE c.org_id = $1"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut categorized: Vec<(String, ManifestTaxonomyCategory)> = categories
        .into_iter()
        .map(|r| {
            (
                r.parent_name.unwrap_or_default(),
                ManifestTaxonomyCategory {
                    id: r.id,
                    parent_id: r.parent_id,
                    name: r.name,
                    description: r.description,
                    is_retired: r.is_retired,
                    sort_order: r.sort_order,
                },
            )
        })
        .collect();
    categorized.sort_by(|a, b| {
        (&a.0, a.1.sort_order, &a.1.name, a.1.id).cmp(&(&b.0, b.1.sort_order, &b.1.name, b.1.id))
    });
    let categories = categorized.into_iter().map(|(_, c)| c).collect();

    Ok(ManifestTaxonomy { types, categories })
}

// --- client workflow overrides (Wave 12 lane 5) -------------------------------------

/// Every client-scoped override of the org workflow defaults, rows verbatim.
/// Customizations-as-data (§4.5/§5): the override registry is the closed key
/// set and the preflight must see which client references which construct.
/// Deterministic order: (client_name, override_key, client_id) — the client's
/// NAME is the stable semantic group key, so a re-import into a fresh org
/// (new ids) still compares equal; ids only break ties.
async fn load_client_workflow_overrides(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<ManifestClientWorkflowOverride>> {
    let rows = sqlx::query!(
        r#"SELECT o.client_id, c.name AS "client_name!", o.override_key, o.state, o.value
           FROM client_workflow_overrides o
           JOIN clients c ON c.org_id = o.org_id AND c.id = o.client_id
           WHERE o.org_id = $1"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut out: Vec<ManifestClientWorkflowOverride> = rows
        .into_iter()
        .map(|r| ManifestClientWorkflowOverride {
            client_id: r.client_id,
            client_name: r.client_name,
            key: r.override_key,
            state: r.state,
            value: r.value,
        })
        .collect();
    out.sort_by(|a, b| {
        (&a.client_name, &a.key, a.client_id).cmp(&(&b.client_name, &b.key, b.client_id))
    });
    Ok(out)
}

// --- roles (Wave 13 lane 6, RB1) --------------------------------------------------

/// ORG-OWNED role rows only: custom roles and forked copies. Shipped
/// preset rows are default state (canonical + restorable), never
/// customization, so the preflight sees no preset drift here.
async fn load_roles(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<ManifestRole>> {
    let rows = sqlx::query!(
        r#"SELECT id, name, builtin, is_system, forked_from, permissions
           FROM roles
           WHERE org_id = $1 AND NOT (is_system AND builtin IS NOT NULL)"#,
        org_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut out: Vec<ManifestRole> = rows
        .into_iter()
        .map(|r| ManifestRole {
            id: r.id,
            name: r.name,
            builtin: r.builtin,
            is_system: r.is_system,
            forked_from: r.forked_from,
            permissions: r.permissions,
        })
        .collect();
    out.sort_by(|a, b| (&a.name, a.id).cmp(&(&b.name, b.id)));
    Ok(out)
}
