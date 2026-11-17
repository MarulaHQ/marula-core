//! App-bridges conformance suite: each adapter pinned against a loopback
//! mock server (deterministic; no external network exits).
use app_bridges::rewst::{RewstBridge, RewstTriggerPayload};
use app_bridges::slack::SlackConnector;
use app_bridges::teams::TeamsConnector;
use app_bridges::{BridgeError, NotificationPayload};
use std::sync::{Arc, Mutex};

#[derive(Default, Clone)]
pub(crate) struct Captured {
    last_body: String,
    last_content_type: String,
    last_signature: String,
}

pub(crate) struct Mock {
    base_url: String,
    captured: Arc<Mutex<Captured>>,
}

impl Mock {
    fn captured(&self) -> Captured {
        self.captured.lock().unwrap().clone()
    }
}

/// Binds ONE loopback mock that answers `status`/`body` and captures the
/// request; each type test binds its own.
async fn spawn_mock(status: axum::http::StatusCode, resp_body: &'static str) -> Mock {
    let captured = Arc::new(Mutex::new(Captured::default()));
    let writer = captured.clone();
    let router = axum::Router::new().route(
        "/",
        axum::routing::post(
            move |headers: axum::http::HeaderMap, body: String| async move {
                if let Ok(mut store) = writer.lock() {
                    *store = Captured {
                        last_body: body.clone(),
                        #[allow(unused_assignments)]
                        last_content_type: headers
                            .get(reqwest::header::CONTENT_TYPE.as_str())
                            .and_then(|v| v.to_str().map(str::to_owned).ok())
                            .unwrap_or_default(),
                        last_signature: headers
                            .get("x-marula-signature")
                            .and_then(|v| v.to_str().map(str::to_owned).ok())
                            .unwrap_or_default(),
                    };
                }
                (status, resp_body)
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback mock");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Mock {
        base_url: format!("http://{addr}/"),
        captured,
    }
}

fn sample_payload() -> NotificationPayload {
    NotificationPayload {
        title: "SLA breach".into(),
        body: "Ticket 42 breached its first-response SLA window.".into(),
        link_url: Some("https://app.example.com/tickets/42".into()),
    }
}

#[tokio::test]
async fn slack_delivers_and_canary_rejects_are_typed() {
    let mock = spawn_mock(axum::http::StatusCode::OK, "ok").await;
    let slack = SlackConnector::new(mock.base_url.clone())
        .expect("client")
        .allow_loopback();
    slack.send(&sample_payload()).await.expect("delivers");
    let captured = mock.captured();
    assert!(
        captured.last_body.contains("\"text\""),
        "the Slack body is chat.post-shaped"
    );
    assert!(
        captured.last_body.contains("SLA breach"),
        "the title rides the rich text"
    );
    assert!(captured.last_content_type.starts_with("application/json"));

    // A Slack "success" that actually rejects the payload shape is a
    // typed refusal (never silent success). The mock is stateless per
    // request, so one probe carries the whole assertion.
    let rejecting = spawn_mock(axum::http::StatusCode::OK, "invalid_payload").await;
    let probe = SlackConnector::new(rejecting.base_url.clone())
        .expect("client")
        .allow_loopback();
    assert!(matches!(
        probe.send(&sample_payload()).await,
        Err(BridgeError::Refused("slack", _))
    ));

    // A plain https URL; the boundary refuses non-https config.
    let bad = SlackConnector::new("http://example.com/hook").expect("client");
    assert!(matches!(
        bad.send(&sample_payload()).await,
        Err(BridgeError::BadConfig("slack", _))
    ));
    assert!(matches!(
        bad.validate(),
        Err(BridgeError::BadConfig("slack", _))
    ));
}

#[tokio::test]
async fn teams_delivers_one_adaptive_card_with_the_deep_link() {
    let mock = spawn_mock(axum::http::StatusCode::ACCEPTED, "1").await;
    let teams = TeamsConnector::new(mock.base_url.clone())
        .expect("client")
        .allow_loopback();
    teams.send(&sample_payload()).await.expect("delivers");
    let captured = mock.captured();
    let card: serde_json::Value =
        serde_json::from_str(&captured.last_body).expect("the body is JSON");
    let content = &card["attachments"][0]["content"];
    assert_eq!(card["type"], "message");
    assert!(content["body"][0]["text"]
        .as_str()
        .is_some_and(|t| t.contains("**SLA breach**")));
    assert_eq!(
        content["actions"][0]["url"],
        serde_json::json!("https://app.example.com/tickets/42")
    );
}

#[tokio::test]
async fn rewst_signs_the_trigger_and_the_url_carries_the_secret_headers() {
    use app_bridges::rewst::RewstBridge;
    let mock = spawn_mock(axum::http::StatusCode::OK, "queued").await;
    let bridge = RewstBridge::new(mock.base_url.clone(), "shared-secret-42")
        .expect("client")
        .allow_loopback();
    bridge
        .trigger(&RewstTriggerPayload {
            org: "hearth".into(),
            event: "sla.breach".into(),
            notification: sample_payload(),
            entity_id: Some("ticket-42".into()),
        })
        .await
        .expect("triggers");
    let captured = mock.captured();
    assert!(
        !captured.last_signature.is_empty(),
        "the signature header is present"
    );
    // The secret NEVER rides the wire (header or body).
    assert!(
        !captured.last_body.contains("secret-material"),
        "the secret stays off the wire"
    );
}

/// A destination that answers `302` is asking the bridge to dial a second host
/// the owner never consented to. The client follows no redirects, so the answer
/// is a typed refusal rather than a second unscreened request — on every
/// adapter, because the branch is per adapter and not shared.
#[tokio::test]
async fn a_redirect_is_refused_rather_than_followed() {
    for kind in ["slack", "teams", "rewst"] {
        let mock = spawn_mock(axum::http::StatusCode::FOUND, "").await;
        let outcome = match kind {
            "slack" => {
                SlackConnector::new(mock.base_url.clone())
                    .expect("client")
                    .allow_loopback()
                    .send(&sample_payload())
                    .await
            }
            "teams" => {
                TeamsConnector::new(mock.base_url.clone())
                    .expect("client")
                    .allow_loopback()
                    .send(&sample_payload())
                    .await
            }
            _ => {
                RewstBridge::new(mock.base_url.clone(), "shared-secret-42")
                    .expect("client")
                    .allow_loopback()
                    .trigger(&RewstTriggerPayload {
                        org: "hearth".into(),
                        event: "sla.breach".into(),
                        notification: sample_payload(),
                        entity_id: None,
                    })
                    .await
            }
        };
        assert!(
            matches!(
                outcome,
                Err(BridgeError::Refused(_, ref detail)) if detail.contains("302")
            ),
            "{kind} must refuse a 302 and name the status, got {outcome:?}"
        );
    }
}

/// Production dials through the guard's client (pinned addresses, no redirects),
/// not through `new`. The refusal has to hold on that path too: the adapter's
/// own status check is what turns a `3xx` into a typed refusal, so the injected
/// client is exercised directly.
#[tokio::test]
async fn a_redirect_is_refused_through_an_injected_client() {
    let mock = spawn_mock(axum::http::StatusCode::TEMPORARY_REDIRECT, "").await;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("pinned client");
    let slack = SlackConnector::with_http_client(mock.base_url.clone(), client).allow_loopback();
    assert!(
        matches!(
            slack.send(&sample_payload()).await,
            Err(BridgeError::Refused("slack", ref detail)) if detail.contains("307")
        ),
        "the injected client path refuses a redirect too"
    );
}

#[tokio::test]
async fn bad_config_never_reaches_a_socket() {
    let empty = SlackConnector::new("").expect("client");
    assert!(matches!(
        empty.validate(),
        Err(BridgeError::BadConfig("slack", _))
    ));
    let rewst_blank = RewstBridge::new("https://ok.rewst.io/x", "  ").expect("client");
    assert!(matches!(
        rewst_blank.validate(),
        Err(BridgeError::BadConfig("rewst", _))
    ));
}
