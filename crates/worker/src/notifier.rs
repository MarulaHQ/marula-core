//! worker/notifier — email-channel fan-out for notification events.
//!
//! Purpose: consume `notification.created` events from the Redis stream
//! and drive the EMAIL leg of each recipient's preference chain: resolve
//! the effective preferences (user → org default → built-in), evaluate
//! quiet hours against the wall clock of the window's timezone, and hand
//! allowed messages to core-api's internal enqueue endpoint — which renders
//! the branded wrapper and writes the outbound ledger (Wave 34 lane 8;
//! single-writer §4.1). Because notification mail rides the ledger, its
//! delivery state is visible on the email settings page and retries follow
//! the ledger's backoff instead of vanishing on a transport hiccup.
//!
//! Non-purpose: no database writes (the API is the single writer §4.1), no
//! in-app delivery (the hub already routes those events per-user), and no
//! rendering (core-api owns the branded wrapper).

use crate::consumer::{self, ConsumerSpec, Delivery, EntryHandler, EntryOutcome, PgLedger};
use core_domain::entities::{NotificationKind, User};
use core_domain::notifications::{decide_channel, Decision, DeliveryChannel, QuietHours};
use core_events::Envelope;
use core_mail::internal::{EnqueueEmailResponse, NotificationEmailRequest};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::watch;

/// The consumer group this leg owns, and its page size.
const CONSUMER_GROUP: &str = "notifier";
const COUNT: usize = 100;
const SPEC: ConsumerSpec = ConsumerSpec::new(CONSUMER_GROUP, COUNT);

/// Hands one allowed notification to core-api for ledgered delivery.
#[async_trait::async_trait]
pub trait NotificationEmailSink: Send + Sync {
    /// Returns the enqueue outcome (`queued`, `skipped_*`) or a transport
    /// error description worth logging.
    async fn enqueue(&self, req: NotificationEmailRequest) -> Result<String, String>;
}

/// HTTP sink: `POST /api/internal/email/notification` with the shared
/// internal token.
pub struct HttpNotificationSink {
    client: reqwest::Client,
    core_api_url: String,
    internal_token: Option<String>,
}

impl HttpNotificationSink {
    #[must_use]
    pub fn new(core_api_url: String, internal_token: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            core_api_url,
            internal_token,
        }
    }
}

#[async_trait::async_trait]
impl NotificationEmailSink for HttpNotificationSink {
    async fn enqueue(&self, req: NotificationEmailRequest) -> Result<String, String> {
        let token = self
            .internal_token
            .as_deref()
            .ok_or_else(|| "MARULA_INTERNAL_TOKEN is unset".to_string())?;
        let response = self
            .client
            .post(format!(
                "{}/api/internal/email/notification",
                self.core_api_url.trim_end_matches('/')
            ))
            .header("X-Internal-Token", token)
            .json(&req)
            .send()
            .await
            .map_err(|e| format!("enqueue call failed: {e}"))?
            .error_for_status()
            .map_err(|e| format!("enqueue rejected: {e}"))?
            .json::<EnqueueEmailResponse>()
            .await
            .map_err(|e| format!("enqueue response unreadable: {e}"))?;
        Ok(response.outcome)
    }
}

/// Extracts the actionable payload when an envelope is a
/// `notification.created`; everything else is ignored here.
pub fn match_notification_created(
    envelope: &Envelope,
) -> Option<core_domain::events::NotificationCreated> {
    if envelope.event_type != core_domain::events::types::NOTIFICATION_CREATED {
        return None;
    }
    serde_json::from_value(envelope.payload.clone()).ok()
}

/// The full fan-out decision for one event: quiet-hours-aware email plan
/// plus the resolved recipient (None when the user row vanished).
pub async fn plan_email(
    pool: &PgPool,
    envelope: &Envelope,
    event: &core_domain::events::NotificationCreated,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<(User, Decision)> {
    let kind: NotificationKind = event.kind.parse().ok()?;
    let user = core_db::users::get_user(pool, envelope.org_id, event.user_id)
        .await
        .ok()?;
    let pref = core_db::notifications::effective_preference(
        pool,
        envelope.org_id,
        event.user_id,
        kind.as_str(),
    )
    .await
    .ok()?;
    let quiet = QuietHours::parse(
        pref.quiet_from.as_deref(),
        pref.quiet_until.as_deref(),
        &pref.timezone,
    )
    .ok()
    .flatten();
    Some((
        user,
        decide_channel(
            DeliveryChannel::Email,
            kind,
            pref.email,
            quiet.as_ref(),
            now,
        ),
    ))
}

/// Runs the notifier until `shutdown` fires.
///
/// This leg used a bare `$` cursor and no group, so a restart started at the
/// live tail and an enqueue that failed was forgotten: the in-app inbox was
/// the only record. It now runs under its own group, which means a failed
/// enqueue stays in the pending list, is retried live after the consumer
/// policy's delay, and lands in `dead_events` with its envelope if it keeps
/// failing — the same posture as every other consumer in this crate.
pub async fn run_notifier(
    pool: PgPool,
    redis_url: String,
    sink: Arc<dyn NotificationEmailSink>,
    shutdown: watch::Receiver<bool>,
) {
    let handler = NotifierHandler {
        pool: pool.clone(),
        sink,
    };
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

struct NotifierHandler {
    pool: PgPool,
    sink: Arc<dyn NotificationEmailSink>,
}

impl EntryHandler for NotifierHandler {
    async fn handle(&self, batch: &[Delivery]) -> Vec<EntryOutcome> {
        let mut outcomes = Vec::with_capacity(batch.len());
        for delivery in batch {
            outcomes.push(self.deliver(&delivery.envelope).await);
        }
        outcomes
    }
}

impl NotifierHandler {
    async fn deliver(&self, envelope: &Envelope) -> EntryOutcome {
        let Some(event) = match_notification_created(envelope) else {
            return EntryOutcome::Settled;
        };
        let Some((_user, decision)) =
            plan_email(&self.pool, envelope, &event, chrono::Utc::now()).await
        else {
            // The recipient or the preference row is gone. That is data we
            // cannot act on and will not be able to later either.
            tracing::warn!(
                event_id = %envelope.id,
                "notification fan-out could not resolve recipient/prefs"
            );
            return EntryOutcome::Settled;
        };
        match decision {
            Decision::Disabled | Decision::QuietDeferred => {
                tracing::info!(
                    user_id = %event.user_id,
                    kind = %event.kind,
                    ?decision,
                    "email leg suppressed by preference chain"
                );
                EntryOutcome::Settled
            }
            Decision::Deliver => {
                let request = NotificationEmailRequest {
                    org_id: envelope.org_id,
                    user_id: event.user_id,
                    kind: event.kind.clone(),
                    title: event.title.clone(),
                    // The event carries the title; the wrapper supplies the
                    // greeting/sign-off/footer around it.
                    body: None,
                    ticket_id: event.ticket_id,
                };
                match self.sink.enqueue(request).await {
                    Ok(outcome) => {
                        tracing::info!(
                            user_id = %event.user_id,
                            ticket_id = ?event.ticket_id,
                            %outcome,
                            "notification email decided and enqueued"
                        );
                        EntryOutcome::Settled
                    }
                    Err(reason) => {
                        // core-api was unreachable or refused. The ledger
                        // insert is the durable record; retrying this entry is
                        // what gives the mail a second chance.
                        EntryOutcome::Retry(format!("notification email enqueue failed: {reason}"))
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_domain::entities::NotificationKind;
    use serde_json::json;
    use std::sync::Mutex;

    fn envelope(event_type: &str, payload: serde_json::Value) -> Envelope {
        Envelope {
            id: uuid::Uuid::now_v7(),
            event_type: event_type.to_string(),
            org_id: uuid::Uuid::now_v7(),
            entity: "notification".into(),
            entity_id: None,
            occurred_at: chrono::Utc::now(),
            payload_schema_ref: "core-domain/v1".into(),
            payload,
        }
    }

    #[test]
    fn matches_only_notification_created_events() {
        let good = envelope(
            core_domain::events::types::NOTIFICATION_CREATED,
            json!({ "user_id": uuid::Uuid::now_v7(), "kind": "mention", "title": "t" }),
        );
        assert!(match_notification_created(&good).is_some());

        let other = envelope("ticket.created", json!({}));
        assert!(match_notification_created(&other).is_none());

        let bad_payload = envelope(
            core_domain::events::types::NOTIFICATION_CREATED,
            json!({ "unexpected": true }),
        );
        assert!(match_notification_created(&bad_payload).is_none());
    }

    /// Records every enqueued request so tests can assert fan-out behavior
    /// without any real HTTP.
    #[derive(Clone, Default)]
    struct RecordingSink {
        requests: std::sync::Arc<Mutex<Vec<NotificationEmailRequest>>>,
    }

    impl RecordingSink {
        fn requests(&self) -> Vec<NotificationEmailRequest> {
            self.requests.lock().expect("mutex").clone()
        }
    }

    #[async_trait::async_trait]
    impl NotificationEmailSink for RecordingSink {
        async fn enqueue(&self, req: NotificationEmailRequest) -> Result<String, String> {
            self.requests.lock().expect("mutex").push(req);
            Ok("queued".to_string())
        }
    }

    #[tokio::test]
    async fn sink_receives_the_event_coordinates() {
        let sink = RecordingSink::default();
        let request = NotificationEmailRequest {
            org_id: uuid::Uuid::now_v7(),
            user_id: uuid::Uuid::now_v7(),
            kind: "mention".into(),
            title: "You were mentioned".into(),
            body: None,
            ticket_id: None,
        };
        let outcome = sink.enqueue(request.clone()).await.expect("enqueue");
        assert_eq!(outcome, "queued");
        let seen = sink.requests();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].title, "You were mentioned");
        assert_eq!(seen[0].org_id, request.org_id);
    }

    #[test]
    fn kind_strings_survive_the_round_trip_through_events() {
        for kind in NotificationKind::ALL {
            let event = core_domain::events::NotificationCreated {
                user_id: uuid::Uuid::now_v7(),
                kind: kind.as_str().to_string(),
                ticket_id: None,
                title: "x".into(),
            };
            let env = envelope(
                core_domain::events::types::NOTIFICATION_CREATED,
                serde_json::to_value(&event).unwrap(),
            );
            let parsed = match_notification_created(&env).expect("parses");
            assert_eq!(parsed.kind.parse::<NotificationKind>().unwrap(), kind);
        }
    }
}
