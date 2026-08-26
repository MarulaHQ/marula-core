//! Custom-field definitions and values (TASKLIST 2.17, BUILD-PROMPT §5).
//!
//! Purpose: the pure rules layer for dynamic forms — the typed field-kind
//! enum, definition validation (names, select option sets), and kind-aware
//! parsing/serialization of JSONB values. The storage contract is migration
//! 0009's schema: `custom_field_defs.config` carries `{"options": [...]}`
//! for choice kinds and `custom_field_values.value` stores one canonical
//! JSON encoding per value (see [`FieldValue`]).
//!
//! Non-purpose: no SQL, no HTTP, no clocks. Everything here is a pure
//! function of its inputs so api/db layers stay thin and every rule is
//! proptest-pinned.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::str::FromStr;
use uuid::Uuid;

/// The field kinds the storage schema admits (migration 0009 CHECK). Every
/// kind maps to exactly one canonical JSON encoding of its values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldKind {
    Text,
    Number,
    Date,
    Select,
    MultiSelect,
    Bool,
    Url,
    User,
}

impl FieldKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            FieldKind::Text => "text",
            FieldKind::Number => "number",
            FieldKind::Date => "date",
            FieldKind::Select => "select",
            FieldKind::MultiSelect => "multiselect",
            FieldKind::Bool => "bool",
            FieldKind::Url => "url",
            FieldKind::User => "user",
        }
    }

    /// Whether this kind constrains values to a configured option set.
    pub const fn is_choice(&self) -> bool {
        matches!(self, FieldKind::Select | FieldKind::MultiSelect)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownFieldKind;

impl FromStr for FieldKind {
    type Err = UnknownFieldKind;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "text" => Ok(FieldKind::Text),
            "number" => Ok(FieldKind::Number),
            "date" => Ok(FieldKind::Date),
            "select" => Ok(FieldKind::Select),
            "multiselect" => Ok(FieldKind::MultiSelect),
            "bool" => Ok(FieldKind::Bool),
            "url" => Ok(FieldKind::Url),
            "user" => Ok(FieldKind::User),
            _ => Err(UnknownFieldKind),
        }
    }
}

/// A validated custom-field value in canonical form. Serialization is the
/// storage/wire encoding: `parse(serialize(v)) == v` always holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum FieldValue {
    Text(String),
    /// Finite number; integral values stay integral on the wire.
    Number(f64),
    /// Calendar date encoded `YYYY-MM-DD`.
    Date(NaiveDate),
    Select(String),
    MultiSelect(Vec<String>),
    Bool(bool),
    Url(String),
    /// Reference to an org user by id (picker lands with the directory).
    User(Uuid),
}

impl FieldValue {
    /// Canonical JSON encoding stored in `custom_field_values.value`.
    pub fn to_json(&self) -> Json {
        match self {
            FieldValue::Text(s) | FieldValue::Select(s) | FieldValue::Url(s) => {
                Json::String(s.clone())
            }
            // serde_json cannot represent NaN/Inf; validated numbers are
            // finite by construction, so this is total in practice.
            FieldValue::Number(n) => match serde_json::Number::from_f64(*n) {
                Some(number) => Json::Number(number),
                None => Json::Null,
            },
            FieldValue::Date(d) => Json::String(d.format("%Y-%m-%d").to_string()),
            FieldValue::MultiSelect(items) => Json::Array(
                items
                    .iter()
                    .map(|o| Json::String(o.clone()))
                    .collect::<Vec<_>>(),
            ),
            FieldValue::Bool(b) => Json::Bool(*b),
            FieldValue::User(id) => Json::String(id.to_string()),
        }
    }
}

/// Parsed view of a definition's `config` jsonb: the closed option set for
/// choice kinds. Unknown config keys are preserved upstream but ignored here
/// (additive evolution).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FieldConfig {
    pub options: Vec<String>,
}

impl FieldConfig {
    /// Reads `{"options": ["a", "b"]}`. Non-string or missing entries yield
    /// an empty set; use [`validate_config`] before trusting it for writes.
    pub fn from_json(raw: &Json) -> Self {
        let options = raw
            .get("options")
            .and_then(|o| o.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        Self { options }
    }

    /// Canonical config encoding (`{}` when the kind takes no options).
    pub fn to_json(&self) -> Json {
        if self.options.is_empty() {
            Json::Object(serde_json::Map::new())
        } else {
            serde_json::json!({ "options": self.options })
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum FieldError {
    #[error("field name must be 1–200 characters")]
    NameLength,
    #[error("{kind} fields do not take options")]
    OptionsNotAllowed { kind: &'static str },
    #[error("choice fields need at least one option with unique non-empty entries")]
    BadOptions,
    #[error("value does not match field type {kind}")]
    TypeMismatch { kind: &'static str },
    #[error("`{option}` is not one of the configured options")]
    NotAnOption { option: String },
    #[error("number is out of representable range")]
    NumberRange,
}

/// Validates definition-level invariants beyond what SQL CHECKs enforce:
/// trimmed name length, and that only choice kinds carry a non-empty,
/// duplicate-free option set. Returns the normalized (trimmed) pieces.
pub fn validate_def(
    name: &str,
    kind: FieldKind,
    config: &FieldConfig,
) -> Result<(String, FieldConfig), FieldError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(FieldError::NameLength);
    }
    if !kind.is_choice() && !config.options.is_empty() {
        return Err(FieldError::OptionsNotAllowed {
            kind: kind.as_str(),
        });
    }
    if kind.is_choice() {
        let mut seen = std::collections::HashSet::new();
        for option in &config.options {
            let option = option.trim();
            if option.is_empty() || option.chars().count() > 200 || !seen.insert(option) {
                return Err(FieldError::BadOptions);
            }
        }
        if config.options.is_empty() {
            return Err(FieldError::BadOptions);
        }
    }
    Ok((
        name.to_owned(),
        FieldConfig {
            options: config.options.clone(),
        },
    ))
}

/// Kind-aware parse + validation of one incoming JSON value against its
/// definition. Accepts pragmatic coercions (numeric strings, integer-valued
/// floats for bool-adjacent kinds are NOT coerced — booleans stay strict)
/// and enforces select/multiselect membership.
pub fn parse_value(
    kind: FieldKind,
    config: &FieldConfig,
    raw: &Json,
) -> Result<FieldValue, FieldError> {
    match kind {
        FieldKind::Text => {
            raw.as_str()
                .map(|s| FieldValue::Text(s.to_owned()))
                .ok_or(FieldError::TypeMismatch {
                    kind: kind.as_str(),
                })
        }
        FieldKind::Url => {
            let s = raw.as_str().ok_or(FieldError::TypeMismatch {
                kind: kind.as_str(),
            })?;
            validate_url(s).map(FieldValue::Url)
        }
        FieldKind::Number => {
            let n = match raw {
                Json::Number(n) => n.as_f64(),
                Json::String(s) => s.trim().parse::<f64>().ok(),
                _ => None,
            };
            match n {
                Some(n) if n.is_finite() && n.abs() <= 1e15 => Ok(FieldValue::Number(n)),
                Some(_) => Err(FieldError::NumberRange),
                None => Err(FieldError::TypeMismatch {
                    kind: kind.as_str(),
                }),
            }
        }
        FieldKind::Date => raw
            .as_str()
            .and_then(|s| NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok())
            .map(FieldValue::Date)
            .ok_or(FieldError::TypeMismatch {
                kind: kind.as_str(),
            }),
        FieldKind::Bool => raw
            .as_bool()
            .map(FieldValue::Bool)
            .ok_or(FieldError::TypeMismatch {
                kind: kind.as_str(),
            }),
        FieldKind::User => raw
            .as_str()
            .and_then(|s| Uuid::parse_str(s.trim()).ok())
            .map(FieldValue::User)
            .ok_or(FieldError::TypeMismatch {
                kind: kind.as_str(),
            }),
        FieldKind::Select => {
            let s = raw.as_str().ok_or(FieldError::TypeMismatch {
                kind: kind.as_str(),
            })?;
            if config.options.iter().any(|o| o == s) {
                Ok(FieldValue::Select(s.to_owned()))
            } else {
                Err(FieldError::NotAnOption {
                    option: s.to_owned(),
                })
            }
        }
        FieldKind::MultiSelect => {
            let items = raw.as_array().ok_or(FieldError::TypeMismatch {
                kind: kind.as_str(),
            })?;
            let mut parsed = Vec::with_capacity(items.len());
            for item in items {
                let s = item.as_str().ok_or(FieldError::TypeMismatch {
                    kind: kind.as_str(),
                })?;
                if !config.options.iter().any(|o| o == s) {
                    return Err(FieldError::NotAnOption {
                        option: s.to_owned(),
                    });
                }
                parsed.push(s.to_owned());
            }
            // Duplicate picks carry no meaning; normalize rather than reject.
            parsed.sort();
            parsed.dedup();
            Ok(FieldValue::MultiSelect(parsed))
        }
    }
}

/// URL validation without pulling a url parser into core-domain: absolute
/// http(s) URLs with a host part, bounded length.
fn validate_url(s: &str) -> Result<String, FieldError> {
    const MAX_URL_LEN: usize = 2048;
    if s.is_empty() || s.len() > MAX_URL_LEN || s != s.trim() {
        return Err(FieldError::TypeMismatch { kind: "url" });
    }
    let rest = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .ok_or(FieldError::TypeMismatch { kind: "url" })?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || !authority.contains('.') && !authority.starts_with('[') {
        return Err(FieldError::TypeMismatch { kind: "url" });
    }
    Ok(s.to_owned())
}

/// Ids of required definitions in `defs` that have no present value in
/// `values` (keyed by def id). Drives create/update enforcement.
pub fn missing_required(
    defs: &[(Uuid, bool)],
    values: &std::collections::HashMap<Uuid, FieldValue>,
) -> Vec<Uuid> {
    defs.iter()
        .filter(|(id, required)| *required && !values.contains_key(id))
        .map(|(id, _)| *id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::HashMap;

    fn options(items: &[&str]) -> FieldConfig {
        FieldConfig {
            options: items.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn every_kind_round_trips_through_str() {
        for kind in [
            FieldKind::Text,
            FieldKind::Number,
            FieldKind::Date,
            FieldKind::Select,
            FieldKind::MultiSelect,
            FieldKind::Bool,
            FieldKind::Url,
            FieldKind::User,
        ] {
            assert_eq!(kind.as_str().parse::<FieldKind>().unwrap(), kind);
        }
        assert!("widget".parse::<FieldKind>().is_err());
    }

    #[test]
    fn validation_matrix_per_kind() {
        let cfg = options(&["Email", "Phone"]);
        // text
        assert_eq!(
            parse_value(FieldKind::Text, &cfg, &Json::String("hello".into())).unwrap(),
            FieldValue::Text("hello".into())
        );
        assert!(parse_value(FieldKind::Text, &cfg, &Json::Number(3.into())).is_err());
        // number: accepts numeric strings (coercion), rejects junk
        assert_eq!(
            parse_value(FieldKind::Number, &cfg, &Json::String(" 42.5 ".into())).unwrap(),
            FieldValue::Number(42.5)
        );
        assert!(parse_value(FieldKind::Number, &cfg, &Json::String("12abc".into())).is_err());
        assert!(parse_value(FieldKind::Number, &cfg, &serde_json::json!(1e300)).is_err());
        // date
        assert_eq!(
            parse_value(FieldKind::Date, &cfg, &Json::String("2026-08-26".into())).unwrap(),
            FieldValue::Date(NaiveDate::from_ymd_opt(2026, 8, 26).unwrap())
        );
        assert!(parse_value(FieldKind::Date, &cfg, &Json::String("08/26/2026".into())).is_err());
        // bool stays strict
        assert!(parse_value(FieldKind::Bool, &cfg, &Json::Bool(true)).is_ok());
        assert!(parse_value(FieldKind::Bool, &cfg, &Json::String("true".into())).is_err());
        // user takes uuids
        let uid = Uuid::now_v7();
        assert!(parse_value(FieldKind::User, &cfg, &Json::String(uid.to_string())).is_ok());
        assert!(parse_value(FieldKind::User, &cfg, &Json::String("nope".into())).is_err());
        // select enforces membership
        assert_eq!(
            parse_value(FieldKind::Select, &cfg, &Json::String("Email".into())).unwrap(),
            FieldValue::Select("Email".into())
        );
        assert!(matches!(
            parse_value(FieldKind::Select, &cfg, &Json::String("Fax".into())),
            Err(FieldError::NotAnOption { .. })
        ));
        // multiselect dedups and validates each entry
        assert_eq!(
            parse_value(
                FieldKind::MultiSelect,
                &cfg,
                &serde_json::json!(["Phone", "Email", "Phone"])
            )
            .unwrap(),
            FieldValue::MultiSelect(vec!["Email".into(), "Phone".into()])
        );
        assert!(parse_value(FieldKind::MultiSelect, &cfg, &Json::String("Email".into())).is_err());
        // url requires absolute http(s)
        assert!(parse_value(
            FieldKind::Url,
            &cfg,
            &Json::String("https://a.example/x".into())
        )
        .is_ok());
        assert!(parse_value(
            FieldKind::Url,
            &cfg,
            &Json::String("ftp://a.example".into())
        )
        .is_err());
        assert!(parse_value(FieldKind::Url, &cfg, &Json::String("https://".into())).is_err());
    }

    #[test]
    fn def_validation_rejects_bad_names_and_option_sets() {
        assert_eq!(
            validate_def("  ", FieldKind::Text, &Default::default()),
            Err(FieldError::NameLength)
        );
        assert_eq!(
            validate_def("x", FieldKind::Text, &options(&["a"])),
            Err(FieldError::OptionsNotAllowed { kind: "text" })
        );
        assert_eq!(
            validate_def("x", FieldKind::Select, &Default::default()),
            Err(FieldError::BadOptions)
        );
        assert_eq!(
            validate_def("x", FieldKind::MultiSelect, &options(&["a", " a"])),
            Err(FieldError::BadOptions)
        );
        let (name, _) = validate_def("  SLA tier  ", FieldKind::Select, &options(&["Gold"]))
            .expect("valid def");
        assert_eq!(name, "SLA tier");
    }

    #[test]
    fn missing_required_lists_unfilled_ids() {
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        let defs = vec![(a, true), (b, false)];
        let mut values = HashMap::new();
        values.insert(a, FieldValue::Text("x".into()));
        assert!(missing_required(&defs, &values).is_empty());
        let mut empty = HashMap::new();
        assert_eq!(missing_required(&defs, &empty), vec![a]);
        empty.insert(b, FieldValue::Number(1.0));
        assert_eq!(missing_required(&defs, &empty), vec![a]);
    }

    // Roundtrip law: canonical encoding parses back to the same value.
    proptest! {
        #[test]
        fn text_roundtrips(s in ".*") {
            let v = FieldValue::Text(s);
            prop_assert_eq!(parse_value(FieldKind::Text, &Default::default(), &v.to_json()).unwrap(), v);
        }

        #[test]
        fn number_roundtrips(n in -1e15f64..1e15) {
            let v = FieldValue::Number(n);
            prop_assert_eq!(parse_value(FieldKind::Number, &Default::default(), &v.to_json()).unwrap(), v);
        }

        #[test]
        fn date_roundtrips(days in -719163i32..=376_219) {
            // CE day range 0001-01-01 ..= 2999-12-31.
            let d = chrono::NaiveDate::from_num_days_from_ce_opt(days).unwrap();
            let v = FieldValue::Date(d);
            prop_assert_eq!(parse_value(FieldKind::Date, &Default::default(), &v.to_json()).unwrap(), v);
        }

        #[test]
        fn select_membership_holds(option in "[a-z]{1,10}", other in "[A-Z]{1,10}") {
            let cfg = options(&[option.as_str(), other.as_str()]);
            let v = FieldValue::Select(option.clone());
            let not_member = Json::String(format!("{}{}", option, other));
            prop_assert_eq!(parse_value(FieldKind::Select, &cfg, &v.to_json()).unwrap(), v);
            prop_assert!(parse_value(FieldKind::Select, &cfg, &not_member).is_err());
        }

        #[test]
        fn multiselect_normalizes_and_roundtrips(picks in proptest::collection::vec("[a-c]", 0..8)) {
            let cfg = options(&["a", "b", "c"]);
            match parse_value(FieldKind::MultiSelect, &cfg, &serde_json::json!(picks)) {
                Ok(v @ FieldValue::MultiSelect(_)) => {
                    if let FieldValue::MultiSelect(items) = &v {
                        let mut sorted = items.clone();
                        sorted.sort();
                        sorted.dedup();
                        prop_assert_eq!(items, &sorted);
                    }
                    prop_assert_eq!(parse_value(FieldKind::MultiSelect, &cfg, &v.to_json()).unwrap(), v);
                }
                _ => panic!("multiselect parse must succeed on [a-c] picks"),
            }
        }

        #[test]
        fn url_roundtrips(host in "[a-z]{2,8}\\.example", path in "[a-z0-9/\\-]{0,12}") {
            let s = format!("https://{host}/{path}");
            let v = FieldValue::Url(s);
            prop_assert_eq!(parse_value(FieldKind::Url, &Default::default(), &v.to_json()).unwrap(), v);
        }
    }
}
