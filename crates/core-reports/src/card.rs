//! Card contract — the ONE vocabulary a report card and a dashboard card
//! share (Wave 46 lane 1, ruled 2026-10-27).
//!
//! Purpose: a card is a [`ReportSpec`] (what to read) plus this module's
//! [`CardSpec`] (how it is drawn: shape, size, window, refresh). Both
//! surfaces embed those two types, so a dashboard card and a report card
//! cannot drift into two ideas of what "a chart" or "an empty card" is.
//! Everything here is pure data and pure validation: no SQL, no execution,
//! no HTTP, no rendering.
//!
//! Three things this module decides, once:
//! * **shape** — [`CardShape`] is DERIVED from the spec's viz and dimensions
//!   ([`CardShape::of`]), never stored separately, so a spec cannot claim a
//!   shape its data cannot fill (a chart with no dimension is a table).
//! * **refusal** — [`materialise`] turns a run's rows into [`CardData`] and
//!   refuses (`ReportsError::EmptyCard`) rather than handing back an empty
//!   card. A chart with no rows and a table with no headers are failures; a
//!   stat with a numeric zero is data.
//! * **timing** — [`CardSpec::validate`] refuses a card whose window is
//!   wider than the data one refresh reads, and one that refreshes less
//!   often than the window it displays.
//!
//! Non-purpose: no storage (core-db owns rows), no placement (a dashboard's
//! grid position is the dashboard surface's concern in W46-2 — this module
//! owns the size vocabulary, not where the card sits), no rendering, no
//! scheduling (a card's refresh is declared here; the schedule surface
//! stores cadences).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ReportsError;
use crate::params::WINDOW_MAX_DAYS;
use crate::spec::{ReportSpec, VizHint};

/// Fastest a card may recompute, in minutes. Below this the card is a live
/// counter, not a report card, and every recompute re-scans a window.
pub const MIN_REFRESH_MINUTES: u32 = 5;
/// Slowest a card may recompute, in minutes (30 days). Past this the card
/// is a scheduled delivery wearing a card's clothes.
pub const MAX_REFRESH_MINUTES: u32 = 43_200;

/// What a card is drawn from, named so the catalogue and the refusal use
/// the same words the authoring model reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CardDatum {
    /// Grouped rows: one per dimension value, enough to plot.
    Rows,
    /// One scalar from the run's summary.
    Scalar,
    /// A projected column order, so the table has headers.
    Headers,
}

impl CardDatum {
    pub const fn slug(self) -> &'static str {
        match self {
            CardDatum::Rows => "rows",
            CardDatum::Scalar => "scalar",
            CardDatum::Headers => "headers",
        }
    }
}

/// The three shapes a card can take. Derived from the spec, never stored as
/// a separate field: see [`CardShape::of`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CardShape {
    Chart,
    Stat,
    Table,
}

impl CardShape {
    pub const ALL: [CardShape; 3] = [CardShape::Chart, CardShape::Stat, CardShape::Table];

    pub const fn slug(self) -> &'static str {
        match self {
            CardShape::Chart => "chart",
            CardShape::Stat => "stat",
            CardShape::Table => "table",
        }
    }

    /// Strict parser (the inverse of the serde encoding). Unknown slugs fail
    /// closed.
    pub fn parse(slug: &str) -> Option<CardShape> {
        Self::ALL.into_iter().find(|s| s.slug() == slug)
    }

    /// The single derivation both surfaces use.
    ///
    /// `table` is always a table; `stat` is always a stat (the registry
    /// requires exactly one measure and no dimensions for it, see
    /// [`ReportSpec::validate`]); a chart viz with no group-by dimension has
    /// nothing to put on an axis and is a table, which is what the builder
    /// preview already renders. Deriving instead of trusting a separate
    /// `shape` field is what stops a spec from claiming a chart it cannot
    /// fill.
    pub fn of(spec: &ReportSpec) -> CardShape {
        match spec.viz {
            VizHint::Table => CardShape::Table,
            VizHint::Stat => CardShape::Stat,
            VizHint::Bar | VizHint::Line | VizHint::Pie => {
                if spec.group_by.is_empty() {
                    CardShape::Table
                } else {
                    CardShape::Chart
                }
            }
        }
    }

    /// What this shape must be given to be drawable.
    pub const fn requires(self) -> &'static [CardDatum] {
        match self {
            CardShape::Chart => &[CardDatum::Rows],
            CardShape::Stat => &[CardDatum::Scalar],
            CardShape::Table => &[CardDatum::Headers, CardDatum::Rows],
        }
    }
}

/// Card width on the dashboard's twelve-column grid. The vocabulary lane 6
/// consumes; the CSS owns the measurement, this owns the names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CardSpan {
    /// One quarter of the grid.
    Quarter,
    /// One third (four columns).
    Third,
    /// Half (six columns): the default.
    Half,
    /// The whole grid.
    Full,
}

impl CardSpan {
    pub const ALL: [CardSpan; 4] = [
        CardSpan::Quarter,
        CardSpan::Third,
        CardSpan::Half,
        CardSpan::Full,
    ];

    pub const fn slug(self) -> &'static str {
        match self {
            CardSpan::Quarter => "quarter",
            CardSpan::Third => "third",
            CardSpan::Half => "half",
            CardSpan::Full => "full",
        }
    }

    /// Columns of the twelve-column grid this span occupies.
    pub const fn columns(self) -> u8 {
        match self {
            CardSpan::Quarter => 3,
            CardSpan::Third => 4,
            CardSpan::Half => 6,
            CardSpan::Full => 12,
        }
    }

    pub fn parse(slug: &str) -> Option<CardSpan> {
        Self::ALL.into_iter().find(|s| s.slug() == slug)
    }
}

/// Card height. Three named steps, not pixels: the grid owns the
/// measurement so a density change never becomes a data-contract change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CardHeight {
    /// A headline figure and its caption.
    Compact,
    /// The default: room for a chart and its legend.
    Regular,
    /// Room for a long table.
    Tall,
}

impl CardHeight {
    pub const ALL: [CardHeight; 3] = [CardHeight::Compact, CardHeight::Regular, CardHeight::Tall];

    pub const fn slug(self) -> &'static str {
        match self {
            CardHeight::Compact => "compact",
            CardHeight::Regular => "regular",
            CardHeight::Tall => "tall",
        }
    }

    pub fn parse(slug: &str) -> Option<CardHeight> {
        Self::ALL.into_iter().find(|h| h.slug() == slug)
    }
}

/// A card's footprint: the one size vocabulary both surfaces read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CardSize {
    #[serde(default = "CardSize::default_span")]
    pub span: CardSpan,
    #[serde(default = "CardSize::default_height")]
    pub height: CardHeight,
}

impl CardSize {
    fn default_span() -> CardSpan {
        CardSpan::Half
    }

    fn default_height() -> CardHeight {
        CardHeight::Regular
    }
}

impl Default for CardSize {
    fn default() -> Self {
        CardSize {
            span: CardSpan::Half,
            height: CardHeight::Regular,
        }
    }
}

/// How often a card recomputes, and how much data one recompute covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CardRefresh {
    /// Minutes between recomputes. Bounded by [`MIN_REFRESH_MINUTES`] and
    /// [`MAX_REFRESH_MINUTES`].
    pub every_minutes: u32,
    /// Days of data one recompute reads. `0` means "the card's whole
    /// window", which is the honest default and the only value most authors
    /// need to send: narrowing it is a deliberate act, and the card is
    /// refused when the window reaches past it.
    #[serde(default)]
    pub reach_days: u32,
}

impl Default for CardRefresh {
    fn default() -> Self {
        CardRefresh {
            every_minutes: 60,
            reach_days: 0,
        }
    }
}

/// Everything about a card that is not the query: its size, the window it
/// displays, and how it refreshes. Stored beside a [`ReportSpec`] by both
/// surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CardSpec {
    #[serde(default)]
    pub size: CardSize,
    /// Inclusive day span the card displays. Bounded by
    /// [`WINDOW_MAX_DAYS`], the same ceiling a report run accepts.
    pub window_days: u32,
    #[serde(default)]
    pub refresh: CardRefresh,
}

impl CardSpec {
    /// The days one refresh actually covers: the declared reach, or the
    /// whole window when the author left it at `0`.
    #[must_use]
    pub fn reach_days(&self) -> u32 {
        if self.refresh.reach_days == 0 {
            self.window_days
        } else {
            self.refresh.reach_days
        }
    }

    /// The window-versus-refresh gate, plus the bounds both numbers need.
    ///
    /// # Errors
    /// [`ReportsError::WindowWiderThanRefresh`] when the window reaches past
    /// what one refresh reads, [`ReportsError::RefreshSlowerThanWindow`] when
    /// the card recomputes less often than the span it displays, and
    /// [`ReportsError::InvalidCard`] for an out-of-range window or refresh.
    pub fn validate(&self) -> Result<(), ReportsError> {
        if self.window_days == 0 || i64::from(self.window_days) > WINDOW_MAX_DAYS {
            return Err(ReportsError::InvalidCard {
                field: "window_days".to_string(),
                reason: format!(
                    "window must be between 1 and {WINDOW_MAX_DAYS} days, got {}",
                    self.window_days
                ),
            });
        }
        if !(MIN_REFRESH_MINUTES..=MAX_REFRESH_MINUTES).contains(&self.refresh.every_minutes) {
            return Err(ReportsError::InvalidCard {
                field: "refresh.every_minutes".to_string(),
                reason: format!(
                    "refresh must be between {MIN_REFRESH_MINUTES} and \
                     {MAX_REFRESH_MINUTES} minutes, got {}",
                    self.refresh.every_minutes
                ),
            });
        }
        // The row's rule: a window wider than one refresh means the older
        // part of the card is never recomputed, so the number on screen and
        // the number the card claims to show are different numbers.
        if self.window_days > self.reach_days() {
            return Err(ReportsError::WindowWiderThanRefresh {
                window_days: self.window_days,
                reach_days: self.reach_days(),
            });
        }
        // Its complement: a card that refreshes less often than the span it
        // displays shows a window it never revisits.
        let window_minutes = self.window_days.saturating_mul(24 * 60);
        if self.refresh.every_minutes > window_minutes {
            return Err(ReportsError::RefreshSlowerThanWindow {
                every_minutes: self.refresh.every_minutes,
                window_days: self.window_days,
            });
        }
        Ok(())
    }
}

/// One scalar a stat card can carry. Typed so a caller branches on the
/// value class instead of inspecting JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scalar {
    Int(i64),
    Float(f64),
    Text(String),
    Bool(bool),
}

impl Scalar {
    /// Decodes one summary member. `null`, a missing key, and an object all
    /// yield `None` — a stat with no scalar is a refusal, not a zero.
    #[must_use]
    pub fn from_json(value: &Value) -> Option<Scalar> {
        match value {
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Some(Scalar::Int(i))
                } else {
                    n.as_f64().map(Scalar::Float)
                }
            }
            Value::String(s) => Some(Scalar::Text(s.clone())),
            Value::Bool(b) => Some(Scalar::Bool(*b)),
            _ => None,
        }
    }
}

/// A run's raw output, as the execution layer hands it over. Borrowed so a
/// caller can inspect a page without copying it. The `CardData` this
/// produces owns its rows, because a card outlives the run it came from.
#[derive(Debug, Clone, Copy)]
pub struct CardRun<'a> {
    /// Stable column order of the projection.
    pub columns: &'a [String],
    /// Grouped rows, already decoded to JSON.
    pub rows: &'a [Value],
    /// Whole-window aggregates (the run's summary object).
    pub summary: &'a Value,
}

/// A card that has passed its contract: exactly what a renderer draws.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "shape")]
pub enum CardData {
    Chart {
        /// The first group-by dimension, the axis.
        dimension: String,
        /// Measure output names, one series each.
        series: Vec<String>,
        rows: Vec<Value>,
    },
    Stat {
        /// The measure's output name, so the caption has a source.
        measure: String,
        value: Scalar,
    },
    Table {
        headers: Vec<String>,
        rows: Vec<Value>,
    },
}

impl CardData {
    pub const fn shape(&self) -> CardShape {
        match self {
            CardData::Chart { .. } => CardShape::Chart,
            CardData::Stat { .. } => CardShape::Stat,
            CardData::Table { .. } => CardShape::Table,
        }
    }
}

/// Why a card could not be drawn. Typed so a repair loop can name the
/// failure class instead of parsing prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case", tag = "failure")]
pub enum CardFailure {
    /// The card's filters resolved to zero rows at this window.
    NoRows,
    /// The table projected no columns, so it has no headers.
    NoHeaders,
    /// The stat's measure is missing from the summary or is null.
    NoScalar { column: String },
}

impl CardFailure {
    pub const fn slug(&self) -> &'static str {
        match self {
            CardFailure::NoRows => "no_rows",
            CardFailure::NoHeaders => "no_headers",
            CardFailure::NoScalar { .. } => "no_scalar",
        }
    }
}

/// Materialise a card from a finished run — the one path both surfaces and
/// the authoring agent's verifier call.
///
/// # Errors
/// [`ReportsError::EmptyCard`] when the resolved data cannot fill the shape
/// (no rows for a chart or table, no headers for a table, no scalar for a
/// stat), and anything [`CardSpec::validate`] refuses.
pub fn materialise(
    spec: &ReportSpec,
    card: &CardSpec,
    run: &CardRun<'_>,
) -> Result<CardData, ReportsError> {
    card.validate()?;
    let empty = |failure: CardFailure| ReportsError::EmptyCard {
        shape: CardShape::of(spec),
        failure,
    };
    match CardShape::of(spec) {
        CardShape::Chart => {
            if run.rows.is_empty() {
                return Err(empty(CardFailure::NoRows));
            }
            let dimension =
                spec.group_by
                    .first()
                    .cloned()
                    .ok_or_else(|| ReportsError::InvalidCard {
                        field: "group_by".to_string(),
                        reason: "a chart card needs a dimension".to_string(),
                    })?;
            Ok(CardData::Chart {
                dimension,
                series: spec.measures.iter().map(|m| m.output_name()).collect(),
                rows: run.rows.to_vec(),
            })
        }
        CardShape::Table => {
            if run.columns.is_empty() {
                return Err(empty(CardFailure::NoHeaders));
            }
            if run.rows.is_empty() {
                return Err(empty(CardFailure::NoRows));
            }
            Ok(CardData::Table {
                headers: run.columns.to_vec(),
                rows: run.rows.to_vec(),
            })
        }
        CardShape::Stat => {
            // The registry admits exactly one measure for a stat, so the
            // first one is the scalar's source.
            let measure = spec
                .measures
                .first()
                .map(|m| m.output_name())
                .ok_or_else(|| ReportsError::InvalidCard {
                    field: "measures".to_string(),
                    reason: "a stat card needs exactly one measure".to_string(),
                })?;
            let value = run
                .summary
                .get(&measure)
                .and_then(Scalar::from_json)
                .ok_or_else(|| {
                    empty(CardFailure::NoScalar {
                        column: measure.clone(),
                    })
                })?;
            Ok(CardData::Stat { measure, value })
        }
    }
}

/// One catalogue entry: what the authoring model reads before it picks a
/// shape. Mirrors the workflow authoring catalogue's `agent_description`
/// pattern (Wave 43 lane 17) so a model learned against one reads the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShapeInfo {
    pub shape: CardShape,
    /// The viz hints that produce this shape in a spec.
    pub viz: &'static [VizHint],
    /// What the shape must be given (the same list the refusal enforces).
    pub data: &'static [CardDatum],
    pub default_size: CardSize,
    /// Prose the authoring model reads. Written plainly on purpose: a model
    /// that misreads this authors into the refusal below.
    pub agent_description: &'static str,
    /// The refusal in plain English, so the model can choose away from it.
    pub refuses_when: &'static str,
}

const COMPACT: CardSize = CardSize {
    span: CardSpan::Third,
    height: CardHeight::Compact,
};
const REGULAR: CardSize = CardSize {
    span: CardSpan::Half,
    height: CardHeight::Regular,
};
const TALL: CardSize = CardSize {
    span: CardSpan::Full,
    height: CardHeight::Tall,
};

/// The shape catalogue, in the order the picker lists it.
pub const SHAPES: [ShapeInfo; 3] = [
    ShapeInfo {
        shape: CardShape::Chart,
        viz: &[VizHint::Bar, VizHint::Line, VizHint::Pie],
        data: CardShape::Chart.requires(),
        default_size: REGULAR,
        agent_description: "A chart plots one measure against one dimension. \
                            Send at least one group-by column (the axis) and one \
                            measure (the bars, line or slices). Bar suits counts \
                            per category, line suits a trend over time, pie suits \
                            a handful of whole-number shares and is refused as \
                            hard to read past about six slices.",
        refuses_when: "no rows resolved for the card's filters at its window; \
                       a chart with nothing to plot is a refusal, not an empty frame",
    },
    ShapeInfo {
        shape: CardShape::Stat,
        viz: &[VizHint::Stat],
        data: CardShape::Stat.requires(),
        default_size: COMPACT,
        agent_description: "A stat shows one number it can call a headline: a count, \
                            a sum, an average, the oldest open ticket, the median \
                            first response. Send exactly one measure and no \
                            group-by column. A zero is a legitimate value and the \
                            card renders it; a null or missing number is not.",
        refuses_when: "the measure has no scalar in the run summary, because a \
                       headline with no number is a blank the reader will blame on \
                       the data",
    },
    ShapeInfo {
        shape: CardShape::Table,
        viz: &[VizHint::Table],
        data: CardShape::Table.requires(),
        default_size: TALL,
        agent_description: "A table lists rows under the columns it projects. \
                            Use it when the reader needs each row (the breaches, \
                            the individual tickets) rather than a shape. A chart \
                            viz with no group-by column also becomes a table, \
                            because there is no axis to plot against.",
        refuses_when: "no rows at the window, or a projection with no columns to \
                       use as headers",
    },
];

/// The catalogue, as a slice.
#[must_use]
pub fn shape_catalogue() -> &'static [ShapeInfo] {
    &SHAPES
}

/// The card rules in the words the authoring model reads. Served with the
/// catalogue so the rules and the shapes travel together.
pub const RULES: [&str; 3] = [
    "A card must draw something. A chart or table whose filters resolve to zero \
     rows at the card's window is refused, with the shape and the failure named. \
     Choose a wider window, loosen a filter, or pick a stat whose number is a \
     legitimate zero.",
    "The window must fit inside what one refresh reads. A card that displays thirty \
     days but recomputes seven is refused, because the older twenty-three days are \
     never recomputed.",
    "A card must refresh at least as often as the window it displays. A card showing \
     a day and recomputing weekly is refused, because it shows a window it never \
     revisits.",
];

/// The rules, as a slice.
#[must_use]
pub fn rules() -> &'static [&'static str] {
    &RULES
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ViewId;
    use crate::spec::{Aggregate, MeasureSpec};
    use serde_json::json;

    fn spec(viz: VizHint, group_by: &[&str], measures: Vec<MeasureSpec>) -> ReportSpec {
        ReportSpec {
            view: ViewId::Tickets,
            filters: vec![],
            group_by: group_by.iter().map(|s| (*s).to_string()).collect(),
            measures,
            viz,
        }
    }

    fn count() -> MeasureSpec {
        MeasureSpec {
            agg: Aggregate::Count,
            column: None,
        }
    }

    fn hours() -> MeasureSpec {
        MeasureSpec {
            agg: Aggregate::Sum,
            column: Some("minutes".to_string()),
        }
    }

    fn card(window_days: u32) -> CardSpec {
        CardSpec {
            size: CardSize::default(),
            window_days,
            refresh: CardRefresh::default(),
        }
    }

    fn columns(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }

    // --- derivation ---------------------------------------------------------

    #[test]
    fn shape_follows_viz_and_dimensions() {
        let grouped = spec(VizHint::Bar, &["priority"], vec![count()]);
        assert_eq!(CardShape::of(&grouped), CardShape::Chart);

        // A chart viz with no dimension has no axis: it is a table.
        let axisless = spec(VizHint::Bar, &[], vec![count()]);
        assert_eq!(CardShape::of(&axisless), CardShape::Table);

        let table = spec(VizHint::Table, &["priority"], vec![count()]);
        assert_eq!(CardShape::of(&table), CardShape::Table);

        let stat = spec(VizHint::Stat, &[], vec![count()]);
        assert_eq!(CardShape::of(&stat), CardShape::Stat);

        for viz in [VizHint::Line, VizHint::Pie] {
            let s = spec(viz, &["priority"], vec![count()]);
            assert_eq!(CardShape::of(&s), CardShape::Chart, "{viz:?}");
        }
    }

    #[test]
    fn every_viz_maps_onto_exactly_one_shape() {
        for viz in VizHint::ALL {
            let grouped = spec(viz, &["priority"], vec![count()]);
            let ungrouped = spec(viz, &[], vec![count()]);
            let shape = CardShape::of(&grouped);
            assert!(CardShape::ALL.contains(&shape));
            // Only a chart depends on the dimensions; the rest are stable.
            if shape != CardShape::Chart {
                assert_eq!(shape, CardShape::of(&ungrouped), "{viz:?}");
            }
        }
    }

    #[test]
    fn a_stat_spec_must_carry_one_measure_and_no_dimensions() {
        let grouped = spec(VizHint::Stat, &["priority"], vec![count()]);
        assert!(matches!(
            grouped.validate(),
            Err(ReportsError::InvalidSpec { ref field, .. }) if field == "viz"
        ));

        let two = spec(VizHint::Stat, &[], vec![count(), hours()]);
        assert!(two.validate().is_err());

        let one = spec(VizHint::Stat, &[], vec![count()]);
        assert!(one.validate().is_ok());
    }

    // --- refusal ------------------------------------------------------------

    #[test]
    fn a_chart_with_no_rows_is_refused_by_name() {
        let s = spec(VizHint::Bar, &["priority"], vec![count()]);
        let summary = json!({ "count": 0 });
        let err = materialise(
            &s,
            &card(30),
            &CardRun {
                columns: &columns(&["priority", "count"]),
                rows: &[],
                summary: &summary,
            },
        )
        .unwrap_err();
        let rendered = err.to_string();
        match err {
            ReportsError::EmptyCard { shape, failure } => {
                assert_eq!(shape, CardShape::Chart);
                assert_eq!(failure, CardFailure::NoRows);
                assert_eq!(failure.slug(), "no_rows");
            }
            other => panic!("unexpected refusal {other:?}"),
        }
        assert!(rendered.contains("no rows"));
    }

    #[test]
    fn a_table_with_no_rows_or_no_headers_is_refused() {
        let s = spec(VizHint::Table, &["priority"], vec![count()]);
        let summary = json!({ "count": 0 });
        let rows = vec![json!({ "priority": "p1", "count": 2 })];
        let no_headers = materialise(
            &s,
            &card(30),
            &CardRun {
                columns: &[],
                rows: &rows,
                summary: &summary,
            },
        )
        .unwrap_err();
        assert!(matches!(
            no_headers,
            ReportsError::EmptyCard {
                failure: CardFailure::NoHeaders,
                ..
            }
        ));
        let no_rows = materialise(
            &s,
            &card(30),
            &CardRun {
                columns: &columns(&["priority", "count"]),
                rows: &[],
                summary: &summary,
            },
        )
        .unwrap_err();
        assert!(matches!(
            no_rows,
            ReportsError::EmptyCard {
                failure: CardFailure::NoRows,
                ..
            }
        ));
    }

    #[test]
    fn a_stat_takes_a_scalar_and_accepts_zero() {
        let s = spec(VizHint::Stat, &[], vec![count()]);
        let zero = json!({ "count": 0 });
        let data = materialise(
            &s,
            &card(30),
            &CardRun {
                columns: &columns(&["count"]),
                rows: &[],
                summary: &zero,
            },
        )
        .expect("zero is data");
        assert_eq!(
            data,
            CardData::Stat {
                measure: "count".to_string(),
                value: Scalar::Int(0),
            }
        );

        for missing in [json!({}), json!({ "count": null })] {
            let err = materialise(
                &s,
                &card(30),
                &CardRun {
                    columns: &columns(&["count"]),
                    rows: &[],
                    summary: &missing,
                },
            )
            .unwrap_err();
            assert!(
                matches!(
                    err,
                    ReportsError::EmptyCard {
                        failure: CardFailure::NoScalar { .. },
                        ..
                    }
                ),
                "{missing} produced {err:?}"
            );
        }
    }

    #[test]
    fn a_chart_carries_its_dimension_and_series() {
        let s = spec(VizHint::Line, &["priority"], vec![count(), hours()]);
        let summary = json!({ "count": 3 });
        let rows = vec![json!({ "priority": "p1", "count": 2, "sum_minutes": 90 })];
        let data = materialise(
            &s,
            &card(30),
            &CardRun {
                columns: &columns(&["priority", "count", "sum_minutes"]),
                rows: &rows,
                summary: &summary,
            },
        )
        .expect("chart");
        assert_eq!(
            data,
            CardData::Chart {
                dimension: "priority".to_string(),
                series: vec!["count".to_string(), "sum_minutes".to_string()],
                rows,
            }
        );
        assert_eq!(data.shape(), CardShape::Chart);
    }

    // --- window versus refresh ---------------------------------------------

    #[test]
    fn reach_defaults_to_the_whole_window() {
        let c = card(30);
        assert_eq!(c.reach_days(), 30);
        assert!(c.validate().is_ok());
    }

    #[test]
    fn a_window_wider_than_one_refresh_is_refused() {
        let mut c = card(30);
        c.refresh.reach_days = 7;
        let err = c.validate().unwrap_err();
        assert_eq!(
            err,
            ReportsError::WindowWiderThanRefresh {
                window_days: 30,
                reach_days: 7,
            }
        );
        assert!(err.to_string().contains("30"));
        assert!(err.to_string().contains("7"));

        // Reaching exactly as far as the window is fine.
        c.refresh.reach_days = 30;
        assert!(c.validate().is_ok());
        // Reaching further than the window reads extra data, which is
        // wasteful but not a lie about what is on screen.
        c.refresh.reach_days = 45;
        assert!(c.validate().is_ok());
    }

    #[test]
    fn a_card_may_not_refresh_less_often_than_its_window() {
        let mut c = card(1);
        c.refresh.every_minutes = MAX_REFRESH_MINUTES;
        assert_eq!(
            c.validate().unwrap_err(),
            ReportsError::RefreshSlowerThanWindow {
                every_minutes: MAX_REFRESH_MINUTES,
                window_days: 1,
            }
        );
        // Daily refresh of a one-day window is the boundary case and passes.
        c.refresh.every_minutes = 1_440;
        assert!(c.validate().is_ok());
    }

    #[test]
    fn window_and_refresh_bounds_fail_closed() {
        assert!(matches!(
            card(0).validate(),
            Err(ReportsError::InvalidCard { ref field, .. }) if field == "window_days"
        ));
        let too_wide = u32::try_from(WINDOW_MAX_DAYS).unwrap() + 1;
        assert!(card(too_wide).validate().is_err());

        for every in [0, 1, MIN_REFRESH_MINUTES - 1, MAX_REFRESH_MINUTES + 1] {
            let mut c = card(30);
            c.refresh.every_minutes = every;
            assert!(
                matches!(
                    c.validate(),
                    Err(ReportsError::InvalidCard { ref field, .. })
                        if field == "refresh.every_minutes"
                ),
                "{every} minutes was accepted"
            );
        }
    }

    // --- wire shape ---------------------------------------------------------

    #[test]
    fn card_specs_round_trip_through_json() {
        let c = CardSpec {
            size: CardSize {
                span: CardSpan::Full,
                height: CardHeight::Tall,
            },
            window_days: 90,
            refresh: CardRefresh {
                every_minutes: 720,
                reach_days: 90,
            },
        };
        let encoded = serde_json::to_value(c).expect("encode");
        assert_eq!(encoded["size"]["span"], json!("full"));
        assert_eq!(encoded["size"]["height"], json!("tall"));
        assert_eq!(encoded["refresh"]["every_minutes"], json!(720));
        let back: CardSpec = serde_json::from_value(encoded).expect("decode");
        assert_eq!(back, c);
    }

    #[test]
    fn omitted_members_take_the_documented_defaults() {
        let c: CardSpec = serde_json::from_value(json!({ "window_days": 30 })).expect("decode");
        assert_eq!(c.size, CardSize::default());
        assert_eq!(c.refresh, CardRefresh::default());
        assert_eq!(c.size.span, CardSpan::Half);
        assert_eq!(c.size.height, CardHeight::Regular);
        assert_eq!(c.refresh.every_minutes, 60);
        assert_eq!(c.refresh.reach_days, 0);
    }

    #[test]
    fn span_and_height_slugs_round_trip_strictly() {
        for span in CardSpan::ALL {
            assert_eq!(CardSpan::parse(span.slug()), Some(span));
        }
        for height in CardHeight::ALL {
            assert_eq!(CardHeight::parse(height.slug()), Some(height));
        }
        for shape in CardShape::ALL {
            assert_eq!(CardShape::parse(shape.slug()), Some(shape));
        }
        for bad in ["", "Half", "huge", "stat ", "12"] {
            assert_eq!(CardSpan::parse(bad), None);
            assert_eq!(CardHeight::parse(bad), None);
            assert_eq!(CardShape::parse(bad), None);
        }
        assert_eq!(CardSpan::Quarter.columns(), 3);
        assert_eq!(CardSpan::Full.columns(), 12);
    }

    // --- catalogue ----------------------------------------------------------

    #[test]
    fn the_catalogue_covers_every_shape_once() {
        let catalogue = shape_catalogue();
        assert_eq!(catalogue.len(), CardShape::ALL.len());
        for shape in CardShape::ALL {
            let entries: Vec<&ShapeInfo> = catalogue.iter().filter(|i| i.shape == shape).collect();
            assert_eq!(
                entries.len(),
                1,
                "{shape:?} is described {} times",
                entries.len()
            );
            let entry = entries[0];
            assert!(!entry.agent_description.is_empty());
            assert!(!entry.refuses_when.is_empty());
            assert_eq!(entry.data, shape.requires());
        }
    }

    #[test]
    fn every_viz_is_claimed_by_exactly_one_shape_entry() {
        for viz in VizHint::ALL {
            let owners: Vec<CardShape> = shape_catalogue()
                .iter()
                .filter(|i| i.viz.contains(&viz))
                .map(|i| i.shape)
                .collect();
            assert_eq!(owners.len(), 1, "{viz:?} is claimed by {owners:?}");
        }
    }

    #[test]
    fn the_rules_describe_the_refusals_that_exist() {
        assert_eq!(rules().len(), RULES.len());
        for rule in rules() {
            assert!(rule.len() > 40, "rule is a stub: {rule}");
        }
        // Each rule names the check it stands for, so a reworded rule that
        // forgets the mechanism is caught here.
        let joined = rules().join(" ");
        assert!(joined.contains("zero"));
        assert!(joined.contains("refresh"));
    }
}
