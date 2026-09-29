//! Contract tests for the Xero export adapter against a stub HTTP server
//! (axum), plus none-mode guarantees. These pin the WIRE contract the
//! public Xero accounting API expects — including the `Xero-tenant-id`
//! header, the ACCREC invoice type, and the array-wrapped response shape.

use super::*;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Json as AxumJson;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;

/// A captured request: (authorization header, tenant header, body).
type Captured = Arc<Mutex<Vec<(String, String, Value)>>>;

async fn stub(status: axum::http::StatusCode, reply: Value) -> (SocketAddr, Captured) {
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));

    let invoices_captured = captured.clone();
    let invoices_reply = reply.clone();
    let invoices_status = status;
    let invoices = post(
        move |headers: axum::http::HeaderMap, AxumJson(body): AxumJson<Value>| async move {
            let auth = headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            let tenant = headers
                .get("Xero-tenant-id")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            invoices_captured.lock().await.push((auth, tenant, body));
            (
                invoices_status,
                AxumJson(invoices_reply.clone()).into_response(),
            )
                .into_response()
        },
    );

    let payments_captured = captured.clone();
    let payments_reply = reply;
    let payments_status = status;
    let payments = post(
        move |headers: axum::http::HeaderMap, AxumJson(body): AxumJson<Value>| async move {
            let auth = headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            let tenant = headers
                .get("Xero-tenant-id")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            payments_captured.lock().await.push((auth, tenant, body));
            (
                payments_status,
                AxumJson(payments_reply.clone()).into_response(),
            )
                .into_response()
        },
    );

    let app = axum::Router::new()
        .route("/api.xro/2.0/Invoices", invoices)
        .route("/api.xro/2.0/Payments", payments)
        .into_make_service();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    (addr, captured)
}

fn client_at(addr: SocketAddr) -> XeroClient {
    let mut config = XeroConfig::new("tok-123", "tenant-abc");
    config.base_url = format!("http://{addr}");
    XeroClient::new(config)
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
        customer_ref: "9102f22a-1c5e-4f61-bc88-6a2b4d0f3f21".into(),
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

#[tokio::test]
async fn push_invoice_maps_to_xero_wire_shape() {
    let (addr, captured) = stub(
        axum::http::StatusCode::OK,
        json!({ "Invoices": [{ "InvoiceID": "9d3f-aa12", "InvoiceNumber": "INV-000042" }] }),
    )
    .await;
    let client = client_at(addr);

    let receipt = client.push_invoice(&sample_invoice()).await.expect("push");
    assert_eq!(receipt.external_id, "9d3f-aa12");
    assert_eq!(receipt.external_number.as_deref(), Some("INV-000042"));

    let (auth, tenant, body) = &captured.lock().await[0];
    assert_eq!(auth, "Bearer tok-123");
    assert_eq!(tenant, "tenant-abc", "the tenant id must ride the header");
    assert_eq!(body["Type"], "ACCREC");
    assert_eq!(body["Contact"]["ContactID"], sample_invoice().customer_ref);
    assert_eq!(body["InvoiceNumber"], "INV-000042");
    assert_eq!(body["Date"], "2026-09-01");
    assert_eq!(body["DueDate"], "2026-10-01");
    assert_eq!(body["CurrencyCode"], "USD");
    assert!(
        body["Reference"]
            .as_str()
            .unwrap()
            .contains("[marula:invoice:"),
        "the marula id must ride Reference for vendor-side auditability"
    );
    let line = &body["LineItems"][0];
    assert_eq!(line["Description"], "Managed IT retainer — September");
    assert_eq!(line["Quantity"].as_f64().unwrap(), 1.0);
    assert_eq!(line["UnitAmount"].as_f64().unwrap(), 1500.0);
    assert_eq!(body["LineItems"][1]["Quantity"].as_f64().unwrap(), 2.5);
    assert_eq!(body["LineItems"][1]["UnitAmount"].as_f64().unwrap(), 125.0);
}

#[tokio::test]
async fn push_payment_links_to_the_vendor_invoice() {
    let (addr, captured) = stub(
        axum::http::StatusCode::OK,
        json!({ "Payments": [{ "PaymentID": "pay-77" }] }),
    )
    .await;
    let client = client_at(addr);

    let payment = ExportPayment {
        payment_id: Uuid::from_u128(7),
        invoice_external_id: "9d3f-aa12".into(),
        account_code: "001".into(),
        applied_on: NaiveDate::from_ymd_opt(2026, 9, 2).unwrap(),
        amount_minor: 150_000,
        currency: "USD".into(),
    };
    let receipt = client.push_payment(&payment).await.expect("push");
    assert_eq!(receipt.external_id, "pay-77");

    let (_, _, body) = &captured.lock().await[0];
    assert_eq!(body["Invoice"]["InvoiceID"], "9d3f-aa12");
    assert_eq!(body["Account"]["Code"], "001");
    assert_eq!(body["Amount"].as_f64().unwrap(), 1500.0);
    assert_eq!(body["Date"], "2026-09-02");
}

#[tokio::test]
async fn vendor_status_errors_map_to_typed_variants() {
    let cases: Vec<ErrorCase<'_>> = vec![
        (
            axum::http::StatusCode::UNAUTHORIZED,
            json!({ "Title": "Unauthorized" }),
            &|e| matches!(e, ExportError::Unauthorized(_)),
        ),
        (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            json!({ "Message": "rate limited" }),
            &|e| matches!(e, ExportError::RateLimited(_)),
        ),
        (
            axum::http::StatusCode::BAD_REQUEST,
            json!({ "Elements": [{ "ValidationErrors": [{ "Message": "Contact not found" }] }] }),
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
async fn missing_contact_or_account_is_invalid_request_without_network() {
    // Unroutable port: any dial would surface as Network, not InvalidRequest.
    let mut config = XeroConfig::new("tok", "tenant");
    config.base_url = "http://127.0.0.1:1".into();
    let client = XeroClient::new(config);

    let mut inv = sample_invoice();
    inv.customer_ref = "  ".into();
    let err = client.push_invoice(&inv).await.unwrap_err();
    assert!(matches!(err, ExportError::InvalidRequest(_)), "got {err:?}");

    let payment = ExportPayment {
        payment_id: Uuid::from_u128(7),
        invoice_external_id: "9d3f-aa12".into(),
        account_code: " ".into(),
        applied_on: NaiveDate::from_ymd_opt(2026, 9, 2).unwrap(),
        amount_minor: 100,
        currency: "USD".into(),
    };
    let err = client.push_payment(&payment).await.unwrap_err();
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
        invoice_external_id: "9d3f-aa12".into(),
        account_code: "001".into(),
        applied_on: NaiveDate::from_ymd_opt(2026, 9, 2).unwrap(),
        amount_minor: 100,
        currency: "USD".into(),
    };
    let err = sink.push_payment(&payment).await.unwrap_err();
    assert!(matches!(err, ExportError::Disabled), "got {err:?}");
}
