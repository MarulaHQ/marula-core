//! Vendor payload normalizers. Each one maps ONE vendor's device /
//! configuration record (exactly the shape its public API or export
//! carries) onto the vendor-neutral [`FeederRecord`] with deterministic
//! identity keys. Pure functions: a live transport only needs to fetch
//! the vendor JSON and call in.

pub mod auvik;
pub mod it_glue;
pub mod ninjaone;

pub use auvik::map_device as map_auvik_device;
pub use it_glue::map_configuration as map_it_glue_configuration;
pub use ninjaone::map_device as map_ninjaone_device;

use serde_json::{Map, Value};
use thiserror::Error;

/// A vendor payload could not be mapped into the feed contract.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NormalizeError {
    /// Not a vendor record at all (wrong shape / missing required ids).
    #[error("unrecognised {vendor} payload: {reason}")]
    Unrecognised {
        vendor: &'static str,
        reason: String,
    },
}

/// Strips a JSON:API envelope down to its `attributes` object when the
/// payload wraps one, else returns the object itself (both shapes are
/// common across IT Glue surfaces).
pub(crate) fn attributes_of<'a>(
    value: &'a Value,
    vendor: &'static str,
) -> Result<&'a Map<String, Value>, NormalizeError> {
    let object = match value {
        Value::Object(o) => o,
        _ => {
            return Err(NormalizeError::Unrecognised {
                vendor,
                reason: "payload is not a JSON object".into(),
            })
        }
    };
    if let Some(inner) = object.get("attributes").and_then(Value::as_object) {
        return Ok(inner);
    }
    Ok(object)
}

/// First present, non-empty string among candidate field names.
pub(crate) fn first_str(fields: &Map<String, Value>, keys: &[&str]) -> Option<String> {
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

/// Collects strings from a field that is either a string, an array of
/// strings, or absent (vendor APIs disagree about single vs plural).
pub(crate) fn string_list(fields: &Map<String, Value>, keys: &[&str]) -> Vec<String> {
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

/// Copies the named vendor fields into `observed_fields` verbatim when
/// present and non-null — the observed tier carries the source's values
/// unchanged (the core's precedence rules govern who wins).
pub(crate) fn observed_fields(fields: &Map<String, Value>, keys: &[&str]) -> Value {
    let mut out = Map::new();
    for key in keys {
        if let Some(value) = fields.get(*key).filter(|v| !v.is_null()) {
            out.insert((*key).to_string(), value.clone());
        }
    }
    Value::Object(out)
}
