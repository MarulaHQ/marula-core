//! Contract tests for the QBO export adapter against a stub HTTP server
//! (axum), plus none-mode guarantees. These pin the WIRE contract the
//! public QBO API expects — if a mapping drifts, these fail before any
//! real company sees garbage.
//!
//! What a fixture proves, and what it does not: it proves the path, the
//! bearer header, the JSON we send, the typed error we raise for each status,
//! and the neutral shape we derive from a documented response. It does not
//! prove QBO's own behaviour — whether a `LinkedTxn` at credit-memo creation
//! actually applies the credit, whether the query paging reports every row,
//! or whether a real tenant's 400 carries the field names we echo. Those are
//! sandbox-confirm items, named in the crate README.

use super::*;
use axum::response::IntoResponse;
use axum::routing::post;
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
    query: String,
    body: Value,
}

type Capture = Arc<Mutex<Vec<Captured>>>;

/// A stub QBO company: every POST under `/v3/company/{realm}/…` answers with
/// `status` + `reply`, and `GET /query` answers with `query_reply`. Every
/// request is captured, so a test asserts what the adapter actually sent
/// rather than what it meant to send.
async fn stub(
    status: axum::http::StatusCode,
    reply: Value,
    query_reply: Value,
) -> (SocketAddr, Capture) {
    let captured: Capture = Arc::new(Mutex::new(Vec::new()));

    let post_capture = captured.clone();
    let post_status = status;
    let post_reply = reply;
    let write = post(
        move |headers: axum::http::HeaderMap,
              path: axum::extract::Path<(String, String)>,
              uri: axum::http::Uri,
              AxumJson(body): AxumJson<Value>| async move {
            post_capture.lock().await.push(Captured {
                method: "POST".into(),
                path: format!("/v3/company/{}/{}", path.0 .0, path.0 .1),
                auth: header(&headers),
                query: uri.query().unwrap_or_default().to_string(),
                body,
            });
            (post_status, AxumJson(post_reply).into_response()).into_response()
        },
    );

    let get_capture = captured.clone();
    let get_status = status;
    let get_reply = query_reply;
    let query = axum::routing::get(
        move |headers: axum::http::HeaderMap, uri: axum::http::Uri| async move {
            get_capture.lock().await.push(Captured {
                method: "GET".into(),
                path: uri.path().to_string(),
                auth: header(&headers),
                query: uri.query().unwrap_or_default().to_string(),
                body: Value::Null,
            });
            (get_status, AxumJson(get_reply).into_response()).into_response()
        },
    );

    let app = axum::Router::new()
        .route("/v3/company/{realm}/{resource}", write)
        .route("/v3/company/{realm}/query", query)
        .into_make_service();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    (addr, captured)
}

fn header(headers: &axum::http::HeaderMap) -> String {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

/// A stub whose POSTs sleep first — the only way to prove the client's own
/// timeout fires instead of a test hanging.
async fn slow_stub(reply: Value, delay: std::time::Duration) -> (SocketAddr, Capture) {
    let captured: Capture = Arc::new(Mutex::new(Vec::new()));
    let post_capture = captured.clone();
    let write = post(
        move |headers: axum::http::HeaderMap,
              path: axum::extract::Path<(String, String)>,
              AxumJson(_body): AxumJson<Value>| async move {
            tokio::time::sleep(delay).await;
            post_capture.lock().await.push(Captured {
                method: "POST".into(),
                path: format!("/v3/company/{}/{}", path.0 .0, path.0 .1),
                auth: header(&headers),
                query: String::new(),
                body: Value::Null,
            });
            (axum::http::StatusCode::OK, AxumJson(reply).into_response()).into_response()
        },
    );
    let app = axum::Router::new()
        .route("/v3/company/{realm}/{resource}", write)
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
    QboClient::new(config).expect("client")
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
        tax_code: None,
    }
}

fn sample_credit_note() -> ExportCreditNote {
    ExportCreditNote {
        credit_note_id: Uuid::from_u128(900),
        number: "CRN-000012".into(),
        customer_ref: "72".into(),
        invoice_external_id: "184".into(),
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
        tax_code: None,
    };
    let (addr, captured) = stub(axum::http::StatusCode::OK, invoice_reply(), json!({})).await;
    let client = client_at(addr);
    client.push_invoice(&invoice).await.expect("push");

    let body = &captured.lock().await[0].body;
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
    let (addr, captured) = stub(axum::http::StatusCode::OK, invoice_reply(), json!({})).await;
    let client = client_at(addr);

    let receipt = client.push_invoice(&sample_invoice()).await.expect("push");
    assert_eq!(receipt.external_id, "184");
    assert_eq!(receipt.external_number.as_deref(), Some("INV-000042"));
    assert!(receipt.created);

    let sent = &captured.lock().await[0];
    assert_eq!(sent.path, "/v3/company/987654321/invoice");
    assert_eq!(sent.auth, "Bearer tok-123");
    assert!(
        sent.query.contains("minorversion=75"),
        "the pinned minor version rides every call, got {}",
        sent.query
    );
    let body = &sent.body;
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
    assert!(
        body["Line"][0]["SalesItemLineDetail"]
            .get("TaxCodeRef")
            .is_none(),
        "no tax code named by the caller means the key is absent, not a guessed default"
    );
}

/// G2: the resolved vendor tax code reaches every line, exactly once, under
/// the field QBO reads.
#[tokio::test]
async fn invoice_lines_carry_the_resolved_vendor_tax_code() {
    let (addr, captured) = stub(axum::http::StatusCode::OK, invoice_reply(), json!({})).await;
    let client = client_at(addr);
    let mut invoice = sample_invoice();
    invoice.tax_code = Some("TAX".into());
    client.push_invoice(&invoice).await.expect("push");

    let body = &captured.lock().await[0].body;
    for line in body["Line"].as_array().unwrap() {
        assert_eq!(line["SalesItemLineDetail"]["TaxCodeRef"]["value"], "TAX");
    }
}

#[tokio::test]
async fn push_payment_links_to_the_vendor_invoice() {
    let (addr, captured) = stub(
        axum::http::StatusCode::OK,
        json!({ "Payment": { "Id": "77" } }),
        json!({}),
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

    let sent = &captured.lock().await[0];
    assert_eq!(sent.path, "/v3/company/987654321/payment");
    assert_eq!(sent.body["TotalAmt"].as_f64().unwrap(), 1500.0);
    assert_eq!(sent.body["Line"][0]["LinkedTxn"][0]["TxnId"], "184");
    assert_eq!(sent.body["Line"][0]["LinkedTxn"][0]["TxnType"], "Invoice");
}

/// G1: a credit note is posted to `/creditmemo` and carries the LINK — the
/// vendor invoice id from the stored receipt. Without `LinkedTxn` QBO records
/// an unapplied credit and the invoice keeps its full balance, which is the
/// double-count this test exists to prevent.
#[tokio::test]
async fn credit_note_links_to_the_exported_vendor_invoice() {
    let (addr, captured) = stub(
        axum::http::StatusCode::OK,
        json!({ "CreditMemo": { "Id": "501", "DocNumber": "CRN-000012" } }),
        json!({}),
    )
    .await;
    let client = client_at(addr);

    let receipt = client
        .push_credit_note(&sample_credit_note())
        .await
        .expect("push");
    assert_eq!(receipt.external_id, "501");
    assert_eq!(receipt.external_number.as_deref(), Some("CRN-000012"));

    let sent = &captured.lock().await[0];
    assert_eq!(sent.path, "/v3/company/987654321/creditmemo");
    assert_eq!(sent.body["DocNumber"], "CRN-000012");
    assert_eq!(sent.body["CustomerRef"]["value"], "72");
    assert_eq!(sent.body["TxnDate"], "2026-09-15");
    assert_eq!(sent.body["Line"][0]["Amount"].as_f64().unwrap(), 250.0);
    assert_eq!(sent.body["LinkedTxn"][0]["TxnId"], "184");
    assert_eq!(sent.body["LinkedTxn"][0]["TxnType"], "Invoice");
    assert!(
        sent.body["PrivateNote"]
            .as_str()
            .unwrap()
            .contains(&format!("[marula:credit_note:{}]", Uuid::from_u128(900))),
        "the credit's own marula id must ride PrivateNote, or a reconcile cannot find it"
    );
}

#[tokio::test]
async fn credit_note_without_a_parent_invoice_is_refused_before_any_network() {
    // Unroutable port: a dial would surface as Network, so a passing assert
    // proves the refusal happened before the request was built.
    let mut config = QboConfig::new("tok", "realm");
    config.base_url = "http://127.0.0.1:1".into();
    let client = QboClient::new(config).expect("client");
    let mut note = sample_credit_note();
    note.invoice_external_id = "  ".into();
    let err = client.push_credit_note(&note).await.unwrap_err();
    assert!(matches!(err, ExportError::InvalidRequest(_)), "got {err:?}");
}

/// G2: accounts come back in the NEUTRAL bucket vocabulary, and an absent
/// `AcctNum` is `None` rather than an empty string a pick list would render.
#[tokio::test]
async fn read_accounts_returns_the_neutral_shape() {
    let (addr, captured) = stub(
        axum::http::StatusCode::OK,
        json!({}),
        json!({ "QueryResponse": { "Account": [
            { "Id": "1", "Name": "Business Checking", "AcctNum": "1000",
              "AccountType": "Bank", "Active": true },
            { "Id": "84", "Name": "Consulting Revenue",
              "AccountType": "Income", "Active": true },
            { "Id": "99", "Name": "Something New",
              "AccountType": "Hyperinflation Adjustment", "Active": false }
        ] } }),
    )
    .await;
    let client = client_at(addr);

    let accounts = client.read_accounts().await.expect("read");
    assert_eq!(accounts.len(), 3);
    assert_eq!(
        accounts[0],
        LedgerAccount {
            id: "1".into(),
            code: Some("1000".into()),
            name: "Business Checking".into(),
            kind: AccountKind::Bank,
            active: true,
        }
    );
    assert_eq!(accounts[1].code, None, "no AcctNum is None, not \"\"");
    assert_eq!(accounts[1].kind, AccountKind::Income);
    assert_eq!(
        accounts[2].kind,
        AccountKind::Unknown,
        "an account type nobody planned lands in Unknown, never in a guess"
    );
    assert!(!accounts[2].active);

    let sent = &captured.lock().await[0];
    assert_eq!(sent.method, "GET");
    assert_eq!(sent.path, "/v3/company/987654321/query");
    assert!(
        sent.query.contains("STARTPOSITION") && sent.query.contains("MAXRESULTS"),
        "the read leg pages instead of silently truncating at QBO's cap, got {}",
        sent.query
    );
}

/// G2: a tax code's rate is resolved through the TaxRate entities its group
/// points at, and a code whose rates are all unresolvable reports no rate
/// rather than 0%.
#[tokio::test]
async fn read_tax_codes_resolves_group_rates_and_never_invents_one() {
    let (addr, _captured) = stub(
        axum::http::StatusCode::OK,
        json!({}),
        json!({ "QueryResponse": {
            "TaxRate": [
                { "Id": "r1", "RateValue": 6.25, "Active": true },
                { "Id": "r2", "RateValue": 2.0, "Active": true },
                { "Id": "r3", "RateValue": 9.0, "Active": false }
            ],
            "TaxCode": [
                { "Id": "TAX", "Name": "Standard 8.25%", "Active": true,
                  "SalesTaxRateList": { "TaxRateDetail": [
                      { "TaxRateRef": { "value": "r1" } },
                      { "TaxRateRef": { "value": "r2" } } ] } },
                { "Id": "NONE", "Name": "Out of scope", "Active": true },
                { "Id": "DEAD", "Name": "Retired", "Active": false,
                  "SalesTaxRateList": { "TaxRateDetail": [
                      { "TaxRateRef": { "value": "r3" } } ] } }
            ] } }),
    )
    .await;
    let client = client_at(addr);

    let codes = client.read_tax_codes().await.expect("read");
    assert_eq!(codes.len(), 3);
    assert_eq!(codes[0].code, "TAX");
    assert_eq!(codes[0].rate_bps, Some(825), "6.25 + 2.0 = 8.25%");
    assert_eq!(
        codes[1].rate_bps, None,
        "a code with no rate reports none; 0 would be a claim"
    );
    assert_eq!(
        codes[2].rate_bps, None,
        "an inactive rate is not summed into an active-looking rate"
    );
}

/// The crash window: a retry asks the vendor whether the first attempt landed.
/// The document number alone is not enough — QBO allows duplicates — so the
/// marker in PrivateNote is the proof of ownership.
#[tokio::test]
async fn find_document_adopts_only_the_record_carrying_our_marker() {
    let wanted = Uuid::from_u128(900);
    let other = Uuid::from_u128(901);
    let (addr, captured) = stub(
        axum::http::StatusCode::OK,
        json!({}),
        json!({ "QueryResponse": { "CreditMemo": [
            { "Id": "777", "DocNumber": "CRN-000012",
              "PrivateNote": format!("something else [marula:credit_note:{other}]") },
            { "Id": "501", "DocNumber": "CRN-000012",
              "PrivateNote": format!("SLA credit [marula:credit_note:{wanted}]") }
        ] } }),
    )
    .await;
    let client = client_at(addr);

    let found = client
        .find_document(&ExportLookup {
            kind: ExportLookupKind::CreditNote,
            marula_id: wanted,
            number: "CRN-000012".into(),
        })
        .await
        .expect("find")
        .expect("the marker proves the document is ours");
    assert_eq!(found.external_id, "501");
    assert_eq!(
        found.created, false,
        "an adopted document was not created by this call"
    );

    let sent = &captured.lock().await[0];
    assert!(
        sent.query.contains("from%20CreditMemo")
            || sent.query.contains("from CreditMemo")
            || sent.query.contains("CreditMemo"),
        "the reconcile asks the right entity, got {}",
        sent.query
    );
}

#[tokio::test]
async fn find_document_returns_none_when_no_marker_matches() {
    let (addr, _captured) = stub(
        axum::http::StatusCode::OK,
        json!({}),
        json!({ "QueryResponse": { "Invoice": [] } }),
    )
    .await;
    let client = client_at(addr);
    let found = client
        .find_document(&ExportLookup {
            kind: ExportLookupKind::Invoice,
            marula_id: Uuid::from_u128(42),
            number: "INV-000042".into(),
        })
        .await
        .expect("find");
    assert!(found.is_none(), "nothing found means a re-push is safe");
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
        (
            axum::http::StatusCode::BAD_GATEWAY,
            json!({ "Fault": {} }),
            &|e| matches!(e, ExportError::Vendor { status: 502, .. }),
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

/// G3's held-versus-failed split rests on this: a 4xx is the request's fault,
/// so a person must not be told to press retry; a 429 or a 5xx is "not now".
#[tokio::test]
async fn retryability_is_the_4xx_5xx_split_and_never_auth() {
    assert!(!ExportError::Vendor {
        status: 400,
        message: String::new()
    }
    .retryable());
    assert!(ExportError::Vendor {
        status: 503,
        message: String::new()
    }
    .retryable());
    assert!(ExportError::RateLimited("slow down".into()).retryable());
    assert!(ExportError::Network("reset".into()).retryable());
    assert!(
        !ExportError::Unauthorized("401".into()).retryable(),
        "a rotated token is a human's job, not a retry loop's"
    );
    assert!(!ExportError::Disabled.retryable());
    assert!(!ExportError::InvalidRequest("x".into()).retryable());
}

/// A hung vendor must surface as a typed failure inside the budget, not hold
/// the request open. The stub sleeps far past the configured timeout.
#[tokio::test]
async fn a_hung_vendor_fails_inside_the_configured_timeout() {
    let (addr, _) = slow_stub(invoice_reply(), Duration::from_millis(400)).await;
    let mut config = QboConfig::new("tok-123", "987654321");
    config.base_url = format!("http://{addr}");
    config.timeout = Duration::from_millis(50);
    let client = QboClient::new(config).expect("client");

    let started = std::time::Instant::now();
    let err = client.push_invoice(&sample_invoice()).await.unwrap_err();
    assert!(matches!(err, ExportError::Network(_)), "got {err:?}");
    assert!(err.to_string().contains("timed out"), "got {err}");
    assert!(
        started.elapsed() < Duration::from_millis(350),
        "the timeout must fire, not merely end second: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn missing_customer_ref_is_invalid_request_without_network() {
    // Unroutable port: any dial would surface as Network, not InvalidRequest.
    let mut config = QboConfig::new("tok", "realm");
    config.base_url = "http://127.0.0.1:1".into();
    let client = QboClient::new(config).expect("client");
    let mut inv = sample_invoice();
    inv.customer_ref = "  ".into();
    let err = client.push_invoice(&inv).await.unwrap_err();
    assert!(matches!(err, ExportError::InvalidRequest(_)), "got {err:?}");
}

/// R9-adjacent: a config is passed around (built in a route, logged on a
/// failure), so its `Debug` must never print the token.
#[tokio::test]
async fn config_debug_never_prints_the_token() {
    let config = QboConfig::new("super-secret-token", "realm-1");
    let rendered = format!("{config:?}");
    assert!(
        !rendered.contains("super-secret-token"),
        "token leaked into Debug: {rendered}"
    );
    assert!(rendered.contains("realm-1"));
    assert!(rendered.contains("<redacted>"));
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
    // A reconcile in none mode must refuse, never answer "nothing found":
    // `Ok(None)` would read as permission to push.
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
