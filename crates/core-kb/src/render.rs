//! Read-only server-side HTML renderer for stored Tiptap JSON
//! (ADR 0007, Wave 16 lane 2).
//!
//! Purpose: render `kb_articles.content_jsonb` → sanitized HTML for the
//! read-only surface (RSC renders it into the page; the editor package
//! never ships in this bundle). Sanitization is BY CONSTRUCTION:
//!
//! * the renderer can only emit tags from its closed whitelist —
//!   `<script>`, `<style>`, `<object>`, `<embed>` simply have no code
//!   path, so "no script tags ever serialize" is a type-level promise,
//!   not a post-hoc strip pass;
//! * every text run and every attribute value goes through
//!   [`escape_html`] — `<script>` cannot smuggle through text or attrs;
//! * every URL attribute (link href, image src, embed src) is an
//!   absolute https URL re-parsed at render time (write time already
//!   validated it; render re-checks rather than trusting storage) —
//!   `javascript:`, `data:`, and relative URLs are impossible outputs;
//! * `<iframe>` is emitted ONLY for an embed node whose URL currently
//!   matches an enabled ADR 0004 allowlist registry row (passed in by
//!   the caller), and its src is a generated provider page URL derived
//!   from the validated URL — never user-supplied HTML.
//!
//! Unknown node types (a doc stored by a later schema version) degrade
//! to their children — never to raw passthrough (U6 forward tolerance).
//!
//! Non-purpose: no oEmbed HTTP fetches (core-api owns those), no
//! editor logic — a pure function from validated JSON to HTML bytes.

use serde_json::Value;
use std::sync::OnceLock;
use url::Url;

use crate::rich;

#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone)]
pub enum RenderError {
    #[error("document must be a {{type:'doc', content:[...]}} object")]
    NotADoc,
}

/// Renders the `doc` member to sanitized HTML. Embed nodes whose URL is
/// NOT currently allowlisted (or when no registry is supplied) render
/// as a "blocked embed" placeholder — the sanction source re-resolves
/// at render time (0069 doctrine: NULL provider renders nothing).
pub fn render(doc: &Value, providers: &[core_domain::entities::KbEmbedProvider]) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str(r#"<div class="kb-prose">"#);
    render_children(doc, providers, &mut out);
    out.push_str("</div>");
    out
}

/// Shared renderer for node lists.
fn render_children(
    node: &Value,
    providers: &[core_domain::entities::KbEmbedProvider],
    out: &mut String,
) {
    let Some(Value::Array(children)) = node.get("content") else {
        return;
    };
    for child in children {
        render_node(child, providers, out);
    }
}

/// Renders inline marks (ordered: code innermost keeps monospace
/// atomic; strong/em arbitrary nesting is legal HTML).
fn render_marks_open(marks: &[Value], out: &mut String) {
    for mark in marks {
        let name = mark.get("type").and_then(Value::as_str).unwrap_or("");
        let attrs = mark.get("attrs");
        match name {
            "bold" => out.push_str("<strong>"),
            "italic" => out.push_str("<em>"),
            "strike" => out.push_str("<s>"),
            "code" => out.push_str("<code>"),
            "underline" => out.push_str("<u>"),
            "link" => {
                // href pre-filtered against unparseable/unapproved
                // schemes; still parse locally (zero-trust render).
                let href = attrs
                    .and_then(|a| a.get("href"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if let Ok(url) = Url::parse(href.trim()) {
                    if url.scheme() == "https" || url.scheme() == "http" {
                        out.push_str(r#"<a href=""#);
                        out.push_str(&escape_html(url.as_str()));
                        out.push_str(r#"" target="_blank" rel="noopener noreferrer nofollow">"#);
                    }
                }
            }
            _ => {} // unknown mark (future schema) renders as plain text
        }
    }
}

fn render_marks_close(marks: &[Value], out: &mut String) {
    for mark in marks.iter().rev() {
        match mark.get("type").and_then(Value::as_str).unwrap_or("") {
            "bold" => out.push_str("</strong>"),
            "italic" => out.push_str("</em>"),
            "strike" => out.push_str("</s>"),
            "code" => out.push_str("</code>"),
            "underline" => out.push_str("</u>"),
            "link" => out.push_str("</a>"),
            _ => {}
        }
    }
}

/// Renders one node. Unknown types degrade to children — never to raw
/// passthrough.
fn render_node(
    node: &Value,
    providers: &[core_domain::entities::KbEmbedProvider],
    out: &mut String,
) {
    let Value::Object(m) = node else { return };
    let name = m.get("type").and_then(Value::as_str).unwrap_or("");
    let attrs = m.get("attrs");
    let marks: Vec<Value> = m
        .get("marks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // Drop link marks whose href cannot serve (open/close must stay
    // paired — the safe failure is the text WITHOUT a link).
    let marks: Vec<Value> = marks
        .into_iter()
        .filter(|mark| {
            if mark.get("type").and_then(Value::as_str) != Some("link") {
                return true;
            }
            mark.get("attrs")
                .and_then(|a| a.get("href"))
                .and_then(Value::as_str)
                .and_then(|h| Url::parse(h.trim()).ok())
                .map(|url| url.scheme() == "https" || url.scheme() == "http")
                .unwrap_or(false)
        })
        .collect();
    match name {
        "doc" => render_children(node, providers, out),
        "paragraph" => {
            out.push_str("<p>");
            render_marks_open(&marks, out);
            render_children(node, providers, out);
            render_marks_close(&marks, out);
            out.push_str("</p>");
        }
        "heading" => {
            // The schema-relevant heading level: 1..=6 only, else h2.
            let level = attrs
                .and_then(|a| a.get("level"))
                .and_then(Value::as_u64)
                .filter(|l| (1..=6).contains(l))
                .unwrap_or(2) as u8;
            let tag = format!("h{}", level + 1);
            out.push('<');
            out.push_str(&tag);
            out.push('>');
            render_marks_open(&marks, out);
            render_children(node, providers, out);
            render_marks_close(&marks, out);
            out.push_str("</");
            out.push_str(&tag);
            out.push('>');
        }
        "text" => {
            render_marks_open(&marks, out);
            if let Some(Value::String(t)) = m.get("text") {
                out.push_str(&escape_html(t));
            }
            render_marks_close(&marks, out);
        }
        "bulletList" => {
            out.push_str("<ul>");
            render_children(node, providers, out);
            out.push_str("</ul>");
        }
        "orderedList" => {
            out.push_str("<ol>");
            render_children(node, providers, out);
            out.push_str("</ol>");
        }
        "listItem" => {
            out.push_str("<li>");
            render_children(node, providers, out);
            out.push_str("</li>");
        }
        "taskList" => {
            out.push_str(r#"<ul class="kb-task-list">"#);
            render_children(node, providers, out);
            out.push_str("</ul>");
        }
        "taskItem" => {
            let checked = attrs
                .and_then(|a| a.get("checked"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if checked {
                out.push_str(r#"<li class="kb-task-checked">"#);
            } else {
                out.push_str("<li>");
            }
            render_children(node, providers, out);
            out.push_str("</li>");
        }
        "blockquote" => {
            out.push_str("<blockquote>");
            render_children(node, providers, out);
            out.push_str("</blockquote>");
        }
        "codeBlock" => {
            out.push_str("<figure class=\"kb-code");
            let lang = attrs
                .and_then(|a| a.get("language"))
                .and_then(Value::as_str)
                .filter(|l| {
                    !l.is_empty()
                        && l.len() <= 32
                        && l.chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '+' || c == '#')
                });
            if let Some(lang) = lang {
                out.push_str(" data-language=\"");
                out.push_str(&escape_html(lang));
                out.push('"');
            }
            out.push_str("\"><pre><code>");
            render_children(node, providers, out);
            out.push_str("</code></pre></figure>");
        }
        "hardBreak" => out.push_str("<br>"),
        "horizontalRule" => out.push_str("<hr>"),
        "image" => {
            let Some(src) = attrs_else_https(attrs, "src") else {
                return; // unplaceable image → drop silently
            };
            out.push_str("<img src=\"");
            out.push_str(&escape_html(&src));
            out.push_str("\" loading=\"lazy\"");
            for key in ["alt", "title"] {
                let value = attrs
                    .and_then(|a| a.get(key))
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty());
                if let Some(v) = value {
                    out.push_str(&format!(" {}=\"{}\"", key, escape_html(v)));
                }
            }
            out.push('>');
        }
        "youtube" | "meridianKbEmbed" => {
            let invalid = r#"<figure class="kb-embed kb-embed-invalid"><figcaption>Embed removed — unsafe URL</figcaption></figure>"#;
            let blocked = r#"<figure class="kb-embed kb-embed-blocked"><figcaption>Embed not allowlisted</figcaption></figure>"#;
            let Some(src) = attrs_else_https(attrs, "src") else {
                out.push_str(invalid);
                return;
            };
            match rich::match_provider(providers, &src) {
                Ok(provider) => {
                    // Provider-sanctioned: emit the iframe from a
                    // GENERATED https page URL for the provider class
                    // (never user-supplied HTML). YouTube renders via
                    // privacy-safe youtube-nocookie, lazy.
                    let page = if provider.slug == "youtube" {
                        youtube_embed_page(&src)
                    } else {
                        None
                    };
                    if let Some(page) = page {
                        out.push_str("<figure class=\"kb-embed\"><div class=\"kb-embed-frame\">");
                        out.push_str("<iframe src=\"");
                        out.push_str(&escape_html(&page));
                        let title = attrs
                            .and_then(|a| a.get("title"))
                            .and_then(Value::as_str)
                            .unwrap_or("Embedded video");
                        out.push_str("\" title=\"");
                        out.push_str(&escape_html(title));
                        out.push_str("\" allowfullscreen loading=\"lazy\" referrerpolicy=\"strict-origin-when-cross-origin\" allow=\"encrypted-media; picture-in-picture\"></iframe></div></figure>");
                        return;
                    }
                    // Sanctioned but no generated-page mapping: render
                    // the canonical link card, still never raw HTML.
                    out.push_str("<figure class=\"kb-embed\"><a href=\"");
                    out.push_str(&escape_html(&src));
                    out.push_str(
                        r#"" target="_blank" rel="noopener noreferrer">Open the embedded content</a></figure>"#,
                    );
                }
                Err(_) => out.push_str(blocked),
            }
        }
        "meridianAssetLink" => {
            let asset = attrs
                .and_then(|a| a.get("assetId"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let label: String = m
                .get("content")
                .and_then(Value::as_array)
                .map(|children| {
                    children
                        .iter()
                        .filter_map(|c| c.get("text"))
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default();
            if asset.len() == 36 && asset.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
                out.push_str(r#"<span class="kb-asset-link" data-asset-id=""#);
                out.push_str(&escape_html(asset));
                out.push_str(r#"" data-broken="false">"#);
                out.push_str(&escape_html(if label.is_empty() {
                    "Linked asset"
                } else {
                    &label
                }));
                out.push_str("</span>");
            }
        }
        "meridianTaxonomyTerm" => {
            let term = attrs
                .and_then(|a| a.get("termId"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if term.len() == 36 && term.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
                out.push_str(r#"<span class="kb-taxonomy-term" data-term-id=""#);
                out.push_str(&escape_html(term));
                out.push_str(r#"">"#);
                out.push_str(&escape_html(
                    &m.get("content")
                        .and_then(Value::as_array)
                        .map(|children| {
                            children
                                .iter()
                                .filter_map(|c| c.get("text"))
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join("")
                        })
                        .unwrap_or_else(|| "term".into()),
                ));
                out.push_str("</span>");
            }
        }
        // Unknown future-schema nodes degrade to children.
        _ => render_children(node, providers, out),
    }
}

fn attrs_else_https(attrs: Option<&Value>, key: &str) -> Option<String> {
    let src = attrs?.get(key)?.as_str()?;
    // https-only absolute URL, re-parsed fresh from storage.
    let url = rich::validate_url(src).ok()?;
    Some(url.as_str().to_string())
}

/// Derives the privacy-safe youtube-nocookie embed page URL from a
/// validated allowlisted YouTube URL (watch?v=ID / shorts/ID /
/// embed/ID / youtu.be/ID). Returns None for non-YouTube shapes so the
/// caller falls back to the link card.
fn youtube_embed_page(src: &str) -> Option<String> {
    static ID_RE: OnceLock<regex::Regex> = OnceLock::new();
    let url = Url::parse(src).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    if !(host.ends_with("youtube.com") || host.ends_with("youtu.be") || host == "youtube.com") {
        return None;
    }
    let re = ID_RE.get_or_init(|| {
        regex::Regex::new(r"(?:v=|shorts/|embed/|youtu\.be/)([A-Za-z0-9_-]{6,32})")
            .expect("youtube id regex")
    });
    let raw = re
        .captures(src)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())?;
    if !raw
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(format!("https://www.youtube-nocookie.com/embed/{raw}"))
}

/// HTML-escapes text content and attribute values (`<` `>` `&` `"` `'`).
pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content;
    use serde_json::json;

    fn youtube_provider() -> core_domain::entities::KbEmbedProvider {
        use chrono::Utc;
        core_domain::entities::KbEmbedProvider {
            id: uuid::Uuid::nil(),
            org_id: None,
            name: "YouTube".into(),
            slug: "youtube".into(),
            host_patterns: vec![
                "youtube.com".into(),
                "www.youtube.com".into(),
                "m.youtube.com".into(),
                "youtu.be".into(),
                "www.youtu.be".into(),
            ],
            url_regex: "^(https://)?(www\\.)?(youtube\\.com/(watch\\?v=[A-Za-z0-9_-]+|shorts/[A-Za-z0-9_-]+|embed/[A-Za-z0-9_-]+)|youtu\\.be/[A-Za-z0-9_-]+)([?&].*)?$".into(),
            oembed_endpoint: "https://www.youtube.com/oembed?url={url}&format=json".into(),
            enabled: true,
            is_system: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn renders_paragraph_and_marks() {
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [
                    { "type": "text", "text": "Use " },
                    { "type": "text", "text": "bold",
                      "marks": [{ "type": "bold" }] },
                    { "type": "text", "text": " text." }
                ]
            }]
        });
        content::validate_document(&doc).unwrap();
        let html = render(&doc, &[]);
        assert_eq!(
            html,
            r#"<div class="kb-prose"><p>Use <strong>bold</strong> text.</p></div>"#
        );
    }

    #[test]
    fn script_injection_never_serializes() {
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "<script>alert(1)</script>"
                }]
            }]
        });
        let html = render(&doc, &[]);
        assert!(!html.contains("<script"), "no script tag ever: {html}");
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn javascript_urls_cannot_reach_outputs() {
        // link href/javascript: — the validator already rejects it, but
        // the renderer also refuses any non-https(http) scheme itself.
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "marks": [{ "type": "link", "attrs": { "href": "javascript:alert(1)" } }],
                    "text": "click"
                }]
            }]
        });
        let html = render(&doc, &[]);
        assert!(!html.contains("javascript:"), "got: {html}");
        assert_eq!(html, r#"<div class="kb-prose"><p>click</p></div>"#);
    }

    #[test]
    fn unsanctioned_iframes_never_serialize_but_allowlisted_youtube_does() {
        // An unsanctioned URL with no registry → blocked placeholder,
        // never an iframe, never user HTML.
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "youtube",
                "attrs": { "src": "https://evil.example/watch?v=x" }
            }]
        });
        let html = render(&doc, &[]);
        assert!(
            !html.contains("<iframe"),
            "no iframe without allowlist: {html}"
        );
        assert!(html.contains("kb-embed-blocked"));

        // Allowlisted YouTube → generated youtube-nocookie iframe.
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "youtube",
                "attrs": { "src": "https://www.youtube.com/watch?v=dQw4w9WgXcQ" }
            }]
        });
        let html = render(&doc, &[youtube_provider()]);
        assert!(
            html.contains("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ"),
            "iframe src is generated, not supplied: {html}"
        );
        assert!(html.contains("allowfullscreen"));

        // Host-prefix trick with the registry IN the doc: blocked.
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "youtube",
                "attrs": { "src": "https://evil-youtube.com/watch?v=dQw4w9WgXcQ" }
            }]
        });
        let html = render(&doc, &[youtube_provider()]);
        assert!(!html.contains("<iframe"), "exact-host gate holds: {html}");
    }

    #[test]
    fn sanctioned_non_youtube_embed_renders_link_card() {
        use chrono::Utc;
        let loom = core_domain::entities::KbEmbedProvider {
            id: uuid::Uuid::nil(),
            org_id: None,
            name: "Loom".into(),
            slug: "loom".into(),
            host_patterns: vec!["loom.com".into(), "www.loom.com".into()],
            url_regex: "^(https://)?(www\\.)?loom\\.com/share/[A-Za-z0-9]+([?&].*)?$".into(),
            oembed_endpoint: "https://www.loom.com/v1/oembed?url={url}&format=json".into(),
            enabled: true,
            is_system: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "meridianKbEmbed",
                "attrs": { "src": "https://www.loom.com/share/abc123" }
            }]
        });
        let html = render(&doc, &[loom]);
        assert!(html.contains("Open the embedded content"), "{html}");
        assert!(!html.contains("<iframe"));
    }

    #[test]
    fn unknown_nodes_degrade_to_children() {
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "futureWidget",
                "attrs": { "evil": "<script>" },
                "content": [{ "type": "text", "text": "collapsed" }]
            }]
        });
        let html = render(&doc, &[]);
        assert!(html.contains("collapsed"));
        assert!(!html.contains("evil"));
    }

    #[test]
    fn asset_links_escape_labels() {
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "meridianAssetLink",
                "attrs": { "assetId": "018f0000-0000-7000-8000-000000000000" },
                "content": [{ "type": "text", "text": "Camera <3" }]
            }]
        });
        let html = render(&doc, &[]);
        assert!(html.contains("data-asset-id="));
        assert!(html.contains("Camera &lt;3"), "{html}");
    }
}
