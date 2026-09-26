//! Label templates (Wave 32 lane 5, `p32-labels-scanning`).
//!
//! A template is a typed, org-scoped print recipe for one ENTITY PURPOSE:
//! which field tokens stack down the label, which token the Code128
//! symbology encodes, whether a QR rides along, and the physical size
//! (common Zebra + DYMO stock presets, or a custom size). Rendering —
//! layout, truncation, ZPL — is deterministic and lives in
//! [`crate::labels`]; this module owns the entity shape and the validation
//! that keeps the stored token lists honest per purpose.
//!
//! The QR always encodes the entity's canonical deep link
//! (`meridian://<kind>/<uuid>`), never a free-form payload: a scan must mean
//! ONE thing on every surface (the shared scanned-payload contract).

use crate::error::DomainError;
use crate::ids::OrgId;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Print purposes a template can serve. Additive-only: widen this list and
/// the DB CHECK together.
pub const LABEL_PURPOSES: [&str; 5] = [
    "stock_item",
    "stock_location",
    "serialized_unit",
    "device_asset",
    "tool_kit",
];

/// Text field tokens a template can stack. `location` is the unit's current
/// stock location; `tag` is a device asset's human asset tag.
pub const LABEL_TOKENS: [&str; 8] = [
    "name",
    "sku",
    "part_number",
    "serial",
    "tag",
    "code",
    "location",
    "org_name",
];

/// Size presets: `(id, width_tenths_mm, height_tenths_mm)`. Zebra entries are
/// common ZPL thermal stock; DYMO entries are LabelWriter die-cut stock.
pub const LABEL_SIZE_PRESETS: [(&str, i32, i32); 6] = [
    ("zebra_2x1", 508, 254),
    ("zebra_3x1", 762, 254),
    ("zebra_4x6", 1016, 1524),
    ("dymo_address", 890, 286),
    ("dymo_multipurpose", 570, 320),
    ("dymo_square", 254, 254),
];

/// The printers a preset targets (doubles as honest labelling for the UI).
pub fn preset_target(preset: &str) -> &'static str {
    match preset {
        "zebra_2x1" | "zebra_3x1" | "zebra_4x6" => "zebra",
        "dymo_address" | "dymo_multipurpose" | "dymo_square" => "dymo",
        _ => "custom",
    }
}

/// Allowed text tokens per purpose. A token outside its purpose's set is a
/// typed validation error, never a silently dropped line.
pub fn allowed_tokens(purpose: &str) -> &'static [&'static str] {
    match purpose {
        "stock_item" => &["name", "sku", "part_number", "org_name"],
        "stock_location" => &["name", "code", "org_name"],
        "serialized_unit" => &["name", "sku", "serial", "location", "org_name"],
        "device_asset" => &["name", "tag", "serial", "org_name"],
        "tool_kit" => &["name", "code", "org_name"],
        _ => &[],
    }
}

/// Tokens the Code128 symbology may encode per purpose.
pub fn barcode_tokens(purpose: &str) -> &'static [&'static str] {
    match purpose {
        "stock_item" => &["sku", "part_number"],
        "stock_location" => &["code"],
        "serialized_unit" => &["serial", "sku"],
        "device_asset" => &["tag", "serial"],
        "tool_kit" => &["code"],
        _ => &[],
    }
}

/// Human label for a token (UI + docs spelling).
pub fn token_label(token: &str) -> &'static str {
    match token {
        "name" => "Name",
        "sku" => "SKU",
        "part_number" => "Part number",
        "serial" => "Serial number",
        "tag" => "Asset tag",
        "code" => "Code",
        "location" => "Current location",
        "org_name" => "Organization",
        _ => "Unknown token",
    }
}

/// Deep-link kind segment for a purpose. These are the canonical scan
/// targets: `meridian://stock-item/<uuid>` and friends.
pub fn deep_link_kind(purpose: &str) -> &'static str {
    match purpose {
        "stock_item" => "stock-item",
        "stock_location" => "stock-location",
        "serialized_unit" => "stock-serial",
        "device_asset" => "device-asset",
        "tool_kit" => "tool-kit",
        _ => "unknown",
    }
}

/// The canonical deep link for a rendered entity: the QR payload, and the
/// round-trip a scanner hands back to `POST /api/scan/resolve`.
pub fn deep_link(purpose: &str, id: Uuid) -> String {
    format!("meridian://{}/{}", deep_link_kind(purpose), id)
}

/// A typed, org-scoped label template.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LabelTemplate {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub name: String,
    pub purpose: String,
    pub size_preset: String,
    /// Present only when `size_preset = 'custom'`.
    pub width_tenths_mm: Option<i32>,
    pub height_tenths_mm: Option<i32>,
    pub dpi: i32,
    pub barcode_token: Option<String>,
    pub include_qr: bool,
    /// Ordered text lines; `Vec` is stored as a JSON array.
    pub fields: Vec<String>,
    pub is_default: bool,
    pub active: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Resolves the physical size in tenths of a millimetre: a preset's fixed
/// dimensions, or the custom pair. Returns `None` for an unknown preset.
pub fn size_tenths_mm(template: &LabelTemplate) -> Option<(i32, i32)> {
    if template.size_preset == "custom" {
        template.width_tenths_mm.zip(template.height_tenths_mm)
    } else {
        LABEL_SIZE_PRESETS
            .iter()
            .find(|(id, _, _)| *id == template.size_preset)
            .map(|(_, w, h)| (*w, *h))
    }
}

impl LabelTemplate {
    pub fn validate(&self) -> Result<(), DomainError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(DomainError::Validation(
                "label template name is required".into(),
            ));
        }
        if name.chars().count() > 80 {
            return Err(DomainError::Validation(
                "label template name must be at most 80 characters".into(),
            ));
        }
        if !LABEL_PURPOSES.contains(&self.purpose.as_str()) {
            return Err(DomainError::Validation(format!(
                "purpose must be one of {}",
                LABEL_PURPOSES.join(", ")
            )));
        }
        if size_tenths_mm(self).is_none() {
            return Err(DomainError::Validation(
                "size_preset must be a known preset or custom with both dimensions".into(),
            ));
        }
        if let Some((w, h)) = size_tenths_mm(self) {
            if !(100..=3000).contains(&w) || !(100..=3000).contains(&h) {
                return Err(DomainError::Validation(
                    "custom label size must be within 10–300 mm".into(),
                ));
            }
        }
        if !matches!(self.dpi, 203 | 300 | 600) {
            return Err(DomainError::Validation(
                "dpi must be 203, 300 or 600".into(),
            ));
        }
        let allowed = allowed_tokens(&self.purpose);
        if self.fields.len() > 6 {
            return Err(DomainError::Validation(
                "a label stacks at most 6 field lines".into(),
            ));
        }
        let mut seen = Vec::new();
        for token in &self.fields {
            if !allowed.contains(&token.as_str()) {
                return Err(DomainError::Validation(format!(
                    "'{token}' is not a field this purpose can print (allowed: {})",
                    allowed.join(", ")
                )));
            }
            if seen.contains(token) {
                return Err(DomainError::Validation(format!(
                    "'{token}' appears twice — a label line prints once"
                )));
            }
            seen.push(token.clone());
        }
        if let Some(token) = &self.barcode_token {
            let barcode_allowed = barcode_tokens(&self.purpose);
            if !barcode_allowed.contains(&token.as_str()) {
                return Err(DomainError::Validation(format!(
                    "the barcode cannot encode '{token}' for this purpose (allowed: {})",
                    barcode_allowed.join(", ")
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(purpose: &str) -> LabelTemplate {
        LabelTemplate {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            name: "Test".into(),
            purpose: purpose.into(),
            size_preset: "zebra_2x1".into(),
            width_tenths_mm: None,
            height_tenths_mm: None,
            dpi: 203,
            barcode_token: None,
            include_qr: true,
            fields: vec!["name".into()],
            is_default: false,
            active: true,
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn purpose_token_matrix_is_enforced() {
        let mut bad = template("stock_location");
        bad.fields = vec!["serial".into()];
        assert!(bad.validate().is_err(), "serial is not a location token");

        let mut ok = template("serialized_unit");
        ok.fields = vec!["name".into(), "serial".into(), "location".into()];
        ok.barcode_token = Some("serial".into());
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn duplicate_lines_and_bad_barcode_tokens_are_rejected() {
        let mut dup = template("stock_item");
        dup.fields = vec!["name".into(), "name".into()];
        assert!(dup.validate().is_err());

        let mut bad_barcode = template("stock_location");
        bad_barcode.barcode_token = Some("sku".into());
        assert!(bad_barcode.validate().is_err());

        let mut ok_barcode = template("stock_location");
        ok_barcode.barcode_token = Some("code".into());
        ok_barcode.fields = vec!["name".into(), "code".into()];
        assert!(ok_barcode.validate().is_ok());
    }

    #[test]
    fn custom_size_needs_both_dimensions_within_range() {
        let mut custom = template("stock_item");
        custom.size_preset = "custom".into();
        assert!(custom.validate().is_err(), "missing dims");
        custom.width_tenths_mm = Some(400);
        custom.height_tenths_mm = Some(300);
        assert!(custom.validate().is_ok());
        custom.height_tenths_mm = Some(9000);
        assert!(custom.validate().is_err(), "out of range");
    }

    #[test]
    fn deep_links_are_canonical_and_stable() {
        let id = Uuid::nil();
        assert_eq!(
            deep_link("stock_item", id),
            format!("meridian://stock-item/{id}")
        );
        assert_eq!(
            deep_link("serialized_unit", id),
            format!("meridian://stock-serial/{id}")
        );
        assert_eq!(
            deep_link("device_asset", id),
            format!("meridian://device-asset/{id}")
        );
        assert_eq!(
            deep_link("stock_location", id),
            format!("meridian://stock-location/{id}")
        );
    }
}
