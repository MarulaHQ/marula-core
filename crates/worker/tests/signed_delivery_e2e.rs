//! Signed webhook delivery: the receiver-side verification a connector ships,
//! plus the production sink's refusal of a destination it must not dial.
//!
//! Purpose: prove the OUTBOUND contract without any Marula internals — the same
//! signature, timestamp binding and replay rejection over actual wire bytes —
//! and prove the public sink end does not deliver to a loopback or metadata
//! target. The wire half of the sink (headers and body as they arrive) is driven
//! from `webhook_out::tests`, where the private transport function lives: the
//! guard refuses a loopback URL before a socket exists, so an integration test
//! cannot point the guarded sink at a local listener and must not be able to
//! skip the guard to do it.
//!
//! Non-purpose: no database, no queue, no core-api — those layers are covered
//! by db_suite (ledger) and unit tests (settle matrix).

use worker::webhook_out::{DeliverySink, ReqwestSink, SinkResult};

fn envelope_body() -> Vec<u8> {
    serde_json::json!({
        "id": uuid::Uuid::now_v7(),
        "type": "ticket.created",
        "org_id": uuid::Uuid::now_v7(),
        "entity": "ticket",
        "entity_id": uuid::Uuid::now_v7(),
        "occurred_at": chrono::Utc::now().to_rfc3339(),
        "payload_schema_ref": "core-domain/v1",
        "payload": {"ticket_id": uuid::Uuid::now_v7().to_string()},
    })
    .to_string()
    .into_bytes()
}

const SECRET: &[u8] = b"whsec_e2e_lane_secret";

/// THE PROPERTY for the public edge: a destination the egress guard refuses is
/// never dialled. The sink reports a transport failure, which the delivery
/// ledger records and the retry policy judges like any other unreachable
/// receiver.
#[tokio::test]
async fn the_production_sink_refuses_a_loopback_destination() {
    let body = envelope_body();
    let now = chrono::Utc::now().timestamp();
    let signature = core_domain::webhooks::signature_value(SECRET, &body, now);

    for url in [
        // A loopback literal, but also a plaintext scheme and a port: every
        // one of the three is a refusal on its own.
        "http://127.0.0.1:9099/marula/hook".to_string(),
        // The spelling that defeated the old hand-rolled guard: userinfo in the
        // authority, metadata address behind it.
        "https://user:pw@169.254.169.254/latest/meta-data/".to_string(),
    ] {
        let result = ReqwestSink::new().post(&url, &signature, now, &body).await;
        assert!(
            matches!(result, SinkResult::Transport(ref reason) if reason.contains("refused before the request")),
            "{url} must be refused before any request: {result:?}"
        );
    }
}

#[tokio::test]
async fn tampered_and_replayed_deliveries_are_rejected_by_receiver() {
    let body = envelope_body();
    let old_ts = chrono::Utc::now().timestamp() - core_domain::webhooks::REPLAY_TOLERANCE_SECS - 5;
    // An attacker captured this delivery five minutes ago...
    let captured = core_domain::webhooks::signature_value(SECRET, &body, old_ts);

    // ...and replays it verbatim now: window rejects it.
    assert_eq!(
        core_domain::webhooks::verify(
            SECRET,
            &body,
            &captured,
            chrono::Utc::now().timestamp(),
            core_domain::webhooks::REPLAY_TOLERANCE_SECS,
        ),
        Err(core_domain::webhooks::VerifyError::TimestampOutsideWindow),
        "replay outside the window must be rejected"
    );

    // Same-timestamp tamper: body swapped under a valid MAC.
    let fresh_ts = chrono::Utc::now().timestamp();
    let fresh_sig = core_domain::webhooks::signature_value(SECRET, &body, fresh_ts);
    let mut tampered = body.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0x20;
    assert_eq!(
        core_domain::webhooks::verify(
            SECRET,
            &tampered,
            &fresh_sig,
            fresh_ts,
            core_domain::webhooks::REPLAY_TOLERANCE_SECS,
        ),
        Err(core_domain::webhooks::VerifyError::SignatureMismatch),
        "tampered payload must fail MAC comparison"
    );

    // Wrong secret never verifies even byte-identical traffic.
    assert_eq!(
        core_domain::webhooks::verify(
            b"whsec_other",
            &body,
            &fresh_sig,
            fresh_ts,
            core_domain::webhooks::REPLAY_TOLERANCE_SECS,
        ),
        Err(core_domain::webhooks::VerifyError::SignatureMismatch),
    );
}
