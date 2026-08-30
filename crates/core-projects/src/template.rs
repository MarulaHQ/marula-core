//! Project template spec: versioned, additive-only JSON (customizations-
//! as-data, policy §5) plus the pure expansion plan the instantiate
//! endpoint executes inside one transaction.
//!
//! Spec shape (`spec_version` 1 — unknown fields tolerated, new fields
//! added, never removed or retyped):
//!
//! ```json
//! {
//!   "spec_version": 1,
//!   "billing_method": "milestone",            // optional, default non_billable
//!   "currency": "USD",                        // optional; required when amounts set
//!   "phases": [
//!     { "key": "discovery", "name": "Discovery",
//!       "is_milestone": false,
//!       "milestone_amount_minor": null,
//!       "tasks": [
//!         { "key": "kickoff", "title": "Kickoff", "depends_on": ["plan"] }
//!       ] }
//!   ],
//!   "tasks": [ { "key": "wrapup", "title": "Wrap-up", "phase": "discovery",
//!                "depends_on": ["kickoff"] } ]
//! }
//! ```
//!
//! Every task carries a stable string `key`; `depends_on` lists (and
//! cross-phase `deps` edges reference) those keys. `phase` places a
//! project-level task into a phase by key. Expansion validates uniqueness,
//! references, phase placement, and acyclicity BEFORE any row is written,
//! producing an ordered [`ExpansionPlan`] of inserts the route layer
//! executes transactionally.

use crate::deps::has_cycle;
use core_domain::DomainError;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub const TEMPLATE_SPEC_VERSION: u32 = 1;

/// Validated template spec (deserialized + checked).
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateSpec {
    pub billing_method: Option<String>,
    pub currency: Option<String>,
    /// Phases in spec order.
    pub phases: Vec<TemplatePhase>,
    /// Project-level tasks (may reference a `phase` key).
    pub tasks: Vec<TemplateTask>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TemplatePhase {
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub sort_order: i32,
    pub is_milestone: bool,
    pub milestone_amount_minor: Option<i64>,
    pub milestone_currency: Option<String>,
    /// Tasks declared INSIDE this phase block.
    pub tasks: Vec<TemplateTask>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateTask {
    pub key: String,
    pub title: String,
    pub description: Option<String>,
    pub sort_order: i32,
    pub assignee_key: Option<String>,
    pub is_milestone: bool,
    pub milestone_amount_minor: Option<i64>,
    pub milestone_currency: Option<String>,
    pub due_offset_days: Option<i64>,
    pub depends_on: Vec<String>,
    /// Phase key for project-level tasks; `None` = no phase.
    pub phase: Option<String>,
}

// --- raw serde shapes (the additive wire contract) ------------------------------

#[derive(Debug, Deserialize)]
struct RawSpec {
    #[serde(default = "default_spec_version")]
    spec_version: u32,
    #[serde(default)]
    billing_method: Option<String>,
    #[serde(default)]
    currency: Option<String>,
    #[serde(default)]
    phases: Vec<RawPhase>,
    #[serde(default)]
    tasks: Vec<RawTask>,
    /// Cross-phase edges by task key (additive alternative to inline
    /// `depends_on` lists; both may be used).
    #[serde(default)]
    deps: Vec<RawDep>,
}

fn default_spec_version() -> u32 {
    TEMPLATE_SPEC_VERSION
}

#[derive(Debug, Clone, Deserialize)]
struct RawPhase {
    key: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    sort_order: i32,
    #[serde(default)]
    is_milestone: bool,
    #[serde(default)]
    milestone_amount_minor: Option<i64>,
    #[serde(default)]
    milestone_currency: Option<String>,
    #[serde(default)]
    tasks: Vec<RawTask>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawTask {
    key: String,
    title: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    sort_order: i32,
    /// Resolved at instantiation to a user id by the route layer
    /// (`assignee_keys` map); unresolvable keys fail validation.
    #[serde(default)]
    assignee_key: Option<String>,
    #[serde(default)]
    is_milestone: bool,
    #[serde(default)]
    milestone_amount_minor: Option<i64>,
    #[serde(default)]
    milestone_currency: Option<String>,
    /// Days after project start for the task's `due_at` (`None` = no due).
    #[serde(default)]
    due_offset_days: Option<i64>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    phase: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawDep {
    task: String,
    depends_on: String,
}

/// Parses + validates a template spec JSON document. Rejections name the
/// exact offending key so authors fix the template, not the instantiated
/// project.
pub fn parse_spec(spec: &serde_json::Value) -> Result<TemplateSpec, DomainError> {
    let raw: RawSpec = serde_json::from_value(spec.clone())
        .map_err(|e| DomainError::Validation(format!("invalid template spec: {e}")))?;
    if raw.spec_version != TEMPLATE_SPEC_VERSION {
        return Err(DomainError::Validation(format!(
            "unsupported template spec_version {} (this build understands {})",
            raw.spec_version, TEMPLATE_SPEC_VERSION
        )));
    }

    let mut phases: Vec<TemplatePhase> = Vec::with_capacity(raw.phases.len());
    let mut phase_keys: HashSet<String> = HashSet::new();
    let mut phase_index: HashMap<String, usize> = HashMap::new();
    for p in &raw.phases {
        if p.key.is_empty() {
            return Err(DomainError::Validation(
                "template phase key must not be empty".into(),
            ));
        }
        if !phase_keys.insert(p.key.clone()) {
            return Err(DomainError::Validation(format!(
                "duplicate phase key: {}",
                p.key
            )));
        }
        if p.name.trim().is_empty() {
            return Err(DomainError::Validation(format!(
                "phase {} must have a name",
                p.key
            )));
        }
        validate_amount(
            p.is_milestone,
            p.milestone_amount_minor,
            &p.milestone_currency,
            &raw.currency,
            &p.key,
        )?;
        phase_index.insert(p.key.clone(), phases.len());
        phases.push(TemplatePhase {
            key: p.key.clone(),
            name: p.name.clone(),
            description: p.description.clone(),
            sort_order: p.sort_order,
            is_milestone: p.is_milestone,
            milestone_amount_minor: p.milestone_amount_minor,
            milestone_currency: p.milestone_currency.clone(),
            tasks: Vec::new(),
        });
    }

    // Flatten phase-block tasks + project-level tasks into one list, each
    // annotated with its resolved phase index.
    struct Flat {
        task: TemplateTask,
        phase_index: Option<usize>,
    }
    let mut flat: Vec<Flat> = Vec::new();
    let mut task_keys: HashSet<String> = HashSet::new();

    fn push_task(
        raw_task: RawTask,
        phase_idx: Option<usize>,
        phase_keys: &HashSet<String>,
        flat: &mut Vec<Flat>,
        task_keys: &mut HashSet<String>,
        default_currency: &Option<String>,
    ) -> Result<(), DomainError> {
        if raw_task.key.is_empty() {
            return Err(DomainError::Validation(
                "template task key must not be empty".into(),
            ));
        }
        if !task_keys.insert(raw_task.key.clone()) {
            return Err(DomainError::Validation(format!(
                "duplicate task key: {}",
                raw_task.key
            )));
        }
        if raw_task.title.trim().is_empty() {
            return Err(DomainError::Validation(format!(
                "task {} must have a title",
                raw_task.key
            )));
        }
        if let Some(phase_key) = &raw_task.phase {
            if !phase_keys.contains(phase_key) {
                return Err(DomainError::Validation(format!(
                    "task {} references unknown phase {}",
                    raw_task.key, phase_key
                )));
            }
        }
        validate_amount(
            raw_task.is_milestone,
            raw_task.milestone_amount_minor,
            &raw_task.milestone_currency,
            default_currency,
            &raw_task.key,
        )?;
        flat.push(Flat {
            phase_index: phase_idx,
            task: TemplateTask {
                key: raw_task.key,
                title: raw_task.title,
                description: raw_task.description,
                sort_order: raw_task.sort_order,
                assignee_key: raw_task.assignee_key,
                is_milestone: raw_task.is_milestone,
                milestone_amount_minor: raw_task.milestone_amount_minor,
                milestone_currency: raw_task.milestone_currency,
                due_offset_days: raw_task.due_offset_days,
                depends_on: raw_task.depends_on,
                phase: raw_task.phase,
            },
        });
        Ok(())
    }

    // Phase-block tasks first (they carry the enclosing phase implicitly).
    for raw_phase in &raw.phases {
        let idx = phase_index.get(&raw_phase.key).copied();
        for t in &raw_phase.tasks {
            let mut t = t.clone();
            t.phase = None;
            push_task(
                t,
                idx,
                &phase_keys,
                &mut flat,
                &mut task_keys,
                &raw.currency,
            )?;
        }
    }
    for t in raw.tasks {
        push_task(
            t,
            None,
            &phase_keys,
            &mut flat,
            &mut task_keys,
            &raw.currency,
        )?;
    }

    // Merge explicit cross-phase dep edges into the inline lists.
    for dep in raw.deps {
        if !task_keys.contains(&dep.task) || !task_keys.contains(&dep.depends_on) {
            return Err(DomainError::Validation(format!(
                "dep edge {} -> {} references an unknown task key",
                dep.task, dep.depends_on
            )));
        }
        let entry = flat
            .iter_mut()
            .find(|f| f.task.key == dep.task)
            .expect("key membership checked above");
        if !entry.task.depends_on.contains(&dep.depends_on) {
            entry.task.depends_on.push(dep.depends_on);
        }
    }

    // Every dependency key must exist.
    for f in &flat {
        for dep in &f.task.depends_on {
            if !task_keys.contains(dep) {
                return Err(DomainError::Validation(format!(
                    "task {} depends on unknown task key {}",
                    f.task.key, dep
                )));
            }
        }
    }

    // Acyclicity over the whole declared graph (self-dep included).
    // Stable pseudo-ids for the pure graph check (never persisted): the
    // key's bytes hashed into a u128 — uuid v5 is not enabled in this
    // workspace.
    let key_to_uuid = |k: &str| {
        let mut h: u128 = 0x6d_65_72_69_64_69_61_6e_u128 << 64;
        for b in k.as_bytes() {
            h = (h ^ u128::from(*b)).wrapping_mul(0x1_0000_0000_0000_00b3_u128);
            h = h.rotate_left(7);
        }
        Uuid::from_u128(h | 1)
    };
    let edges: Vec<(Uuid, Uuid)> = flat
        .iter()
        .flat_map(|f| {
            f.task
                .depends_on
                .iter()
                .map(|d| (key_to_uuid(&f.task.key), key_to_uuid(d)))
        })
        .collect();
    if has_cycle(&edges) {
        return Err(DomainError::Validation(
            "template dependency graph contains a cycle".into(),
        ));
    }

    // Split tasks back out: phase-block tasks into their phase, the rest
    // into the project-level list (keeping declared order).
    let mut tasks: Vec<TemplateTask> = Vec::new();
    let mut phase_tasks: HashMap<usize, Vec<TemplateTask>> = HashMap::new();
    for f in flat {
        match f.phase_index {
            Some(idx) => phase_tasks.entry(idx).or_default().push(f.task),
            None => tasks.push(f.task),
        }
    }
    for (idx, list) in phase_tasks {
        phases[idx].tasks = list;
    }

    Ok(TemplateSpec {
        billing_method: raw.billing_method,
        currency: raw.currency,
        phases,
        tasks,
    })
}

fn validate_amount(
    is_milestone: bool,
    amount: Option<i64>,
    currency: &Option<String>,
    default_currency: &Option<String>,
    key: &str,
) -> Result<(), DomainError> {
    if let Some(minor) = amount {
        if minor < 0 {
            return Err(DomainError::Validation(format!(
                "task/phase {key}: milestone amount must not be negative"
            )));
        }
        let effective = currency.as_deref().or(default_currency.as_deref());
        if effective.is_none() {
            return Err(DomainError::Validation(format!(
                "task/phase {key}: milestone amount requires a currency \
                 (milestone_currency or the spec-level currency)"
            )));
        }
    } else if is_milestone && currency.is_some() {
        return Err(DomainError::Validation(format!(
            "task/phase {key}: milestone_currency set without an amount"
        )));
    }
    Ok(())
}

// --- expansion plan ---------------------------------------------------------------

/// One resolved phase insert.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedPhase {
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub sort_order: i32,
    pub is_milestone: bool,
    pub milestone_amount_minor: Option<i64>,
    pub milestone_currency: Option<String>,
}

/// One resolved task insert; `phase_key` is `Some` when the task lives in
/// a phase. Dependencies reference task KEYS — the route layer resolves
/// key → generated id as it inserts (deps are written in a second pass).
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedTask {
    pub key: String,
    pub phase_key: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub sort_order: i32,
    pub assignee_key: Option<String>,
    pub is_milestone: bool,
    pub milestone_amount_minor: Option<i64>,
    pub milestone_currency: Option<String>,
    pub due_offset_days: Option<i64>,
    pub depends_on: Vec<String>,
}

/// The fully-validated insert plan for instantiating a template.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpansionPlan {
    pub billing_method: Option<String>,
    pub currency: Option<String>,
    pub phases: Vec<PlannedPhase>,
    pub tasks: Vec<PlannedTask>,
}

impl TemplateSpec {
    /// Produces the insert plan. Pure: ids are generated by the caller.
    pub fn expand(&self) -> ExpansionPlan {
        ExpansionPlan {
            billing_method: self.billing_method.clone(),
            currency: self.currency.clone(),
            phases: self
                .phases
                .iter()
                .map(|p| PlannedPhase {
                    key: p.key.clone(),
                    name: p.name.clone(),
                    description: p.description.clone(),
                    sort_order: p.sort_order,
                    is_milestone: p.is_milestone,
                    milestone_amount_minor: p.milestone_amount_minor,
                    milestone_currency: p.milestone_currency.clone(),
                })
                .collect(),
            tasks: self
                .phases
                .iter()
                .flat_map(|p| {
                    p.tasks.iter().map(move |t| PlannedTask {
                        phase_key: Some(p.key.clone()),
                        ..PlannedTask::from_template(t)
                    })
                })
                .chain(self.tasks.iter().map(PlannedTask::from_template))
                .collect(),
        }
    }
}

impl PlannedTask {
    fn from_template(t: &TemplateTask) -> Self {
        Self {
            key: t.key.clone(),
            phase_key: t.phase.clone(),
            title: t.title.clone(),
            description: t.description.clone(),
            sort_order: t.sort_order,
            assignee_key: t.assignee_key.clone(),
            is_milestone: t.is_milestone,
            milestone_amount_minor: t.milestone_amount_minor,
            milestone_currency: t.milestone_currency.clone(),
            due_offset_days: t.due_offset_days,
            depends_on: t.depends_on.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn spec_round_trips_and_expands() {
        let spec = json!({
            "spec_version": 1,
            "billing_method": "milestone",
            "currency": "USD",
            "phases": [
                { "key": "discovery", "name": "Discovery", "sort_order": 0,
                  "tasks": [
                    { "key": "kickoff", "title": "Kickoff", "depends_on": ["plan"] },
                    { "key": "plan", "title": "Plan", "is_milestone": true,
                      "milestone_amount_minor": 150000 }
                  ] }
            ],
            "tasks": [
                { "key": "wrapup", "title": "Wrap-up", "phase": "discovery",
                  "depends_on": ["kickoff"] },
                { "key": "lessons", "title": "Lessons learned" }
            ],
            "deps": [ { "task": "lessons", "depends_on": "wrapup" } ]
        });
        let parsed = parse_spec(&spec).expect("valid spec");
        assert_eq!(parsed.phases.len(), 1);
        assert_eq!(parsed.phases[0].tasks.len(), 2);
        assert_eq!(parsed.tasks.len(), 2);
        // wrapup kept its phase placement, lessons gained the cross-edge.
        let wrapup = parsed
            .tasks
            .iter()
            .find(|t| t.key == "wrapup")
            .expect("wrapup task");
        assert_eq!(wrapup.phase.as_deref(), Some("discovery"));
        let lessons = parsed
            .tasks
            .iter()
            .find(|t| t.key == "lessons")
            .expect("lessons task");
        assert!(lessons.depends_on.contains(&"wrapup".to_string()));
        let plan = parsed.expand();
        assert_eq!(plan.phases.len(), 1);
        assert_eq!(plan.tasks.len(), 4);
        assert_eq!(plan.tasks[0].phase_key.as_deref(), Some("discovery"));
    }

    #[test]
    fn unknown_phase_reference_is_rejected() {
        let spec = json!({
            "spec_version": 1,
            "tasks": [ { "key": "a", "title": "A", "phase": "nope" } ]
        });
        let err = parse_spec(&spec).unwrap_err().to_string();
        assert!(err.contains("unknown phase"), "{err}");
    }

    #[test]
    fn duplicate_keys_are_rejected() {
        let spec = json!({
            "spec_version": 1,
            "tasks": [
                { "key": "a", "title": "A" },
                { "key": "a", "title": "A again" }
            ]
        });
        let err = parse_spec(&spec).unwrap_err().to_string();
        assert!(err.contains("duplicate task key"), "{err}");
    }

    #[test]
    fn cycles_are_rejected() {
        let spec = json!({
            "spec_version": 1,
            "tasks": [
                { "key": "a", "title": "A", "depends_on": ["b"] },
                { "key": "b", "title": "B", "depends_on": ["a"] }
            ]
        });
        let err = parse_spec(&spec).unwrap_err().to_string();
        assert!(err.contains("cycle"), "{err}");
    }

    #[test]
    fn amount_without_currency_is_rejected() {
        let spec = json!({
            "spec_version": 1,
            "tasks": [
                { "key": "a", "title": "A", "is_milestone": true,
                  "milestone_amount_minor": 100 }
            ]
        });
        let err = parse_spec(&spec).unwrap_err().to_string();
        assert!(err.contains("requires a currency"), "{err}");
    }

    #[test]
    fn wrong_spec_version_is_rejected() {
        let spec = json!({ "spec_version": 99 });
        let err = parse_spec(&spec).unwrap_err().to_string();
        assert!(err.contains("unsupported template spec_version"), "{err}");
    }
}
