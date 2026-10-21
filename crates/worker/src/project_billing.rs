//! worker/project_billing — milestone/fee releases into invoicing (Wave 39
//! lane 4, report P1/P4).
//!
//! Purpose: consume `project.milestone_completed` (and the phase-completion
//! companion) via a durable consumer group and stage one
//! `project_billing_releases` row per event — the ledger row IS the invoice
//! candidate. Money rides the event (amount + currency); scheduling (what
//! gets billed when) rides the invoice run over the release's window. No
//! event writes here: the ledger row is durable state the run consumes.
//!
//! Exactly-once: the ledger's partial unique index
//! `(project_id, phase_id) WHERE status <> 'cancelled'` absorbs an at-
//! least-once redelivery (transactional INSERT … ON CONFLICT DO UPDATE,
//! and the row simply re-reads). A crash between stream read and insert
//! leaves the entry unacked, so the group redelivers — the seatbelt is
//! the ledger, not the transport.
//!
//! Non-purpose: no phase auto-completion, no contract lookups (the
//! project→contract link resolves at evaluation), no invoice-row writes
//! (the run owns every invoice document).

use crate::consumer::{self, ConsumerSpec, Delivery, EntryHandler, EntryOutcome, PgLedger};
use core_domain::events::types;
use core_events::Envelope;
use sqlx::PgPool;
use tokio::sync::watch;

const CONSUMER_GROUP: &str = "project_billing";
const COUNT: usize = 50;
const SPEC: ConsumerSpec = ConsumerSpec::new(CONSUMER_GROUP, COUNT);

/// Parses the released milestone out of an envelope. Only
/// `project.milestone_completed` carries release money; informational
/// milestones (`amount: None`) return `None` and are acked as no-ops.
fn release_of(envelope: &Envelope) -> Option<StagedRelease> {
    if envelope.event_type != types::PROJECT_MILESTONE_COMPLETED {
        return None;
    }
    let payload = &envelope.payload;
    let amount = payload.get("amount")?;
    let minor = amount.get("minor")?.as_i64()?;
    let currency = amount.get("currency")?.as_str()?.to_string();
    let project_id = payload
        .get("project_id")
        .and_then(|v| v.as_str())
        .and_then(|v| uuid::Uuid::parse_str(v).ok())?;
    let phase_id = payload
        .get("phase_id")
        .and_then(|v| v.as_str())
        .and_then(|v| uuid::Uuid::parse_str(v).ok());
    let task_id = payload
        .get("task_id")
        .and_then(|v| v.as_str())
        .and_then(|v| uuid::Uuid::parse_str(v).ok());
    let name = payload.get("name")?.as_str()?.to_string();
    Some(StagedRelease {
        org_id: envelope.org_id,
        project_id,
        phase_id,
        task_id,
        label: name,
        amount_minor: minor,
        currency,
        released_on: envelope.occurred_at.date_naive(),
        source_event_id: envelope.id,
    })
}

pub struct StagedRelease {
    pub org_id: uuid::Uuid,
    pub project_id: uuid::Uuid,
    pub phase_id: Option<uuid::Uuid>,
    pub task_id: Option<uuid::Uuid>,
    pub label: String,
    pub amount_minor: i64,
    pub currency: String,
    pub released_on: chrono::NaiveDate,
    pub source_event_id: uuid::Uuid,
}

/// Runs the milestone-release consumer until shutdown. The harness owns the
/// group, the per-replica name, recovery, claiming, the live retry and the
/// dead-letter queue; this module decides what a completed milestone means.
pub async fn run_project_billing(pool: PgPool, redis_url: String, shutdown: watch::Receiver<bool>) {
    let handler = ReleaseHandler { pool: pool.clone() };
    let ledger = PgLedger::new(pool);
    let mut shutdown = shutdown;
    consumer::run(
        &redis_url,
        &SPEC,
        consumer::consumer_name(CONSUMER_GROUP),
        &ledger,
        &handler,
        &mut shutdown,
    )
    .await;
}

/// Stages the ledger row for one page of completion events.
struct ReleaseHandler {
    pool: PgPool,
}

impl EntryHandler for ReleaseHandler {
    async fn handle(&self, batch: &[Delivery]) -> Vec<EntryOutcome> {
        let mut outcomes = Vec::with_capacity(batch.len());
        for delivery in batch {
            outcomes.push(self.stage_one(&delivery.envelope).await);
        }
        outcomes
    }
}

impl ReleaseHandler {
    async fn stage_one(&self, envelope: &Envelope) -> EntryOutcome {
        let Some(release) = release_of(envelope) else {
            // Not our event, or an informational milestone: settle.
            return EntryOutcome::Settled;
        };
        match stage(&self.pool, &release).await {
            Ok(release_id) => {
                tracing::info!(
                    event_id = %envelope.id,
                    %release_id,
                    "milestone release staged for invoicing"
                );
                EntryOutcome::Settled
            }
            Err(error) => {
                // The ledger's partial unique index keeps the retry inert
                // once the row exists, so retrying is always safe here.
                EntryOutcome::Retry(format!("release staging failed: {error}"))
            }
        }
    }
}

/// Inserts the ledger row through core-db's upsert (the partial unique
/// index makes redeliveries idempotent). Returns the release id.
async fn stage(pool: &PgPool, release: &StagedRelease) -> anyhow::Result<uuid::Uuid> {
    let row = core_db::projects::insert_release(
        pool,
        &core_db::projects::NewRelease {
            org_id: release.org_id,
            project_id: release.project_id,
            phase_id: release.phase_id,
            task_id: release.task_id,
            label: release.label.clone(),
            amount_minor: release.amount_minor,
            currency: release.currency.clone(),
            released_on: release.released_on,
            source_event_id: Some(release.source_event_id),
        },
    )
    .await?;
    Ok(row.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_events::Envelope;

    fn envelope(event_type: &str, payload: serde_json::Value) -> Envelope {
        Envelope {
            id: uuid::Uuid::now_v7(),
            event_type: event_type.into(),
            org_id: uuid::Uuid::now_v7(),
            entity: "project".into(),
            entity_id: None,
            occurred_at: chrono::Utc::now(),
            payload_schema_ref: "core-domain/v1".into(),
            payload,
        }
    }

    fn milestone(amount: Option<(i64, &str)>) -> serde_json::Value {
        serde_json::json!({
            "org_id": uuid::Uuid::now_v7(),
            "project_id": uuid::Uuid::now_v7(),
            "phase_id": uuid::Uuid::now_v7(),
            "task_id": null,
            "name": "Discovery milestone",
            "amount": amount.map(|(minor, currency)| serde_json::json!({
                "minor": minor, "currency": currency
            })),
            "billing_method": "milestone",
            "completed_at": chrono::Utc::now(),
        })
    }

    #[test]
    fn milestone_with_amount_stages_a_release() {
        let env = envelope(
            types::PROJECT_MILESTONE_COMPLETED,
            milestone(Some((150_000, "USD"))),
        );
        let release = release_of(&env).expect("informational-billed milestone parses");
        assert_eq!(release.amount_minor, 150_000);
        assert_eq!(release.currency, "USD");
        assert!(release.phase_id.is_some());
        assert!(release.task_id.is_none());
    }

    #[test]
    fn informational_milestone_stages_nothing() {
        let env = envelope(types::PROJECT_MILESTONE_COMPLETED, milestone(None));
        assert!(release_of(&env).is_none(), "no amount ⇒ no release");
    }

    #[test]
    fn other_event_types_staging_nothing() {
        let env = envelope(types::PROJECT_PHASE_COMPLETED, milestone(Some((1, "USD"))));
        assert!(release_of(&env).is_none());
        let env = envelope("ticket.created", milestone(Some((1, "USD"))));
        assert!(release_of(&env).is_none());
    }

    #[test]
    fn malformed_money_never_stages() {
        let mut payload = milestone(Some((1, "USD")));
        payload["amount"] = serde_json::json!({ "minor": "lots", "currency": "USD" });
        let env = envelope(types::PROJECT_MILESTONE_COMPLETED, payload);
        assert!(release_of(&env).is_none(), "bad minor ⇒ no release");
        let mut payload = milestone(Some((1, "USD")));
        payload["project_id"] = serde_json::json!("not-a-uuid");
        let env = envelope(types::PROJECT_MILESTONE_COMPLETED, payload);
        assert!(release_of(&env).is_none(), "bad project id ⇒ no release");
    }
}
