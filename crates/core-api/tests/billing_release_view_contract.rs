//! The release ledger's money is financial data (RB2, ADR 0004): the list read
//! serializes `null` without `financial:read` instead of refusing the whole
//! ledger. The web panel keys that "Amount hidden" state off the null, so the
//! published document has to keep saying the fields are nullable.
//!
//! The phase-burn read model carries the same rule: `projects(Action::View)`
//! alone opens the route, and the amounts (plus the over-budget alert) are
//! redacted for a caller without `financial:read`.

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

/// The burn view is what the route answers with now, so the document has to
/// publish `PhaseBurnView` (not the entity's non-nullable `consumed_minor`) and
/// mark every amount nullable. The alert stays published as the typed enum:
/// without `financial:read` it always answers `none`, and a schema that allowed
/// null would make the redaction indistinguishable from "unbudgeted".
#[test]
fn the_phase_burn_view_publishes_nullable_money() {
    let doc = core_api::routes::openapi::openapi_json();
    let schemas = doc
        .get("components")
        .and_then(|components| components.get("schemas"))
        .and_then(|schemas| schemas.as_object())
        .expect("the document has component schemas");
    let view = schemas
        .get("PhaseBurnView")
        .unwrap_or_else(|| panic!("PhaseBurnView is not published ({} schemas)", schemas.len()));

    for field in ["consumed_minor", "budget_minor", "currency"] {
        let kind = &view["properties"][field]["type"];
        let nullable = kind
            .as_array()
            .is_some_and(|kinds| kinds.iter().any(|kind| kind.as_str() == Some("null")))
            || view["properties"][field]["nullable"] == true;
        assert!(nullable, "{field} is not published as nullable: {kind}");
    }
    // Minutes and the phase identity are not money: they stay required.
    assert_eq!(view["properties"]["consumed_minutes"]["type"], "integer");
    assert!(view["required"]
        .as_array()
        .is_some_and(|required| required.iter().any(|f| f == "consumed_minutes")));
    assert!(view["required"]
        .as_array()
        .is_some_and(|required| required.iter().any(|f| f == "phase_id")));
}
