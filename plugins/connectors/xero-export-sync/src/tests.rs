//! Contract tests for the Xero export adapter against a stub HTTP server
//! (axum), plus none-mode guarantees. These pin the WIRE contract the
//! public Xero accounting API expects — including the `Xero-tenant-id`
//! header, the ACCREC/ACCRECCREDIT document types, and the array-wrapped
//! response shape.
//!
//! What a fixture proves, and what it does not: it proves the path, the two
//! headers, the JSON we send and the typed error we raise per status. It does
//! not prove Xero's behaviour — whether an inline `Allocations` array on a
//! `CreditNotes` POST is accepted (Xero documents a separate allocations PUT),
//! whether `where` filtering behaves at the tenant's limits, or how a real
//! validation error is worded. Those are sandbox-confirm items, named in the
//! crate README.

use super::*;
use axum::response::IntoResponse;
use axum::Json as AxumJson;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;

/// One captured request, whichever verb produced it.
#[derive(Clone, Debug)]
struct Captured {
    method: String,
    path: String,
    auth: String,
    tenant: String,
    query: String,
    body: Value,
}

type Capture = Arc<Mutex<Vec<Captured>>>;

/// A stub Xero tenant. Every POST under `/api.xro/2.0/{resource}` answers with
/// `status` + `reply`; every GET answers with the entry in `gets` keyed by the
/// resource segment (an absent key answers `{}`). Both verbs capture.
async fn stub(status: axum::http::StatusCode, reply: Value, gets: Value) -> (SocketAddr, Capture) {
    let captured: Capture = Arc::new(Mutex::new(Vec::new()));

    let post_capture = captured.clone();
    let post_status = status;
    let post_reply = reply;
    let write = move |headers: axum::http::HeaderMap,
                      resource: axum::extract::Path<String>,
                      AxumJson(body): AxumJson<Value>| {
        let post_capture = post_capture.clone();
        let post_reply = post_reply.clone();
        async move {
            post_capture.lock().await.push(Captured {
                method: "POST".into(),
                path: format!("/api.xro/2.0/{}", resource.0),
                auth: header(&headers, axum::http::header::AUTHORIZATION),
                tenant: header(&headers, "Xero-tenant-id"),
                query: String::new(),
                body,
            });
            (post_status, AxumJson(post_reply).into_response()).into_response()
        }
    };

    let get_capture = captured.clone();
    let get_status = status;
    let get_reply = gets;
    let read = axum::routing::get(
        move |headers: axum::http::HeaderMap,
              resource: axum::extract::Path<String>,
              uri: axum::http::Uri| {
            let get_capture = get_capture.clone();
            let get_reply = get_reply.clone();
            async move {
                let key = resource.0.clone();
                get_capture.lock().await.push(Captured {
                    method: "GET".into(),
                    path: format!("/api.xro/2.0/{key}"),
                    auth: header(&headers, axum::http::header::AUTHORIZATION),
                    tenant: header(&headers, "Xero-tenant-id"),
                    query: uri.query().unwrap_or_default().to_string(),
                    body: Value::Null,
                });
                let body = get_reply.get(&key).cloned().unwrap_or_else(|| json!({}));
                (get_status, AxumJson(body).into_response()).into_response()
            }
        },
    );

    let app = axum::Router::new().route(
        "/api.xro/2.0/{resource}",
        axum::routing::get(read).post(write),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let app = app.into_make_service();
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    (addr, captured)
}

fn header(headers: &axum::http::HeaderMap, name: impl axum::http::header::AsHeaderName) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

fn client_at(addr: SocketAddr) -> XeroClient {
    let mut config = XeroConfig::new("tok-123", "tenant-abc");
    config.base_url = format!("http://{addr}");
    XeroClient::new(config).expect("client")
}

fn invoice_reply() -> Value {
    json!({ "Invoices": [{ "InvoiceID": "9d3f-aa12", "InvoiceNumber": "INV-000042" }] })
}

fn sample_invoice() -> ExportInvoice {
    ExportInvoice {
        invoice_id: Uuid::from_u128(42),
        number: "INV-000042".into(),
        customer_ref: "contact-72".into(),
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
        tax_code: None,
    }
}

fn sample_credit_note() -> ExportCreditNote {
    ExportCreditNote {
        credit_note_id: Uuid::from_u128(900),
        number: "CRN-000012".into(),
        customer_ref: "contact-72".into(),
        invoice_external_id: "9d3f-aa12".into(),
        issued_on: NaiveDate::from_ymd_opt(2026, 9, 15).unwrap(),
        currency: "USD".into(),
        lines: vec![ExportLine {
            description: "Service credit — outage 12 Sep".into(),
            quantity: "1".into(),
            unit_amount_minor: 25_000,
        }],
        tax_code: None,
        reason: Some("SLA credit".into()),
    }
}

#[tokio::test]
async fn push_invoice_maps_to_xero_wire_shape() {
    let (addr, captured) = stub(axum::http::StatusCode::OK, invoice_reply(), json!({})).await;
    let client = client_at(addr);

    let receipt = client.push_invoice(&sample_invoice()).await.expect("push");
    assert_eq!(receipt.external_id, "9d3f-aa12");
    assert_eq!(receipt.external_number.as_deref(), Some("INV-000042"));

    let sent = &captured.lock().await[0];
    assert_eq!(sent.path, "/api.xro/2.0/Invoices");
    assert_eq!(sent.auth, "Bearer tok-123");
    assert_eq!(
        sent.tenant, "tenant-abc",
        "the tenant id must ride the header"
    );
    let body = &sent.body;
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
    assert!(
        line.get("TaxType").is_none(),
        "no tax code named by the caller means the key is absent, not a guessed default"
    );
}

/// G2: the resolved vendor tax code reaches every line as Xero's `TaxType`.
#[tokio::test]
async fn invoice_lines_carry_the_resolved_tax_type() {
    let (addr, captured) = stub(axum::http::StatusCode::OK, invoice_reply(), json!({})).await;
    let client = client_at(addr);
    let mut invoice = sample_invoice();
    invoice.tax_code = Some("OUTPUT2".into());
    client.push_invoice(&invoice).await.expect("push");

    let body = &captured.lock().await[0].body;
    for line in body["LineItems"].as_array().unwrap() {
        assert_eq!(line["TaxType"], "OUTPUT2");
    }
}

#[tokio::test]
async fn push_payment_links_to_the_vendor_invoice() {
    let (addr, captured) = stub(
        axum::http::StatusCode::OK,
        json!({ "Payments": [{ "PaymentID": "pay-77" }] }),
        json!({}),
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

    let sent = &captured.lock().await[0];
    assert_eq!(sent.path, "/api.xro/2.0/Payments");
    assert_eq!(sent.body["Invoice"]["InvoiceID"], "9d3f-aa12");
    assert_eq!(sent.body["Account"]["Code"], "001");
    assert_eq!(sent.body["Amount"].as_f64().unwrap(), 1500.0);
    assert_eq!(sent.body["Date"], "2026-09-02");
}

/// G1: a credit note is posted as `ACCRECCREDIT` and carries the ALLOCATION
/// to the vendor invoice id from the stored receipt. The allocation is what
/// applies the credit; without it the invoice keeps its full balance.
#[tokio::test]
async fn credit_note_is_allocated_against_the_exported_vendor_invoice() {
    let (addr, captured) = stub(
        axum::http::StatusCode::OK,
        json!({ "CreditNotes": [{
            "CreditNoteID": "cn-501", "CreditNoteNumber": "CRN-000012"
        }] }),
        json!({}),
    )
    .await;
    let client = client_at(addr);

    let receipt = client
        .push_credit_note(&sample_credit_note())
        .await
        .expect("push");
    assert_eq!(receipt.external_id, "cn-501");
    assert_eq!(receipt.external_number.as_deref(), Some("CRN-000012"));

    let sent = &captured.lock().await[0];
    assert_eq!(sent.path, "/api.xro/2.0/CreditNotes");
    let body = &sent.body;
    assert_eq!(body["Type"], "ACCRECCREDIT");
    assert_eq!(body["CreditNoteNumber"], "CRN-000012");
    assert_eq!(body["Contact"]["ContactID"], "contact-72");
    assert_eq!(body["Date"], "2026-09-15");
    assert_eq!(body["CurrencyCode"], "USD");
    assert_eq!(body["LineItems"][0]["UnitAmount"].as_f64().unwrap(), 250.0);
    assert_eq!(body["Allocations"][0]["Invoice"]["InvoiceID"], "9d3f-aa12");
    assert_eq!(
        body["Allocations"][0]["Amount"].as_f64().unwrap(),
        250.0,
        "the allocation carries the credit note's total"
    );
    assert!(
        body["Reference"]
            .as_str()
            .unwrap()
            .contains(&format!("[marula:credit_note:{}]", Uuid::from_u128(900))),
        "the credit's own marula id must ride Reference, or a reconcile cannot find it"
    );
}

/// The allocation total is the priced line sum, not the face value of the
/// first line — a multi-line credit must allocate what it actually credits.
#[tokio::test]
async fn credit_note_allocation_sums_every_line() {
    let (addr, captured) = stub(
        axum::http::StatusCode::OK,
        json!({ "CreditNotes": [{ "CreditNoteID": "cn-1" }] }),
        json!({}),
    )
    .await;
    let client = client_at(addr);
    let mut note = sample_credit_note();
    note.lines = vec![
        ExportLine {
            description: "first".into(),
            quantity: "2".into(),
            unit_amount_minor: 12_500,
        },
        ExportLine {
            description: "second".into(),
            quantity: "1".into(),
            unit_amount_minor: 5_000,
        },
    ];
    client.push_credit_note(&note).await.expect("push");
    let body = &captured.lock().await[0].body;
    assert_eq!(body["Allocations"][0]["Amount"].as_f64().unwrap(), 300.0);
}

#[tokio::test]
async fn credit_note_without_a_parent_invoice_is_refused_before_any_network() {
    let mut config = XeroConfig::new("tok", "tenant");
    config.base_url = "http://127.0.0.1:1".into();
    let client = XeroClient::new(config).expect("client");
    let mut note = sample_credit_note();
    note.invoice_external_id = String::new();
    let err = client.push_credit_note(&note).await.unwrap_err();
    assert!(matches!(err, ExportError::InvalidRequest(_)), "got {err:?}");
}

/// G2: accounts come back in the NEUTRAL bucket vocabulary. Xero's own type
/// strings must not reach Core, or the mapping rule would have to know both
/// vendors.
#[tokio::test]
async fn read_accounts_returns_the_neutral_shape() {
    let (addr, captured) = stub(
        axum::http::StatusCode::OK,
        json!({}),
        json!({ "Accounts": { "Accounts": [
            { "AccountID": "a1", "Code": "090", "Name": "Business Bank Account",
              "Type": "BANK", "Status": "ACTIVE" },
            { "AccountID": "a2", "Code": "", "Name": "Accounts Receivable",
              "Type": "CURRENT", "Status": "ACTIVE" },
            { "AccountID": "a3", "Code": "300", "Name": "Sales",
              "Type": "REVENUE", "Status": "ARCHIVED" },
            { "AccountID": "a4", "Name": "Mystery", "Type": "NOTATYPE",
              "Status": "ACTIVE" }
        ] } }),
    )
    .await;
    let client = client_at(addr);

    let accounts = client.read_accounts().await.expect("read");
    assert_eq!(accounts.len(), 4);
    assert_eq!(accounts[0].kind, AccountKind::Bank);
    assert_eq!(accounts[0].code.as_deref(), Some("090"));
    assert!(accounts[0].active);
    assert_eq!(
        accounts[1].code, None,
        "an empty Code is None, not a blank pick-list row"
    );
    assert_eq!(accounts[1].kind, AccountKind::OtherCurrentAsset);
    assert_eq!(accounts[2].kind, AccountKind::Income);
    assert!(!accounts[2].active, "ARCHIVED is not active");
    assert_eq!(
        accounts[3].kind,
        AccountKind::Unknown,
        "an unmapped Xero type lands in Unknown, never in a guess"
    );

    let sent = &captured.lock().await[0];
    assert_eq!(sent.method, "GET");
    assert_eq!(sent.path, "/api.xro/2.0/Accounts");
    assert_eq!(sent.tenant, "tenant-abc");
}

#[tokio::test]
async fn read_tax_codes_maps_xero_rates_to_basis_points() {
    let (addr, _captured) = stub(
        axum::http::StatusCode::OK,
        json!({}),
        json!({ "TaxRates": { "TaxRates": [
            { "TaxType": "OUTPUT2", "Name": "GST on Income",
              "Status": "ACTIVE", "EffectiveRate": 10.0 },
            { "TaxType": "NONE", "Name": "Zero Rated", "Status": "ACTIVE" },
            { "TaxType": "INPUT2", "Name": "Retired", "Status": "DELETED",
              "EffectiveRate": 5.0 }
        ] } }),
    )
    .await;
    let client = client_at(addr);

    let codes = client.read_tax_codes().await.expect("read");
    assert_eq!(codes.len(), 3);
    assert_eq!(
        codes[0].code, "OUTPUT2",
        "the TaxType is the id a line carries"
    );
    assert_eq!(codes[0].rate_bps, Some(1000), "10.0% is 1000 bps");
    assert!(codes[0].active);
    assert_eq!(
        codes[1].rate_bps, None,
        "an unpublished rate is None; 0 would be a claim"
    );
    assert!(!codes[2].active, "DELETED is not active");
}

/// The crash window: a retry asks the tenant whether the first attempt landed.
/// The marker in `Reference` is the proof of ownership.
#[tokio::test]
async fn find_document_adopts_only_the_record_carrying_our_marker() {
    let wanted = Uuid::from_u128(42);
    let other = Uuid::from_u128(43);
    let (addr, captured) = stub(
        axum::http::StatusCode::OK,
        json!({}),
        json!({ "Invoices": { "Invoices": [
            { "InvoiceID": "foreign", "InvoiceNumber": "INV-000042",
              "Reference": format!("[marula:invoice:{other}]") },
            { "InvoiceID": "9d3f-aa12", "InvoiceNumber": "INV-000042",
              "Reference": format!("[marula:invoice:{wanted}] Standing retainer") }
        ] } }),
    )
    .await;
    let client = client_at(addr);

    let found = client
        .find_document(&ExportLookup {
            kind: ExportLookupKind::Invoice,
            marula_id: wanted,
            number: "INV-000042".into(),
        })
        .await
        .expect("find")
        .expect("the marker proves the document is ours");
    assert_eq!(found.external_id, "9d3f-aa12");
    assert!(!found.created);

    let sent = &captured.lock().await[0];
    assert_eq!(sent.method, "GET");
    assert!(
        sent.query.contains("InvoiceNumber"),
        "the reconcile filters on the number it sent, got {}",
        sent.query
    );
}

#[tokio::test]
async fn find_document_returns_none_when_no_marker_matches() {
    let (addr, _captured) = stub(
        axum::http::StatusCode::OK,
        json!({}),
        json!({ "CreditNotes": { "CreditNotes": [
            { "CreditNoteID": "someone-elses", "CreditNoteNumber": "CRN-000012",
              "Reference": "" }
        ] } }),
    )
    .await;
    let client = client_at(addr);
    let found = client
        .find_document(&ExportLookup {
            kind: ExportLookupKind::CreditNote,
            marula_id: Uuid::from_u128(900),
            number: "CRN-000012".into(),
        })
        .await
        .expect("find");
    assert!(found.is_none(), "nothing found means a re-push is safe");
}

#[tokio::test]
async fn vendor_status_errors_map_to_typed_variants() {
    type ErrorCase<'a> = (
        axum::http::StatusCode,
        Value,
        &'a dyn Fn(&ExportError) -> bool,
    );
    let cases: Vec<ErrorCase<'_>> = vec![
        (
            axum::http::StatusCode::UNAUTHORIZED,
            json!({ "Message": "AuthenticationUnsuccessful" }),
            &|e| matches!(e, ExportError::Unauthorized(_)),
        ),
        (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            json!({ "Message": "Rate limit exceeded" }),
            &|e| matches!(e, ExportError::RateLimited(_)),
        ),
        (
            axum::http::StatusCode::BAD_REQUEST,
            json!({ "Message": "ValidationFailure", "Elements": [] }),
            &|e| matches!(e, ExportError::Vendor { status: 400, .. }),
        ),
    ];
    for (status, reply, matches_pred) in cases {
        let (addr, _) = stub(status, reply, json!({})).await;
        let client = client_at(addr);
        let err = client.push_invoice(&sample_invoice()).await.unwrap_err();
        assert!(
            matches_pred(&err),
            "expected typed error for {status}, got {err:?}"
        );
    }
}

#[tokio::test]
async fn retryability_is_the_4xx_5xx_split_and_never_auth() {
    assert!(!ExportError::Vendor {
        status: 422,
        message: String::new()
    }
    .retryable());
    assert!(ExportError::Vendor {
        status: 503,
        message: String::new()
    }
    .retryable());
    assert!(ExportError::RateLimited("slow down".into()).retryable());
    assert!(!ExportError::Unauthorized("401".into()).retryable());
    assert!(!ExportError::Disabled.retryable());
}

#[tokio::test]
async fn missing_contact_or_account_is_invalid_request_without_network() {
    let mut config = XeroConfig::new("tok", "tenant");
    config.base_url = "http://127.0.0.1:1".into();
    let client = XeroClient::new(config).expect("client");

    let mut inv = sample_invoice();
    inv.customer_ref = "  ".into();
    let err = client.push_invoice(&inv).await.unwrap_err();
    assert!(matches!(err, ExportError::InvalidRequest(_)), "got {err:?}");

    let mut pay = ExportPayment {
        payment_id: Uuid::from_u128(7),
        invoice_external_id: "9d3f-aa12".into(),
        account_code: "  ".into(),
        applied_on: NaiveDate::from_ymd_opt(2026, 9, 2).unwrap(),
        amount_minor: 100,
        currency: "USD".into(),
    };
    pay.account_code = " ".into();
    let err = client.push_payment(&pay).await.unwrap_err();
    assert!(matches!(err, ExportError::InvalidRequest(_)), "got {err:?}");
}

/// A hung tenant must surface as a typed failure inside the budget.
#[tokio::test]
async fn a_hung_tenant_fails_inside_the_configured_timeout() {
    let (addr, _) = stub(axum::http::StatusCode::OK, invoice_reply(), json!({})).await;
    let mut config = XeroConfig::new("tok-123", "tenant-abc");
    config.base_url = format!("http://{addr}");
    // An address that never answers: a closed port refuses fast, so the
    // budget is proven by the client's own configuration instead.
    config.base_url = "http://10.255.255.1:9".into();
    config.timeout = Duration::from_millis(50);
    let client = XeroClient::new(config).expect("client");
    let started = std::time::Instant::now();
    let err = client.read_accounts().await.unwrap_err();
    assert!(matches!(err, ExportError::Network(_)), "got {err:?}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the timeout must fire, not merely end eventually: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn config_debug_never_prints_the_token() {
    let config = XeroConfig::new("super-secret-token", "tenant-abc");
    let rendered = format!("{config:?}");
    assert!(
        !rendered.contains("super-secret-token"),
        "token leaked into Debug: {rendered}"
    );
    assert!(rendered.contains("tenant-abc"));
    assert!(rendered.contains("<redacted>"));
}

#[tokio::test]
async fn none_mode_is_disabled_and_never_dials() {
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
    let err = sink
        .push_credit_note(&sample_credit_note())
        .await
        .unwrap_err();
    assert!(matches!(err, ExportError::Disabled), "got {err:?}");
    assert!(matches!(
        sink.read_accounts().await.unwrap_err(),
        ExportError::Disabled
    ));
    assert!(matches!(
        sink.read_tax_codes().await.unwrap_err(),
        ExportError::Disabled
    ));
    assert!(matches!(
        sink.find_document(&ExportLookup {
            kind: ExportLookupKind::Invoice,
            marula_id: Uuid::from_u128(42),
            number: "INV-000042".into(),
        })
        .await
        .unwrap_err(),
        ExportError::Disabled
    ));
}
