//! Plugin SURFACE registry persistence (Wave 34 lane 4, §4.6): the
//! `plugin_surface_entries` rows derived from a pinned manifest's
//! `surface_profile` — widgets, automation action kinds, report render
//! kinds, and custom-field validators.
//!
//! Non-purpose: profile validation (core-domain owns `validate()`);
//! consent decisions and sync orchestration (plugin-host + routes).

use crate::node_registry::PluginNodeEntry;
use crate::{DbError, DbResult};
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

/// One persisted surface entry, surface-shaped for the API and the web.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SurfaceEntryRow {
    pub id: Uuid,
    pub install_id: Uuid,
    pub surface: String,
    pub entry_key: String,
    pub label: String,
    pub definition: Value,
    pub enabled: bool,
}

fn parse_surface(value: String) -> DbResult<&'static str> {
    match value.as_str() {
        "widget" => Ok("widget"),
        "automation_action" => Ok("automation_action"),
        "report_render" => Ok("report_render"),
        "field_validator" => Ok("field_validator"),
        "automation_trigger" => Ok("automation_trigger"),
        "read" => Ok("read"),
        _ => Err(DbError::Conflict(format!(
            "unknown plugin surface kind {value:?}"
        ))),
    }
}

/// Replaces the install's entries for ONE surface with `entries`
/// (definition JSONs). Called per-section by the plugin-host sync so a
/// manifest upgrade rewrites exactly the section that changed.
///
/// A definition may carry a `node_contract` object (the A2 six-field
/// contract from Wave 43 lane 13): its five persistence fields are lifted
/// into the row's own columns, so a consumer reading them directly sees
/// the same facts the registry parses. Sections without one — widgets,
/// render kinds, validators, triggers — keep writing exactly what they
/// wrote before, and the columns fall back to the migration's
/// conservative defaults.
pub async fn sync_surface_section(
    tx: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    surface: &str,
    entries: &[(String, String, Value)],
) -> DbResult<()> {
    sqlx::query!(
        "DELETE FROM plugin_surface_entries
         WHERE org_id = $1 AND install_id = $2 AND surface = $3",
        org_id,
        install_id,
        surface,
    )
    .execute(&mut *tx)
    .await?;
    for (entry_key, label, definition) in entries {
        let contract = node_contract_columns(definition);
        sqlx::query!(
            r#"INSERT INTO plugin_surface_entries
                 (id, org_id, install_id, surface, entry_key, label, definition,
                  output_schema, idempotent, emits, audience, agent_description)
               VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb,
                       $8::jsonb, $9, $10::text[], $11, $12)"#,
            Uuid::now_v7(),
            org_id,
            install_id,
            surface,
            entry_key,
            label,
            definition,
            contract.output_schema,
            contract.idempotent,
            &contract.emits,
            contract.audience,
            contract.agent_description,
        )
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

/// The five contract columns a definition's `node_contract` declares, with
/// the conservative defaults the migration states: no output schema (so
/// nothing it returns is referenceable), retry-unsafe, no emits (so the
/// loop refusal can never link it to a wait) and human-only (so it stays
/// off the LLM tool catalogue until the plugin says otherwise).
struct NodeContractColumns {
    output_schema: Option<Value>,
    idempotent: bool,
    emits: Vec<String>,
    audience: String,
    agent_description: Option<String>,
}

fn node_contract_columns(definition: &Value) -> NodeContractColumns {
    let Some(contract) = definition.get("node_contract") else {
        return NodeContractColumns {
            output_schema: None,
            idempotent: false,
            emits: Vec::new(),
            audience: "human".to_string(),
            agent_description: None,
        };
    };
    NodeContractColumns {
        output_schema: contract
            .get("output_schema")
            .filter(|s| !s.is_null())
            .cloned(),
        idempotent: contract
            .get("idempotent")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        emits: contract
            .get("emits")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        audience: match contract.get("audience").and_then(Value::as_str) {
            Some("ai") => "ai",
            Some("both") => "both",
            _ => "human",
        }
        .to_string(),
        agent_description: contract
            .get("agent_description")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

/// Removes every entry the install registered (revoke / uninstall path).
pub async fn delete_install_entries(
    tx: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
) -> DbResult<u64> {
    let result = sqlx::query!(
        "DELETE FROM plugin_surface_entries WHERE org_id = $1 AND install_id = $2",
        org_id,
        install_id,
    )
    .execute(&mut *tx)
    .await?;
    Ok(result.rows_affected())
}

/// Lists the org's enabled entries for one surface, joined against the
/// install so the caller can enforce the enable state (an entry whose
/// install is disabled renders nowhere — enforced by the caller's join
/// filter, kept here as data only).
pub async fn list_entries<'e, E>(
    executor: E,
    org_id: Uuid,
    surface: &str,
) -> DbResult<Vec<SurfaceEntryRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT e.id, e.install_id, e.surface, e.entry_key, e.label,
                  e.definition, e.enabled
           FROM plugin_surface_entries e
           JOIN plugin_installs i ON i.id = e.install_id AND i.org_id = e.org_id
           WHERE e.org_id = $1 AND e.surface = $2 AND e.enabled
             AND i.status = 'enabled' AND i.revoked_at IS NULL
           ORDER BY e.entry_key"#,
        org_id,
        surface,
    )
    .fetch_all(executor)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(SurfaceEntryRow {
            id: r.id,
            install_id: r.install_id,
            surface: parse_surface(r.surface)?.to_string(),
            entry_key: r.entry_key,
            label: r.label,
            definition: r.definition,
            enabled: r.enabled,
        });
    }
    Ok(out)
}

/// Lists every entry of one install (install-detail / consent views).
pub async fn list_install_entries<'e, E>(
    executor: E,
    org_id: Uuid,
    install_id: Uuid,
) -> DbResult<Vec<SurfaceEntryRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, install_id, surface, entry_key, label, definition, enabled
           FROM plugin_surface_entries
           WHERE org_id = $1 AND install_id = $2
           ORDER BY surface, entry_key"#,
        org_id,
        install_id,
    )
    .fetch_all(executor)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(SurfaceEntryRow {
            id: r.id,
            install_id: r.install_id,
            surface: parse_surface(r.surface)?.to_string(),
            entry_key: r.entry_key,
            label: r.label,
            definition: r.definition,
            enabled: r.enabled,
        });
    }
    Ok(out)
}

/// Fetches one entry by key for invocation-time authorization
/// (widget render / action execute / validator apply).
pub async fn get_entry_by_key<'e, E>(
    executor: E,
    org_id: Uuid,
    surface: &str,
    entry_key: &str,
) -> DbResult<Option<SurfaceEntryRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let r = sqlx::query!(
        r#"SELECT e.id, e.install_id, e.surface, e.entry_key, e.label,
                  e.definition, e.enabled
           FROM plugin_surface_entries e
           JOIN plugin_installs i ON i.id = e.install_id AND i.org_id = e.org_id
           WHERE e.org_id = $1 AND e.surface = $2 AND e.entry_key = $3 AND e.enabled
             AND i.status = 'enabled' AND i.revoked_at IS NULL"#,
        org_id,
        surface,
        entry_key,
    )
    .fetch_optional(executor)
    .await?;
    r.map(|r| {
        Ok(SurfaceEntryRow {
            id: r.id,
            install_id: r.install_id,
            surface: parse_surface(r.surface)?.to_string(),
            entry_key: r.entry_key,
            label: r.label,
            definition: r.definition,
            enabled: r.enabled,
        })
    })
    .transpose()
}

/// The two surfaces whose entries are workflow NODES (A2, Wave 43 lane
/// 13). `automation_trigger` is deliberately absent: a plugin trigger is
/// an event type the connector declares through the signed ingest, which
/// the event-type registry makes subscribable. Rendering it as a node
/// would invent a second trigger path beside the trigger catalogue.
const NODE_SURFACES: [&str; 2] = ["automation_action", "read"];

/// Lists the org's enabled plugin-contributed workflow nodes, with the
/// five contract fields beside the entry. The caller parses them through
/// `core_domain::node_registry::PluginNodeContract`.
pub async fn list_node_entries<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<PluginNodeEntry>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT e.install_id, e.entry_key, e.surface, e.label, e.definition,
                  e.output_schema, e.idempotent, e.emits, e.audience,
                  e.agent_description
           FROM plugin_surface_entries e
           JOIN plugin_installs i ON i.id = e.install_id AND i.org_id = e.org_id
           WHERE e.org_id = $1 AND e.surface = ANY($2) AND e.enabled
             AND i.status = 'enabled' AND i.revoked_at IS NULL
           ORDER BY e.entry_key"#,
        org_id,
        &NODE_SURFACES as &[&str],
    )
    .fetch_all(executor)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(PluginNodeEntry {
            install_id: r.install_id,
            entry_key: r.entry_key,
            surface: parse_surface(r.surface)?.to_string(),
            label: r.label,
            definition: r.definition,
            output_schema: r.output_schema,
            idempotent: r.idempotent,
            emits: r.emits,
            audience: r.audience,
            agent_description: r.agent_description,
        });
    }
    Ok(out)
}
