//! Consuming stock puts money on the job, through the REAL router.
//!
//! Pins the row's acceptance:
//! 1. 100 units at 0.02 against a ticket write one 2.00 cost line, drop the
//!    shelf by 100, and offer the charge to the invoice run; the movement and
//!    the line cite each other.
//! 2. The project rollup shows the same line and amount.
//! 3. A zero-rated consumption moves the shelf and writes no line, so the job's
//!    costs are unchanged.
//! 4. A reversal restores the shelf and nets the line; a second reversal is a
//!    typed 409.
//! 5. A serialized consumption names the unit on its line.

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

/// The shelf level for one item at one location, scoped to the fixture org.
async fn shelf(pool: &sqlx::PgPool, org_id: Uuid, item_id: Uuid) -> i64 {
    sqlx::query_scalar!(
        r#"SELECT qty_milli FROM stock_levels
            WHERE org_id = $1 AND stock_item_id = $2"#,
        org_id,
        item_id,
    )
    .fetch_one(pool)
    .await
    .expect("the item has a level")
}

#[tokio::test]
async fn consuming_bills_the_job_and_reverses_both_sides() {
    let (state, app) =
        boot_with(|config| config.secret_key = core_api::config::SecretKey([41; 32])).await;
    let slug = format!("consume-{}", Uuid::now_v7().simple());
    let token = register_org_token(&app, "Consume Co", &slug).await;

    // A project with a budget, a task, and the ticket promoted from that task:
    // the ticket is linked to the project, so the rollup can see its costs.
    let (status, project) = post(
        &app,
        &token,
        "/api/v1/projects",
        json!({
            "name": "Consume a part",
            "budget_minor": 500_000,
            "currency": "USD",
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "project");
    let project_id = project["id"].as_str().unwrap().to_string();
    let (status, task) = post(
        &app,
        &token,
        &format!("/api/v1/projects/{project_id}/tasks"),
        json!({
            "title": "Install the patch panel",
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "task");
    let task_id = task["id"].as_str().unwrap().to_string();
    let (status, promoted) = post(
        &app,
        &token,
        &format!("/api/v1/projects/{project_id}/tasks/{task_id}/promote"),
        json!({ "idempotency_key": Uuid::now_v7().to_string() }),
    )
    .await;
    assert_eq!(status, 200, "promote: {promoted}");
    let ticket_id = promoted["ticket_id"].as_str().unwrap().to_string();

    // The fixture's org, resolved once from the project it just created, so
    // every direct assertion below is scoped like a product read.
    let org_id: Uuid = sqlx::query_scalar!(
        r#"SELECT org_id FROM projects WHERE id = $1"#,
        Uuid::parse_str(&project_id).unwrap(),
    )
    .fetch_one(&state.pool)
    .await
    .expect("the project has an org");

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
    let item_uuid: Uuid = item_id.parse().unwrap();
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
    assert_eq!(shelf(&state.pool, org_id, item_uuid).await, 200_000);

    // The rollup starts empty for this project.
    let rollup = get(
        &app,
        &token,
        &format!("/api/v1/projects/{project_id}/cost-rollup"),
    )
    .await;
    assert_eq!(rollup["ticket_cost_base_minor"], json!(0));

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
    assert_eq!(shelf(&state.pool, org_id, item_uuid).await, 100_000);

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
    assert_eq!(line["org_id"], json!(org_id));
    let line_uuid: Uuid = line["id"].as_str().unwrap().parse().unwrap();

    // The mutual citation is a row-level fact: the line names the movement as
    // its source, and the append-only movement names the line it wrote.
    let cited = sqlx::query!(
        r#"SELECT source_kind, source_id FROM ticket_cost_lines
            WHERE org_id = $1 AND id = $2"#,
        org_id,
        line_uuid,
    )
    .fetch_one(&state.pool)
    .await
    .expect("the line names its source");
    assert_eq!(cited.source_kind.as_deref(), Some("stock_movement"));
    assert_eq!(cited.source_id, Some(movement_uuid));
    let linked = sqlx::query_scalar!(
        r#"SELECT ticket_cost_line_id FROM stock_movements
            WHERE org_id = $1 AND id = $2"#,
        org_id,
        movement_uuid,
    )
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert_eq!(linked, Some(line_uuid));

    // The charge is offered to the invoice run, and the project sees it too.
    let candidates = get(&app, &token, "/api/v1/invoice-candidates").await;
    let offered = |body: &Value| {
        body.as_array()
            .unwrap()
            .iter()
            .filter(|row| row["ticket_id"] == json!(ticket_id))
            .count()
    };
    assert_eq!(offered(&candidates), 1, "the charge is billable");
    let rollup = get(
        &app,
        &token,
        &format!("/api/v1/projects/{project_id}/cost-rollup"),
    )
    .await;
    assert_eq!(rollup["ticket_cost_base_minor"], json!(200));
    let rollup_lines = rollup["ticket_cost_lines"].as_array().unwrap();
    assert_eq!(rollup_lines.len(), 1);
    assert_eq!(rollup_lines[0]["line_id"], json!(line_uuid));
    assert_eq!(rollup_lines[0]["amount_minor"], json!(200));

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
    assert_eq!(shelf(&state.pool, org_id, item_uuid).await, 99_000);
    let free_movement: Uuid = free["movement"]["id"].as_str().unwrap().parse().unwrap();
    let free_line = sqlx::query_scalar!(
        r#"SELECT ticket_cost_line_id FROM stock_movements
            WHERE org_id = $1 AND id = $2"#,
        org_id,
        free_movement,
    )
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

    let rollup = get(
        &app,
        &token,
        &format!("/api/v1/projects/{project_id}/cost-rollup"),
    )
    .await;
    assert_eq!(
        rollup["ticket_cost_base_minor"],
        json!(200),
        "a zero-rated use leaves the project total alone"
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
    assert_eq!(shelf(&state.pool, org_id, item_uuid).await, 199_000);

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
    let rollup = get(
        &app,
        &token,
        &format!("/api/v1/projects/{project_id}/cost-rollup"),
    )
    .await;
    assert_eq!(
        rollup["ticket_cost_base_minor"],
        json!(0),
        "the project total returns to where it was"
    );

    // A negative line is an offset, not a charge.
    let negative_id = rows
        .iter()
        .find(|row| row["quantity"] == json!(-100.0))
        .and_then(|row| row["id"].as_str())
        .expect("the reversal line");
    let patch = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/tickets/{ticket_id}/cost-lines/{negative_id}"),
        &[auth(&token)],
        Some(json!({ "billable": true, "idempotency_key": Uuid::now_v7().to_string() })),
    )
    .await;
    assert_eq!(
        patch.status().as_u16(),
        409,
        "a reversal offset cannot be billed"
    );

    let patch_original = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/tickets/{ticket_id}/cost-lines/{line_uuid}"),
        &[auth(&token)],
        Some(json!({ "billable": true, "idempotency_key": Uuid::now_v7().to_string() })),
    )
    .await;
    assert_eq!(
        patch_original.status().as_u16(),
        409,
        "a reversed charge cannot be made billable again"
    );

    let (status, again) = post(
        &app,
        &token,
        &format!("/api/v1/stock/consume/{movement_uuid}/reverse"),
        json!({ "idempotency_key": Uuid::now_v7().to_string() }),
    )
    .await;
    assert_eq!(status, 409, "a consumption reverses once");
    assert_eq!(
        again["error"]["reason"],
        json!("consumption_already_reversed"),
        "the repeat is the typed conflict"
    );
    assert_eq!(shelf(&state.pool, org_id, item_uuid).await, 199_000);

    // A same-key retry replays the original response instead of failing.
    let key = Uuid::now_v7().to_string();
    let (status, first) = post(
        &app,
        &token,
        "/api/v1/stock/consume",
        json!({
            "item_id": item_id,
            "location_id": location_id,
            "qty_milli": 1000,
            "ticket_id": ticket_id,
            "idempotency_key": key,
        }),
    )
    .await;
    assert_eq!(status, 200, "fresh consumption");
    let (status, replay) = post(
        &app,
        &token,
        "/api/v1/stock/consume",
        json!({
            "item_id": item_id,
            "location_id": location_id,
            "qty_milli": 1000,
            "ticket_id": ticket_id,
            "idempotency_key": key,
        }),
    )
    .await;
    assert_eq!(status, 200, "replay");
    assert_eq!(replay["movement"]["id"], first["movement"]["id"]);
    assert_eq!(shelf(&state.pool, org_id, item_uuid).await, 198_000);

    // The consumed charge is not deletable; the reversal is its undo.
    let delete = send(
        &app,
        axum::http::Method::DELETE,
        &format!(
            "/api/v1/tickets/{ticket_id}/cost-lines/{}",
            rows[0]["id"].as_str().unwrap()
        ),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(
        delete.status().as_u16(),
        409,
        "a stock-sourced line is not deleted"
    );
}

#[tokio::test]
async fn serialized_consumption_names_the_unit_on_the_line() {
    let (state, app) =
        boot_with(|config| config.secret_key = core_api::config::SecretKey([42; 32])).await;
    let slug = format!("consume-sn-{}", Uuid::now_v7().simple());
    let token = register_org_token(&app, "Serial Co", &slug).await;

    let (status, item) = post(
        &app,
        &token,
        "/api/v1/stock-items",
        json!({
            "name": "Switch",
            "kind": "serialized",
            "unit": "each",
            "currency": "USD",
            "last_unit_cost_minor": 90_000,
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
    let (status, received) = post(
        &app,
        &token,
        "/api/v1/stock/receive",
        json!({
            "item_id": item_id,
            "location_id": location_id,
            "qty_milli": 1000,
            "unit_cost_minor": 90_000,
            "serial_numbers": ["SW-0042"],
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "receive: {received}");
    let serial_id = received["movement"]["serial_id"]
        .as_str()
        .expect("the receive names its serial")
        .to_string();

    let boards = get(&app, &token, "/api/v1/boards").await;
    let board_id = boards[0]["id"].as_str().unwrap().to_string();
    let (status, ticket) = post(
        &app,
        &token,
        "/api/v1/tickets",
        json!({
            "board_id": board_id,
            "title": "Install the switch",
            "priority": "p3",
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "ticket");
    let ticket_id = ticket["id"].as_str().unwrap().to_string();

    let (status, consumed) = post(
        &app,
        &token,
        "/api/v1/stock/consume",
        json!({
            "item_id": item_id,
            "location_id": location_id,
            "qty_milli": 1000,
            "serial_id": serial_id,
            "ticket_id": ticket_id,
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "consume: {consumed}");
    let lines = get(
        &app,
        &token,
        &format!("/api/v1/tickets/{ticket_id}/cost-lines"),
    )
    .await;
    let rows = lines.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    let description = rows[0]["description"].as_str().unwrap();
    assert!(
        description.contains("SN SW-0042"),
        "the line names the unit: {description}"
    );
    assert_eq!(rows[0]["quantity"], json!(1.0));
    assert_eq!(rows[0]["unit_cost_minor"], json!(90_000));
    let org_id: Uuid = rows[0]["org_id"].as_str().unwrap().parse().unwrap();
    let movement_id = consumed["movement"]["id"].as_str().unwrap().to_string();
    let serial_uuid: Uuid = serial_id.parse().unwrap();

    // The reversal puts the unit back in stock at the shelf.
    let (status, _) = post(
        &app,
        &token,
        &format!("/api/v1/stock/consume/{movement_id}/reverse"),
        json!({ "idempotency_key": Uuid::now_v7().to_string() }),
    )
    .await;
    assert_eq!(status, 200, "serialized reversal");
    let serial = sqlx::query!(
        r#"SELECT status, location_id FROM stock_serials WHERE org_id = $1 AND id = $2"#,
        org_id,
        serial_uuid,
    )
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert_eq!(serial.status, "in_stock");
    assert_eq!(
        serial.location_id,
        Some(Uuid::parse_str(&location_id).unwrap())
    );

    // A unit that came back another way is not restored twice: consume it,
    // receive it again, then reverse — the custody check refuses.
    let (status, second) = post(
        &app,
        &token,
        "/api/v1/stock/consume",
        json!({
            "item_id": item_id,
            "location_id": location_id,
            "qty_milli": 1000,
            "serial_id": serial_id,
            "ticket_id": ticket_id,
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "consume again");
    let (status, _) = post(
        &app,
        &token,
        "/api/v1/stock/receive",
        json!({
            "item_id": item_id,
            "location_id": location_id,
            "qty_milli": 1000,
            "unit_cost_minor": 90_000,
            "serial_numbers": ["SW-0042"],
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "received elsewhere");
    let second_movement = second["movement"]["id"].as_str().unwrap().to_string();
    let (status, conflict) = post(
        &app,
        &token,
        &format!("/api/v1/stock/consume/{second_movement}/reverse"),
        json!({ "idempotency_key": Uuid::now_v7().to_string() }),
    )
    .await;
    assert_eq!(status, 409, "the unit moved since: {conflict}");
}

#[tokio::test]
async fn a_billable_use_needs_a_stocked_cost() {
    let (_state, app) =
        boot_with(|config| config.secret_key = core_api::config::SecretKey([43; 32])).await;
    let slug = format!("consume-cost-{}", Uuid::now_v7().simple());
    let token = register_org_token(&app, "Cost Co", &slug).await;

    let (status, item) = post(
        &app,
        &token,
        "/api/v1/stock-items",
        json!({
            "name": "Unknown part",
            "kind": "consumable",
            "unit": "each",
            "currency": "USD",
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
            "qty_milli": 10_000,
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "receive without a cost");

    let boards = get(&app, &token, "/api/v1/boards").await;
    let board_id = boards[0]["id"].as_str().unwrap().to_string();
    let (status, ticket) = post(
        &app,
        &token,
        "/api/v1/tickets",
        json!({
            "board_id": board_id,
            "title": "Use an unpriced part",
            "priority": "p3",
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "ticket");
    let ticket_id = ticket["id"].as_str().unwrap().to_string();

    let (status, refusal) = post(
        &app,
        &token,
        "/api/v1/stock/consume",
        json!({
            "item_id": item_id,
            "location_id": location_id,
            "qty_milli": 1000,
            "ticket_id": ticket_id,
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 409, "a billable use needs a cost: {refusal}");

    // Zero-rated has no line to price, so the shelf move is allowed.
    let (status, _) = post(
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
    assert_eq!(status, 200, "zero-rated needs no cost");
}

#[tokio::test]
async fn a_dispatcher_sees_no_money_in_a_consumption() {
    let (state, app) =
        boot_with(|config| config.secret_key = core_api::config::SecretKey([44; 32])).await;
    let slug = format!("consume-mask-{}", Uuid::now_v7().simple());
    let token = register_org_token(&app, "Mask Co", &slug).await;

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
            "qty_milli": 10_000,
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
            "title": "Use a cable",
            "priority": "p3",
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "ticket");
    let ticket_id = ticket["id"].as_str().unwrap().to_string();

    // A dispatcher has commercial edit but no financial read.
    let email = format!("dispatcher-{}@example.test", Uuid::now_v7().simple());
    let invite = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/org/users",
        &[auth(&token)],
        Some(json!({
            "email": email,
            "display_name": "Desk",
            "role": "dispatcher",
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    assert_eq!(invite.status().as_u16(), 200, "invite");
    let setup_token = body_json(invite).await["invite"]["setup_token"]
        .as_str()
        .unwrap()
        .to_string();
    let accept = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/org/users/accept-invite",
        &[],
        Some(json!({ "token": setup_token, "password": "another-correct-horse" })),
    )
    .await;
    assert_eq!(accept.status().as_u16(), 200, "accept");
    let desk = body_json(accept).await["access_token"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, body) = post(
        &app,
        &desk,
        "/api/v1/stock/consume",
        json!({
            "item_id": item_id,
            "location_id": location_id,
            "qty_milli": 1000,
            "ticket_id": ticket_id,
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "dispatcher consumes: {body}");
    assert!(body["movement"]["cost_minor"].is_null(), "cost is masked");
    assert!(
        body["mutation"]["after"]["cost_minor"].is_null(),
        "the mutation snapshot is masked"
    );
    assert!(
        body["mutation"]["after"]["base_cost_minor"].is_null(),
        "the base amount is masked"
    );

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/tickets/{ticket_id}/cost-lines"),
        &[auth(&desk)],
        None,
    )
    .await;
    assert_eq!(
        response.status().as_u16(),
        403,
        "reading money needs the permission"
    );
    let _ = &state;
}

/// The lock order every leg uses: serial (when there is one), then the item,
/// then the level rows. A concurrent receipt or withdrawal must not be able to
/// close a cycle and deadlock the ledger.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_movements_keep_one_lock_order() {
    let (_state, app) =
        boot_with(|config| config.secret_key = core_api::config::SecretKey([45; 32])).await;
    let slug = format!("consume-lock-{}", Uuid::now_v7().simple());
    let token = register_org_token(&app, "Lock Co", &slug).await;

    let (status, item) = post(
        &app,
        &token,
        "/api/v1/stock-items",
        json!({
            "name": "Lock cable",
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
            "qty_milli": 50_000,
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
            "title": "Concurrent use",
            "priority": "p3",
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "ticket");
    let ticket_id = ticket["id"].as_str().unwrap().to_string();

    let consume_a = post(
        &app,
        &token,
        "/api/v1/stock/consume",
        json!({
            "item_id": item_id,
            "location_id": location_id,
            "qty_milli": 1000,
            "ticket_id": ticket_id,
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    );
    let receive_b = post(
        &app,
        &token,
        "/api/v1/stock/receive",
        json!({
            "item_id": item_id,
            "location_id": location_id,
            "qty_milli": 1000,
            "unit_cost_minor": 3,
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    );
    let consume_c = post(
        &app,
        &token,
        "/api/v1/stock/consume",
        json!({
            "item_id": item_id,
            "location_id": location_id,
            "qty_milli": 1000,
            "ticket_id": ticket_id,
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    );
    let joined = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        tokio::join!(consume_a, receive_b, consume_c)
    })
    .await
    .expect("no deadlock: every leg finished");
    for (status, body) in [joined.0, joined.1, joined.2] {
        assert_eq!(status, 200, "every leg lands: {body}");
    }

    // A serialized unit takes the serial lock before the item lock, the same
    // order a ship-out takes; the pair must resolve, not deadlock.
    let (status, serial_item) = post(
        &app,
        &token,
        "/api/v1/stock-items",
        json!({
            "name": "Lock switch",
            "kind": "serialized",
            "unit": "each",
            "currency": "USD",
            "last_unit_cost_minor": 90_000,
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "serial item");
    let serial_item_id = serial_item["id"].as_str().unwrap().to_string();
    let (status, received) = post(
        &app,
        &token,
        "/api/v1/stock/receive",
        json!({
            "item_id": serial_item_id,
            "location_id": location_id,
            "qty_milli": 1000,
            "unit_cost_minor": 90_000,
            "serial_numbers": ["LOCK-1"],
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    )
    .await;
    assert_eq!(status, 200, "serial receive");
    let serial_id = received["movement"]["serial_id"]
        .as_str()
        .unwrap()
        .to_string();

    let consume_serial = post(
        &app,
        &token,
        "/api/v1/stock/consume",
        json!({
            "item_id": serial_item_id,
            "location_id": location_id,
            "qty_milli": 1000,
            "serial_id": serial_id,
            "ticket_id": ticket_id,
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    );
    let ship_serial = post(
        &app,
        &token,
        "/api/v1/stock/ship-out",
        json!({
            "item_id": serial_item_id,
            "location_id": location_id,
            "qty_milli": 1000,
            "serial_id": serial_id,
            "ticket_id": ticket_id,
            "carrier": "UPS",
            "idempotency_key": Uuid::now_v7().to_string(),
        }),
    );
    let pair = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        tokio::join!(consume_serial, ship_serial)
    })
    .await
    .expect("no deadlock between a use and a ship-out of one unit");
    for (status, body) in [pair.0, pair.1] {
        assert!(
            status == 200 || status == 409,
            "a contested unit resolves to a landing or a typed conflict: {status} {body}"
        );
    }
}
