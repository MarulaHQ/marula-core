//! The release ledger's money is financial data (RB2, ADR 0004): the list read
//! serializes `null` without `financial:read` instead of refusing the whole
//! ledger. The web panel keys that "Amount hidden" state off the null, so the
//! published document has to keep saying the fields are nullable.

#[test]
fn the_release_ledger_view_publishes_nullable_money() {
    let doc = core_api::routes::openapi::openapi_json();
    let schemas = doc
        .get("components")
        .and_then(|components| components.get("schemas"))
        .and_then(|schemas| schemas.as_object())
        .expect("the document has component schemas");
    let view = schemas.get("BillingReleaseView").unwrap_or_else(|| {
        panic!(
            "BillingReleaseView is not published ({} schemas)",
            schemas.len()
        )
    });

    for field in ["amount_minor", "currency"] {
        let kind = &view["properties"][field]["type"];
        let nullable = kind
            .as_array()
            .is_some_and(|kinds| kinds.iter().any(|kind| kind.as_str() == Some("null")))
            || view["properties"][field]["nullable"] == true;
        assert!(nullable, "{field} is not published as nullable: {kind}");
    }
}
