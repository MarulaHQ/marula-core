//! Consuming stock puts money on the job, through the REAL router.
//!
//! Pins the row's acceptance:
//! 1. 100 units at 0.02 against a ticket write one 2.00 cost line and a
//!    movement that carries the same money, each citing the other.
//! 2. A zero-rated consumption moves the shelf and writes no line, so the job's
//!    costs are unchanged.
//! 3. A reversal restores the shelf and negates the line; a second reversal is
//!    a typed 409.

use crate::common::{auth, body_json, boot_with, register_org_token, send_headers as send};
use axum::Router;
use serde_json::{json, Value};
use uuid::Uuid;

async fn post(app: &Router, token: &str, path: &str, body: Value) -> (u16, Value) {
    let response = send(
        app,
        axum::http::Method::POST,
        path,
        &[auth(token)],
        Some(body),
    )
    .await;
    let status = response.status().as_u16();
    (status, body_json(response).await)
}

async fn get(app: &Router, token: &str, path: &str) -> Value {
    let response = send(app, axum::http::Method::GET, path, &[auth(token)], None).await;
    body_json(response).await
}

#[tokio::test]
async fn consuming_bills_the_job_and_reverses_both_sides() {
    let (state, app) =
        boot_with(|config| config.secret_key = core_api::config::SecretKey([41; 32])).await;
    let slug = format!("consume-{}", Uuid::now_v7().simple());
    let token = register_org_token(&app, "Consume Co", &slug).await;

    let (status, item) = post(
        &app,
        &token,
        "/api/v1/stock-items",
        json!({
            "name": "Patch cable",
            "kind": "consumable",
            "unit": "each",
            "currency": "USD",
            "last_unit_cost_minor": 2,
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "item");
    let item_id = item["id"].as_str().unwrap().to_string();
    let (status, location) = post(
        &app,
        &token,
        "/api/v1/stock-locations",
        json!({ "name": "Shelf", "kind": "warehouse", "idempotency_key": Uuid::now_v7().to_string() }),
    )
    .await;
    assert_eq!(status, 200, "location");
    let location_id = location["id"].as_str().unwrap().to_string();
    let (status, _) = post(
        &app,
        &token,
        "/api/v1/stock/receive",
        json!({
            "item_id": item_id,
            "location_id": location_id,
            "qty_milli": 200_000,
            "unit_cost_minor": 2,
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "receive");

    let boards = get(&app, &token, "/api/v1/boards").await;
    let board_id = boards[0]["id"].as_str().unwrap().to_string();
    let (status, ticket) = post(
        &app,
        &token,
        "/api/v1/tickets",
        json!({
            "board_id": board_id,
            "title": "Consume a patch cable",
            "priority": "p3",
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "ticket");
    let ticket_id = ticket["id"].as_str().unwrap().to_string();

    // A ticket id from nowhere is not a ticket to bill.
    let (status, _) = post(
        &app,
        &token,
        "/api/v1/stock/consume",
        json!({
            "item_id": item_id,
            "location_id": location_id,
            "qty_milli": 1000,
            "ticket_id": Uuid::now_v7(),
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 404, "an unknown ticket is refused");

    let (status, billed) = post(
        &app,
        &token,
        "/api/v1/stock/consume",
        json!({
            "item_id": item_id,
            "location_id": location_id,
            "qty_milli": 100_000,
            "ticket_id": ticket_id,
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "consume: {billed}");
    assert_eq!(billed["movement"]["cost_minor"], json!(2));
    let movement_uuid: Uuid = billed["movement"]["id"].as_str().unwrap().parse().unwrap();

    let lines = get(
        &app,
        &token,
        &format!("/api/v1/tickets/{ticket_id}/cost-lines"),
    )
    .await;
    let rows = lines.as_array().expect("cost lines");
    assert_eq!(rows.len(), 1, "one consumption, one line");
    let line = &rows[0];
    assert_eq!(line["billable"], json!(true));
    assert_eq!(line["quantity"], json!(100.0));
    assert_eq!(line["unit_cost_minor"], json!(2));
    assert_eq!(line["kind"], json!("parts"));
    let line_uuid: Uuid = line["id"].as_str().unwrap().parse().unwrap();

    // The mutual citation is a row-level fact: the line names the movement as
    // its source, and the append-only movement names the line it wrote.
    let (source_kind, source_id) = sqlx::query_as::<_, (String, Uuid)>(
        "SELECT source_kind, source_id FROM ticket_cost_lines WHERE id = $1",
    )
    .bind(line_uuid)
    .fetch_one(&state.pool)
    .await
    .expect("the line names its source");
    assert_eq!(source_kind, "stock_movement");
    assert_eq!(source_id, movement_uuid);
    let linked: Option<Uuid> =
        sqlx::query_scalar("SELECT ticket_cost_line_id FROM stock_movements WHERE id = $1")
            .bind(movement_uuid)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(linked, Some(line_uuid));

    // The charge is offered to the invoice run before it is reversed.
    let candidates = get(&app, &token, "/api/v1/invoice-candidates").await;
    let offered = |body: &Value| {
        body.as_array()
            .unwrap()
            .iter()
            .filter(|row| row["ticket_id"] == json!(ticket_id))
            .count()
    };
    assert_eq!(offered(&candidates), 1, "the charge is billable");

    // Zero-rated: the shelf still moves and the job's costs do not change.
    let (status, free) = post(
        &app,
        &token,
        "/api/v1/stock/consume",
        json!({
            "item_id": item_id,
            "location_id": location_id,
            "qty_milli": 1000,
            "ticket_id": ticket_id,
            "billable": false,
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "zero-rated consume");
    assert_eq!(free["movement"]["cost_minor"], json!(2), "the cost is kept");
    let free_movement: Uuid = free["movement"]["id"].as_str().unwrap().parse().unwrap();
    let free_line: Option<Uuid> =
        sqlx::query_scalar("SELECT ticket_cost_line_id FROM stock_movements WHERE id = $1")
            .bind(free_movement)
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(free_line, None, "a zero-rated use writes no line");
    let after_free = get(
        &app,
        &token,
        &format!("/api/v1/tickets/{ticket_id}/cost-lines"),
    )
    .await;
    assert_eq!(
        after_free.as_array().unwrap().len(),
        1,
        "the job's costs are unchanged"
    );

    // Reversal: shelf back, line negated, and only once.
    let (status, reversed) = post(
        &app,
        &token,
        &format!("/api/v1/stock/consume/{movement_uuid}/reverse"),
        json!({ "idempotency_key": Uuid::now_v7().to_string() }),
    )
    .await;
    assert_eq!(status, 200, "reverse: {reversed}");
    assert_eq!(reversed["movement"]["kind"], json!("consume_reverse"));
    assert_eq!(reversed["movement"]["qty_milli"], json!(100_000));
    assert_eq!(reversed["movement"]["cost_minor"], json!(2));

    let lines = get(
        &app,
        &token,
        &format!("/api/v1/tickets/{ticket_id}/cost-lines"),
    )
    .await;
    let rows = lines.as_array().unwrap();
    assert_eq!(rows.len(), 2, "one billable use and its reversal");
    assert_eq!(
        rows.iter()
            .filter(|row| row["quantity"] == json!(-100.0))
            .count(),
        1,
        "the reversal is the one negative line"
    );
    let net: f64 = rows
        .iter()
        .map(|row| row["quantity"].as_f64().unwrap() * row["unit_cost_minor"].as_f64().unwrap())
        .sum();
    assert_eq!(net, 0.0, "the reversal takes the charge back off the job");

    assert!(
        rows.iter().all(|row| row["billable"] == json!(false)),
        "nothing is left to invoice"
    );
    let candidates = get(&app, &token, "/api/v1/invoice-candidates").await;
    assert_eq!(
        offered(&candidates),
        0,
        "a reversed consumption cannot be invoiced"
    );

    let (status, _) = post(
        &app,
        &token,
        &format!("/api/v1/stock/consume/{movement_uuid}/reverse"),
        json!({ "idempotency_key": Uuid::now_v7().to_string() }),
    )
    .await;
    assert_eq!(status, 409, "a consumption reverses once");

    // 200 received, 100 consumed (billable), 1 consumed (zero-rated), 100
    // restored.
    let on_shelf: i64 =
        sqlx::query_scalar("SELECT qty_milli FROM stock_levels WHERE stock_item_id = $1")
            .bind(Uuid::parse_str(&item_id).unwrap())
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(on_shelf, 199_000, "the reversal puts the quantity back");
}
