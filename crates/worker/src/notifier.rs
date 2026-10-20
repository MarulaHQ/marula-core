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

use crate::reconnect::Reconnect;
use core_domain::entities::{NotificationKind, User};
use core_domain::notifications::{decide_channel, Decision, DeliveryChannel, QuietHours};
use core_events::{Envelope, EVENT_STREAM};
use core_mail::internal::{EnqueueEmailResponse, NotificationEmailRequest};
use redis::streams::{StreamReadOptions, StreamReadReply};
use redis::AsyncCommands;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::watch;

/// Blocking-XREAD wait per poll; bounds shutdown latency.
const BLOCK_MS: usize = 2_000;
const COUNT: usize = 100;

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

/// Runs the notifier until `shutdown` fires. Redis errors back off and
/// reconnect; malformed entries are logged and skipped so one bad event
/// never stalls the stream.
pub async fn run_notifier(
    pool: PgPool,
    redis_url: String,
    sink: Arc<dyn NotificationEmailSink>,
    mut shutdown: watch::Receiver<bool>,
) {
    let client = match redis::Client::open(redis_url.as_str()) {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(%error, "invalid redis url; notifier exiting");
            return;
        }
    };
    let mut backoff = Reconnect::consumer();
    loop {
        if *shutdown.borrow() {
            return;
        }
        match client.get_multiplexed_async_connection().await {
            Ok(mut conn) => {
                backoff.reset();
                let broke = follow_stream(&pool, &mut conn, sink.as_ref(), &mut shutdown).await;
                if broke.is_none() {
                    return;
                }
                tracing::warn!("notifier stream connection ended; reconnecting");
            }
            Err(error) => {
                tracing::error!(%error, "notifier redis connect failed");
            }
        }
        backoff.sleep().await;
    }
}

async fn follow_stream(
    pool: &PgPool,
    conn: &mut redis::aio::MultiplexedConnection,
    sink: &dyn NotificationEmailSink,
    shutdown: &mut watch::Receiver<bool>,
) -> Option<()> {
    // `$` starts at the live tail: restarts skip history by design; the
    // in-app inbox remains the durable record of anything missed.
    let mut cursor = "$".to_string();
    let options = StreamReadOptions::default().block(BLOCK_MS).count(COUNT);
    let keys = [EVENT_STREAM];
    loop {
        if *shutdown.borrow() {
            return None;
        }
        let ids = [cursor.as_str()];
        tokio::select! {
            _ = shutdown.changed() => return None,
            result = conn.xread_options::<_, _, StreamReadReply>(&keys, &ids, &options) => {
                match result {
                    Ok(reply) => {
                        for key in reply.keys {
                            for entry in key.ids {
                                cursor = entry.id.clone();
                                handle_entry(pool, sink, &entry).await;
                            }
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "notifier xread failed");
                        return Some(());
                    }
                }
            }
        }
    }
}

async fn handle_entry(
    pool: &PgPool,
    sink: &dyn NotificationEmailSink,
    entry: &redis::streams::StreamId,
) {
    let Some(raw) = entry.get::<String>("data") else {
        tracing::warn!(entry_id = %entry.id, "stream entry missing data field");
        return;
    };
    let Ok(envelope) = serde_json::from_str::<Envelope>(&raw) else {
        tracing::warn!(entry_id = %entry.id, "unparseable event");
        return;
    };
    let Some(event) = match_notification_created(&envelope) else {
        return;
    };
    let now = chrono::Utc::now();
    let Some((_user, decision)) = plan_email(pool, &envelope, &event, now).await else {
        tracing::warn!(event_id = %envelope.id, "notification fan-out could not resolve recipient/prefs");
        return;
    };
    match decision {
        Decision::Disabled | Decision::QuietDeferred => {
            tracing::info!(
                user_id = %event.user_id,
                kind = %event.kind,
                ?decision,
                "email leg suppressed by preference chain"
            );
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
            match sink.enqueue(request).await {
                Ok(outcome) => tracing::info!(
                    user_id = %event.user_id,
                    ticket_id = ?event.ticket_id,
                    %outcome,
                    "notification email decided and enqueued"
                ),
                Err(reason) => {
                    // Best-effort: log and move on; the in-app inbox is the
                    // durable record and the next event carries the state.
                    tracing::warn!(user_id = %event.user_id, %reason, "notification email enqueue failed");
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
