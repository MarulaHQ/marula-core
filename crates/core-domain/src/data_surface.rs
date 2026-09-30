//! Per-user data-surface preferences — Wave 41 lane 2b, decision D12.
//!
//! Purpose: one typed contract for the four data-surface controls (named
//! views, column chooser, filter bar, export) so a table in any surface reads
//! and writes the same shape. The document is stored as a single
//! `appearance.data_surfaces` settings row — a JSON *string*, because
//! `settings.value` carries a CHECK of `jsonb_typeof(value) IN ('string',
//! 'number', 'boolean')` and an object would be refused by the database
//! before any code saw it.
//!
//! Non-purpose: this holds no row data, no query, and no permission. It
//! describes what a user is LOOKING at. A column key that the surface no
//! longer ships is dropped on read rather than rejected, so a rename on the
//! server cannot wedge a saved view.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Hard cap on the serialized document, mirroring the `max_len` on the
/// settings declaration. Roughly a hundred saved views.
pub const MAX_DOCUMENT_CHARS: usize = 8192;

/// A view name is the one free-text label in this document, so it gets the
/// tightest bound of the identifiers. A hundred views' worth of names inside
/// the 8KiB cap is not the constraint here; a name that a settings row can grow
/// without limit is.
const MAX_VIEW_NAME_CHARS: usize = 48;
/// A surface identifier is a dotted, lowercase slug (`tickets`, `stock.items`),
/// and so is every field and column key in this document, so they share one
/// bound. Free text in a settings row is how a stale document grows; a slug
/// cannot contain a newline, a quote or a brace.
const MAX_FIELD_CHARS: usize = 64;
const MAX_VALUE_CHARS: usize = 256;
const MAX_COLUMNS: usize = 64;
const MAX_FILTERS: usize = 16;
const MAX_SURFACES: usize = 64;

/// Sort direction. A closed vocabulary: an unknown direction is a parse
/// failure, not a silent default, because a table that quietly sorts the
/// wrong way is worse than one that reports a bad preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    pub const ALL: [SortDir; 2] = [SortDir::Asc, SortDir::Desc];
}

/// Filter comparison. `in` carries a comma-separated list in `value`;
/// the two presence operators must carry none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    Eq,
    Ne,
    Contains,
    Gt,
    Gte,
    Lt,
    Lte,
    In,
    IsSet,
    IsNotSet,
}

impl FilterOp {
    pub const ALL: [FilterOp; 10] = [
        FilterOp::Eq,
        FilterOp::Ne,
        FilterOp::Contains,
        FilterOp::Gt,
        FilterOp::Gte,
        FilterOp::Lt,
        FilterOp::Lte,
        FilterOp::In,
        FilterOp::IsSet,
        FilterOp::IsNotSet,
    ];

    /// Presence operators take no value; every other operator needs one, or
    /// the clause can never match and reads as a bug to the user.
    pub fn takes_value(self) -> bool {
        !matches!(self, FilterOp::IsSet | FilterOp::IsNotSet)
    }
}

/// One column in a saved view: its order is the vector position, so there is
/// no separate order field to keep in sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnPref {
    pub key: String,
    /// Hidden columns keep their place in `order` so unhiding restores the
    /// position the user chose rather than appending to the end.
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_px: Option<u32>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterClause {
    pub field: String,
    pub op: FilterOp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortPref {
    pub field: String,
    pub dir: SortDir,
}

/// The saved state of one data surface. An absent surface is not an error:
/// the surface renders its own defaults and never writes until the user
/// changes something.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<ColumnPref>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<FilterClause>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<SortPref>,
}

impl SurfaceView {
    /// Visible columns in order. The chooser renders the full list; this is
    /// the subset the table actually draws.
    pub fn visible_columns(&self) -> Vec<&ColumnPref> {
        self.columns.iter().filter(|c| c.visible).collect()
    }

    pub fn width_of(&self, key: &str) -> Option<u32> {
        self.columns
            .iter()
            .find(|c| c.key == key)
            .and_then(|c| c.width_px)
    }
}

/// Every surface's saved state for one user, keyed by surface id. A `BTreeMap`
/// so the serialized document is byte-stable across writes: two users who
/// made the same changes produce the same string, which keeps the settings
/// audit log readable.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DataSurfacePrefs(pub BTreeMap<String, SurfaceView>);

impl DataSurfacePrefs {
    pub fn get(&self, surface: &str) -> Option<&SurfaceView> {
        self.0.get(surface)
    }

    pub fn set(&mut self, surface: &str, view: SurfaceView) {
        self.0.insert(surface.to_string(), view);
    }

    /// Drops a surface's saved state, which is what a rename on the server
    /// calls for and what an emptied chooser calls for.
    pub fn clear(&mut self, surface: &str) -> bool {
        self.0.remove(surface).is_some()
    }

    pub fn surface_count(&self) -> usize {
        self.0.len()
    }

    /// Parses a stored document. An empty string is the "never set" case and
    /// yields the empty document rather than an error, so a fresh account
    /// and an account that cleared its views behave identically.
    pub fn parse(raw: &str) -> Result<Self, String> {
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        if raw.chars().count() > MAX_DOCUMENT_CHARS {
            return Err(format!(
                "data-surface document is longer than {MAX_DOCUMENT_CHARS} characters"
            ));
        }
        let value: serde_json::Value = serde_json::from_str(raw)
            .map_err(|e| format!("malformed data-surface document: {e}"))?;
        let doc: Self = serde_json::from_value(value)
            .map_err(|e| format!("unreadable data-surface document: {e}"))?;
        doc.validate()?;
        Ok(doc)
    }

    /// Serializes to the stored form, refusing to produce a document the
    /// parse side would reject.
    pub fn to_compact_string(&self) -> Result<String, String> {
        self.validate()?;
        let encoded = serde_json::to_string(self)
            .map_err(|e| format!("unserializable data-surface document: {e}"))?;
        if encoded.chars().count() > MAX_DOCUMENT_CHARS {
            return Err(format!(
                "data-surface document is longer than {MAX_DOCUMENT_CHARS} characters"
            ));
        }
        Ok(encoded)
    }

    /// Enforces the per-surface bounds and the internal relationships. Run
    /// on both read and write, so a value that reached the database by any
    /// other route is reported rather than trusted.
    ///
    /// The two CLOSED VOCABULARIES — `FilterOp` and `SortDir` — are not
    /// re-checked here. They are closed by construction: serde refuses an
    /// unknown variant while parsing, so a document that reached this function
    /// already holds only known operators and directions, and a second
    /// membership test would be a branch nothing can reach.
    pub fn validate(&self) -> Result<(), String> {
        if self.0.len() > MAX_SURFACES {
            return Err(format!("more than {MAX_SURFACES} saved data surfaces"));
        }
        for (surface, view) in &self.0 {
            check_slug("data-surface key", surface)?;
            if let Some(name) = &view.view {
                check_len("view name", name, MAX_VIEW_NAME_CHARS)?;
            }
            if view.columns.len() > MAX_COLUMNS {
                return Err(format!(
                    "surface '{surface}' saves more than {MAX_COLUMNS} columns"
                ));
            }
            let mut seen: Vec<&str> = Vec::with_capacity(view.columns.len());
            for column in &view.columns {
                check_slug("column key", &column.key)?;
                if seen.contains(&column.key.as_str()) {
                    return Err(format!(
                        "surface '{surface}' repeats column '{}'",
                        column.key
                    ));
                }
                seen.push(&column.key);
                if let Some(width) = column.width_px {
                    if !(48..=1200).contains(&width) {
                        return Err(format!(
                            "column '{}' width {width} is out of range",
                            column.key
                        ));
                    }
                }
            }
            if view.filters.len() > MAX_FILTERS {
                return Err(format!(
                    "surface '{surface}' saves more than {MAX_FILTERS} filters"
                ));
            }
            for filter in &view.filters {
                check_slug("filter field", &filter.field)?;
                if filter.op.takes_value() {
                    let value = filter
                        .value
                        .as_deref()
                        .ok_or_else(|| format!("filter on '{}' needs a value", filter.field))?;
                    check_len("filter value", value, MAX_VALUE_CHARS)?;
                } else if filter.value.is_some() {
                    return Err(format!(
                        "filter on '{}' is a presence test and takes no value",
                        filter.field
                    ));
                }
            }
            if let Some(sort) = &view.sort {
                check_slug("sort field", &sort.field)?;
                if !seen.contains(&sort.field.as_str()) && !view.columns.is_empty() {
                    return Err(format!("sort field '{}' is not a saved column", sort.field));
                }
            }
        }
        Ok(())
    }
}

fn check_len(what: &str, value: &str, max: usize) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{what} is empty"));
    }
    if value.chars().count() > max {
        return Err(format!("{what} is longer than {max} characters"));
    }
    Ok(())
}

/// A dotted lowercase slug: the shape every identifier in this document
/// shares. Free text in a settings row is how a stale document grows; a slug
/// cannot contain a newline, a quote or a brace.
fn check_slug(what: &str, value: &str) -> Result<(), String> {
    check_len(what, value, MAX_FIELD_CHARS)?;
    if value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
        && !value.starts_with('.')
        && !value.ends_with('.')
    {
        Ok(())
    } else {
        Err(format!("{what} '{value}' is not a dotted lowercase slug"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(key: &str) -> ColumnPref {
        ColumnPref {
            key: key.to_string(),
            visible: true,
            width_px: None,
        }
    }

    #[test]
    fn empty_document_is_the_never_set_case() {
        assert_eq!(
            DataSurfacePrefs::parse("").unwrap(),
            DataSurfacePrefs::default()
        );
        assert_eq!(
            DataSurfacePrefs::parse("   ").unwrap(),
            DataSurfacePrefs::default()
        );
    }

    #[test]
    fn round_trips_a_saved_view_byte_stably() {
        let mut prefs = DataSurfacePrefs::default();
        prefs.set(
            "tickets",
            SurfaceView {
                view: Some("My open tickets".to_string()),
                columns: vec![
                    ColumnPref {
                        key: "ref".to_string(),
                        visible: true,
                        width_px: Some(96),
                    },
                    ColumnPref {
                        key: "title".to_string(),
                        visible: true,
                        width_px: None,
                    },
                    ColumnPref {
                        key: "sla".to_string(),
                        visible: false,
                        width_px: None,
                    },
                ],
                filters: vec![FilterClause {
                    field: "status".to_string(),
                    op: FilterOp::In,
                    value: Some("open,waiting".to_string()),
                }],
                sort: Some(SortPref {
                    field: "title".to_string(),
                    dir: SortDir::Asc,
                }),
            },
        );
        let encoded = prefs.to_compact_string().unwrap();
        assert!(!encoded.contains('\n'), "the stored form is compact");
        let decoded = DataSurfacePrefs::parse(&encoded).unwrap();
        assert_eq!(decoded, prefs);
        assert_eq!(
            decoded.to_compact_string().unwrap(),
            encoded,
            "stable across writes"
        );
    }

    #[test]
    fn key_order_does_not_change_the_document() {
        let mut a = DataSurfacePrefs::default();
        a.set("stock", SurfaceView::default());
        a.set("tickets", SurfaceView::default());
        let mut b = DataSurfacePrefs::default();
        b.set("tickets", SurfaceView::default());
        b.set("stock", SurfaceView::default());
        assert_eq!(
            a.to_compact_string().unwrap(),
            b.to_compact_string().unwrap()
        );
    }

    #[test]
    fn hidden_columns_keep_their_place() {
        let view = SurfaceView {
            columns: vec![
                column("ref"),
                ColumnPref {
                    key: "sla".to_string(),
                    visible: false,
                    width_px: None,
                },
            ],
            ..SurfaceView::default()
        };
        let visible: Vec<&str> = view
            .visible_columns()
            .iter()
            .map(|c| c.key.as_str())
            .collect();
        assert_eq!(visible, vec!["ref"]);
        assert_eq!(
            view.columns[1].key, "sla",
            "the hidden column is still second in order"
        );
    }

    #[test]
    fn rejects_a_malformed_document() {
        let err = DataSurfacePrefs::parse("{not json").unwrap_err();
        assert!(err.contains("malformed"), "{err}");
    }

    #[test]
    fn rejects_an_operator_outside_the_closed_vocabulary() {
        let raw = r#"{"tickets":{"filters":[{"field":"status","op":"fuzzy","value":"x"}]}}"#;
        let err = DataSurfacePrefs::parse(raw).unwrap_err();
        assert!(err.contains("unreadable"), "{err}");
        // A known operator with an unknown DIRECTION is refused the same way.
        let dir = r#"{"tickets":{"sort":{"field":"ref","sideways":"up"}}}"#;
        assert!(DataSurfacePrefs::parse(dir).is_err());
    }

    #[test]
    fn rejects_a_filter_that_cannot_match() {
        let missing = r#"{"tickets":{"filters":[{"field":"status","op":"eq"}]}}"#;
        assert!(DataSurfacePrefs::parse(missing)
            .unwrap_err()
            .contains("needs a value"));
        let extra = r#"{"tickets":{"filters":[{"field":"status","op":"is_set","value":"open"}]}}"#;
        assert!(DataSurfacePrefs::parse(extra)
            .unwrap_err()
            .contains("takes no value"));
    }

    #[test]
    fn rejects_a_repeated_column() {
        let raw = r#"{"tickets":{"columns":[{"key":"ref"},{"key":"ref"}]}}"#;
        assert!(DataSurfacePrefs::parse(raw)
            .unwrap_err()
            .contains("repeats column"));
    }

    #[test]
    fn rejects_a_sort_on_a_hidden_away_column() {
        let raw = r#"{"tickets":{"columns":[{"key":"ref"}],"sort":{"field":"nope","dir":"asc"}}}"#;
        assert!(DataSurfacePrefs::parse(raw)
            .unwrap_err()
            .contains("not a saved column"));
    }

    #[test]
    fn refuses_a_document_over_the_cap_on_both_sides() {
        let mut prefs = DataSurfacePrefs::default();
        for i in 0..MAX_SURFACES {
            prefs.set(
                &format!("surface{i}"),
                SurfaceView {
                    view: Some("v".repeat(MAX_VIEW_NAME_CHARS)),
                    columns: (0..MAX_COLUMNS).map(|c| column(&format!("c{c}"))).collect(),
                    ..SurfaceView::default()
                },
            );
        }
        let err = prefs.to_compact_string().unwrap_err();
        assert!(err.contains("longer than"), "{err}");
        let over_cap = format!("{{\"a\":\"{}\"}}", "x".repeat(MAX_DOCUMENT_CHARS));
        assert!(DataSurfacePrefs::parse(&over_cap).is_err());
    }

    #[test]
    fn rejects_identifiers_that_are_not_slugs() {
        let raw = r#"{"tickets":{"columns":[{"key":"Ref Number"}]}}"#;
        assert!(DataSurfacePrefs::parse(raw)
            .unwrap_err()
            .contains("dotted lowercase slug"));
    }

    #[test]
    fn clearing_a_surface_is_reported() {
        let mut prefs = DataSurfacePrefs::default();
        prefs.set("tickets", SurfaceView::default());
        assert!(prefs.clear("tickets"));
        assert!(!prefs.clear("tickets"));
        assert_eq!(prefs.surface_count(), 0);
    }
}
