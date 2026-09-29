//! End-to-end signed webhook delivery against a LOCAL HTTP SINK.
//!
//! Purpose: prove the OUTBOUND half of plugin runtime v1 without any
//! Marula internals — the production `ReqwestSink` delivers to a real
//! TCP listener, and the receiver-side verification (the same primitive
//! first-party connectors ship) validates signature, timestamp binding,
//! and replay rejection over actual wire bytes.
//!
//! Non-purpose: no database, no queue, no core-api — those layers are
//! covered by db_suite (ledger) and unit tests (settle matrix).

use worker::webhook_out::{DeliverySink, ReqwestSink};

/// Minimal HTTP server: reads one POST, records method/path/headers/body,
/// answers 200 OK. Returns (local_addr, join handle → ReceivedRequest).
async fn spawn_sink() -> (std::net::SocketAddr, tokio::task::JoinHandle<Received>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let addr = listener.local_addr().expect("addr");

    let handle = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.expect("one connection");
        let mut buf = Vec::new();
        // Read until end of headers.
        let header_end = loop {
            let mut chunk = [0_u8; 1024];
            let n = sock.read(&mut chunk).await.expect("read headers");
            assert!(n > 0, "peer closed before headers finished");
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = find_subsequence(&buf, b"\r\n\r\n") {
                break pos;
            }
        };
        let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
        // Content-Length decides how much body to expect.
        let content_length = head
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.trim()
                    .eq_ignore_ascii_case("content-length")
                    .then(|| v.trim().parse::<usize>().ok())?
            })
            .unwrap_or(0);
        let mut body = buf[header_end + 4..].to_vec();
        while body.len() < content_length {
            let mut chunk = [0_u8; 1024];
            let n = sock.read(&mut chunk).await.expect("read body");
            assert!(n > 0, "peer closed mid-body");
            body.extend_from_slice(&chunk[..n]);
        }
        sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
            .await
            .expect("respond");
        let mut lines = head.lines();
        let request_line = lines.next().unwrap_or_default().to_string();
        let headers: Vec<(String, String)> = lines
            .filter_map(|l| {
                let (k, v) = l.split_once(':')?;
                Some((k.trim().to_lowercase(), v.trim().to_string()))
            })
            .collect();
        Received {
            request_line,
            headers,
            body,
        }
    });
    (addr, handle)
}

pub struct Received {
    pub request_line: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Received {
    fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k == &name)
            .map(|(_, v)| v.as_str())
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

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

#[tokio::test]
async fn sink_receives_signed_delivery_and_verifies_it() {
    let (addr, handle) = spawn_sink().await;
    let url = format!("http://{addr}/marula/hook");
    let body = envelope_body();

    // Sign exactly what we send — the production contract.
    let now_unix = chrono::Utc::now().timestamp();
    let signature = core_domain::webhooks::signature_value(SECRET, &body, now_unix);

    let sink = ReqwestSink::new();
    let result = sink.post(&url, &signature, now_unix, &body).await;
    assert_eq!(
        result,
        worker::webhook_out::SinkResult::Delivered(200),
        "sink must accept the signed delivery"
    );

    let received = handle.await.expect("sink task");
    assert!(
        received.request_line.starts_with("POST /marula/hook "),
        "must hit the configured path: {}",
        received.request_line
    );
    assert_eq!(received.body, body, "wire bytes must match signed bytes");
    assert_eq!(
        received.header(core_domain::webhooks::TIMESTAMP_HEADER),
        Some(now_unix.to_string()).as_deref(),
        "timestamp header mirrors the t= component"
    );

    // Receiver side: the exact check a connector performs.
    let sig_header = received
        .header(core_domain::webhooks::SIGNATURE_HEADER)
        .expect("signature header present on the wire");
    assert_eq!(sig_header, signature);
    core_domain::webhooks::verify(
        SECRET,
        &received.body,
        sig_header,
        chrono::Utc::now().timestamp(),
        core_domain::webhooks::REPLAY_TOLERANCE_SECS,
    )
    .expect("fresh authentic delivery must verify");
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
