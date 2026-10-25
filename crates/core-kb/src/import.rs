//! D5 KB import (the private build tracker Phase 3, import/export/documents track): turn
//! Markdown/HTML knowledge exports into import-engine kb rows.
//!
//! Purpose: a PURE converter — [`ingest_bundle`] flattens a Markdown or
//! HTML document into exactly the canonical rows the import engine's
//! `kb_category` / `kb_article` families accept (scalar string fields by
//! design, so `format=json` ingestion needs no mapping template). H1
//! headings introduce categories; H2 headings introduce articles under
//! the current category; H3+ headings stay inline content. Articles
//! without an enclosing H1 hoist to top level (no category). HTML is
//! flattened to Markdown-shaped text first (h1→`#`, h2→`##`, block tags
//! to line breaks, other tags stripped, common entities unescaped) so one
//! heading scan serves both. YAML front matter (opening `---` fence)
//! overrides `title`, `slug`, `status`, `client_visible`, and
//! `external_key` for the article family.
//!
//! Non-purpose: no sanitization (bodies are stored as-authored and render
//! under the product's existing content policy — the same trust level as
//! a 2.8 structured note, never an arbitrary-iframe surface), no
//! attachments/images, no SQL/HTTP (rows reach storage only through the
//! import engine's dry-run/batch surface), no semantic cleanup — output
//! is exactly what was parsed, and a bundle with nothing recognizable
//! fails loudly instead of inventing rows.

use std::collections::BTreeMap;

/// Source content kind of a bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentFormat {
    Markdown,
    Html,
}

/// One flat kb row ready for the import engine. Field names map 1:1 onto
/// the `kb_category` / `kb_article` canonical catalogs
/// (`core_domain::import_core`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KbImportRow {
    Category(KbCategoryImportRow),
    Article(KbArticleImportRow),
}

/// A flattened category row (all scalars as strings — the flat-bag shape
/// the engine parses from CSV/JSON alike).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KbCategoryImportRow {
    /// The category slug itself. Deterministic, so re-ingestion is
    /// idempotent against the external_ids map, AND it is the exact key
    /// articles' `category_slug` resolves against (the engine resolves
    /// parent keys through the parent family's external keys).
    pub external_key: String,
    pub name: String,
    pub slug: String,
    /// Interstitial prose between the H1 and the first H2 (may be empty).
    pub description: String,
    /// Ingestion order, so the imported tree lists as the source did.
    pub position: String,
    pub client_visible: String,
}

/// A flattened article row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KbArticleImportRow {
    /// `article:<slug>`, or the front-matter `external_key` override when
    /// the bundle declares one (multi-article bundles sharing an override
    /// collide by external key — the engine reports that loudly rather
    /// than guessing an attribution).
    pub external_key: String,
    /// Absent for top-level articles (no enclosing H1).
    pub category_slug: Option<String>,
    pub title: String,
    pub slug: String,
    pub body: String,
    /// Empty string → storage default (draft); `published` flows through
    /// the regular lifecycle so the FTS indexer picks the article up.
    pub status: String,
    pub client_visible: String,
}

/// Splits a leading YAML-ish front-matter fence into a flat key/value map.
/// Values are single-line scalars only; no nesting is invented. An
/// unclosed fence is plain body text (a horizontal rule), not an error.
/// Returns `(front_matter, body_without_fence)`.
pub fn split_front_matter(source: &str) -> (BTreeMap<String, String>, String) {
    let mut meta = BTreeMap::new();
    let stripped = source.strip_prefix('\u{feff}').unwrap_or(source);
    let trimmed = stripped.trim_start().to_string();
    if trimmed.starts_with("---") {
        let all: Vec<&str> = trimmed.lines().collect();
        if all.first().map(|l| l.trim()) == Some("---") {
            let mut closer: Option<usize> = None;
            for (i, line) in all.iter().enumerate().skip(1) {
                if line.trim() == "---" {
                    closer = Some(i);
                    break;
                }
            }
            if let Some(closer) = closer {
                for line in &all[1..closer] {
                    if let Some((key, value)) = line.split_once(':') {
                        let key = key.trim().to_ascii_lowercase();
                        let value = value.trim().to_string();
                        if !key.is_empty() {
                            meta.insert(key, value);
                        }
                    }
                }
                let body = all[closer + 1..].join("\n");
                return (meta, body);
            }
        }
    }
    (meta, trimmed)
}

/// Flattens HTML to Markdown-shaped text preserving the heading structure:
/// `h1` → `# `, `h2` → `## `, `h3` → `### `, known block tags (`p`, `br`,
/// `div`, `li`, `tr`, …) to line breaks, every other tag stripped, and the
/// common XML entities unescaped. Non-heading text is best-effort body
/// material — this is an import filter, not a renderer.
pub fn html_to_markdown(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        match rest.find('<') {
            None => {
                out.push_str(rest);
                break;
            }
            Some(start) => {
                out.push_str(&rest[..start]);
                let after = &rest[start + 1..];
                let Some(rel_end) = after.find('>') else {
                    out.push_str(&rest[start..]);
                    break;
                };
                let tag = after[..rel_end].to_ascii_lowercase();
                rest = &after[rel_end + 1..];
                if tag.starts_with("h1") {
                    ensure_line_start(&mut out);
                    out.push_str("# ");
                } else if tag.starts_with("h2") {
                    ensure_line_start(&mut out);
                    out.push_str("## ");
                } else if tag.starts_with("h3") {
                    ensure_line_start(&mut out);
                    out.push_str("### ");
                } else if tag.starts_with('/') {
                    // Closing tag: close a heading line; other closers
                    // become plain separators.
                    if tag.starts_with("/h1") || tag.starts_with("/h2") || tag.starts_with("/h3") {
                        out.push('\n');
                    } else {
                        out.push(' ');
                    }
                } else if is_block_tag(&tag) {
                    out.push('\n');
                } else {
                    out.push(' ');
                }
            }
        }
    }
    unescape_entities(&out)
}

fn ensure_line_start(out: &mut String) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

fn is_block_tag(tag: &str) -> bool {
    let name: String = tag
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    matches!(
        name.as_str(),
        "p" | "br" | "div" | "li" | "tr" | "table" | "header" | "footer"
    )
}

fn unescape_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Flattens a Markdown or HTML bundle into import rows in source order:
/// categories appear at their H1, articles at their H2. The engine
/// enforces every field bound afterward. A bundle with no headings and no
/// front-matter `title` is a plain `Err` — never invented rows.
pub fn ingest_bundle(source: &str, format: ContentFormat) -> Result<Vec<KbImportRow>, String> {
    let (meta, rest) = split_front_matter(source);
    let markdown = match format {
        ContentFormat::Markdown => rest,
        ContentFormat::Html => html_to_markdown(&rest),
    };
    let rows = scan_markdown(&markdown, &meta);
    if rows.is_empty() {
        match meta.get("title") {
            Some(title) if !title.trim().is_empty() => {
                return Ok(vec![standalone_article(
                    title.trim(),
                    &markdown,
                    None,
                    &meta,
                )])
            }
            _ => {
                return Err(
                    "no H1/H2 headings found and no front-matter `title` — nothing to import"
                        .into(),
                )
            }
        }
    }
    Ok(rows)
}

fn standalone_article(
    title: &str,
    body: &str,
    category_slug: Option<String>,
    meta: &BTreeMap<String, String>,
) -> KbImportRow {
    let slug = article_slug(title, meta);
    KbImportRow::Article(KbArticleImportRow {
        external_key: article_external_key(&slug, meta),
        category_slug,
        title: title.to_string(),
        slug,
        body: body.trim().to_string(),
        status: article_status(meta),
        client_visible: article_client_visible(meta),
    })
}

/// Front-matter `slug` wins; otherwise the title is slugified.
fn article_slug(title: &str, meta: &BTreeMap<String, String>) -> String {
    meta.get("slug")
        .map(String::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| crate::slugify(title))
}

/// Front-matter `external_key` wins; otherwise derived from the slug.
/// A multi-article bundle sharing one override collides by external key —
/// the engine reports that loudly rather than guessing an attribution.
fn article_external_key(slug: &str, meta: &BTreeMap<String, String>) -> String {
    meta.get("external_key")
        .cloned()
        .unwrap_or_else(|| format!("article:{slug}"))
}

fn article_status(meta: &BTreeMap<String, String>) -> String {
    meta.get("status")
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default()
}

fn article_client_visible(meta: &BTreeMap<String, String>) -> String {
    meta.get("client_visible")
        .cloned()
        .unwrap_or_else(|| "false".into())
}

fn scan_markdown(markdown: &str, meta: &BTreeMap<String, String>) -> Vec<KbImportRow> {
    let mut rows = Vec::new();
    let mut position: i64 = 0;
    // The most recent H1's slug: every later H2 sits under it until the
    // next H1 (linear document structure).
    let mut current_category: Option<String> = None;
    // Open category: (its row under construction, description accumulator).
    // `Some` means not yet emitted; categories flush the moment their
    // description is final (first article under them, next H1, or EOF) so
    // the flattened order always lists the parent before its children.
    let mut open_cat: Option<(KbCategoryImportRow, Vec<String>)> = None;
    // Open article: (its row under construction, body accumulator).
    let mut open_art: Option<(KbArticleImportRow, Vec<String>)> = None;

    for line in markdown.lines() {
        let trimmed_end = line.trim_end();
        if let Some(rest) = trimmed_end.strip_prefix("# ") {
            // New category closes any open article and category.
            if let Some((mut a, body)) = open_art.take() {
                a.body = body.join("\n").trim().to_string();
                rows.push(KbImportRow::Article(a));
            }
            if let Some((mut c, desc)) = open_cat.take() {
                c.description = desc.join(" ").trim().to_string();
                rows.push(KbImportRow::Category(c));
            }
            let name = rest.trim();
            let slug = crate::slugify(name);
            position += 1;
            current_category = Some(slug.clone());
            open_cat = Some((
                KbCategoryImportRow {
                    // The slug doubles as the external key: articles
                    // reference parents by `category_slug`, and the engine
                    // resolves parent keys through external keys.
                    external_key: slug.clone(),
                    name: name.to_string(),
                    slug: slug.clone(),
                    description: String::new(),
                    position: position.to_string(),
                    client_visible: "false".into(),
                },
                Vec::new(),
            ));
        } else if let Some(rest) = trimmed_end.strip_prefix("## ") {
            if let Some((mut a, body)) = open_art.take() {
                a.body = body.join("\n").trim().to_string();
                rows.push(KbImportRow::Article(a));
            }
            // First article under the current category closes the
            // category's description — emit the parent before the child.
            if let Some((mut c, desc)) = open_cat.take() {
                c.description = desc.join(" ").trim().to_string();
                rows.push(KbImportRow::Category(c));
            }
            // Front-matter overrides apply per article; with several
            // articles in one bundle a shared `slug`/`external_key`
            // override collides and the engine reports it loudly.
            let title = meta
                .get("title")
                .map(String::as_str)
                .filter(|t| !t.trim().is_empty())
                .unwrap_or(rest.trim());
            let slug = article_slug(title, meta);
            open_art = Some((
                KbArticleImportRow {
                    external_key: article_external_key(&slug, meta),
                    category_slug: current_category.clone(),
                    title: title.to_string(),
                    slug,
                    body: String::new(),
                    status: article_status(meta),
                    client_visible: article_client_visible(meta),
                },
                Vec::new(),
            ));
        } else if trimmed_end.starts_with("### ") {
            // H3+ stays content, never structure.
            if let Some((_, body)) = open_art.as_mut() {
                body.push(trimmed_end.to_string());
            }
        } else if let Some((_, body)) = open_art.as_mut() {
            body.push(line.trim_end().to_string());
        } else if let Some((_, desc)) = open_cat.as_mut() {
            desc.push(line.trim().to_string());
        }
        // Lines before any heading are preamble and dropped (page chrome).
    }
    if let Some((mut a, body)) = open_art.take() {
        a.body = body.join("\n").trim().to_string();
        rows.push(KbImportRow::Article(a));
    }
    if let Some((mut c, desc)) = open_cat.take() {
        c.description = desc.join(" ").trim().to_string();
        rows.push(KbImportRow::Category(c));
    }
    rows
}

/// Serializes flattened rows to a curation CSV (a `kind` discriminator
/// column, then the union of both catalogs' columns) so operators can
/// eyeball/edit in a spreadsheet and push through `format=csv`. Body
/// newlines ride inside RFC 4180 quoting — the engine's parser is
/// quoted-cell-safe by construction.
///
/// Every cell goes through [`core_reports::csv::cell`], the same writer the
/// interactive and scheduled report lanes encode with: a title or body that
/// a spreadsheet would evaluate takes the text prefix before quoting, so the
/// export is a data delivery and not a code execution channel.
pub fn rows_to_csv(rows: &[KbImportRow]) -> String {
    let mut out = String::from(
        "kind,external_key,category_slug,name,title,slug,body,status,client_visible,description,position\n",
    );
    for row in rows {
        match row {
            KbImportRow::Category(c) => {
                out.push_str(&format!(
                    "kb_category,{},{},{},{},{},{},{},{},{},{}\n",
                    core_reports::csv::cell(&c.external_key),
                    core_reports::csv::cell(""),
                    core_reports::csv::cell(&c.name),
                    core_reports::csv::cell(""),
                    core_reports::csv::cell(&c.slug),
                    core_reports::csv::cell(""),
                    core_reports::csv::cell(""),
                    core_reports::csv::cell(&c.client_visible),
                    core_reports::csv::cell(&c.description),
                    core_reports::csv::cell(&c.position),
                ));
            }
            KbImportRow::Article(a) => {
                out.push_str(&format!(
                    "kb_article,{},{},{},{},{},{},{},{},{},{}\n",
                    core_reports::csv::cell(&a.external_key),
                    core_reports::csv::cell(a.category_slug.as_deref().unwrap_or("")),
                    core_reports::csv::cell(""),
                    core_reports::csv::cell(&a.title),
                    core_reports::csv::cell(&a.slug),
                    core_reports::csv::cell(&a.body),
                    core_reports::csv::cell(&a.status),
                    core_reports::csv::cell(&a.client_visible),
                    core_reports::csv::cell(""),
                    core_reports::csv::cell(""),
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_h1_h2_flattens_to_category_plus_articles() {
        let src = "# Getting started\n\nWelcome text.\n\n## Reset a password\n\nStep one.\n\n## VPN setup\n\nUse the client.\n";
        let rows = ingest_bundle(src, ContentFormat::Markdown).expect("parses");
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0],
            KbImportRow::Category(KbCategoryImportRow {
                external_key: "getting-started".into(),
                name: "Getting started".into(),
                slug: "getting-started".into(),
                description: "Welcome text.".into(),
                position: "1".into(),
                client_visible: "false".into(),
            })
        );
        let KbImportRow::Article(a) = &rows[1] else {
            panic!("expected article");
        };
        assert_eq!(a.title, "Reset a password");
        assert_eq!(a.slug, "reset-a-password");
        assert_eq!(a.category_slug.as_deref(), Some("getting-started"));
        assert_eq!(a.body, "Step one.");
        let KbImportRow::Article(b) = &rows[2] else {
            panic!("expected article");
        };
        assert_eq!(b.slug, "vpn-setup");
    }

    #[test]
    fn standalone_h2_hoists_without_category_and_h3_stays_body() {
        let src = "## Guide\n\nintro\n\n### subtopic\n\ndeep text\n";
        let rows = ingest_bundle(src, ContentFormat::Markdown).expect("parses");
        assert_eq!(rows.len(), 1);
        let KbImportRow::Article(a) = &rows[0] else {
            panic!("expected article");
        };
        assert!(a.category_slug.is_none());
        assert_eq!(a.slug, "guide");
        assert!(a.body.contains("### subtopic"));
        assert!(a.body.contains("deep text"));
    }

    #[test]
    fn front_matter_overrides_apply_to_articles() {
        let src = "---\ntitle: Imported page\nslug: imported\nstatus: published\nclient_visible: true\nexternal_key: wiki-42\n---\n\n## Section A\n\nbody a\n";
        let rows = ingest_bundle(src, ContentFormat::Markdown).expect("parses");
        let KbImportRow::Article(a) = &rows[0] else {
            panic!("expected article");
        };
        assert_eq!(a.slug, "imported");
        assert_eq!(a.status, "published");
        assert_eq!(a.client_visible, "true");
        assert_eq!(a.external_key, "wiki-42");
    }

    #[test]
    fn html_flattens_headings_and_strips_tags() {
        let src = "<html><body><h1>Support</h1><h2>Password reset</h2><p>Use &quot;self-service&quot;.</p><ul><li>step one</li></ul></body></html>";
        let rows = ingest_bundle(src, ContentFormat::Html).expect("parses");
        assert_eq!(rows.len(), 2);
        assert!(
            matches!(&rows[0], KbImportRow::Category(c) if c.name == "Support" && c.slug == "support")
        );
        let KbImportRow::Article(a) = &rows[1] else {
            panic!("expected article");
        };
        assert_eq!(a.title, "Password reset");
        assert!(a.body.contains("Use \"self-service\"."));
        assert!(a.body.contains("step one"));
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn html_without_headings_uses_front_matter_title_or_errors() {
        let page = "<p>plain content only</p>";
        assert!(ingest_bundle(page, ContentFormat::Html).is_err());
        let titled = format!("---\ntitle: Just a page\n---\n{page}");
        let rows = ingest_bundle(&titled, ContentFormat::Html).expect("title fallback");
        let [KbImportRow::Article(a)] = rows.as_slice() else {
            panic!("expected one article");
        };
        assert_eq!(a.title, "Just a page");
        assert_eq!(a.slug, "just-a-page");
        assert!(a.body.contains("plain content only"));
    }

    #[test]
    fn unclosed_fence_is_body_not_front_matter_and_empty_bundle_is_loud() {
        let src = "---\n\nsome text about rules\n";
        assert!(ingest_bundle(src, ContentFormat::Markdown).is_err());
    }

    #[test]
    fn curation_csv_round_trips_through_the_engine_parser() {
        let rows = ingest_bundle(
            "# Cat One\n\ndesc text\n\n## An article, with comma\n\nhello **world**\nline two\n",
            ContentFormat::Markdown,
        )
        .expect("parses");
        let csv = rows_to_csv(&rows);
        assert!(csv.starts_with("kind,external_key"));
        let parsed = core_domain::import_core::parse_csv_chunk(&csv).expect("engine re-parse");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].fields["kind"], "kb_category");
        assert_eq!(parsed[1].fields["kind"], "kb_article");
        // Multi-line quoted cells survive verbatim (RFC 4180).
        assert!(parsed[1].fields["body"].contains('\n'));
    }

    /// The export is a delivery to a spreadsheet, so every cell takes the
    /// shared report writer's contract: a formula-leading title or body gets
    /// the text prefix, a comma-bearing value is quoted after that prefix,
    /// and a plain number or ISO date survives as data rather than text.
    #[test]
    fn curation_export_prefixes_formulas_and_leaves_numbers_and_dates_bare() {
        fn article(title: &str, body: &str) -> KbImportRow {
            KbImportRow::Article(KbArticleImportRow {
                external_key: "art".into(),
                category_slug: None,
                title: title.into(),
                slug: "art".into(),
                body: body.into(),
                status: "draft".into(),
                client_visible: "false".into(),
            })
        }
        let row = |csv: &str| {
            let parsed = core_domain::import_core::parse_csv_chunk(csv).expect("re-parse");
            parsed[0].fields.clone()
        };

        // A title a spreadsheet would evaluate, and a body that hides the
        // introducer behind whitespace.
        let fields = row(&rows_to_csv(&[article("=SUM(A1)", "\t=1+1")]));
        assert_eq!(fields["title"], "'=SUM(A1)");
        assert_eq!(fields["body"], "'\t=1+1");

        // The prefix lands before RFC 4180 quoting, not inside a bare cell.
        let csv = rows_to_csv(&[article("=A1,B1", "plain")]);
        assert!(csv.contains("\"'=A1,B1\""), "{csv}");
        assert_eq!(row(&csv)["title"], "'=A1,B1");

        // Numbers and dates are data: prefixing them would turn a numeric or
        // date column into text. (The exemption is per cell, so it holds for a
        // cell that is only a number, only a date.)
        let fields = row(&rows_to_csv(&[article("Acme, Inc", "-120")]));
        assert_eq!(fields["body"], "-120");
        assert_eq!(fields["title"], "Acme, Inc");
        let fields = row(&rows_to_csv(&[article(
            "Reset by 2026-10-22",
            "2026-10-22",
        )]));
        assert_eq!(fields["body"], "2026-10-22");
        assert_eq!(fields["title"], "Reset by 2026-10-22");
    }

    /// The engine mirrors core-kb's portability bounds by value — this test
    /// is the cross-crate tripwire so the two can never drift apart.
    #[test]
    fn import_engine_limits_stay_in_lockstep() {
        use core_domain::import_core::core_kb_limits;
        assert_eq!(core_kb_limits::MAX_TITLE_CHARS, crate::MAX_TITLE_CHARS);
        assert_eq!(core_kb_limits::MAX_BODY_BYTES, crate::MAX_BODY_BYTES);
    }

    /// The flattened shape must be accepted by the real import engine:
    /// rows validate cleanly and the article projection carries exactly the
    /// operator-content keys (no slugs) so re-ingestion stays a no-op.
    #[test]
    fn flattened_rows_are_accepted_by_the_import_engine() {
        use core_domain::import_core::{build_specs, parse_json_chunk, EntityKind};
        let rows = ingest_bundle(
            "# Cat One\n\ndesc\n\n## An article\n\nhello **world**\n",
            ContentFormat::Markdown,
        )
        .expect("parses");
        let cat = match &rows[0] {
            KbImportRow::Category(c) => serde_json::json!({
                "external_key": c.external_key,
                "name": c.name,
                "slug": c.slug,
                "description": c.description,
                "position": c.position,
                "client_visible": c.client_visible,
            }),
            _ => unreachable!(),
        };
        let art = match &rows[1] {
            KbImportRow::Article(a) => serde_json::json!({
                "external_key": a.external_key,
                "category_slug": a.category_slug,
                "title": a.title,
                "slug": a.slug,
                "body": a.body,
                "status": a.status,
                "client_visible": a.client_visible,
            }),
            _ => unreachable!(),
        };
        let cat_rows = parse_json_chunk(&serde_json::json!([cat])).expect("cat rows");
        let specs = build_specs(EntityKind::KbCategory, &cat_rows);
        assert!(specs[0].spec.is_ok(), "{:?}", specs[0].spec);

        let art_rows = parse_json_chunk(&serde_json::json!([art])).expect("art rows");
        let specs = build_specs(EntityKind::KbArticle, &art_rows);
        let spec = specs[0].spec.as_ref().expect("article validates");
        assert_eq!(spec.projection().len(), 4);
    }
}
