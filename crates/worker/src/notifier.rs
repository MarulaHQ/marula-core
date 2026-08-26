//! worker/notifier — email-channel fan-out for notification events.
//!
//! Purpose: consume `notification.created` events from the Redis stream
//! and drive the EMAIL leg of each recipient's preference chain: resolve
//! the effective preferences (user → org default → built-in), evaluate
//! quiet hours against the wall clock of the window's timezone, and hand
//! allowed messages to the `MailTransport` seam (shipped default is the
//! noop "none" mode; real adapters land with the email-in/out lane).
//!
//! Non-purpose: no database writes (the API is the single writer §4.1),
//! no in-app delivery (the hub already routes those events per-user), and
//! no durable retry — at-least-once email lands with the Stage-2 job queue,
//! so this consumer is best-effort by design and never blocks the relay.

use core_domain::entities::{NotificationKind, User};
use core_domain::notifications::{
    decide_channel, Decision, DeliveryChannel, EmailMessage, MailError, MailTransport, QuietHours,
};
use core_events::{Envelope, EVENT_STREAM};
use redis::streams::{StreamReadOptions, StreamReadReply};
use redis::AsyncCommands;
use sqlx::PgPool;
use tokio::sync::watch;

/// Blocking-XREAD wait per poll; bounds shutdown latency.
const BLOCK_MS: usize = 2_000;
const COUNT: usize = 100;
const MAX_BACKOFF_MS: u64 = 5_000;

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
pub async fn run_notifier<T: MailTransport>(
    pool: PgPool,
    redis_url: String,
    transport: T,
    mut shutdown: watch::Receiver<bool>,
) {
    let client = match redis::Client::open(redis_url.as_str()) {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(%error, "invalid redis url; notifier exiting");
            return;
        }
    };
    let mut backoff_ms = 100_u64;
    loop {
        if *shutdown.borrow() {
            return;
        }
        match client.get_multiplexed_async_connection().await {
            Ok(mut conn) => {
                backoff_ms = 100;
                let broke = follow_stream(&pool, &mut conn, &transport, &mut shutdown).await;
                if broke.is_none() {
                    return;
                }
                tracing::warn!("notifier stream connection ended; reconnecting");
            }
            Err(error) => {
                tracing::error!(%error, "notifier redis connect failed");
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
        backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
    }
}

async fn follow_stream<T: MailTransport>(
    pool: &PgPool,
    conn: &mut redis::aio::MultiplexedConnection,
    transport: &T,
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
                                handle_entry(pool, transport, &entry).await;
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

async fn handle_entry<T: MailTransport>(
    pool: &PgPool,
    transport: &T,
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
    let Some((user, decision)) = plan_email(pool, &envelope, &event, now).await else {
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
            let message = EmailMessage {
                to: user.email.clone(),
                subject: event.title.clone(),
                text_body: format!(
                    "You have a new {} notification in Meridian.\n\n{}",
                    event.kind, event.title
                ),
            };
            match transport.send(message).await {
                Ok(()) => tracing::info!(
                    user_id = %event.user_id,
                    ticket_id = ?event.ticket_id,
                    "email notification handed to transport"
                ),
                Err(MailError::Unavailable | MailError::Rejected(_)) => {
                    // Best-effort v1: log and move on; retries land with
                    // the job queue.
                    tracing::warn!(user_id = %event.user_id, "email transport refused message");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_domain::entities::NotificationKind;
    use core_domain::notifications::{EmailMessage, MailError};
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

    /// Records every handed message so tests can assert fan-out behavior
    /// without any real transport.
    #[derive(Clone, Default)]
    struct RecordingTransport {
        sent: std::sync::Arc<Mutex<Vec<EmailMessage>>>,
    }

    impl RecordingTransport {
        fn messages(&self) -> Vec<EmailMessage> {
            self.sent.lock().expect("mutex").clone()
        }
    }

    impl MailTransport for RecordingTransport {
        async fn send(&self, message: EmailMessage) -> Result<(), MailError> {
            self.sent.lock().expect("mutex").push(message);
            Ok(())
        }
    }

    #[tokio::test]
    async fn transport_receives_allowed_messages() {
        let transport = RecordingTransport::default();
        transport
            .send(EmailMessage {
                to: "a@b.c".into(),
                subject: "s".into(),
                text_body: "b".into(),
            })
            .await
            .expect("send");
        let messages = transport.messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].to, "a@b.c");
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
