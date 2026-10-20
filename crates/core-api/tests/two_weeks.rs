//! THE acceptance harness (the private build tracker 2.4 / Stage-2 anchor): "email-to-ticket
//! runs two simulated weeks with zero lost messages."
//!
//! Purpose: a deterministic, in-process, generator-driven simulation of 14
//! simulated days of email traffic driven through the REAL pipeline —
//! `core_api::email::process_inbound` for every inbound message (scan →
//! route → idempotency gate → threading → comment/new-ticket → events) and
//! the claim/transport/report loop for outbound (a flaky wrapper fails the
//! first attempt of every message, forcing the retry/backoff path).
//!
//! Non-purpose: no wall-clock sleeping (the queue clock is injected), no
//! real network (recording transport + scripted scanner), no UI.

mod support;

use core_mail::inbound::InboundEmail;
use support::{MailHarness, Scenario};

const DAYS: i64 = 14;

#[tokio::test]
async fn two_simulated_weeks_with_zero_lost_messages() {
    let h = MailHarness::boot().await;
    let mut sim: Scenario = h.scenario();

    // --- generate the two-week scenario ------------------------------------
    let senders = [
        "client@example.test",
        "vip@bigco.test",
        "nobody@stranger.test",
    ];
    let mut live_messages: Vec<String> = Vec::new();
    let mut expected_inbound = 0_usize; // unique Message-IDs delivered
    let mut expected_replays = 0_usize;
    let mut expected_quarantined = 0_usize;
    let mut expected_comments = 0_usize;
    let mut expected_tickets = 0_usize;
    let mut expected_outbound = 0_usize;
    let mut expected_staff_replies = 0_usize;

    for day in 0..DAYS {
        let day_base = day * 86_400;
        // Morning burst: fresh requests from the roster.
        for i in 0..3i64 {
            let sender = senders[((day + i) % 3) as usize];
            let rfc = format!("d{day}-m{i}@client.test");
            let virus = day % 7 == 3 && i == 2; // one poisoned mail per week
            sim.inbound(day_base + 360 * i, fresh_email(&rfc, sender, virus));
            expected_inbound += 1;
            if virus {
                expected_quarantined += 1;
            } else {
                expected_tickets += 1;
                expected_outbound += 1; // autoresponder confirmation
                live_messages.push(rfc);
            }
        }
        // Afternoon: threaded reply onto an existing conversation.
        if !live_messages.is_empty() && day % 2 == 1 {
            let target = live_messages[(day % live_messages.len() as i64) as usize].clone();
            sim.reply(
                day_base + 7_200,
                &format!("d{day}-reply@client.test"),
                &target,
                senders[0],
            );
            expected_inbound += 1;
            expected_comments += 1;
        }
        // Subject-token fallback reply.
        if !live_messages.is_empty() && day % 3 == 2 {
            sim.token_reply(day_base + 10_800, &format!("d{day}-token@client.test"));
            expected_inbound += 1;
            expected_comments += 1;
        }
        // Provider replays an earlier delivery — MUST be inert.
        if day % 4 == 3 {
            if let Some(rfc) = live_messages.first().cloned() {
                sim.replay(day_base + 20_000, &rfc);
                expected_replays += 1;
            }
        }
        // Staff answers a requester through the API enqueue hook.
        if !live_messages.is_empty() && day % 2 == 0 {
            sim.staff_reply(day_base + 43_200, "Printer");
            expected_outbound += 1;
            expected_staff_replies += 1;
        }
    }

    // --- run the pipeline over simulated time -------------------------------
    let outcomes = h.run_inbound(&mut sim).await;

    assert_eq!(
        outcomes.commented, expected_comments,
        "one comment per reply/token mail"
    );
    assert_eq!(
        outcomes.tickets_created, expected_tickets,
        "one ticket per clean fresh mail"
    );
    assert_eq!(outcomes.quarantined, expected_quarantined);
    assert_eq!(
        outcomes.duplicate, expected_replays,
        "replays absorbed, never duplicated"
    );
    assert_eq!(
        outcomes.unroutable, 0,
        "scenario stays inside routed addresses"
    );
    assert_eq!(outcomes.staff_replies, expected_staff_replies);

    // --- drain the outbound queue through retries ----------------------------
    h.drain_outbound().await;

    // --- zero-loss assertions -------------------------------------------------
    let conn = &h.admin;

    // Ledger: every generated Message-ID exists EXACTLY once.
    for rfc in sim.all_inbound_ids() {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM email_messages \
             WHERE direction='inbound' AND lower(rfc_message_id)=lower($1)",
        )
        .bind(rfc)
        .fetch_one(conn)
        .await
        .expect("count");
        assert_eq!(n, 1, "message {rfc} must exist exactly once");
    }

    // Routing outcomes on ledger rows agree with intent. (expected_inbound
    // already counts only unique Message-IDs — replays never registered.)
    let (routed, total_in): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE scan_status='clean' AND comment_id IS NOT NULL), \
                count(*) FROM email_messages WHERE direction='inbound'",
    )
    .fetch_one(conn)
    .await
    .expect("aggregate");
    assert_eq!(
        total_in as usize, expected_inbound,
        "no ledger row lost or duplicated"
    );
    assert_eq!(
        routed as usize, expected_comments,
        "exactly one comment-linked row per threaded message"
    );

    let quarantined: i64 =
        sqlx::query_scalar("SELECT count(*) FROM email_messages WHERE scan_status='infected'")
            .fetch_one(conn)
            .await
            .expect("quarantined count");
    assert_eq!(quarantined as usize, expected_quarantined);

    // Outbound: every queued message sent, recorded exactly once, and the
    // retry path genuinely exercised for each (flaky failed attempt #1).
    let (sent_rows, dead_rows): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE status='sent'), count(*) FILTER (WHERE status='dead') \
         FROM email_outbound",
    )
    .fetch_one(conn)
    .await
    .expect("queue aggregate");
    assert_eq!(
        sent_rows as usize, expected_outbound,
        "every outbound eventually sent"
    );
    assert_eq!(dead_rows, 0, "nothing parked as dead");

    let recordings = h.transport_recordings().await;
    assert_eq!(
        recordings.len(),
        expected_outbound,
        "transport saw each distinct message exactly once"
    );
    for (rfc, attempts) in h.flaky_attempts().await {
        assert_eq!(
            recordings.get(&rfc),
            Some(&1),
            "{rfc} recorded exactly once"
        );
        assert!(
            attempts >= 2,
            "{rfc} exercised retry/backoff (attempts={attempts})"
        );
    }

    // Domain events match outcomes one-for-one — nothing lost between DB
    // mutation and outbox.
    let ev: (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE event_type='ticket.created'), \
                count(*) FILTER (WHERE event_type='ticket.comment_added'), \
                count(*) FILTER (WHERE event_type='email.message_recorded') \
         FROM outbox_events",
    )
    .fetch_one(conn)
    .await
    .expect("events aggregate");
    assert_eq!(
        ev.0 as usize, expected_tickets,
        "ticket.created per clean fresh mail"
    );
    assert_eq!(
        ev.1 as usize, expected_comments,
        "comment_added per threaded mail"
    );
    assert_eq!(
        ev.2 as usize,
        expected_inbound + expected_outbound,
        "one email.message_recorded per persisted row"
    );
}

/// A morning-burst provider payload. Poisoned ones carry the scanner's
/// `[VIRUS]` marker so ScriptedScan quarantines them.
fn fresh_email(rfc: &str, from: &str, poison: bool) -> InboundEmail {
    InboundEmail {
        rfc_message_id: rfc.to_string(),
        in_reply_to: None,
        references: vec![],
        subject: Some("Printer trouble".into()),
        from_address: from.to_string(),
        from_name: Some("Client".into()),
        to: vec!["support@acme.test".into()],
        cc: vec![],
        text: Some(if poison {
            "[VIRUS] see attachment".to_string()
        } else {
            "printer is broken again".into()
        }),
        html: None,
        received_at: None,
        attachments: vec![],
    }
}
