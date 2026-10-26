//! Tool-catalog derivation: the committed OpenAPI snapshot → MCP tool
//! descriptors. Mechanically derived, never hand-written: one catalog
//! build per process (the spec is a build artifact), one tool per safe
//! read operation.

use serde_json::{json, Value};
use std::collections::HashSet;

/// One derived MCP tool: coordinates + JSON-Schema input.
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    /// MCP tool name: the operationId (`clients_list`), already unique
    /// and stable per the spec's generator.
    pub name: String,
    pub description: String,
    /// The capability this tool requires (`clients:read`).
    pub capability: String,
    /// The absolute API path with `{param}` templates intact.
    pub path: String,
    /// HTTP method (always GET for read tools).
    pub method: String,
    /// JSON-Schema object built from the operation's parameters.
    pub input_schema: Value,
}

/// Builds the read-tool catalog from a parsed OpenAPI document.
///
/// Inclusion rule: `GET` operations only, excluding the sensitive
/// prefixes (`EXCLUDED_PATH_PREFIXES`) and any operation without an
/// `operationId`. Each tool's input schema covers both path parameters
/// (required) and query parameters (optional), typed from the spec.
pub fn build_catalog(spec: &Value) -> Vec<Tool> {
    let mut tools = Vec::new();
    let Some(paths) = spec.get("paths").and_then(Value::as_object) else {
        return tools;
    };
    for (path, item) in paths {
        let Some(get_op) = item.get("get").and_then(Value::as_object) else {
            continue;
        };
        if crate::is_excluded(path) {
            continue;
        }
        let Some(operation_id) = get_op.get("operationId").and_then(Value::as_str) else {
            continue;
        };
        let Some(capability_ns) = crate::capability_of(path) else {
            continue;
        };
        let description = get_op
            .get("summary")
            .and_then(Value::as_str)
            .or_else(|| get_op.get("description").and_then(Value::as_str))
            .unwrap_or(operation_id)
            .to_string();

        // Merge path + query parameters into one schema. Path params are
        // required; query params optional. Inline $ref resolution stays a
        // v1 gap: the generated spec inlines scalar parameter schemas.
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        if let Some(params) = get_op.get("parameters").and_then(Value::as_array) {
            for param in params {
                let (Some(name), Some(location)) = (
                    param.get("name").and_then(Value::as_str),
                    param.get("in").and_then(Value::as_str),
                ) else {
                    continue;
                };
                let schema = param.get("schema").cloned().unwrap_or(json!({}));
                properties.insert(name.to_string(), schema);
                if location == "path"
                    || (param.get("required").and_then(Value::as_bool) == Some(true))
                {
                    required.push(json!(name));
                }
            }
        }
        let input_schema = json!({
            "type": "object",
            "properties": properties,
            "required": required,
        });

        tools.push(Tool {
            name: operation_id.to_string(),
            description,
            capability: format!("{capability_ns}:read"),
            path: path.to_string(),
            method: "GET".into(),
            input_schema,
        });
    }
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    tools
}

/// Intersects the derived catalog with the org's granted capability
/// NAMESPACES (the `:read` side of the registry). Deny-by-default: a
/// namespace without a grant row disappears from `tools/list` entirely.
/// `tools/call` re-checks through the scoped backend (defense in depth).
pub fn filter_granted(tools: Vec<Tool>, granted_namespaces: &HashSet<String>) -> Vec<Tool> {
    tools
        .into_iter()
        .filter(|t| {
            t.capability
                .strip_suffix(":read")
                .is_some_and(|ns| granted_namespaces.contains(ns))
        })
        .collect()
}

/// Interpolates `{param}` template segments from the call arguments; a
/// missing path parameter is a validation error. Every value is
/// percent-encoded before substitution: an argument can never add a path
/// segment (`/`), open a query (`?`), or close the path (`#`) — the
/// proxied request lands on exactly the endpoint the tool declared.
pub fn resolve_path(tool: &Tool, args: &Value) -> Result<String, String> {
    let mut out = tool.path.clone();
    for seg in tool.path.split('/') {
        if let Some(name) = seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            let value = args
                .get(name)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("missing path parameter: {name}"))?;
            out = out.replace(seg, &urlencoded(value));
        }
    }
    Ok(out)
}

/// Splits call arguments into path-query (for [`resolve_path`]) and the
/// query string appended to the proxied request.
pub fn query_string(tool: &Tool, args: &Value) -> String {
    let mut pairs = Vec::new();
    if let Some(obj) = args.as_object() {
        for (key, value) in obj {
            if tool.path.contains(&format!("{{{key}}}")) {
                continue;
            }
            let rendered = match value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            pairs.push(format!("{}={}", urlencoded(key), urlencoded(&rendered)));
        }
    }
    if pairs.is_empty() {
        String::new()
    } else {
        format!("?{}", pairs.join("&"))
    }
}

fn urlencoded(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_spec() -> Value {
        json!({
            "paths": {
                "/api/clients": { "get": {
                    "operationId": "clients_list",
                    "summary": "List clients",
                    "parameters": [
                        {"name": "limit", "in": "query", "schema": {"type": "integer"}},
                        {"name": "X-Ignore", "in": "header", "schema": {"type": "string"}}
                    ]
                }},
                "/api/clients/{id}": { "get": {
                    "operationId": "clients_get_one",
                    "summary": "Get one client",
                    "parameters": [
                        {"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}
                    ]
                }},
                "/api/attachments/{id}/download": { "get": {
                    "operationId": "attachments_download",
                    "summary": "must be excluded"
                }},
                "/api/tickets": { "post": {
                    "operationId": "tickets_create",
                    "summary": "writes are excluded from v1"
                }}
            }
        })
    }

    #[test]
    fn catalog_derives_get_tools_only() {
        let tools = build_catalog(&sample_spec());
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["clients_get_one", "clients_list"]);
    }

    #[test]
    fn capabilities_derive_from_path() {
        let tools = build_catalog(&sample_spec());
        assert!(tools.iter().all(|t| t.capability.ends_with(":read")));
        assert_eq!(tools[0].capability, "clients:read");
    }

    #[test]
    fn path_params_are_required() {
        let tools = build_catalog(&sample_spec());
        let one = tools.iter().find(|t| t.name == "clients_get_one").unwrap();
        assert_eq!(one.input_schema["required"], json!(["id"]));
    }

    #[test]
    fn resolve_path_interpolates_and_validates() {
        let tools = build_catalog(&sample_spec());
        let one = tools.iter().find(|t| t.name == "clients_get_one").unwrap();
        assert_eq!(
            resolve_path(one, &json!({"id": "abc"})).unwrap(),
            "/api/clients/abc"
        );
        assert!(resolve_path(one, &json!({})).is_err());
    }

    /// An argument can never reshape the proxied URL: `/` is encoded so a
    /// value cannot add segments (traversal onto another endpoint), and
    /// `?`/`#` cannot open a query or close the path.
    #[test]
    fn resolve_path_percent_encodes_reshaping_characters() {
        let tools = build_catalog(&sample_spec());
        let one = tools.iter().find(|t| t.name == "clients_get_one").unwrap();
        let path = resolve_path(one, &json!({"id": "../api/v1/audit-events"})).unwrap();
        assert_eq!(path, "/api/clients/..%2Fapi%2Fv1%2Faudit-events");
        assert!(!path.contains("/../"), "no literal traversal segment");
        let path = resolve_path(one, &json!({"id": "x?admin=1#y"})).unwrap();
        assert_eq!(path, "/api/clients/x%3Fadmin%3D1%23y");
    }

    #[test]
    fn query_string_skips_path_params() {
        let tools = build_catalog(&sample_spec());
        let list = tools.iter().find(|t| t.name == "clients_list").unwrap();
        let q = query_string(list, &json!({"limit": 5, "offset": 2}));
        assert!(q.contains("limit=5"));
        assert!(q.contains("offset=2"));
        let one = tools.iter().find(|t| t.name == "clients_get_one").unwrap();
        assert_eq!(query_string(one, &json!({"id": "abc"})), "");
    }
}
