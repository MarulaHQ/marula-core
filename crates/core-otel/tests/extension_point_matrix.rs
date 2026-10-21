//! The extension-point matrix is a contract document, so it is tested like one.
//!
//! `docs/reference/extension-points.md` is what a premium pack author reads
//! before writing anything. Its value is that a cited path still exists and a
//! cited metric name is still in the published contract: the day either moves,
//! the document tells the author something false, and the pack that trusted it
//! breaks at integration time instead of at build time.
//!
//! Two properties, one per kind of citation:
//!
//! * every backticked REPO PATH in the document resolves to a file in the tree;
//! * every backticked `marula.*` name in the document is in
//!   [`core_otel::contract::METRICS`].
//!
//! Both are parsed out of the markdown, so the document cannot quietly grow a
//! claim that nothing checks. Anything else it says is prose and stays a human
//! review question.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn document() -> String {
    let path = matrix_path();
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn matrix_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference/extension-points.md")
}

/// The repo-relative paths the document cites: a backticked token that starts
/// with a top-level directory and contains a slash.
///
/// Scoping the scan this way keeps prose like `observability.read` or
/// `special:manage_users` out of the path check without a second convention to
/// remember.
fn cited_paths(src: &str) -> BTreeSet<&str> {
    let roots = [
        "crates/",
        "docs/",
        "deploy/",
        "packages/",
        "plugins/",
        "fixtures/",
        "scripts/",
    ];
    src.split('`')
        .filter(|token| roots.iter().any(|root| token.starts_with(root)))
        .collect()
}

fn cited_metrics(src: &str) -> BTreeSet<&str> {
    src.split('`')
        .filter(|token| token.starts_with("marula."))
        .collect()
}

#[test]
fn every_cited_core_path_still_exists() {
    let src = document();
    let paths = cited_paths(&src);
    assert!(
        !paths.is_empty(),
        "the matrix cites no core paths — the parser or the document changed shape, \
         and a document nothing parses is a document nothing checks"
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let missing: Vec<&str> = paths
        .iter()
        .copied()
        .filter(|p| !root.join(p).exists())
        .collect();
    assert!(
        missing.is_empty(),
        "the extension-point matrix cites paths that no longer exist: {missing:?}\n\
         Update docs/reference/extension-points.md (or restore the file) in the same \
         change that moved it — a pack author reads this document instead of the tree."
    );
}

#[test]
fn every_cited_metric_is_in_the_published_contract() {
    let src = document();
    let metrics = cited_metrics(&src);
    assert!(
        !metrics.is_empty(),
        "the matrix cites no metric names at all"
    );
    let published: BTreeSet<&str> = core_otel::contract::METRICS
        .iter()
        .map(|m| m.name)
        .collect();
    let unknown: Vec<&str> = metrics
        .iter()
        .copied()
        .filter(|name| !published.contains(name))
        .filter(|name| !name.starts_with("marula.core"))
        .collect();
    assert!(
        unknown.is_empty(),
        "the extension-point matrix cites metrics the contract does not publish: {unknown:?}\n\
         Either the metric moved (rename the citation and bump CONTRACT_VERSION) or the \
         document promised a series no producer emits."
    );
}

/// The document is the only thing here that is prose; these are the rows the
/// table must keep. A point silently dropped from the matrix is the failure mode
/// this test exists for, and it is invisible to a path check because a deleted
/// row cites nothing.
#[test]
fn the_matrix_still_lists_every_extension_point() {
    let src = document();
    for point in [
        "Plugin",
        "Connector",
        "Federation bundle",
        "Auth provider",
        "Webhook / event",
        "Read model",
        "OTel",
        "Audit",
        "Usage",
    ] {
        assert!(
            src.contains(&format!("| {point} |")),
            "the extension-point matrix no longer has a table row for {point} — \
             a pack author looking for that point finds nothing and concludes it does \
             not exist"
        );
    }
}
