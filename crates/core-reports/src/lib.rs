//! core-reports — reporting engine types, field catalog, and query compiler
//! for the versioned `ops.v1_*` read-model layer.
//!
//! Purpose (the private build tracker R0/R1; ADR 0001): a THIN, typed compiler that turns
//! prebuilt report kinds into parameterized SQL over the read-model views,
//! plus the fixed field catalog every identifier flows from. This crate is
//! the security boundary of the reporting track:
//! * identifiers are static catalog constants (`catalog`),
//! * user values bind exclusively as `$N` parameters (`builder::Bind`),
//! * validation is pure and property-tested (`params`, `kinds`).
//! * every report CSV cell is encoded by one writer (`csv`), so the
//!   interactive and scheduled lanes cannot disagree about which values a
//!   spreadsheet would evaluate.
//!
//! Non-purpose (binding):
//! * **Never reads physical tables.** Every statement this crate emits
//!   references ONLY `ops.v1_*` views (migration 0026) — reports must
//!   remain upgrade-stable through view evolution per upgrade-compatibility
//!   policy §5. There is no code path to ticket/time/SLA base tables.
//! * No SQL execution lives here (core-api owns execution against its
//!   pool); storage for stored definitions is core-db `report_definitions`
//!   (R3); no scheduling/worker concerns (R4); no HTTP shapes (core-api
//!   `routes::reports` for the prebuilt kinds, `routes::report_defs` for
//!   stored definitions).

pub mod builder;
pub mod catalog;
pub mod csv;
pub mod error;
pub mod kinds;
pub mod params;
pub mod spec;

pub use builder::{compile, next_cursor, Bind, CompiledReport, CursorShape, OutType, OutputColumn};
pub use catalog::{lookup_column, resolve_filter_column, ViewId};
pub use error::ReportsError;
pub use kinds::{
    AgingParams, FirstResponseParams, KindParams, ReportKind, SlaOutcomesParams, ThroughputParams,
    TimeByUserParams, TimeReportParams, WindowParams,
};
pub use params::{
    clamp_limit, resolve_window, validate_limit_raw, Scope, Window, PAGE_LIMIT_DEFAULT,
    PAGE_LIMIT_MAX, WINDOW_MAX_DAYS,
};
pub use spec::{
    Aggregate, FilterValue, MeasureSpec, ReportSpec, RunWindow, SpecBody, SpecFilter, VizHint,
    MAX_FILTERS, MAX_GROUP_BY, MAX_MEASURES,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Injection-attempt strings must fail closed at EVERY public lookup
    /// surface: column resolution, filter resolution, kind parsing, cursor
    /// decoding. The payloads are realistic smuggling attempts; structural
    /// rejection is the contract (no code path turns text into SQL text).
    mod injection_rejection {
        use super::*;

        const PAYLOADS: &[&str] = &[
            "' OR '1'='1",
            "1; DROP TABLE tickets; --",
            "user_id IS NULL OR 1=1 --",
            "started_at > now() - interval '10 years'",
            "te.user_id = gen_random_uuid() UNION SELECT 1",
            "\"group_id\"",
            "$$ dollar-quoted $$",
            "${jndi}",
            "/* stealth */ minutes",
        ];

        #[test]
        fn unknown_columns_never_resolve_on_any_view() {
            for payload in PAYLOADS {
                for view in ViewId::ALL {
                    assert!(
                        lookup_column(view, payload).is_none(),
                        "{payload:?} resolved on {view:?}"
                    );
                    let err = resolve_filter_column(view, payload).unwrap_err();
                    assert!(matches!(err, ReportsError::UnknownColumn { .. }));
                }
            }
        }

        #[test]
        fn adversarial_kind_slugs_fail_closed() {
            for payload in PAYLOADS {
                assert_eq!(ReportKind::parse(payload), None);
            }
        }

        #[test]
        fn adversarial_cursors_fail_closed() -> Result<(), ReportsError> {
            for payload in PAYLOADS {
                // Garbage payloads must not roundtrip through a seal/open
                // pair: any error class other than InvalidCursor here would
                // indicate parsing progress on hostile input.
                match crate::builder::tests_support::open_for_test(payload) {
                    Err(ReportsError::InvalidCursor) => {}
                    Ok(_) => panic!("cursor accepted {payload:?}"),
                    Err(other) => panic!("unexpected error class {other:?}"),
                }
            }
            Ok(())
        }

        /// Windows are typed `NaiveDate` before they reach the compiler —
        /// free text cannot become SQL at all. Assert the widest legal span
        /// compiles to a sane half-open instant range.
        #[test]
        fn typed_windows_bound_the_query_surface() {
            let anchor = chrono::DateTime::from_timestamp(1_780_000_000, 0).unwrap();
            let from = chrono::NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
            let w = resolve_window(
                Some(from),
                Some(from + chrono::Duration::days(WINDOW_MAX_DAYS - 1)),
                anchor,
            )
            .expect("max window representable");
            assert_eq!(w.span_days(), WINDOW_MAX_DAYS);
        }
    }
}
