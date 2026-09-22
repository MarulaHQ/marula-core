//! 3CX adapter conformance over contract-shaped stub servers: the
//! OAuth2 client-credentials token flow, OData `$top`/`$skip` call-journal
//! paging, the CDR → VoIP-ingest mapping (wire shape byte-exact),
//! ticket-context linkage, click-to-call (MakeCall), and none mode.
//! No live PBX.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    map_call, CallLogSink, MeridianCallLogConfig, MeridianCallLogSink, NoneCallLogSink,
    ThreecxClient, ThreecxSync,
};

async fn spawn(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn cdr(id: &str, direction: &str, duration: Value) -> Value {
    json!({
        "id": id,
        "direction": direction,
        "callerNumber": "+15551230003",
        "calleeNumber": "+15551230002",
        "duration": duration,
        "startedAt": "2026-09-22T10:00:00Z",
        "statusText": "answered"
    })
}

/// The PBX stub's shared state: request log + the call journal.
#[derive(Clone)]
struct Pbx {
    seen: Arc<Mutex<Vec<String>>>,
    journal: Arc<Vec<Value>>,
}

impl Pbx {
    fn log(&self, entry: String) {
        self.seen.lock().unwrap().push(entry);
    }
}

/// The 3CX stub: token endpoint (client_credentials grant) + the XAPI
/// journal (paged `$top`/`$skip`, bearer-gated) + MakeCall.
fn pbx_stub(journal: Vec<Value>) -> (Router, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let state = Pbx {
        seen: seen.clone(),
        journal: Arc::new(journal),
    };

    async fn token(
        State(state): State<Pbx>,
        Form(form): Form<HashMap<String, String>>,
    ) -> Json<Value> {
        state.log(format!(
            "token grant={} client={}",
            form.get("grant_type").cloned().unwrap_or_default(),
            form.get("client_id").cloned().unwrap_or_default()
        ));
        Json(json!({ "access_token": "threecx-token", "expires_in": 3600 }))
    }

    async fn cdrs(
        State(state): State<Pbx>,
        headers: HeaderMap,
        Query(query): Query<HashMap<String, String>>,
    ) -> Response {
        if headers.get("authorization").and_then(|v| v.to_str().ok())
            != Some("Bearer threecx-token")
        {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let top: usize = query
            .get("$top")
            .and_then(|o| o.parse().ok())
            .unwrap_or(100);
        let skip: usize = query.get("$skip").and_then(|o| o.parse().ok()).unwrap_or(0);
        state.log(format!("cdrs top={top} skip={skip} bearer"));
        let page: Vec<Value> = state.journal.iter().skip(skip).take(top).cloned().collect();
        Json(Value::Array(page)).into_response()
    }

    async fn make_call(
        State(state): State<Pbx>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Response {
        if headers.get("authorization").and_then(|v| v.to_str().ok())
            != Some("Bearer threecx-token")
        {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        state.log(format!(
            "make_call dn={} destination={}",
            body["dn"].as_str().unwrap_or_default(),
            body["destination"].as_str().unwrap_or_default()
        ));
        Json(json!({ "id": "call-abc-1", "state": "Dialing" })).into_response()
    }

    let app = Router::new()
        .route("/connect/token", post(token))
        .route("/xapi/v1/Cdr", get(cdrs))
        .route("/xapi/v1/Users/Pbx.MakeCall", post(make_call))
        .with_state(state);
    (app, seen)
}

#[tokio::test]
async fn token_grant_then_bearer_journal_pull() {
    let journal: Vec<Value> = (1..=105)
        .map(|i| cdr(&format!("call-{i}"), "out", json!(i)))
        .collect();
    let (app, seen) = pbx_stub(journal);
    let base = spawn(app).await;

    let threecx = ThreecxClient::new(&base, "cid", "secret");
    let cdrs = threecx.cdrs().await.unwrap();
    assert_eq!(cdrs.len(), 105); // $top 100 + short page 5

    let seen = seen.lock().unwrap();
    // The token endpoint saw the client_credentials grant.
    assert!(seen
        .iter()
        .any(|e| e.contains("token grant=client_credentials client=cid")));
    // Second CDR page rides $skip=100.
    assert!(seen
        .iter()
        .any(|e| e.contains("cdrs top=100 skip=100 bearer")));
    // Every CDR request rode the granted bearer token.
    assert!(seen
        .iter()
        .filter(|e| e.starts_with("cdrs"))
        .all(|e| e.ends_with("bearer")));
}

#[tokio::test]
async fn cdrs_map_onto_the_voip_ingest_wire_shape() {
    let inbound = json!({
        "id": "call-in-1",
        "direction": "IN",
        "callerNumber": "+15551230003",
        "duration": "46",
        "startedAt": "2026-09-22T10:05:00Z",
        "statusText": "voicemail"
    });
    let record = map_call(&inbound).unwrap();
    assert_eq!(record.call_id, "call-in-1");
    assert_eq!(record.direction, "in");
    // External party = the caller on an inbound call.
    assert_eq!(record.external_number, "+15551230003");
    assert_eq!(record.duration_seconds, 46); // string seconds parse
    assert_eq!(record.summary.as_deref(), Some("voicemail"));
    assert_eq!(record.occurred_at, "2026-09-22T10:05:00Z");

    // Number duration parses too; outbound external party = callee.
    let record = map_call(&cdr("c2", "out", json!(120))).unwrap();
    assert_eq!(record.direction, "out");
    assert_eq!(record.external_number, "+15551230002");
    assert_eq!(record.duration_seconds, 120);

    // Unknown direction or missing id → None, never a fabricated call.
    assert!(map_call(&json!({ "id": "x", "direction": "sideways" })).is_none());
    assert!(map_call(&json!({ "direction": "in" })).is_none());
}

#[tokio::test]
async fn sync_pushes_call_logs_with_ticket_context() {
    let pushed: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
    let slot = pushed.clone();
    let app = Router::new().route(
        "/api/v1/connectors/voip/call-logs",
        post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let slot = slot.clone();
            async move {
                let key = headers
                    .get("x-api-key")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .to_string();
                slot.lock().unwrap().push((key, body));
                Json(json!({ "id": Uuid::now_v7().to_string() }))
            }
        }),
    );
    let base = spawn(app).await;

    let journal = vec![
        cdr("call-ctx-1", "out", json!(95)),
        cdr("call-no-ctx", "in", json!(10)),
    ];
    let (app, _) = pbx_stub(journal);
    let pbx = spawn(app).await;

    let ticket = Uuid::now_v7();
    let context = HashMap::from([("call-ctx-1".to_string(), ticket)]);
    let sink = MeridianCallLogSink::new(MeridianCallLogConfig {
        base_url: base.clone(),
        api_key: "mrd_test_key".into(),
    });
    let threecx = ThreecxClient::new(&pbx, "cid", "secret");
    let report = ThreecxSync::run(&threecx, Some(&context), &sink)
        .await
        .unwrap();

    assert_eq!(report.total, 2);
    assert_eq!(report.pushed, 2);
    assert!(report.is_clean());

    let pushed = pushed.lock().unwrap();
    assert!(pushed.iter().all(|(key, _)| key == "mrd_test_key"));
    let linked = pushed
        .iter()
        .find(|(_, body)| body["call_id"] == "call-ctx-1")
        .unwrap();
    assert_eq!(linked.1["ticket_id"], json!(ticket));
    assert_eq!(linked.1["direction"], "out");
    assert_eq!(linked.1["external_number"], "+15551230002");
    let unlinked = pushed
        .iter()
        .find(|(_, body)| body["call_id"] == "call-no-ctx")
        .unwrap();
    assert!(unlinked.1["ticket_id"].is_null());
}

#[tokio::test]
async fn none_mode_counts_and_make_call_drives_click_to_call() {
    let (app, _) = pbx_stub(vec![cdr("call-1", "in", json!(5))]);
    let pbx = spawn(app).await;

    let threecx = ThreecxClient::new(&pbx, "cid", "secret");
    let report = ThreecxSync::run(&threecx, None, &NoneCallLogSink)
        .await
        .unwrap();
    assert_eq!(report.total, 1);
    assert_eq!(report.would_push, 1);
    assert_eq!(report.pushed, 0);
    assert!(report.is_clean());

    // Click-to-call: the dial returns the call id that seeds the
    // ticket context (the same map the sync consumes on completion).
    let call_id = ThreecxSync::dial(&threecx, "101", "+15551230002")
        .await
        .unwrap();
    assert_eq!(call_id, "call-abc-1");
}

#[tokio::test]
async fn meridian_rejections_map_to_typed_errors() {
    async fn sink_at(status: StatusCode) -> MeridianCallLogSink {
        let app =
            Router::new().route(
                "/api/v1/connectors/voip/call-logs",
                post(move || async move {
                    (status, Json(json!({"message": "refused"}))).into_response()
                }),
            );
        let base = spawn(app).await;
        MeridianCallLogSink::new(MeridianCallLogConfig {
            base_url: base.clone(),
            api_key: "mrd_test_key".into(),
        })
    }

    let sink = sink_at(StatusCode::UNAUTHORIZED).await;
    let call = map_call(&cdr("c1", "in", json!(1))).unwrap();
    let err = sink.push(&call).await.unwrap_err();
    assert!(err.contains("unauthorized"));

    let sink = sink_at(StatusCode::CONFLICT).await;
    let err = sink.push(&call).await.unwrap_err();
    assert!(err.contains("409"));
}
