//! Engine unit + property tests: the parser matrix (CSV quoting/encoding
//! quirks, JSON nulls), the conflict-policy truth table
//! ×{missing,identical,different}, dry-run report accuracy inputs, and
//! shrinkable generators proving no input can panic a mapper.

use super::*;
use proptest::prelude::*;
use std::collections::HashMap;

// --- CSV matrix ----------------------------------------------------------------

#[test]
fn csv_bom_crlf_quotes_and_embedded_newlines_roundtrip() {
    let input = "\u{feff}external_key,name,notes\r\nACME,\"Acme, Inc.\",\"line one\r\nline two \"\"q\"\"\"\r\nGLOB,Glob Inc,\r\n";
    let rows = parse_csv_chunk(input).expect("parses");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].fields["name"].clone(), "Acme, Inc.");
    // RFC 4180: line breaks inside quoted cells are preserved verbatim
    // (CRLF here), and doubled quotes unescape once.
    assert_eq!(
        rows[0].fields["notes"].clone(),
        "line one\r\nline two \"q\""
    );
    assert_eq!(rows[1].fields["name"].clone(), "Glob Inc");
    // BOM stripped, so column names are canonical.
    assert_eq!(rows[0].fields["external_key"].clone(), "ACME");
}

#[test]
fn csv_blank_lines_lone_cr_and_no_final_newline_parse() {
    let input = "external_key,name\n\nA,Alpha\nB,Beta\rC\r\n";
    let rows = parse_csv_chunk(input).expect("parses");
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1].fields["external_key"].clone(), "B");
    // A bare CR also terminates records, so C forms its own short row and
    // pads to an empty optional-style trailing column.
    assert_eq!(rows[2].fields["external_key"].clone(), "C");
    assert_eq!(rows[2].fields["name"].clone(), "");
}

#[test]
fn csv_short_rows_pad_and_long_rows_are_fatal() {
    let ok = parse_csv_chunk("external_key,name,notes\nKEY-1,Solo\n").expect("short row pads");
    assert_eq!(ok[0].fields.get("notes").map(String::as_str), Some(""));

    let long = parse_csv_chunk("external_key,name\nK,N,extra,junk\n");
    assert!(
        matches!(long, Err(ImportError::Malformed(ref m)) if m.contains("extra data")),
        "extra columns must be fatal, got {long:?}"
    );
}

#[test]
fn csv_header_problems_are_fatal() {
    for bad in [
        "external_key,external_key\nA,B\n",
        "external_key,,name\nA,,B\n",
    ] {
        assert!(
            matches!(parse_csv_chunk(bad), Err(ImportError::Malformed(_))),
            "expected fatal header problem: {bad}"
        );
    }
    assert!(matches!(
        parse_csv_chunk("\n\n"),
        Err(ImportError::Malformed(m)) if m.contains("no header")
    ));
}

#[test]
fn csv_unterminated_quote_does_not_panic() {
    // Tolerant tail rule: an unterminated quote swallows to end-of-input as
    // literal content instead of panicking or dropping the row silently.
    let rows = parse_csv_chunk("external_key,name\n\"OPEN,oops\n").expect("tolerant tail");
    assert!(rows[0].fields["external_key"].trim().starts_with("OPEN"));
}

// --- JSON matrix -----------------------------------------------------------------

#[test]
fn json_null_behaves_like_absent() {
    let payload = serde_json::json!([
        { "external_key": "K1", "name": "Acme", "notes": null },
        { "external_key": null, "name": "Keyless" }
    ]);
    let rows = parse_json_chunk(&payload).expect("parse");
    assert_eq!(rows[0].fields.get("notes"), None);
    let specs = build_specs(EntityKind::Client, &rows);
    assert!(specs[0].spec.is_ok(), "{:?}", specs[0].spec);
    match &specs[1] {
        SpecResult {
            index: 1,
            external_key: None,
            spec: Err(errors),
        } => assert!(errors.iter().any(|e| e.contains("external_key"))),
        other => panic!("expected keyless failure, got {other:?}"),
    }
}

#[test]
fn json_scalars_coerce_and_nested_values_are_fatal() {
    let payload = serde_json::json!([
        { "external_key": "S1", "client_key": "C1", "name": "HQ",
          "latitude": 52.52, "longitude": -13, "is_primary": true }
    ]);
    let rows = parse_json_chunk(&payload).expect("parse");
    assert_eq!(rows[0].fields["latitude"], "52.52");
    assert_eq!(rows[0].fields["longitude"], "-13");

    let bad = serde_json::json!([{ "external_key": "X", "name": [1] }]);
    assert!(matches!(
        parse_json_chunk(&bad),
        Err(ImportError::Malformed(m)) if m.contains("nested array")
    ));
    assert!(matches!(
        parse_json_chunk(&serde_json::json!({"not": "an array"})),
        Err(ImportError::Malformed(m)) if m.contains("array of row objects")
    ));
}

// --- field validation --------------------------------------------------------------

#[test]
fn unknown_columns_and_bad_scalars_surface_per_row() {
    let rows = vec![RawRow {
        index: 0,
        fields: [
            ("external_key".to_string(), "K".to_string()),
            ("name".to_string(), "N".to_string()),
            ("bogus_column".to_string(), "?".to_string()),
            ("email".to_string(), "not-an-email".to_string()),
        ]
        .into_iter()
        .collect(),
    }];
    let specs = build_specs(EntityKind::Contact, &rows);
    let errs = specs[0].spec.as_ref().unwrap_err();
    assert!(errs.iter().any(|e| e.contains("unknown column")));
    assert!(errs.iter().any(|e| e.starts_with("email:")));
    assert!(errs.iter().any(|e| e.contains("client_key")));
}

#[test]
fn scalar_ranges_and_validators_hold() {
    assert_eq!(validate_email("ops@example.com"), Ok(()));
    assert!(validate_email("no at sign").is_err());
    assert_eq!(validate_url("https://meridian.example/x?y=1"), Ok(()));
    assert!(validate_url("ftp://host").is_err());
    assert!(validate_url("https://bad host").is_err());

    let site_ok = build_specs(
        EntityKind::Site,
        &[RawRow {
            index: 0,
            fields: [
                ("external_key", "S"),
                ("client_key", "C"),
                ("name", "HQ"),
                ("latitude", "-90.0"),
                ("longitude", "180.000001"),
                ("timezone", "Europe/Berlin"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        }],
    );
    let errors = site_ok[0].spec.as_ref().unwrap_err();
    assert!(errors.iter().any(|e| e.contains("longitude")));

    let tz_bad = build_specs(
        EntityKind::Site,
        &[RawRow {
            index: 0,
            fields: [
                ("external_key", "S"),
                ("client_key", "C"),
                ("name", "HQ"),
                ("timezone", "Europe/Berlin; drop table"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        }],
    );
    assert!(tz_bad[0]
        .spec
        .as_ref()
        .unwrap_err()
        .iter()
        .any(|e| e.contains("timezone")));
}

// --- conflict truth table ----------------------------------------------------------

struct Lookups {
    mapped: std::collections::HashMap<String, Uuid>,
    parents: std::collections::HashMap<(EntityKind, String), Uuid>,
    stored: std::collections::HashMap<Uuid, BTreeMap<String, String>>,
}

fn identity_lookups() -> Lookups {
    Lookups {
        mapped: std::collections::HashMap::new(),
        parents: std::collections::HashMap::new(),
        stored: std::collections::HashMap::new(),
    }
}

impl Lookups {
    fn parent<'a>(&'a self) -> impl Fn(EntityKind, &str) -> Option<Uuid> + 'a {
        move |kind, key| self.parents.get(&(kind, key.to_string())).copied()
    }
    fn mapping<'a>(&'a self) -> impl Fn(&str) -> Option<Uuid> + 'a {
        move |key| self.mapped.get(key).copied()
    }
    fn projection<'a>(&'a self) -> impl Fn(Uuid) -> Option<BTreeMap<String, String>> + 'a {
        move |id| self.stored.get(&id).cloned()
    }
}

fn client_row(index: usize, key: &str, name: &str) -> RawRow {
    RawRow {
        index,
        fields: [("external_key", key), ("name", name), ("notes", "n")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

/// Conflict semantics across every policy × {missing, identical, different}.
/// Rows sharing a key run in SEPARATE chunks — intra-chunk duplicate
/// detection is a different rule and gets its own test below.
#[test]
fn conflict_truth_table_is_exact() {
    let existing_id = Uuid::from_u128(42);
    let mut base = identity_lookups();
    base.mapped.insert("DUP".into(), existing_id);
    // The stored projection must be the CANONICAL one — exactly what a live
    // Client loaded through core-db projects to.
    let stored_spec = build_specs(EntityKind::Client, &[client_row(9, "DUP", "Old Name")]);
    let projection = stored_spec[0]
        .spec
        .as_ref()
        .expect("seed validates")
        .projection();
    assert_eq!(projection.get("notes").map(String::as_str), Some("n"));
    base.stored.insert(existing_id, projection);

    for policy in [
        ConflictPolicy::Skip,
        ConflictPolicy::Update,
        ConflictPolicy::Fail,
    ] {
        // Missing: unmapped keys create under every policy.
        let fresh = plan_chunk(
            &build_specs(EntityKind::Client, &[client_row(0, "NEW", "Fresh")]),
            policy,
            &base.parent(),
            &base.mapping(),
            &base.projection(),
            true,
        );
        assert_eq!(fresh.reports[0].outcome, RowOutcome::Ok, "{policy:?}");
        assert_eq!(fresh.actions.len(), 1);

        // Identical: skips BEFORE policy under all three — the idempotency
        // doctrine that keeps re-pushes eventless.
        let identical = plan_chunk(
            &build_specs(EntityKind::Client, &[client_row(0, "DUP", "Old Name")]),
            policy,
            &base.parent(),
            &base.mapping(),
            &base.projection(),
            true,
        );
        assert_eq!(
            identical.reports[0].outcome,
            RowOutcome::WouldSkip {
                reason: "identical to existing row".into(),
            },
            "{policy:?}"
        );
        assert!(identical.actions.is_empty());

        // Different: policy decides skip/update/fail-conflict. The fail
        // variant always carries a named conflict error for operators.
        let differing = plan_chunk(
            &build_specs(EntityKind::Client, &[client_row(0, "DUP", "Renamed")]),
            policy,
            &base.parent(),
            &base.mapping(),
            &base.projection(),
            false,
        );
        match (&differing.reports[0].outcome, policy) {
            (RowOutcome::WouldSkip { reason }, ConflictPolicy::Skip) => {
                assert!(reason.contains("skip"))
            }
            (RowOutcome::WouldUpdate, ConflictPolicy::Update) => {}
            (RowOutcome::Failed { errors }, ConflictPolicy::Fail) => {
                assert!(errors.iter().any(|e| e.contains("conflict")));
            }
            (other, p) => panic!("unexpected outcome {other:?} for {p:?}"),
        }
    }
}

#[test]
fn fail_policy_reports_conflicts_but_dry_run_continues() {
    let existing_id = Uuid::from_u128(7);
    let mut lk = identity_lookups();
    lk.mapped.insert("X".into(), existing_id);
    // Full canonical projection of the stored row (name AND empty notes) so
    // an exact re-push short-circuits to no-op skip.
    let mut stored_row = std::collections::BTreeMap::new();
    stored_row.insert("name".to_string(), "Stored".to_string());
    stored_row.insert("notes".to_string(), String::new());
    lk.stored.insert(existing_id, stored_row);
    let rows = vec![client_row(0, "X", "Other"), client_row(1, "Y", "Fine")];
    let specs = build_specs(EntityKind::Client, &rows);

    // Dry-run pass (fail_fast=false): report stays per-row.
    let plan = plan_chunk(
        &specs,
        ConflictPolicy::Fail,
        &lk.parent(),
        &lk.mapping(),
        &lk.projection(),
        false,
    );
    assert_eq!(plan.summary.failed, 1);
    assert_eq!(plan.summary.ok, 1);
    // Commit pass for a fail batch: the chunk is tainted — nothing may
    // execute even though clean rows still produced planned actions; the
    // executor gates on chunk_is_executable before running any of them.
    let tainted = plan_chunk(
        &specs,
        ConflictPolicy::Fail,
        &lk.parent(),
        &lk.mapping(),
        &lk.projection(),
        true,
    );
    assert!(!chunk_is_executable(&tainted));
    assert_eq!(tainted.summary.failed, 1);
    // …but an exact-content re-push still commits (as a pure skip).
    let quiet_rows = vec![RawRow {
        index: 0,
        fields: [("external_key", "X"), ("name", "Stored"), ("notes", "")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }];
    let quiet = plan_chunk(
        &build_specs(EntityKind::Client, &quiet_rows),
        ConflictPolicy::Fail,
        &lk.parent(),
        &lk.mapping(),
        &lk.projection(),
        true,
    );
    assert!(chunk_is_executable(&quiet));
    assert!(quiet.actions.is_empty());
}

#[test]
fn duplicates_within_chunk_and_missing_parents_fail_cleanly() {
    let mut lk = identity_lookups();
    lk.parents
        .insert((EntityKind::Client, "C1".into()), Uuid::from_u128(3));

    let dupes = vec![client_row(0, "SAME", "One"), client_row(1, "SAME", "Two")];
    let plan = plan_chunk(
        &build_specs(EntityKind::Client, &dupes),
        ConflictPolicy::Update,
        &lk.parent(),
        &lk.mapping(),
        &lk.projection(),
        true,
    );
    match &plan.reports[1].outcome {
        RowOutcome::Failed { errors } => assert!(errors[0].contains("duplicate external_key")),
        other => panic!("expected duplicate failure, got {other:?}"),
    }

    let orphan_site = serde_json::json!([{
        "external_key": "SITE-1", "client_key": "GHOST", "name": "Branch"
    }]);
    let rows = parse_json_chunk(&orphan_site).expect("rows");
    let plan = plan_chunk(
        &build_specs(EntityKind::Site, &rows),
        ConflictPolicy::Skip,
        &lk.parent(),
        &lk.mapping(),
        &lk.projection(),
        false,
    );
    match &plan.reports[0].outcome {
        RowOutcome::Failed { errors } => assert!(errors[0].contains("client_key")),
        other => panic!("expected unresolved parent failure, got {other:?}"),
    }
}

#[test]
fn contact_parent_chain_requires_resolvable_site_when_present() {
    let mut lk = identity_lookups();
    let client_id = Uuid::from_u128(11);
    let site_id = Uuid::from_u128(12);
    lk.parents
        .insert((EntityKind::Client, "CL".into()), client_id);
    lk.parents.insert((EntityKind::Site, "ST".into()), site_id);

    let mk = |site: Option<&str>| {
        let mut f = std::collections::BTreeMap::new();
        f.insert("external_key".to_string(), "CT".to_string());
        f.insert("client_key".to_string(), "CL".to_string());
        f.insert("name".to_string(), "Pat".to_string());
        if let Some(s) = site {
            f.insert("site_key".to_string(), s.to_string());
        }
        RawRow {
            index: 0,
            fields: f,
        }
    };
    for site_key in [None, Some("ST")] {
        let plan = plan_chunk(
            &build_specs(EntityKind::Contact, &[mk(site_key)]),
            ConflictPolicy::Update,
            &lk.parent(),
            &lk.mapping(),
            &lk.projection(),
            true,
        );
        assert_eq!(plan.reports[0].outcome, RowOutcome::Ok, "{site_key:?}");
    }
    let plan = plan_chunk(
        &build_specs(EntityKind::Contact, &[mk(Some("NOPE"))]),
        ConflictPolicy::Update,
        &lk.parent(),
        &lk.mapping(),
        &lk.projection(),
        false,
    );
    assert!(matches!(
        &plan.reports[0].outcome,
        RowOutcome::Failed { errors } if errors[0].contains("site_key")
    ));
}

#[test]
fn chunk_limits_are_enforced() {
    assert_eq!(enforce_limits(MAX_ROWS_PER_CHUNK, MAX_CHUNK_BYTES), Ok(()));
    assert!(matches!(
        enforce_limits(MAX_ROWS_PER_CHUNK + 1, 10),
        Err(ImportError::TooManyRows(_))
    ));
    assert!(matches!(
        enforce_limits(1, MAX_CHUNK_BYTES + 1),
        Err(ImportError::TooManyBytes(_))
    ));
}

// --- properties ----------------------------------------------------------------------

fn arb_simple_text(max: usize) -> impl Strategy<Value = String> {
    prop::collection::vec("[a-zA-Z0-9 _.-]", 0..max)
        .prop_map(|chars| chars.into_iter().collect::<String>())
}

proptest! {
    /// Any clean site row survives a JSON serialize → parse → validate
    /// round trip bit-for-bit (floats included).
    #[test]
    fn json_roundtrip_preserves_specs(
        key in "[a-zA-Z0-9_-]{1,40}",
        name in "[a-zA-Z0-9][a-zA-Z0-9 _.-]{0,59}",
        notes in proptest::option::of(arb_simple_text(80)),
        lat in proptest::option::of(-90.0f64..90.0),
        lon in proptest::option::of(-180.0f64..180.0),
    ) {
        let mut obj = serde_json::Map::new();
        obj.insert("external_key".into(), serde_json::Value::String(key.clone()));
        obj.insert("client_key".into(), serde_json::Value::String("C1".into()));
        obj.insert("name".into(), serde_json::Value::String(name.clone()));
        if let Some(n) = &notes { obj.insert("address".into(), serde_json::Value::String(n.clone())); }
        if let Some(v) = lat { obj.insert("latitude".into(), serde_json::json!(v)); }
        if let Some(v) = lon { obj.insert("longitude".into(), serde_json::json!(v)); }
        let payload = serde_json::Value::Array(vec![serde_json::Value::Object(obj)]);
        let rows = parse_json_chunk(&payload).expect("generated payload parses");
        let specs = build_specs(EntityKind::Site, &rows);
        let spec = specs[0].spec.as_ref().expect("generated row validates");
        match spec {
            ImportSpec::Client(_) | ImportSpec::Contact(_) | ImportSpec::Board(_) | ImportSpec::Ticket(_) | ImportSpec::TimeEntry(_) | ImportSpec::KbCategory(_) | ImportSpec::KbArticle(_) => panic!("wrong family"),
            ImportSpec::Site(s) => {
                // Validation trims scalar text by design; compare against
                // the same canonical form.
                prop_assert_eq!(&s.external_key, key.trim());
                prop_assert_eq!(&s.name, name.trim());
                prop_assert_eq!(
                    s.address.as_deref().filter(|v| !v.is_empty()),
                    // A fully-blank optional collapses to absent by design.
                    notes
                        .as_deref()
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                );
                prop_assert_eq!(s.latitude.map(|v| v.to_string()), lat.map(|v| v.to_string()));
                prop_assert_eq!(s.longitude.map(|v| v.to_string()), lon.map(|v| v.to_string()));
            }
        }
    }

    /// Arbitrary text can never panic or hang the CSV parser, and parsing is
    /// deterministic.
    #[test]
    fn csv_parser_is_total_and_deterministic(input in "\\PC{0,400}") {
        let first = parse_csv_chunk(&input);
        let second = parse_csv_chunk(&input);
        prop_assert_eq!(first.is_ok(), second.is_ok());
        if let (Ok(a), Ok(b)) = (first, second) {
            prop_assert_eq!(a, b);
        }
    }

    /// Quoting an ordinary comma-free cell never changes its parsed meaning.
    /// Cells WITH commas are covered by the dedicated unit tests (quoting is
    /// precisely what protects them, so plain-vs-quoted equivalence would be
    /// false there by construction).
    #[test]
    fn quoting_equivalence(cell in "[a-zA-Z0-9 .;:_-]{0,60}") {
        let plain = format!("h\n{cell}\ntail\n");
        let quoted = format!("h\n\"{}\"\ntail\n", cell.replace('"', ""));
        let a = parse_csv_chunk(&plain).expect("plain parses");
        let b = parse_csv_chunk(&quoted).expect("quoted parses");
        prop_assert_eq!(a[0].fields["h"].clone(), b[0].fields["h"].clone());
    }

    /// The resolver is total over (policy × difference) and every outcome is
    /// exactly the law in `resolve_conflict`'s doc: identical ⇒ no-op skip
    /// before policy consult.
    #[test]
    fn resolver_law_holds_everywhere(
        policy in proptest::prop_oneof![
            Just(ConflictPolicy::Skip),
            Just(ConflictPolicy::Update),
            Just(ConflictPolicy::Fail),
        ],
        differs in proptest::bool::ANY,
    ) {
        let id = Uuid::from_u128(555);
        let incoming_name = if differs { "Incoming".to_string() } else { "Same".to_string() };
        let spec = ImportSpec::Client(ClientSpec {
            external_key: "K".into(),
            name: incoming_name.clone(),
            notes: None,
        });
        let mut stored = spec.projection();
        stored.insert("name".into(), if differs { "Stored".into() } else { incoming_name });
        let decision = resolve_conflict(policy, &spec, Some((id, stored)));
        if !differs {
            prop_assert_eq!(decision, Decision::SkipNoChange);
        } else {
            match policy {
                ConflictPolicy::Skip => prop_assert_eq!(decision, Decision::SkipPolicy),
                ConflictPolicy::Update => prop_assert_eq!(decision, Decision::Update { internal_id: id }),
                ConflictPolicy::Fail => prop_assert_eq!(decision, Decision::FailConflict { internal_id: id }),
            }
        }
        // Absent mapping always creates under every policy.
        let probe = ImportSpec::Client(ClientSpec {
            external_key: "Z".into(),
            name: "N".into(),
            notes: None,
        });
        for p in [ConflictPolicy::Skip, ConflictPolicy::Update, ConflictPolicy::Fail] {
            prop_assert_eq!(resolve_conflict(p, &probe, None), Decision::Create);
        }
    }

    /// Email/url validators agree with their documented shape rules.
    #[test]
    fn email_validator_matches_shape_rule(addr in ".*") {
        let law = addr.contains('@')
            && addr.contains('.')
            && !addr.contains(' ')
            && addr.len() >= 5;
        prop_assert_eq!(validate_email(&addr).is_ok(), law);
    }

    #[test]
    fn url_validator_accepts_only_absolute_http(scheme in 0u8..3, rest in "[a-z0-9./_-]{0,50}") {
        let raw_rest = rest.replace('\\', "");
        let url = match scheme {
            0 => format!("https://{raw_rest}"),
            1 => format!("http://{raw_rest}"),
            _ => format!("https://{raw_rest} x"),
        };
        let expect_ok = scheme != 2 && !raw_rest.trim().is_empty();
        prop_assert_eq!(validate_url(&url).is_ok(), expect_ok);
    }

    /// Projections decide equality by operator-meaningful fields only:
    /// trimming differences in source whitespace do not create phantom
    /// updates because validate trims before projecting.
    #[test]
    fn whitespace_trim_prevents_phantom_updates(name_a in " [A-Za-z]{1,20} ", name_b in "[A-Za-z]{1,20}") {
        let trim_eq = name_a.trim() == name_b;
        let ra = client_row(0, "W", &name_a);
        let rb = client_row(1, "W", &name_b);
        let sa = build_specs(EntityKind::Client, &[ra]);
        let sb = build_specs(EntityKind::Client, &[rb]);
        if let (Ok(x), Ok(y)) = (&sa[0].spec, &sb[0].spec) {
            prop_assert_eq!(x.projection() == y.projection(), trim_eq);
        } else {
            prop_assert!(sa[0].spec.is_err() || sb[0].spec.is_err());
        }
    }
}

// --- D2 remainder: board / ticket / time-entry families ------------------------

#[test]
fn board_rows_validate_with_defaults() {
    let rows = parse_csv_chunk(
        "external_key,name,description,is_default\nSRV,Service Desk,Desk board,true\nPRJ,Projects,,\n",
    )
    .expect("parses");
    let specs = build_specs(EntityKind::Board, &rows);
    assert_eq!(specs.len(), 2);
    let board = match &specs[0].spec {
        Ok(ImportSpec::Board(b)) => b,
        other => panic!("expected board spec, got {other:?}"),
    };
    assert_eq!(board.name, "Service Desk");
    assert_eq!(board.description.as_deref(), Some("Desk board"));
    assert!(board.is_default);
    let board_b = match &specs[1].spec {
        Ok(ImportSpec::Board(b)) => b,
        other => panic!("expected board spec, got {other:?}"),
    };
    assert!(!board_b.is_default, "absent flag defaults to false");
    assert_eq!(
        specs[1].spec.as_ref().unwrap().projection()["description"],
        ""
    );
}

#[test]
fn board_rows_reject_bad_flags_and_unknown_columns() {
    let rows = parse_csv_chunk("external_key,name,is_default\nB,Board,maybe\n").expect("parses");
    let specs = build_specs(EntityKind::Board, &rows);
    let Err(errors) = &specs[0].spec else {
        panic!("bad flag must fail");
    };
    assert!(
        errors.iter().any(|e| e.contains("is_default")),
        "flag error reported, got {errors:?}"
    );

    let rows = parse_csv_chunk("external_key,name,color\nB,Board,red\n").expect("parses");
    let specs = build_specs(EntityKind::Board, &rows);
    let Err(errors) = &specs[0].spec else {
        panic!("unknown column must fail");
    };
    assert!(errors.iter().any(|e| e.contains("color")));
}

#[test]
fn ticket_rows_validate_parents_priority_and_due_at() {
    let rows = parse_json_chunk(&serde_json::json!([
        {
            "external_key": "T-1",
            "board_key": "SRV",
            "title": "Printer on fire",
            "body": "again",
            "priority": "p1",
            "client_key": "ACME",
            "due_at": "2026-09-01T10:00:00Z"
        },
        { "external_key": "T-2", "title": "No board" }
    ]))
    .expect("parses");
    let specs = build_specs(EntityKind::Ticket, &rows);
    let ticket = match &specs[0].spec {
        Ok(ImportSpec::Ticket(t)) => t,
        other => panic!("expected ticket spec, got {other:?}"),
    };
    assert_eq!(ticket.board_key, "SRV");
    assert_eq!(ticket.priority.as_str(), "p1");
    assert_eq!(
        ticket.due_at.map(|t| t.to_rfc3339()),
        Some("2026-09-01T10:00:00+00:00".to_string())
    );
    // Projection covers operator-meaningful fields, never parents (identity).
    let projection = specs[0].spec.as_ref().unwrap().projection();
    assert_eq!(projection["title"], "Printer on fire");
    assert!(!projection.contains_key("board_key"));

    let Err(errors) = &specs[1].spec else {
        panic!("missing board_key must fail");
    };
    assert!(errors.iter().any(|e| e.contains("board_key")));
    // Absent priority defaults to P3 without an error.
    let rows = parse_json_chunk(&serde_json::json!([
        { "external_key": "T-3", "board_key": "SRV", "title": "Calm" }
    ]))
    .expect("parses");
    let specs = build_specs(EntityKind::Ticket, &rows);
    let ticket = match &specs[0].spec {
        Ok(ImportSpec::Ticket(t)) => t,
        other => panic!("expected ticket spec, got {other:?}"),
    };
    assert_eq!(ticket.priority, crate::status::Priority::P3);
}

#[test]
fn time_entry_rows_validate_minutes_instants_and_defaults() {
    let rows = parse_json_chunk(&serde_json::json!([
        {
            "external_key": "TE-1",
            "ticket_key": "T-1",
            "started_at": "2026-08-30T09:00:00Z",
            "minutes": "90",
            "billable": "true",
            "user_email": "tech@example.test"
        },
        {
            "external_key": "TE-2",
            "ticket_key": "T-1",
            "started_at": "not-a-time",
            "minutes": "0"
        },
        {
            "external_key": "TE-3",
            "ticket_key": "T-1",
            "started_at": "2026-08-30T09:00:00Z",
            "minutes": "100000"
        }
    ]))
    .expect("parses");
    let specs = build_specs(EntityKind::TimeEntry, &rows);
    let entry = match &specs[0].spec {
        Ok(ImportSpec::TimeEntry(e)) => e,
        other => panic!("expected time entry spec, got {other:?}"),
    };
    assert_eq!(entry.minutes, 90);
    assert!(entry.billable);
    assert_eq!(entry.user_email.as_deref(), Some("tech@example.test"));
    // ended_at stays None when absent — the commit derives it from
    // started_at + minutes.
    assert!(entry.ended_at.is_none());

    let Err(errors_b) = &specs[1].spec else {
        panic!("bad started_at must fail");
    };
    assert!(errors_b.iter().any(|e| e.contains("started_at")));
    assert!(errors_b.iter().any(|e| e.contains("minutes")));

    let Err(errors_c) = &specs[2].spec else {
        panic!("absurd minutes must fail");
    };
    assert!(errors_c.iter().any(|e| e.contains("minutes")));
}

#[test]
fn ticket_and_time_entry_parent_keys_resolve_through_the_lookup() {
    let board_id = Uuid::now_v7();
    let ticket_id = Uuid::now_v7();
    let parents: HashMap<(EntityKind, String), Uuid> = [
        ((EntityKind::Board, "SRV".to_string()), board_id),
        ((EntityKind::Ticket, "T-1".to_string()), ticket_id),
    ]
    .into_iter()
    .collect();
    let lookup: ParentLookup<'_> = &|kind, key| parents.get(&(kind, key.to_string())).copied();

    let ticket = ImportSpec::Ticket(TicketSpec {
        external_key: "T-1".into(),
        board_key: "SRV".into(),
        title: "x".into(),
        body: None,
        priority: crate::status::Priority::P3,
        client_key: None,
        site_key: None,
        contact_key: None,
        due_at: None,
    });
    assert!(check_parents(&ticket, lookup).is_ok());
    let ImportSpec::Ticket(ticket_spec) = ticket else {
        panic!("constructed ticket");
    };
    let orphan = ImportSpec::Ticket(TicketSpec {
        board_key: "MISSING".into(),
        ..ticket_spec
    });
    let errors = check_parents(&orphan, lookup).unwrap_err();
    assert!(errors[0].contains("board_key"));

    let entry = ImportSpec::TimeEntry(TimeEntrySpec {
        external_key: "TE-1".into(),
        ticket_key: "T-1".into(),
        user_email: None,
        started_at: chrono::Utc::now(),
        ended_at: None,
        minutes: 30,
        billable: true,
        notes: None,
        work_type: None,
    });
    assert!(check_parents(&entry, lookup).is_ok());
    let ImportSpec::TimeEntry(entry_spec) = entry else {
        panic!("constructed entry");
    };
    let orphan_entry = ImportSpec::TimeEntry(TimeEntrySpec {
        ticket_key: "NOPE".into(),
        ..entry_spec
    });
    let errors = check_parents(&orphan_entry, lookup).unwrap_err();
    assert!(errors[0].contains("ticket_key"));
}

#[test]
fn kb_rows_validate_statuses_defaults_and_unknown_columns() {
    let rows = parse_json_chunk(&serde_json::json!([
        {
            "external_key": "getting-started",
            "name": "Getting started",
            "slug": "getting-started",
            "description": "First steps",
            "position": "2",
            "client_visible": "true"
        },
        {
            "external_key": "article:reset-password",
            "category_slug": "getting-started",
            "title": "Reset a password",
            "slug": "reset-password",
            "body": "Step one.",
            "status": "published",
            "client_visible": "true"
        },
        {
            "external_key": "article:draft-note",
            "title": "Internal note",
            "slug": "internal-note"
        },
        {
            "external_key": "article:bad",
            "title": "Bad",
            "slug": "bad",
            "status": "live"
        },
        {
            "external_key": "article:noslug",
            "title": "No slug"
        }
    ]))
    .expect("parses");
    let specs = build_specs(EntityKind::KbCategory, &rows[..1]);
    let category = match &specs[0].spec {
        Ok(ImportSpec::KbCategory(c)) => c,
        other => panic!("expected category spec, got {other:?}"),
    };
    assert_eq!(category.slug, "getting-started");
    assert_eq!(category.position, 2);
    assert!(category.client_visible);

    let specs = build_specs(EntityKind::KbArticle, &rows[1..]);
    let published = match &specs[0].spec {
        Ok(ImportSpec::KbArticle(a)) => a,
        other => panic!("expected article spec, got {other:?}"),
    };
    assert_eq!(published.status, Some(KbArticleStatus::Published));
    assert_eq!(published.category_slug.as_deref(), Some("getting-started"));

    // Absent status stays None (storage default draft at commit), absent
    // category_slug is a legal top-level article.
    let draft = match &specs[1].spec {
        Ok(ImportSpec::KbArticle(a)) => a,
        other => panic!("expected article spec, got {other:?}"),
    };
    assert_eq!(draft.status, None);
    assert!(draft.category_slug.is_none());
    assert!(!draft.client_visible, "absent flag defaults to false");

    let Err(bad_status) = &specs[2].spec else {
        panic!("unknown status must fail");
    };
    assert!(bad_status.iter().any(|e| e.contains("status")));

    let Err(no_slug) = &specs[3].spec else {
        panic!("missing slug must fail");
    };
    assert!(no_slug.iter().any(|e| e.contains("slug")));

    // Unknown columns are rejected per family, and projection carries
    // operator content only — never slugs (identity).
    let rows = parse_csv_chunk("external_key,title,slug,color\nK,Bad,Bad,red\n").expect("parses");
    let specs = build_specs(EntityKind::KbArticle, &rows);
    let Err(errors) = &specs[0].spec else {
        panic!("unknown column must fail");
    };
    assert!(errors.iter().any(|e| e.contains("color")));
    let projection = build_specs(
        EntityKind::KbCategory,
        &parse_json_chunk(&serde_json::json!([
            { "external_key": "c", "name": "C", "slug": "c" }
        ]))
        .expect("parses"),
    )[0]
    .spec
    .as_ref()
    .unwrap()
    .projection();
    assert!(!projection.contains_key("slug"));
    assert_eq!(projection["name"], "C");
    assert_eq!(projection["position"], "0", "absent position defaults to 0");
}

#[test]
fn kb_article_parent_keys_resolve_through_the_lookup() {
    let category_id = Uuid::now_v7();
    let parents: HashMap<(EntityKind, String), Uuid> = [(
        (EntityKind::KbCategory, "getting-started".to_string()),
        category_id,
    )]
    .into_iter()
    .collect();
    let lookup: ParentLookup<'_> = &|kind, key| parents.get(&(kind, key.to_string())).copied();

    let category = ImportSpec::KbCategory(KbCategorySpec {
        external_key: "getting-started".into(),
        name: "Getting started".into(),
        slug: "getting-started".into(),
        description: None,
        position: 0,
        client_visible: false,
    });
    // Categories are roots: nothing to resolve.
    assert!(check_parents(&category, lookup).is_ok());

    let article = ImportSpec::KbArticle(KbArticleSpec {
        external_key: "article:reset-password".into(),
        category_slug: Some("getting-started".into()),
        title: "Reset a password".into(),
        slug: "reset-password".into(),
        body: Some("Step one.".into()),
        status: Some(KbArticleStatus::Published),
        client_visible: true,
    });
    assert!(check_parents(&article, lookup).is_ok());
    let ImportSpec::KbArticle(article_spec) = article else {
        panic!("constructed article");
    };
    let orphan = ImportSpec::KbArticle(KbArticleSpec {
        category_slug: Some("missing".into()),
        ..article_spec
    });
    let errors = check_parents(&orphan, lookup).unwrap_err();
    assert!(errors[0].contains("category_slug"));
}

#[test]
fn new_family_projections_agree_between_incoming_and_stored() {
    use crate::entities::{Board, Ticket, TimeEntry};
    use crate::ids::TicketNumber;

    let now = chrono::Utc::now();
    let spec = ImportSpec::Ticket(TicketSpec {
        external_key: "T-1".into(),
        board_key: "SRV".into(),
        title: "Same title".into(),
        body: Some("body".into()),
        priority: crate::status::Priority::P2,
        client_key: None,
        site_key: None,
        contact_key: None,
        due_at: Some(now),
    });
    let stored = Ticket {
        id: Uuid::now_v7(),
        org_id: Uuid::now_v7(),
        number: TicketNumber("MER-1".into()),
        board_id: Uuid::now_v7(),
        status_id: Uuid::now_v7(),
        title: "Same title".into(),
        body: Some("body".into()),
        priority: crate::status::Priority::P2,
        client_id: None,
        site_id: None,
        contact_id: None,
        assignee_id: None,
        requester_id: None,
        category: crate::status::TicketStatusCategory::Open,
        due_at: Some(now),
        resolved_at: None,
        created_at: now,
        updated_at: now,
        merged_into_ticket_id: None,
    };
    assert_eq!(spec.projection(), ticket_projection(&stored));

    let board_spec = ImportSpec::Board(BoardSpec {
        external_key: "B".into(),
        name: "Board".into(),
        description: None,
        is_default: false,
    });
    let board = Board {
        id: Uuid::now_v7(),
        org_id: Uuid::now_v7(),
        name: "Board".into(),
        description: None,
        is_default: false,
        require_checklist_close: false,
        created_at: now,
    };
    assert_eq!(board_spec.projection(), board_projection(&board));

    let started = chrono::Utc::now();
    let entry_spec = ImportSpec::TimeEntry(TimeEntrySpec {
        external_key: "TE".into(),
        ticket_key: "T-1".into(),
        user_email: None,
        started_at: started,
        ended_at: Some(started),
        minutes: 15,
        billable: false,
        notes: Some("n".into()),
        work_type: Some("remote".into()),
    });
    let entry = TimeEntry {
        id: Uuid::now_v7(),
        org_id: Uuid::now_v7(),
        ticket_id: Uuid::now_v7(),
        user_id: Uuid::now_v7(),
        started_at: started,
        ended_at: started,
        minutes: 15,
        billable: false,
        notes: Some("n".into()),
        work_type: Some("remote".into()),
        timer_session_id: None,
        created_at: now,
    };
    assert_eq!(entry_spec.projection(), time_entry_projection(&entry));

    let category_spec = ImportSpec::KbCategory(KbCategorySpec {
        external_key: "category:c".into(),
        name: "Cat".into(),
        slug: "c".into(),
        description: Some("About".into()),
        position: 3,
        client_visible: true,
    });
    let category = crate::entities::KbCategory {
        id: Uuid::now_v7(),
        org_id: Uuid::now_v7(),
        name: "Cat".into(),
        slug: "c".into(),
        description: "About".into(),
        position: 3,
        client_visible: true,
        created_at: now,
        updated_at: now,
    };
    assert_eq!(
        category_spec.projection(),
        kb_category_projection(&category)
    );

    let article_spec = ImportSpec::KbArticle(KbArticleSpec {
        external_key: "article:a".into(),
        category_slug: Some("c".into()),
        title: "Same title".into(),
        slug: "a".into(),
        body: Some("body".into()),
        status: Some(KbArticleStatus::Published),
        client_visible: true,
    });
    let article = crate::entities::KbArticle {
        id: Uuid::now_v7(),
        org_id: Uuid::now_v7(),
        category_id: Some(Uuid::now_v7()),
        title: "Same title".into(),
        slug: "a".into(),
        body: "body".into(),
        status: KbArticleStatus::Published,
        client_visible: true,
        created_by: None,
        updated_by: None,
        created_at: now,
        updated_at: now,
    };
    assert_eq!(article_spec.projection(), kb_article_projection(&article));
}

// --- D2 remainder: saved field-mapping templates -------------------------------

#[test]
fn mapping_validation_rejects_bad_targets_and_duplicate_targets() {
    let mut mapping = BTreeMap::new();
    mapping.insert("Company".to_string(), "name".to_string());
    assert!(validate_mapping(EntityKind::Client, &mapping).is_ok());

    // A target that is not a canonical column is rejected.
    mapping.insert("Company2".to_string(), "title".to_string());
    let errors = validate_mapping(EntityKind::Client, &mapping).unwrap_err();
    assert!(errors.iter().any(|e| e.contains("title")));

    // Two sources claiming one target is ambiguous.
    let mut mapping = BTreeMap::new();
    mapping.insert("A".to_string(), "name".to_string());
    mapping.insert("B".to_string(), "name".to_string());
    let errors = validate_mapping(EntityKind::Client, &mapping).unwrap_err();
    assert!(errors.iter().any(|e| e.contains("more than one source")));

    // Empty mapping rejected.
    let empty = BTreeMap::new();
    assert!(validate_mapping(EntityKind::Client, &empty).is_err());

    // Cross-kind targets do not leak between catalogs.
    let mut mapping = BTreeMap::new();
    mapping.insert("Min".to_string(), "minutes".to_string());
    assert!(validate_mapping(EntityKind::TimeEntry, &mapping).is_ok());
    assert!(validate_mapping(EntityKind::Client, &mapping).is_err());
}

#[test]
fn apply_field_mapping_renames_and_flags_collisions() {
    let rows = parse_csv_chunk("ID,Company,notes\nK1,Acme,hello\n").expect("parses");
    let mut mapping = BTreeMap::new();
    mapping.insert("ID".to_string(), "external_key".to_string());
    mapping.insert("Company".to_string(), "name".to_string());
    let mapped = apply_field_mapping(rows, &mapping).expect("applies");
    assert_eq!(mapped[0].fields["external_key"], "K1");
    assert_eq!(mapped[0].fields["name"], "Acme");
    assert_eq!(
        mapped[0].fields["notes"], "hello",
        "unmapped columns pass through"
    );

    // Renaming onto an existing column is an ambiguous template.
    let rows = parse_csv_chunk("ID,name\nK1,Acme\n").expect("parses");
    let mut mapping = BTreeMap::new();
    mapping.insert("ID".to_string(), "name".to_string());
    let err = apply_field_mapping(rows, &mapping).unwrap_err();
    assert!(matches!(err, ImportError::Malformed(ref m) if m.contains("collides")));
}

#[test]
fn import_order_is_a_total_runbook() {
    let mut kinds = [
        EntityKind::KbArticle,
        EntityKind::TimeEntry,
        EntityKind::Ticket,
        EntityKind::Contact,
        EntityKind::Board,
        EntityKind::Site,
        EntityKind::Client,
        EntityKind::KbCategory,
    ];
    kinds.sort_by_key(|k| k.import_order());
    let names: Vec<&str> = kinds.iter().map(|k| k.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "board",
            "client",
            "site",
            "contact",
            "ticket",
            "time_entry",
            "kb_category",
            "kb_article"
        ]
    );
}
