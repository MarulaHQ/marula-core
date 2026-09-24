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

/// Vendor-field extraction is ONE definition consumed by every adapter
/// (the CI3 identity-key path must not diverge between connectors):
/// `adapters_core::vendor_fields` (Wave 15 lane 4).
pub(crate) use adapters_core::{first_str, observed_fields, string_list};
