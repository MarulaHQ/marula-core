//! KB article content model (ADR 0007, Wave 16 lane 2 `p16-kb-tiptap-model`).
//!
//! Purpose: the ONE home for the canonical rich-content storage contract:
//!
//! * [`ContentEnvelope`]-shape — the versioned wrapper `{"schema_ver": 1,
//!   "doc": <tiptap-doc>}` stored in `kb_articles.content_jsonb`
//!   (schema first; future versions coexist with the v1 render path —
//!   U6 preflight-friendly, additive migration holds);
//! * [`validate_document`] — structural validation of the Tiptap JSON
//!   document against the PINNED published Tiptap JSON schema
//!   (`schemas/tiptap-json-schema.json`, pinned at adoption; upgrades are
//!   CI diff-reviewed) plus the Meridian extension allowlist (schema
//!   control = extension allowlist; no paid bundles — ADR 0007);
//! * [`validate_content`] — the whole write gate: byte bound reusing
//!   [`MAX_BODY_BYTES`], depth bound, structural + allowlist + attribute
//!   validation;
//! * [`validate_embeds`] / [`embed_urls`] — every embed node (YouTube /
//!   KB-embed) carries a https URL resolved through the ADR 0004
//!   allowlist registry (`rich.rs` — the ONE match path, no new code);
//! * [`plain_text`] — deterministic text projection for the `body`
//!   fallback column / FTS weighting.
//!
//! Sanitization guarantee lives with the renderer: HTML rendered from
//! stored JSON passes through [`crate::render`], where script tags,
//! `javascript:` URLs, and unsanctioned iframes are a construction-time
//! impossibility, never a post-hoc filter.
//!
//! Non-purpose: no SQL (core-db), no HTTP (core-api), no editor — the
//! crate owns rules, not storage, transport, or fetches.

use serde_json::Value;

use crate::rich;

/// Envelope version written by this code generation. A bump is a
/// REVIEWED contract step: pinned schema diff in-repo
/// (`schemas/README.md`), migration notes, and a render path that keeps
/// understanding older stored versions (U7 lifecycle).
pub const ENVELOPE_SCHEMA_VER_V1: i64 = 1;

/// Structural depth bound (`doc` = depth 0) so pathological nesting
/// (500-deep blockquote pastes) cannot blow recursion or render time.
/// Size reuses the [`MAX_BODY_BYTES`] semantics.
pub const MAX_DOC_DEPTH: usize = 40;

/// Per-level child bound so a single pathological node cannot explode
/// the walk.
const MAX_CHILDREN: usize = 4096;

/// Tiptap marks Meridian loads (all MIT: StarterKit set + Link + Underline).
pub const ALLOWED_MARKS: &[&str] = &["bold", "italic", "strike", "code", "underline", "link"];

/// Tiptap nodes Meridian loads (all MIT: StarterKit 3 + Image +
/// TaskList/TaskItem + YouTube + Meridian OSS extensions).
pub const ALLOWED_NODES: &[&str] = &[
    "paragraph",
    "heading",
    "text",
    "bulletList",
    "orderedList",
    "listItem",
    "blockquote",
    "codeBlock",
    "hardBreak",
    "horizontalRule",
    "image",
    "taskList",
    "taskItem",
    "youtube",
    // Meridian-authored OSS extensions (published from plugins/):
    "meridianKbEmbed",      // allowlisted KB-embed card (rich.rs registry)
    "meridianAssetLink",    // RelatedAssetLink RecordLink records (ADR 0006)
    "meridianTaxonomyTerm", // note-taxonomy picker term
];

/// Node types that count as embeds and must clear the ADR 0004
/// allowlist registry on every write.
pub const EMBED_NODES: &[&str] = &["youtube", "meridianKbEmbed"];

#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone)]
pub enum ContentError {
    #[error("content envelope must be exactly {{schema_ver, doc}}")]
    BadEnvelopeShape,
    #[error("unknown content envelope schema_ver {0}")]
    UnknownSchemaVer(i64),
    #[error("document exceeds the serialized-size bound")]
    TooLarge,
    #[error("document nesting exceeds the {MAX_DOC_DEPTH}-level bound")]
    TooDeep,
    #[error("children of {path} exceed the {MAX_CHILDREN}-child bound")]
    TooWide { path: String },
    #[error("doc must be a {{type:'doc', content:[...]}} object")]
    NotADoc,
    #[error("node {name} at {path}: {problem}")]
    BadNode {
        name: String,
        path: String,
        problem: String,
    },
    #[error("mark {name} at {path}: {problem}")]
    BadMark {
        name: String,
        path: String,
        problem: String,
    },
    #[error("embed url {url} is not allowlisted ({source})")]
    EmbedNotAllowlisted {
        url: String,
        source: rich::RichError,
    },
}

/// Validates the envelope + document shape. Byte bound first (the
/// serialized envelope reuses the 512 KiB `MAX_BODY_BYTES` semantics —
/// pathological pastes cannot bloat rows or the write path), then the
/// document tree.
pub fn validate_content(bytes: &[u8]) -> Result<(), ContentError> {
    if bytes.len() > crate::MAX_BODY_BYTES {
        return Err(ContentError::TooLarge);
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| ContentError::BadEnvelopeShape)?;
    let Value::Object(map) = &value else {
        return Err(ContentError::BadEnvelopeShape);
    };
    if map.len() != 2
        || !matches!(map.get("schema_ver"), Some(Value::Number(_)))
        || map.get("doc").is_none()
    {
        return Err(ContentError::BadEnvelopeShape);
    }
    let schema_ver = map["schema_ver"]
        .as_i64()
        .ok_or(ContentError::BadEnvelopeShape)?;
    if schema_ver != ENVELOPE_SCHEMA_VER_V1 {
        return Err(ContentError::UnknownSchemaVer(schema_ver));
    }
    validate_document(&map["doc"])
}

/// Validates the `doc` member as a Tiptap JSON document (pinned
/// published schema's structural rules + the Meridian allowlist).
pub fn validate_document(doc: &Value) -> Result<(), ContentError> {
    let Value::Object(top) = doc else {
        return Err(ContentError::NotADoc);
    };
    if top.get("type").and_then(Value::as_str) != Some("doc") {
        return Err(ContentError::NotADoc);
    }
    // `doc` exists exactly once, at the root; children validate normally.
    for key in top.keys() {
        if !matches!(key.as_str(), "type" | "content" | "attrs") {
            return Err(ContentError::BadNode {
                name: "doc".into(),
                path: "doc".into(),
                problem: format!("unknown key {key}"),
            });
        }
    }
    if let Some(Value::Array(children)) = top.get("content") {
        if children.len() > MAX_CHILDREN {
            return Err(ContentError::TooWide { path: "doc".into() });
        }
        for (i, child) in children.iter().enumerate() {
            validate_node(child, 1, &format!("doc[{i}]"))?;
        }
    }
    Ok(())
}

fn name_of(v: Option<&Value>) -> String {
    v.and_then(Value::as_str).unwrap_or("<none>").into()
}

/// The embed nodes, in document order. The API layer resolves each
/// through the ADR 0004 allowlist registry.
pub fn embed_nodes(doc: &Value) -> Vec<&Value> {
    let mut out = Vec::new();
    collect_nodes(doc, &mut out);
    out
}

/// Depth-first walk collecting embed-type nodes.
fn collect_nodes<'a>(node: &'a Value, out: &mut Vec<&'a Value>) {
    let Value::Object(m) = node else { return };
    let is_embed = m
        .get("type")
        .and_then(Value::as_str)
        .map(|t| EMBED_NODES.contains(&t))
        .unwrap_or(false);
    if is_embed {
        out.push(node);
    }
    if let Some(Value::Array(children)) = m.get("content") {
        for c in children {
            collect_nodes(c, out);
        }
    }
}

/// The embed candidate URLs (attrs.src of every embed node), in order.
pub fn embed_urls(doc: &Value) -> Vec<String> {
    embed_nodes(doc)
        .into_iter()
        .filter_map(|n| {
            n.get("attrs")
                .and_then(|a| a.get("src"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

/// Validates every embed node's URL against the ADR 0004 allowlist
/// registry. Errors name the offending URL (never the article).
pub fn validate_embeds(
    doc: &Value,
    providers: &[core_domain::entities::KbEmbedProvider],
) -> Result<(), ContentError> {
    for node in embed_nodes(doc) {
        let url = node
            .get("attrs")
            .and_then(|a| a.get("src"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if let Err(source) = rich::match_provider(providers, url) {
            return Err(ContentError::EmbedNotAllowlisted {
                url: url.to_string(),
                source,
            });
        }
    }
    Ok(())
}

/// Validates one node recursively: pinned-schema structural rules
/// (additionalProperties false, text-node/text-key exclusivity, content
/// arrays of nodes, marks arrays of marks) + allowlist + attributes.
fn validate_node(node: &Value, depth: usize, path: &str) -> Result<(), ContentError> {
    if depth > MAX_DOC_DEPTH {
        return Err(ContentError::TooDeep);
    }
    let Value::Object(m) = node else {
        return Err(ContentError::BadNode {
            name: "<non-object>".into(),
            path: path.into(),
            problem: "node must be an object".into(),
        });
    };
    let Some(Value::String(name)) = m.get("type") else {
        return Err(ContentError::BadNode {
            name: name_of(m.get("type")),
            path: path.into(),
            problem: "type must be a string".into(),
        });
    };
    // `doc` only at depth 0 (validated by enforce_document).

    if !ALLOWED_NODES.contains(&name.as_str()) {
        return Err(ContentError::BadNode {
            name: name.clone(),
            path: path.into(),
            problem: "node not in the Meridian extension allowlist".into(),
        });
    }
    for key in m.keys() {
        if !matches!(
            key.as_str(),
            "type" | "attrs" | "content" | "marks" | "text"
        ) {
            return Err(ContentError::BadNode {
                name: name.clone(),
                path: path.into(),
                problem: format!("unknown key {key}"),
            });
        }
    }
    let is_text = name == "text";
    if is_text && m.get("text").and_then(Value::as_str).is_none() {
        return Err(ContentError::BadNode {
            name: name.clone(),
            path: path.into(),
            problem: "text node requires a text key".into(),
        });
    }
    if !is_text && m.get("text").is_some() {
        return Err(ContentError::BadNode {
            name: name.clone(),
            path: path.into(),
            problem: "only text nodes may carry a text key".into(),
        });
    }
    if let Some(attrs) = m.get("attrs") {
        validate_node_attrs(name, attrs, path)?;
    }
    if let Some(marks) = m.get("marks") {
        let Value::Array(marks) = marks else {
            return Err(ContentError::BadNode {
                name: name.clone(),
                path: path.into(),
                problem: "marks must be an array".into(),
            });
        };
        if marks.len() > MAX_CHILDREN {
            return Err(ContentError::TooWide { path: path.into() });
        }
        for (i, mark) in marks.iter().enumerate() {
            validate_mark(mark, &format!("{path}.marks[{i}]"))?;
        }
    }
    if let Some(content) = m.get("content") {
        let Value::Array(children) = content else {
            return Err(ContentError::BadNode {
                name: name.clone(),
                path: path.into(),
                problem: "content must be an array".into(),
            });
        };
        if children.len() > MAX_CHILDREN {
            return Err(ContentError::TooWide { path: path.into() });
        }
        for (i, child) in children.iter().enumerate() {
            validate_node(child, depth + 1, &format!("{path} > {name}[{i}]"))?;
        }
    }
    Ok(())
}

fn validate_mark(mark: &Value, path: &str) -> Result<(), ContentError> {
    let Some(m) = mark.as_object() else {
        return Err(ContentError::BadMark {
            name: "<non-object>".into(),
            path: path.into(),
            problem: "mark must be an object".into(),
        });
    };
    if m.len() > 2 {
        return Err(ContentError::BadMark {
            name: name_of(m.get("type")),
            path: path.into(),
            problem: "mark carries extra keys".into(),
        });
    }
    let Some(Value::String(name)) = m.get("type") else {
        return Err(ContentError::BadMark {
            name: name_of(m.get("type")),
            path: path.into(),
            problem: "type must be a string".into(),
        });
    };
    if !ALLOWED_MARKS.contains(&name.as_str()) {
        return Err(ContentError::BadMark {
            name: name.clone(),
            path: path.into(),
            problem: "mark not in the Meridian extension allowlist".into(),
        });
    }
    if let Some(attrs) = m.get("attrs") {
        validate_mark_attrs(name, attrs, path)?;
    } else if name == "link" {
        return Err(ContentError::BadMark {
            name: name.clone(),
            path: path.into(),
            problem: "link mark requires an href attr".into(),
        });
    }
    Ok(())
}

fn validate_mark_attrs(name: &str, attrs: &Value, path: &str) -> Result<(), ContentError> {
    let Value::Object(a) = attrs else {
        return Err(ContentError::BadMark {
            name: name.into(),
            path: path.into(),
            problem: "attrs must be an object".into(),
        });
    };
    for (k, v) in a {
        if !matches!(k.as_str(), "href" | "target" | "rel") {
            return Err(ContentError::BadMark {
                name: name.into(),
                path: path.into(),
                problem: format!("unknown attr {k}"),
            });
        }
        if !v.is_string() {
            return Err(ContentError::BadMark {
                name: name.into(),
                path: path.into(),
                problem: format!("attr {k} must be a string"),
            });
        }
    }
    if a.len() > 3 {
        return Err(ContentError::BadMark {
            name: name.into(),
            path: path.into(),
            problem: "too many attrs".into(),
        });
    }
    if name == "link" {
        let href = a
            .get("href")
            .and_then(Value::as_str)
            .ok_or_else(|| ContentError::BadMark {
                name: name.into(),
                path: path.into(),
                problem: "href required".into(),
            })?;
        // https-only (rich::validate_url — javascript:, data:, http: all
        // fail the scheme gate; javascript: URLs can never serialize).
        rich::validate_url(href).map_err(|e| ContentError::BadMark {
            name: name.into(),
            path: path.into(),
            problem: e.to_string(),
        })?;
    }
    Ok(())
}

fn validate_node_attrs(name: &str, attrs: &Value, path: &str) -> Result<(), ContentError> {
    let Value::Object(a) = attrs else {
        return Err(ContentError::BadNode {
            name: name.into(),
            path: path.into(),
            problem: "attrs must be an object".into(),
        });
    };
    match name {
        "heading" => match a.get("textAlign") {
            Some(Value::String(s))
                if ["left", "center", "right", "justify"].contains(&s.as_str()) => {}
            Some(_) | None => {
                return Err(ContentError::BadNode {
                    name: name.into(),
                    path: path.into(),
                    problem: "textAlign must be left|center|right|justify".into(),
                })
            }
        },
        "paragraph" => match a.get("textAlign") {
            None => {}
            Some(Value::String(s))
                if ["left", "center", "right", "justify"].contains(&s.as_str()) => {}
            Some(_) => {
                return Err(ContentError::BadNode {
                    name: name.into(),
                    path: path.into(),
                    problem: "textAlign value unsupported".into(),
                })
            }
        },
        "image" | "youtube" | "meridianKbEmbed" => {
            let src = a
                .get("src")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| ContentError::BadNode {
                    name: name.into(),
                    path: path.into(),
                    problem: "src required".into(),
                })?;
            // https-only absolute URL; embeds additionally clear the
            // registry in validate_embeds (API layer passes org rows).
            let url = rich::validate_url(src).map_err(|e| ContentError::BadNode {
                name: name.into(),
                path: path.into(),
                problem: e.to_string(),
            })?;
            if url.scheme() != "https" {
                return Err(ContentError::BadNode {
                    name: name.into(),
                    path: path.into(),
                    problem: "src must be https".into(),
                });
            }
            if name == "image" {
                for (k, v) in a {
                    if !matches!(k.as_str(), "src" | "alt" | "title") {
                        return Err(ContentError::BadNode {
                            name: name.into(),
                            path: path.into(),
                            problem: format!("unknown attr {k}"),
                        });
                    }
                    if k != "src" && !v.is_string() {
                        return Err(ContentError::BadNode {
                            name: name.into(),
                            path: path.into(),
                            problem: format!("attr {k} must be a string"),
                        });
                    }
                }
            }
        }
        "taskItem" => {
            if !matches!(a.get("checked"), Some(Value::Bool(_))) {
                return Err(ContentError::BadNode {
                    name: name.into(),
                    path: path.into(),
                    problem: "checked must be a boolean".into(),
                });
            }
        }
        "codeBlock" | "meridianAssetLink" | "meridianTaxonomyTerm" => {
            for (k, v) in a {
                if k == "language" && name == "codeBlock" && v.is_string() {
                    continue;
                }
                if k == "assetId" && name == "meridianAssetLink" && v.is_string() {
                    continue;
                }
                if k == "termId" && name == "meridianTaxonomyTerm" && v.is_string() {
                    continue;
                }
                return Err(ContentError::BadNode {
                    name: name.into(),
                    path: path.into(),
                    problem: format!("unknown attr {k}"),
                });
            }
        }
        _ => {
            if !a.is_empty() {
                return Err(ContentError::BadNode {
                    name: name.into(),
                    path: path.into(),
                    problem: "node takes no attrs".into(),
                });
            }
        }
    }
    Ok(())
}

/// The 0077 wrap-on-write shape: a plain-text body becomes the v1
/// envelope (one paragraph per newline-delimited line, escaped text
/// nodes, no markup interpretation) — the SAME shape the 0077 backfill
/// produced for legacy rows. Used by the API write path when a caller
/// supplies only text, so every NEW write lands canonically.
pub fn plain_text_envelope(body: &str) -> Value {
    let children: Vec<Value> = if body.is_empty() {
        vec![serde_json::json!({ "type": "paragraph" })]
    } else {
        body.split('\n')
            .map(|line| {
                serde_json::json!({
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": line }]
                })
            })
            .collect()
    };
    serde_json::json!({
        "schema_ver": ENVELOPE_SCHEMA_VER_V1,
        "doc": { "type": "doc", "content": children }
    })
}

/// Deterministic plain-text projection of the document (text nodes in
/// order, one line per paragraph/block boundary). Back-compat feed for
/// the `body` column and FTS weighting.
pub fn plain_text(doc: &Value) -> String {
    let mut out = String::new();
    append_plain(doc, &mut out);
    out
}

fn append_plain(node: &Value, out: &mut String) {
    let Value::Object(m) = node else { return };
    let name = m.get("type").and_then(Value::as_str).unwrap_or("");
    match name {
        "text" => {
            if let Some(Value::String(t)) = m.get("text") {
                out.push_str(t);
            }
        }
        "hardBreak" => out.push('\n'),
        "horizontalRule" => out.push_str("\n—\n"),
        "paragraph" | "heading" | "taskItem" | "listItem" => {
            for line in m
                .get("content")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[])
            {
                append_plain(line, out);
            }
            out.push('\n');
        }
        _ => {
            if let Some(Value::Array(children)) = m.get("content") {
                for c in children {
                    append_plain(c, out);
                }
            }
            out.push('\n');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn envelope(doc: Value) -> Vec<u8> {
        serde_json::json!({ "schema_ver": 1, "doc": doc })
            .to_string()
            .into_bytes()
    }

    fn simple_doc() -> Value {
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [
                    { "type": "text", "text": "Hello, " },
                    { "type": "text", "text": "world",
                      "marks": [{ "type": "bold" }, { "type": "italic" }] }
                ]
            }]
        })
    }

    #[test]
    fn envelope_shape_gates() {
        assert_eq!(
            validate_content(b"null"),
            Err(ContentError::BadEnvelopeShape)
        );
        assert_eq!(
            validate_content(b"{\"schema_ver\":1}"),
            Err(ContentError::BadEnvelopeShape)
        );
        assert_eq!(
            validate_content(b"{\"schema_ver\":1,\"doc\":{},\"x\":1}"),
            Err(ContentError::BadEnvelopeShape)
        );
        assert_eq!(
            validate_content(b"{\"schema_ver\":true,\"doc\":{}}"),
            Err(ContentError::BadEnvelopeShape)
        );
        assert_eq!(
            validate_content(b"{\"schema_ver\":2,\"doc\":{}}"),
            Err(ContentError::UnknownSchemaVer(2))
        );
        assert!(validate_content(&envelope(simple_doc())).is_ok());
    }

    #[test]
    fn byte_bound_reuses_body_semantics() {
        let doc = envelope(simple_doc());
        assert!(validate_content(&doc).is_ok(), "small docs always fit");
        // Exact same bound as the body fallback column: monotonic
        // guarantee is what matters, the number is one lease.
        assert_eq!(crate::MAX_BODY_BYTES, 512 * 1024);
        let big = vec![b'a'; crate::MAX_BODY_BYTES + 1];
        assert_eq!(
            crate::validate_body(std::str::from_utf8(&big).unwrap()),
            Err(crate::KbError::BodyTooLarge)
        );
        let padded = format!(
            "{{\"schema_ver\":1,\"doc\":{{\"type\":\"doc\",\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}}]}}}}",
            "a".repeat(crate::MAX_BODY_BYTES)
        );
        assert_eq!(
            validate_content(padded.as_bytes()),
            Err(ContentError::TooLarge)
        );
    }

    #[test]
    fn unknown_nodes_and_marks_rejected() {
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "scriptNode", "content": [] }]
            }]
        });
        let err = validate_document(&doc).unwrap_err();
        assert!(
            matches!(err, ContentError::BadNode { ref name, .. } if name == "scriptNode"),
            "got: {err:?}"
        );
        assert!(err.to_string().contains("allowlist"));

        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "x",
                    "marks": [{ "type": "aiSuggest" }]
                }]
            }]
        });
        assert!(
            matches!(
                validate_document(&doc),
                Err(ContentError::BadMark { ref name, .. }) if name == "aiSuggest"
            ),
            "paid/AI marks never pass the allowlist"
        );
    }

    #[test]
    fn text_key_exclusivity_matches_pinned_schema_all_of() {
        // text node without text → rejected (schema allOf `then.required`).
        let doc = json!({
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{ "type": "text" }] }]
        });
        assert!(matches!(
            validate_document(&doc),
            Err(ContentError::BadNode { .. })
        ));
        // non-text node WITH text → rejected (`else.not required text`).
        let doc = json!({
            "type": "doc",
            "content": [{ "type": "hardBreak", "text": "oops" }]
        });
        assert!(matches!(
            validate_document(&doc),
            Err(ContentError::BadNode { .. })
        ));
    }

    #[test]
    fn unknown_keys_rejected_additional_properties_false() {
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "evilCustom": 1,
                "content": []
            }]
        });
        assert!(
            matches!(validate_document(&doc), Err(ContentError::BadNode { ref problem, .. }) if problem.contains("unknown key"))
        );
    }

    #[test]
    fn depth_bound() {
        let mut node = json!({ "type": "text", "text": "x" });
        for _ in 0..MAX_DOC_DEPTH + 3 {
            node = json!({ "type": "blockquote", "content": [node] });
        }
        let doc = json!({ "type": "doc", "content": [node] });
        assert_eq!(validate_document(&doc), Err(ContentError::TooDeep));
    }

    #[test]
    fn children_bound() {
        let mut children = Vec::new();
        for _ in 0..MAX_CHILDREN + 1 {
            children.push(json!({ "type": "hardBreak" }));
        }
        let doc = json!({
            "type": "doc",
            "content": [{ "type": "paragraph", "content": children }]
        });
        let err = validate_document(&doc).unwrap_err();
        assert!(err.to_string().contains("child bound"), "got {err}");
    }

    #[test]
    fn link_and_embed_urls_are_https_only() {
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "x",
                    "marks": [{ "type": "link", "attrs": { "href": "javascript:alert(1)" } }]
                }]
            }]
        });
        let err = validate_document(&doc).unwrap_err();
        assert!(err.to_string().contains("absolute https url"), "got {err}");

        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "image",
                "attrs": { "src": "javascript:alert(1)", "alt": "" }
            }]
        });
        let err = validate_document(&doc).unwrap_err();
        assert!(err.to_string().contains("absolute https"), "got {err}");
    }

    #[test]
    fn embeds_validate_through_rich_registry() {
        let youtube = json!({
            "type": "youtube",
            "attrs": { "src": "https://www.youtube.com/watch?v=dQw4w9WgXcQ" }
        });
        let doc = json!({ "type": "doc", "content": [youtube] });
        assert_eq!(
            embed_urls(&doc),
            vec!["https://www.youtube.com/watch?v=dQw4w9WgXcQ"]
        );
        assert!(validate_embeds(&doc, &[youtube_provider()]).is_ok());
        // evil-youtube.com: host prefix tricks fail the exact-host gate.
        let evil = json!({
            "type": "youtube",
            "attrs": { "src": "https://evil-youtube.com/watch?v=dQw4w9WgXcQ" }
        });
        let doc = json!({ "type": "doc", "content": [evil] });
        let err = validate_embeds(&doc, &[youtube_provider()]).unwrap_err();
        assert!(
            matches!(err, ContentError::EmbedNotAllowlisted { .. }),
            "got {err:?}"
        );
        // A disabled provider never sanctions an embed.
        let mut disabled = youtube_provider();
        disabled.enabled = false;
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "youtube",
                "attrs": { "src": "https://youtu.be/dQw4w9WgXcQ" }
            }]
        });
        assert!(validate_embeds(&doc, std::slice::from_ref(&disabled)).is_err());
    }

    #[test]
    fn plain_text_is_deterministic() {
        let doc = json!({
            "type": "doc",
            "content": [
                { "type": "heading", "content": [{ "type": "text", "text": "Reset a password" }] },
                { "type": "paragraph", "content": [
                    { "type": "text", "text": "Use " },
                    { "type": "text", "text": "the console",
                      "marks": [{ "type": "bold" }] },
                    { "type": "hardBreak" },
                    { "type": "text", "text": "then reboot." }
                ]}
            ]
        });
        assert_eq!(
            plain_text(&doc),
            "Reset a password\nUse the console\nthen reboot.\n\n"
        );
    }

    #[test]
    fn attr_geometry() {
        // heading level is validated at the renderer edge; structure accepts
        // the documented attr shape.
        let doc = json!({
            "type": "doc",
            "content": [
                { "type": "heading", "attrs": { "textAlign": "left" },
                  "content": [{ "type": "text", "text": "h" }] },
                { "type": "paragraph", "content": [] },
                { "type": "codeBlock", "attrs": { "language": "sql" },
                  "content": [{ "type": "text", "text": "select 1" }] },
                { "type": "taskList", "content": [
                    { "type": "taskItem", "attrs": { "checked": true }, "content": [
                        { "type": "paragraph", "content": [
                            { "type": "text", "text": "done" }
                        ]}
                    ]}
                ]}
            ]
        });
        assert!(validate_document(&doc).is_ok());
        // Unknown node attributes fail loudly.
        let bad = json!({
            "type": "doc",
            "content": [{
                "type": "image",
                "attrs": { "src": "https://e.example/a.png", "evil": true }
            }]
        });
        assert!(matches!(
            validate_document(&bad),
            Err(ContentError::BadNode { ref problem, .. }) if problem.contains("unknown attr")
        ));
        // taskItem checked must be a real boolean.
        let bad = json!({
            "type": "doc",
            "content": [{ "type": "taskItem", "attrs": { "checked": "true" } }]
        });
        assert!(matches!(
            validate_document(&bad),
            Err(ContentError::BadNode { ref problem, .. }) if problem.contains("boolean")
        ));
        // Image alt/title must be strings (no smuggled objects).
        let bad = json!({
            "type": "doc",
            "content": [{
                "type": "image",
                "attrs": { "src": "https://e.example/a.png", "alt": { "html": "<script>" } }
            }]
        });
        assert!(matches!(
            validate_document(&bad),
            Err(ContentError::BadNode { ref problem, .. }) if problem.contains("must be a string")
        ));
    }
}
