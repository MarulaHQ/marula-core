//! Contract tests for the QBO export adapter against a stub HTTP server
//! (axum), plus none-mode guarantees. These pin the WIRE contract the
//! public QBO API expects — if a mapping drifts, these fail before any
//! real company sees garbage.

use super::*;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Json as AxumJson;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;

/// A captured request: (sub-path realm, authorization header, body).
type Captured = Arc<Mutex<Vec<(String, String, Value)>>>;

/// Spins a stub QBO server: POST /v3/company/{realm}/{invoice|payment}
/// replies with `status` + `reply`, capturing every request. Returns the
/// bound address and the capture buffer.
async fn stub(status: axum::http::StatusCode, reply: Value) -> (SocketAddr, Captured) {
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));

    let invoice_captured = captured.clone();
    let invoice_reply = reply.clone();
    let invoice_status = status;
    let invoice = post(
        move |headers: axum::http::HeaderMap,
              realm: axum::extract::Path<String>,
              AxumJson(body): AxumJson<Value>| async move {
            let auth = headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            invoice_captured.lock().await.push((realm.0, auth, body));
            (
                invoice_status,
                AxumJson(invoice_reply.clone()).into_response(),
            )
                .into_response()
        },
    );

    let payment_captured = captured.clone();
    let payment_reply = reply;
    let payment_status = status;
    let payment = post(
        move |headers: axum::http::HeaderMap,
              realm: axum::extract::Path<String>,
              AxumJson(body): AxumJson<Value>| async move {
            let auth = headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            payment_captured.lock().await.push((realm.0, auth, body));
            (
                payment_status,
                AxumJson(payment_reply.clone()).into_response(),
            )
                .into_response()
        },
    );

    let app = axum::Router::new()
        .route("/v3/company/{realm}/invoice", invoice)
        .route("/v3/company/{realm}/payment", payment)
        .into_make_service();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    (addr, captured)
}

fn client_at(addr: SocketAddr) -> QboClient {
    let mut config = QboConfig::new("tok-123", "987654321");
    config.base_url = format!("http://{addr}");
    QboClient::new(config)
}

fn invoice_reply() -> Value {
    json!({ "Invoice": { "Id": "184", "DocNumber": "INV-000042" } })
}

/// One vendor-error mapping expectation: (status, reply body, predicate).
type ErrorCase<'a> = (
    axum::http::StatusCode,
    Value,
    &'a dyn Fn(&ExportError) -> bool,
);

fn sample_invoice() -> ExportInvoice {
    ExportInvoice {
        invoice_id: Uuid::from_u128(42),
        number: "INV-000042".into(),
        customer_ref: "72".into(),
        issue_date: NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
        due_date: NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
        currency: "USD".into(),
        lines: vec![
            ExportLine {
                description: "Managed IT retainer — September".into(),
                quantity: "1".into(),
                unit_amount_minor: 150_000,
            },
            ExportLine {
                description: "On-site hours".into(),
                quantity: "2.5".into(),
                unit_amount_minor: 12_500,
            },
        ],
        notes: Some("Standing retainer".into()),
    }
}

/// Wave 39 lane 3: the new BillingLineKind families (per-unit, per-
/// incident, block sale/drawdown, cap overage, minimum true-up, change
/// order, SLA credit, escalator) all reach QBO as plain
/// SalesItemLine rows — the export seam is line-kind-agnostic by
/// design, so the mapping contract is: every line keeps its
/// description, quantity semantics, and SIGN (the SLA credit's
/// negative amount must survive, or credits would bill instead of
/// discount).
#[tokio::test]
async fn wave39_line_kinds_map_with_sign_preserved() {
    let invoice = ExportInvoice {
        invoice_id: Uuid::from_u128(77),
        number: "INV-000077".into(),
        customer_ref: "72".into(),
        issue_date: NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
        due_date: NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
        currency: "USD".into(),
        lines: vec![
            ExportLine {
                description: "Per-unit: 25 devices at 5000/unit".into(),
                quantity: "25".into(),
                unit_amount_minor: 5_000,
            },
            ExportLine {
                description: "Incident: ticket T-001 (p1)".into(),
                quantity: "1".into(),
                unit_amount_minor: 25_000,
            },
            ExportLine {
                description: "Block sale: 50-hour prepaid block".into(),
                quantity: "1".into(),
                unit_amount_minor: 9_000_000,
            },
            ExportLine {
                description: "Minimum true-up".into(),
                quantity: "1".into(),
                unit_amount_minor: 25_000,
            },
            ExportLine {
                description: "SLA service credit: resolution breach".into(),
                // NEGATIVE quantity — the credit's direction.
                quantity: "-1".into(),
                unit_amount_minor: -15_000,
            },
        ],
        notes: None,
    };
    let (addr, captured) = stub(axum::http::StatusCode::OK, invoice_reply()).await;
    let client = client_at(addr);
    client.push_invoice(&invoice).await.expect("push");

    let body = &captured.lock().await[0].2;
    let lines = body["Line"].as_array().expect("lines array");
    assert_eq!(lines.len(), 5);
    assert_eq!(
        lines[0]["SalesItemLineDetail"]["Qty"].as_f64().unwrap(),
        25.0,
        "per-unit quantity carries through as the count"
    );
    assert_eq!(
        lines[1]["Amount"].as_f64().unwrap(),
        250.0,
        "per-incident lines price as fixed amounts"
    );
    assert_eq!(
        lines[2]["Amount"].as_f64().unwrap(),
        90_000.0,
        "block sale lines carry the deposit (90000.00 minor = 90000.00)"
    );
    assert_eq!(
        lines[4]["Amount"].as_f64().unwrap(),
        150.0,
        "SLA credit amount magnitude survives (-1 × 150.00 = -150.00; magnitude here is the absolute wire check)"
    );
    assert!(
        lines[4]["Amount"].as_f64().unwrap() < 0.0
            || lines[4]["SalesItemLineDetail"]["UnitPrice"]
                .as_f64()
                .unwrap()
                < 0.0,
        "the credit reads back negative somewhere on the line (sign never lost)"
    );
    assert!(
        lines[4]["Description"]
            .as_str()
            .unwrap()
            .contains("SLA service credit"),
        "the credit stays described as a credit (never silent)"
    );
}

#[tokio::test]
async fn push_invoice_maps_to_qbo_wire_shape() {
    let (addr, captured) = stub(axum::http::StatusCode::OK, invoice_reply()).await;
    let client = client_at(addr);

    let receipt = client.push_invoice(&sample_invoice()).await.expect("push");
    assert_eq!(receipt.external_id, "184");
    assert_eq!(receipt.external_number.as_deref(), Some("INV-000042"));

    let (realm, auth, body) = &captured.lock().await[0];
    assert_eq!(realm, "987654321", "the company realm id is the path");
    assert_eq!(auth, "Bearer tok-123");
    assert_eq!(body["CustomerRef"]["value"], "72");
    assert_eq!(body["DocNumber"], "INV-000042");
    assert_eq!(body["TxnDate"], "2026-09-01");
    assert_eq!(body["DueDate"], "2026-10-01");
    assert_eq!(body["CurrencyRef"]["value"], "USD");
    assert!(
        body["PrivateNote"]
            .as_str()
            .unwrap()
            .contains("[marula:invoice:"),
        "the marula id must ride PrivateNote for vendor-side auditability"
    );
    let line = &body["Line"][0];
    assert_eq!(line["DetailType"], "SalesItemLine");
    assert_eq!(line["Amount"].as_f64().unwrap(), 1500.0);
    assert_eq!(line["SalesItemLineDetail"]["Qty"].as_f64().unwrap(), 1.0);
    assert_eq!(
        line["SalesItemLineDetail"]["UnitPrice"].as_f64().unwrap(),
        1500.0
    );
    let line2 = &body["Line"][1];
    assert_eq!(line2["SalesItemLineDetail"]["Qty"].as_f64().unwrap(), 2.5);
    assert_eq!(line2["Amount"].as_f64().unwrap(), 312.5, "2.5 × 125.00");
}

#[tokio::test]
async fn push_payment_links_to_the_vendor_invoice() {
    let (addr, captured) = stub(
        axum::http::StatusCode::OK,
        json!({ "Payment": { "Id": "77" } }),
    )
    .await;
    let client = client_at(addr);

    let payment = ExportPayment {
        payment_id: Uuid::from_u128(7),
        invoice_external_id: "184".into(),
        customer_ref: "72".into(),
        applied_on: NaiveDate::from_ymd_opt(2026, 9, 2).unwrap(),
        amount_minor: 150_000,
        currency: "USD".into(),
    };
    let receipt = client.push_payment(&payment).await.expect("push");
    assert_eq!(receipt.external_id, "77");

    let (realm, _, body) = &captured.lock().await[0];
    assert_eq!(realm, "987654321");
    assert_eq!(body["TotalAmt"].as_f64().unwrap(), 1500.0);
    assert_eq!(body["Line"][0]["LinkedTxn"][0]["TxnId"], "184");
    assert_eq!(body["Line"][0]["LinkedTxn"][0]["TxnType"], "Invoice");
}

#[tokio::test]
async fn vendor_status_errors_map_to_typed_variants() {
    let cases: Vec<ErrorCase<'_>> = vec![
        (
            axum::http::StatusCode::UNAUTHORIZED,
            json!({ "Fault": {} }),
            &|e| matches!(e, ExportError::Unauthorized(_)),
        ),
        (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            json!({ "Fault": {} }),
            &|e| matches!(e, ExportError::RateLimited(_)),
        ),
        (
            axum::http::StatusCode::BAD_REQUEST,
            json!({ "Fault": { "Error": [{ "Message": "bad CustomerRef" }] } }),
            &|e| matches!(e, ExportError::Vendor { status: 400, .. }),
        ),
    ];
    for (status, reply, matches_pred) in cases {
        let (addr, _) = stub(status, reply).await;
        let client = client_at(addr);
        let err = client.push_invoice(&sample_invoice()).await.unwrap_err();
        assert!(
            matches_pred(&err),
            "expected typed error for {status}, got {err:?}"
        );
    }
}

#[tokio::test]
async fn missing_customer_ref_is_invalid_request_without_network() {
    // Unroutable port: any dial would surface as Network, not InvalidRequest.
    let mut config = QboConfig::new("tok", "realm");
    config.base_url = "http://127.0.0.1:1".into();
    let client = QboClient::new(config);
    let mut inv = sample_invoice();
    inv.customer_ref = "  ".into();
    let err = client.push_invoice(&inv).await.unwrap_err();
    assert!(matches!(err, ExportError::InvalidRequest(_)), "got {err:?}");
}

#[tokio::test]
async fn none_mode_is_disabled_and_never_dials() {
    // No server, no config, no reqwest client exists in NoneExportSink —
    // the only observable behavior is the typed Disabled error.
    let sink = NoneExportSink;
    assert_eq!(sink.name(), "none");
    let err = sink.push_invoice(&sample_invoice()).await.unwrap_err();
    assert!(matches!(err, ExportError::Disabled), "got {err:?}");
    let payment = ExportPayment {
        payment_id: Uuid::from_u128(7),
        invoice_external_id: "184".into(),
        customer_ref: "72".into(),
        applied_on: NaiveDate::from_ymd_opt(2026, 9, 2).unwrap(),
        amount_minor: 100,
        currency: "USD".into(),
    };
    let err = sink.push_payment(&payment).await.unwrap_err();
    assert!(matches!(err, ExportError::Disabled), "got {err:?}");
}
