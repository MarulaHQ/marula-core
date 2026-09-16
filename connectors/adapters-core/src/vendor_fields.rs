//! Canonical vendor-field extraction helpers — ONE definition consumed by
//! every connector adapter (`ci-feeders`, `immybot`, `m365-graph`).
//!
//! These helpers sit on the CI3 identity-key path: the values they extract
//! feed `IdentityKeysBuilder` (serial / hostname / MAC), so a behavioral
//! divergence between two connector copies would silently break convergence
//! for the same physical device (the two records for one device would stop
//! converging). They were
//! therefore unified here (Wave 15 lane 4) — connectors import these and
//! must not carry their own copies. If an adapter ever needs different
//! extraction semantics, that is a CONTRACT change: add a named variant
//! here with its own test, never a local copy in the connector.
//!
//! Semantics (pinned by [`tests`]):
//! * [`first_str`] — first candidate key carrying a non-null, non-empty,
//!   trimmable string; the value is trimmed. A null/absent/empty candidate
//!   falls through to the next key.
//! * [`string_list`] — strings or arrays of strings across candidate keys,
//!   in candidate-then-element order; nothing is trimmed (MAC lists are
//!   compared verbatim by the identity builder).
//! * [`observed_fields`] — copies the named fields verbatim when present
//!   and non-null (the observed tier carries the source's values
//!   unchanged; the core's precedence rules govern who wins).

use serde_json::{Map, Value};

/// First present, non-empty string among candidate field names.
pub fn first_str(fields: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = fields
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return Some(value.to_string());
        }
    }
    None
}

/// Collects strings from fields that are either a string, an array of
/// strings, or absent (vendor APIs disagree about single vs plural).
pub fn string_list(fields: &Map<String, Value>, keys: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for key in keys {
        match fields.get(*key) {
            Some(Value::String(s)) => out.push(s.clone()),
            Some(Value::Array(items)) => {
                for item in items {
                    if let Some(s) = item.as_str() {
                        out.push(s.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Copies the named vendor fields verbatim when present and non-null —
/// the observed tier carries the source's values unchanged.
pub fn observed_fields(fields: &Map<String, Value>, keys: &[&str]) -> Value {
    let mut out = Map::new();
    for key in keys {
        if let Some(value) = fields.get(*key).filter(|v| !v.is_null()) {
            out.insert((*key).to_string(), value.clone());
        }
    }
    Value::Object(out)
}

/// [`first_str`] over a whole payload (`Value` object). A non-object
/// payload behaves like an empty map (None).
pub fn first_str_in(payload: &Value, keys: &[&str]) -> Option<String> {
    payload
        .as_object()
        .and_then(|fields| first_str(fields, keys))
}

/// [`observed_fields`] over a whole payload (`Value` object). A non-object
/// payload yields an empty object — the same semantics a whole-payload
/// `.get(key)` walk had on the connector side.
pub fn observed_fields_in(payload: &Value, keys: &[&str]) -> Value {
    payload
        .as_object()
        .map(|fields| observed_fields(fields, keys))
        .unwrap_or_else(|| Value::Object(Map::new()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn first_str_skips_null_empty_and_mismatched() {
        let fields = json!({
            "a": "  spaced  ",
            "b": "",
            "c": null,
            "d": 7,
        })
        .as_object()
        .unwrap()
        .clone();
        assert_eq!(first_str(&fields, &["b", "c", "a"]), Some("spaced".into()));
        assert_eq!(first_str(&fields, &["d", "missing"]), None);
    }

    #[test]
    fn string_list_flattens_string_or_array() {
        let fields = json!({"a": "one", "b": ["two", "three"], "c": 4})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(string_list(&fields, &["a", "b", "c"]), vec!["one", "two", "three"]);
        assert_eq!(string_list(&fields, &["c"]), Vec::<String>::new());
    }

    #[test]
    fn observed_fields_copies_verbatim_and_skips_null() {
        let fields = json!({"a": "x", "b": null, "c": [1, 2]})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(
            observed_fields(&fields, &["a", "b", "c", "missing"]),
            json!({"a": "x", "c": [1, 2]})
        );
    }

    #[test]
    fn whole_payload_forms_match_object_forms() {
        let payload = json!({"serial": " SR-1 ", "mac": null});
        assert_eq!(first_str_in(&payload, &["serial"]), Some("SR-1".into()));
        assert_eq!(
            observed_fields_in(&payload, &["serial", "mac"]),
            json!({"serial": " SR-1 "})
        );
        // Non-object payloads degrade to empty semantics, not errors.
        assert_eq!(first_str_in(&json!([1, 2]), &["serial"]), None);
        assert_eq!(observed_fields_in(&json!([1, 2]), &["serial"]), json!({}));
    }
}
