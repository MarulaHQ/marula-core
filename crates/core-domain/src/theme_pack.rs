//! Theme-token schema v1 + theme-pack manifests (Wave 24 lane 1
//! `p24-theming-system`).
//!
//! Purpose: generalize the Marula token set behind docs/theme.md into a
//! VERSIONED token schema. A theme is a map of LIGHT/DARK values over the
//! closed semantic role vocabulary (docs/theme.md §3); "install a theme"
//! means "adopt a validated value set that restyles surfaces". Two
//! guarantees make restyling safe by construction:
//!
//! 1. **Closed keys, typed rejection.** A theme may only assign values to
//!    roles the schema knows. Off-schema keys are rejected with the exact
//!    key name — a pack can never declare motion, typography, radius, or
//!    any other dimension the schema does not expose (docs/theme.md override
//!    rules win).
//! 2. **Contrast floors at the data boundary.** Text pairs on the canvas
//!    and surface must meet AA (4.5:1), interactive pairs the 3:1 UI floor,
//!    status foreground/background pairs the 3:1 tag floor — the same
//!    numbers `packages/ui/tests/tokens.test.ts` proves for the built-ins.
//!
//! Non-purpose: persistence (core-db owns the `theme_packs` table), the
//! HTTP surface, and preset VALUES (they live in `packages/ui` +
//! `tokens.css`; the built-in preset ids are mirrored here ONLY so
//! enumeration validation is backend-typed).

use crate::error::DomainError;
use serde::{Deserialize, Serialize};

/// Current schema version. A manifest written against another version is
/// refused — versioned interface, deny-by-default (same posture as
/// PLUGIN_RUNTIME_INTERFACE_VERSION).
pub const THEME_TOKEN_SCHEMA_VERSION: u32 = 1;

/// The closed semantic-role vocabulary a theme may carry. Every role has
/// one value per mode (light / dark). Hex-only — derived tones are
/// recomputed client-side from the base values (color-mix mirrors
/// tokens.css derivation), never declared in a pack.
pub const TOKEN_ROLES: &[&str] = &[
    "background",
    "surface",
    "overlay",
    "border",
    "text",
    "text_secondary",
    "text_muted",
    "accent",
    "on_accent",
    "destructive",
    "status_success_fg",
    "status_success_bg",
    "status_warning_fg",
    "status_warning_bg",
    "status_danger_fg",
    "status_danger_bg",
    "status_neutral_fg",
    "status_neutral_bg",
    "chart_1",
    "chart_2",
    "chart_3",
    "chart_4",
    "chart_5",
];

/// The built-in presets shipped by Leg A, repointed to Marula in wave41
/// lane 2. Enumerated here so the typed-settings registry (and any backend
/// validation of custom-pack ids) rejects unknown preset ids without an
/// ad-hoc option list.
///
/// `slate` and `copper` are dropped (wave41 D8) and `tangerine` is renamed
/// to `marula`; migration 0115 moves the stored values.
pub const BUILTIN_PRESET_OPTIONS: &[&str] = &["marula", "marula-warm", "high_contrast"];

/// The roles a theme is ALLOWED to move (wave41 D8, brand spec §4.5).
///
/// A theme restyles surfaces; it does not redefine what a colour means. The
/// primary and its ink-to-gold mode flip, `destructive`, every status pair and
/// the assistant inversion are the brand's, and an interface built against
/// them has to keep working when the brand is restyled. The roles below are
/// the complement: neutrals, chrome and the accent TINT.
///
/// Mirrored by SHIFTABLE_AXIS in packages/ui/src/theme-tokens.ts; the two are
/// asserted equal by the frontend preset test, so they cannot drift.
pub const SHIFTABLE_PRESET_ROLES: &[&str] = &[
    "background",
    "surface",
    "overlay",
    "border",
    "text",
    "text_secondary",
    "text_muted",
    "accent",
    "on_accent",
    "chart_1",
    "chart_2",
    "chart_3",
    "chart_4",
    "chart_5",
];

/// True when `role` is outside the shiftable axis, i.e. a preset naming it is
/// trying to restyle a brand semantic and must be rejected at the boundary.
pub fn is_brand_fixed_role(role: &str) -> bool {
    TOKEN_ROLES.contains(&role) && !SHIFTABLE_PRESET_ROLES.contains(&role)
}

/// Per-role caps (deny oversized declarations at the boundary).
pub const MAX_TOKENS_PER_MODE: usize = 23; // exactly the role vocabulary

/// Values one mode may override, keyed by token role. The wire shape of a
/// theme pack's `tokens.light` / `tokens.dark` maps.
pub type TokenValues = std::collections::BTreeMap<String, String>;

/// The declarative theme-pack manifest: the ONLY content a theme pack
/// carries — identity + token overrides. No capabilities, no code, no
/// settings schema: themes restyle and nothing else.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThemePackManifest {
    /// Literal `"kind": "theme"` — the distribution seam's dispatch key.
    pub kind: String,
    pub name: String,
    pub version: String,
    pub schema_version: u32,
    #[serde(default)]
    pub description: String,
    /// Values applied on the light canvas. Absent ⇒ no light overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<TokenValues>,
    /// Values applied when dark mode is active. Absent ⇒ the preset's
    /// own dark values ride (light overrides must exist for a valid pack).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dark: Option<TokenValues>,
}

impl ThemePackManifest {
    /// Validates the manifest: identity, pinned schema version, the exact
    /// closed-key vocabulary (typed off-schema-token rejection), hex
    /// format everywhere, and the AA/3:1 contrast floors.
    pub fn validate(&self) -> Result<(), DomainError> {
        let validation = |m: String| Err(DomainError::Validation(m));
        if self.kind != "theme" {
            return validation(format!(
                "theme pack kind must be \"theme\", got {:?}",
                self.kind
            ));
        }
        if self.name.trim().is_empty() {
            return validation("theme pack name must not be empty".into());
        }
        if self.version.trim().is_empty() {
            return validation("theme pack version must not be empty".into());
        }
        if self.schema_version != THEME_TOKEN_SCHEMA_VERSION {
            return validation(format!(
                "theme pack schema_version {} is not supported (runtime speaks {})",
                self.schema_version, THEME_TOKEN_SCHEMA_VERSION
            ));
        }
        for (mode, values) in [("light", self.light.as_ref()), ("dark", self.dark.as_ref())] {
            let Some(values) = values else { continue };
            for role in values.keys() {
                if !TOKEN_ROLES.contains(&role.as_str()) {
                    return validation(format!(
                        "unknown token role {role:?} in {mode} values — the theme-token schema v{THEME_TOKEN_SCHEMA_VERSION} vocabulary is closed"
                    ));
                }
            }
        }
        if self.light.is_none() && self.dark.is_none() {
            return validation("theme pack carries no token values".into());
        }
        for (mode, values) in [("light", self.light.as_ref()), ("dark", self.dark.as_ref())] {
            let Some(values) = values else { continue };
            for (role, hex) in values.iter() {
                if is_valid_hex(hex) {
                    continue;
                }
                return validation(format!(
                    "token role {role:?} value {hex:?} is not a lowercase #rrggbb hex color ({mode} values)"
                ));
            }
        }
        self.assert_contrast_floors()?;
        Ok(())
    }

    /// Coerces a RAW theme export — a tweakcn export
    /// (`design/marula/marula.json` is the vendored exemplar:
    /// `{ name, cssVars: { light, dark } }` over the shadcn variable
    /// vocabulary, frequently `oklch()`) or a shadcn registry-item theme
    /// (same `cssVars` shape) — into a validated [`ThemePackManifest`].
    ///
    /// This is the "import by export" posture the marketplace/settings
    /// surfaces lean on: the designer's export file IS the pack
    /// manifest, no hand translation. Identity comes from the export's
    /// `name` (version 1.0.0 and the current schema version are stamped);
    /// unmapped variables (sidebar ramps, hover tints, radius/fonts) are
    /// dropped, never guessed — the schema stays restyle-only over the
    /// closed role vocabulary; the coerced manifest passes the SAME
    /// validate() gate as a native one, so an export whose converted
    /// basics violate the contrast floors is refused outright.
    pub fn coerce_from_export(value: &serde_json::Value) -> Result<Self, DomainError> {
        let err = |m: String| Err(DomainError::Validation(m));
        if value.get("kind").and_then(|v| v.as_str()) == Some("theme") {
            let native: ThemePackManifest = serde_json::from_value(value.clone())
                .map_err(|e| DomainError::Validation(format!("manifest is invalid: {e}")))?;
            native.validate()?;
            return Ok(native);
        }
        let Some(css_vars) = value.get("cssVars").and_then(|v| v.as_object()) else {
            return err(
                "manifest is neither a native theme pack (kind: \"theme\") nor a tweakcn/shadcn cssVars export".to_string(),
            );
        };
        let collect = |mode: Option<&serde_json::Value>,
                       label: &str|
         -> Result<TokenValues, DomainError> {
            let mut out = TokenValues::new();
            let Some(obj) = mode.and_then(|m| m.as_object()) else {
                return Ok(out);
            };
            for (key, raw) in obj {
                let Some(role) = role_for(key) else {
                    continue;
                };
                let Some(raw_str) = raw.as_str() else {
                    continue;
                };
                let Some(hex) = normalize_color(raw_str) else {
                    return Err(DomainError::Validation(format!(
                        "export variable --{key} ({label}) is {raw:?} — only #rrggbb hex or oklch() values coerce"
                    )));
                };
                out.insert(role.to_string(), hex);
            }
            Ok(out)
        };
        let light = collect(css_vars.get("light"), "light")?;
        let dark = collect(css_vars.get("dark"), "dark")?;
        if light.is_empty() && dark.is_empty() {
            return err(
                "the export carries no mappable variables — a theme pack must restyle at least one token role".to_string(),
            );
        }
        let name = value
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("custom theme")
            .trim();
        let manifest = ThemePackManifest {
            kind: "theme".into(),
            name: if name.is_empty() {
                "custom theme".into()
            } else {
                name.into()
            },
            version: "1.0.0".into(),
            schema_version: THEME_TOKEN_SCHEMA_VERSION,
            description: "coerced from a tweakcn/shadcn theme export".into(),
            light: Some(light),
            dark: Some(dark),
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// The theme id (slug) for display. Derived from the name.
    pub fn slug_of(name: &str) -> String {
        name.trim()
            .to_lowercase()
            .replace(' ', "-")
            .chars()
            .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
            .take(32)
            .collect()
    }

    /// The contrast floors. Only pairs the pack DECLARES on both sides
    /// can be checked in isolation (a half-declared pair inherits the
    /// active preset for the missing side, which its published preset
    /// validated), and every declared pair must clear the same floors the
    /// shipped presets are held to:
    /// - text / text_secondary / text_muted over background AND surface:
    ///   AA text floor 4.5:1.
    /// - on_accent over accent: 3:1 UI floor (the docs/theme.md §2.6 tradeoff).
    /// - each status fg over its bg: AA text floor 4.5:1 (the pair carries
    ///   error/warning copy, not just a non-text indicator; p29 lane).
    fn assert_contrast_floors(&self) -> Result<(), DomainError> {
        for (mode, values) in [("light", self.light.as_ref()), ("dark", self.dark.as_ref())] {
            let Some(values) = values else { continue };
            for fg_role in ["text", "text_secondary", "text_muted"] {
                let Some(fg) = values.get(fg_role) else {
                    continue;
                };
                for bg_role in ["background", "surface"] {
                    let Some(bg) = values.get(bg_role) else {
                        continue;
                    };
                    let ratio = contrast_ratio(fg, bg);
                    if ratio < 4.5 {
                        return Err(DomainError::Validation(format!(
                            "{mode} values: {fg_role} on {bg_role} is {ratio:.2}:1 (AA text floor is 4.5:1)"
                        )));
                    }
                }
            }
            if let (Some(accent), Some(on)) = (values.get("accent"), values.get("on_accent")) {
                let ratio = contrast_ratio(accent, on);
                if ratio < 3.0 {
                    return Err(DomainError::Validation(format!(
                        "{mode} values: on_accent over accent is {ratio:.2}:1 (UI floor is 3:1)"
                    )));
                }
            }
            for tone in ["success", "warning", "danger"] {
                let (Some(fg), Some(bg)) = (
                    values.get(&format!("status_{tone}_fg")),
                    values.get(&format!("status_{tone}_bg")),
                ) else {
                    continue;
                };
                let ratio = contrast_ratio(fg, bg);
                if ratio < 4.5 {
                    return Err(DomainError::Validation(format!(
                        "{mode} values: status_{tone}_fg over its bg is {ratio:.2}:1 (AA text floor is 4.5:1; the pair carries error/warning copy, not just an indicator)"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// The theme id (slug) used in settings values: one of the built-in
/// preset ids, or a pack slug of lowercase alphanumerics/dashes (no
/// leading/trailing dash), 32 chars max.
pub fn is_preset_or_pack_slug(value: &str) -> bool {
    if value.is_empty() || value.len() > 32 || value.starts_with('-') || value.ends_with('-') {
        return false;
    }
    if BUILTIN_PRESET_OPTIONS.contains(&value) {
        return true;
    }
    value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `#rrggbb` lowercase hex, the only color wire format.
pub fn is_valid_hex(value: &str) -> bool {
    let body = value.strip_prefix('#').unwrap_or(value);
    body.len() == 6
        && body
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

// --- tweakcn / shadcn export coercion -------------------------------------------

/// Converts an `oklch(L C H)` (or `oklch(L% C H)`) color to `#rrggbb`
/// sRGB, clamping out-of-gamut channels. This is what makes a tweakcn or
/// shadcn registry theme export a zero-translation pack manifest: both
/// publish the shadcn variable vocabulary (frequently `oklch()`), and
/// the theme-token schema is hex-only.
pub fn oklch_to_hex(value: &str) -> Option<String> {
    let inner = value.trim().strip_prefix("oklch(")?.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }
    let num = |raw: &str| -> Option<f64> {
        raw.trim_end_matches('%').parse::<f64>().ok().map(|v| {
            if raw.ends_with('%') {
                v / 100.0
            } else {
                v
            }
        })
    };
    let l = num(parts[0])?;
    let c = num(parts[1])?;
    let h = num(parts[2])?;
    let alpha = parts.len() < 4 || parts[3].starts_with('1') || parts[3].starts_with("100%");
    if !alpha {
        return None;
    }
    // oklch → oklab
    let h_rad = h.to_radians();
    let a = c * h_rad.cos();
    let b = c * h_rad.sin();
    // oklab → LMS (Björn Ottosson's reference constants)
    let l_ = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = l - 0.089_484_177_5 * a - 1.291_485_548_0 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    // LMS³ → linear sRGB
    let r_lin = 4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3;
    let g_lin = -1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3;
    let b_lin = -0.196_952_486_4 * l3 - 0.918_361_587_4 * m3 + 1.829_602_195_4 * s3;
    let gamma = |x: f64| -> u8 {
        let v = if x <= 0.003_130_8 {
            12.92 * x
        } else {
            1.055 * x.abs().powf(1.0 / 2.4) - 0.055
        };
        (v.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        gamma(r_lin),
        gamma(g_lin),
        gamma(b_lin)
    ))
}

/// Accepts `#rrggbb` or `oklch(...)` and normalizes to lowercase hex.
fn normalize_color(value: &str) -> Option<String> {
    if let Some(hex) = value.trim().strip_prefix('#') {
        return if hex.len() == 6 {
            Some(format!("#{}", hex.to_lowercase()))
        } else {
            None
        };
    }
    if value.trim_start().starts_with("oklch(") {
        return oklch_to_hex(value).map(|h| h.to_lowercase());
    }
    None
}

/// shadcn variable name → theme-token role. Unmapped export variables
/// (sidebar ramps, hover tints, radius, fonts) are NOT carried — the
/// schema is restyle-only over the closed role vocabulary, and derived
/// tones recompute client-side from the carried basics.
fn role_for(shadcn_key: &str) -> Option<&'static str> {
    Some(match shadcn_key {
        "background" => "background",
        "card" => "surface",
        "popover" => "overlay",
        "border" | "input" => "border",
        "foreground" => "text",
        "secondary-foreground" => "text_secondary",
        "muted-foreground" => "text_muted",
        "primary" => "accent",
        "primary-foreground" => "on_accent",
        "destructive" => "destructive",
        "chart-1" => "chart_1",
        "chart-2" => "chart_2",
        "chart-3" => "chart_3",
        "chart-4" => "chart_4",
        "chart-5" => "chart_5",
        _ => return None,
    })
}

/// WCAG 2.1 relative luminance of `#rrggbb`.
fn luminance(hex: &str) -> f64 {
    let body = hex.strip_prefix('#').unwrap_or(hex);
    let channel = |i: usize| -> f64 {
        let raw = u16::from_str_radix(&body[i..i + 2], 16).unwrap_or(0) as f64 / 255.0;
        if raw <= 0.03928 {
            raw / 12.92
        } else {
            ((raw + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4)
}

/// WCAG 2.1 contrast ratio between two ``#rrggbb`` values.
pub fn contrast_ratio(fg: &str, bg: &str) -> f64 {
    let l1 = luminance(fg);
    let l2 = luminance(bg);
    let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn oklch_conversion_round_trips_the_vendored_primary() {
        // marula.json carries primary as oklch(0.2800 0.0180 300.0000)
        // and the vendored sRGB value in tokens.css is #2B2A33.
        let hex = oklch_to_hex("oklch(0.2800 0.0180 300.0000)").unwrap();
        let exact = "#2b2a33".to_string();
        let chan = |h: &str, i: usize| i64::from_str_radix(&h[1 + i..3 + i], 16).unwrap();
        for i in 0..3 {
            let diff = chan(&hex, i).abs_diff(chan(&exact, i));
            assert!(
                diff <= 2,
                "converted {hex} channel {i} is {diff} off #2b2a33"
            );
        }
    }

    #[test]
    fn tweakcn_export_coerces_without_hand_translation() {
        let export = json!({
            "$schema": "https://tweakcn.com/r/themes/marula.json",
            "name": "Marula",
            "type": "registry:style",
            "cssVars": {
                "light": {
                    "background": "oklch(0.9750 0.0040 90.0000)",
                    "card": "#ffffff",
                    "foreground": "oklch(0.2800 0.0180 300.0000)",
                    "primary": "#2b2a33",
                    "primary-foreground": "#e8b83a",
                    "chart-1": "#a8641a",
                    "sidebar": "oklch(0.9600 0.0050 90.0000)"
                },
                "dark": {
                    "background": "#2b2a33"
                }
            }
        });
        let pack = ThemePackManifest::coerce_from_export(&export).unwrap();
        assert_eq!(pack.name, "Marula");
        // Mapped roles carry; unmapped (sidebar) are dropped silently.
        assert!(pack.light.as_ref().unwrap().get("background").is_some());
        assert!(pack.light.as_ref().unwrap().get("chart_1").is_some());
        assert!(pack.light.as_ref().unwrap().get("sidebar").is_none());
        assert!(pack.dark.as_ref().unwrap().get("background").is_some());
    }

    #[test]
    fn export_with_non_color_variable_refuses() {
        let export = json!({
            "name": "Broken",
            "cssVars": {"light": {"background": "linear-gradient(90deg, red, blue)"}}
        });
        let err = ThemePackManifest::coerce_from_export(&export).unwrap_err();
        assert!(err.to_string().contains("--background"), "{err}");
    }

    #[test]
    fn native_and_export_shapes_are_both_accepted() {
        let native = json!({
            "kind": "theme", "name": "N", "version": "1.0.0",
            "schema_version": THEME_TOKEN_SCHEMA_VERSION,
            "light": {"background": "#f8f4ec"}
        });
        assert!(ThemePackManifest::coerce_from_export(&native).is_ok());
        assert!(ThemePackManifest::coerce_from_export(&json!({"hello": 1})).is_err());
    }

    fn pack(light: Vec<(&str, &str)>, dark: Vec<(&str, &str)>) -> ThemePackManifest {
        let to_map = |rows: Vec<(&str, &str)>| -> TokenValues {
            rows.into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        ThemePackManifest {
            kind: "theme".into(),
            name: "Warm Ochre".into(),
            version: "1.0.0".into(),
            schema_version: THEME_TOKEN_SCHEMA_VERSION,
            description: "brand-warm alternate".into(),
            light: Some(to_map(light)),
            dark: Some(to_map(dark)),
        }
    }

    #[test]
    fn valid_pack_with_contrasting_tokens_is_accepted() {
        let p = pack(
            vec![
                ("background", "#faeade"),
                ("surface", "#fff8f0"),
                ("text", "#333333"),
                ("text_secondary", "#4b5563"),
                ("text_muted", "#58606d"),
                ("accent", "#8a4b1a"),
                ("on_accent", "#ffffff"),
                ("chart_1", "#96694a"),
            ],
            vec![("background", "#221810"), ("text", "#e5e5e5")],
        );
        assert!(p.validate().is_ok());
    }

    #[test]
    fn off_schema_token_role_is_typed_rejected() {
        let mut light: TokenValues = TokenValues::new();
        light.insert("background".into(), "#efeade".into());
        light.insert("font_family".into(), "Comic Sans".into());
        let p = ThemePackManifest {
            kind: "theme".into(),
            name: "Bad Keys".into(),
            version: "1.0.0".into(),
            schema_version: THEME_TOKEN_SCHEMA_VERSION,
            description: String::new(),
            light: Some(light),
            dark: None,
        };
        let err = p.validate().expect_err("off-schema key must reject");
        assert!(err.to_string().contains("font_family"), "{err}");
    }

    #[test]
    fn interface_mismatch_reject() {
        let p = pack(vec![("background", "#ZZZZZZ")], vec![]);
        assert!(p.validate().is_err());
        let mut p = pack(vec![("background", "#efeade")], vec![]);
        p.schema_version = THEME_TOKEN_SCHEMA_VERSION + 1;
        assert!(p.validate().is_err());
    }

    #[test]
    fn contrast_floor_rejects_low_pairs_both_modes() {
        // text barely above background — AA fails in mode declared
        let p = pack(vec![("background", "#fafafa"), ("text", "#eeeeee")], vec![]);
        let err = p.validate().expect_err("AA floor must reject");
        assert!(err.to_string().contains("4.5"), "{err}");
    }

    #[test]
    fn accent_pair_uses_the_three_to_one_floor() {
        // accent #E05D38 with white — 3.63:1, matches the docs/theme.md
        // §2.6 pinned tradeoff and passes; a near-invisible pair fails.
        let ok = pack(
            vec![("accent", "#e05d38"), ("on_accent", "#ffffff")],
            vec![],
        );
        assert!(ok.validate().is_ok());
        let bad = pack(
            vec![("accent", "#fef3c7"), ("on_accent", "#fffbeb")],
            vec![],
        );
        let err = bad.validate().expect_err("3:1 floor must reject");
        assert!(err.to_string().contains("on_accent"), "{err}");
    }

    #[test]
    fn status_pair_uses_the_three_to_one_floor() {
        let p = pack(
            vec![
                ("status_success_fg", "#15803d"),
                ("status_success_bg", "#dcfce7"),
            ],
            vec![],
        );
        assert!(p.validate().is_ok());
        let p = pack(
            vec![
                ("status_warning_fg", "#fef3c7"),
                ("status_warning_bg", "#fbbf24"),
            ],
            vec![],
        );
        let err = p.validate().expect_err("3:1 tag floor must reject");
        assert!(err.to_string().contains("status_warning"), "{err}");
    }
}
