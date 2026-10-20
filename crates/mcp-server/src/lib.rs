//! mcp-server — MCP protocol adapter over the public API (the private build tracker
//! Phase 3, Wave 9 lane 4; docs/10-agent-platform.md §1).
//!
//! Purpose: expose the org's capability-granted READ surface over the
//! Model Context Protocol. The tool catalog derives MECHANICALLY from the
//! committed OpenAPI snapshot (`packages/sdk/openapi.json`): every safe
//! read operation (`GET` without sensitive paths) becomes one MCP tool
//! whose input schema is that operation's query parameters — zero
//! hand-written tools, zero business logic. The adapter holds no
//! database: every tool call proxies core-api with the caller's own
//! credentials.
//!
//! Two transports:
//! - stdio JSON-RPC (the canonical MCP transport) via [`stdio::serve`];
//! - the same protocol over HTTP for the test suite and local dev
//!   ([`http::router`], mounted at `/mcp`).
//!
//! Capability registry: a capability slug that the org has NOT granted is
//! deny-by-default — tools whose derived capability is not granted are
//! hidden from `tools/list` and refuse `tools/call` with a typed error.
//! Read tools filter on `:read` grants (the API key's own scopes still
//! apply downstream in core-api, defense in depth).
//!
//! Write tools: intentionally ABSENT in v1. Agent-authored record changes
//! enter through the IF8 proposal pipeline (`POST /api/agent-proposals`,
//! accepted from `x-api-key` callers) and apply only after human approval
//! through the IF1 path — never through a direct MCP write tool.
//!
//! Non-purpose: no direct database access (it calls core-api
//! exclusively), no custom tool implementations, no auth decisions of its
//! own (delegates to API keys; every call is audit-logged core-side).

pub mod catalog;
pub mod http;
pub mod protocol;
pub mod stdio;
pub mod tools;

pub use protocol::{RpcRequest, RpcResponse, Server};
pub use tools::{Backend, CoreApiBackend, ScopedBackend, ToolError};

/// The committed OpenAPI snapshot, embedded at build time: the tool
/// catalog is pinned to the reviewed spec, never to a mutable runtime
/// file (the binary does not trust its CWD).
pub const OPENAPI_SNAPSHOT: &str = include_str!("../../../packages/sdk/openapi.json");

/// Parses the embedded snapshot.
pub fn openapi_spec() -> serde_json::Value {
    serde_json::from_str(OPENAPI_SNAPSHOT).expect("committed openapi.json parses")
}

/// Sensitive prefixes a GET path may never carry into the tool catalog:
/// document downloads, export bundles, and webhook delivery logs are data
/// exfiltration surfaces, not agent reads. The committed snapshot lists
/// canonical `/api/v1` paths, so exclusions are written against that
/// mount; `capability_of` normalizes any legacy `/api` twin first.
const EXCLUDED_PATH_PREFIXES: &[&str] = &[
    "/api/v1/attachments",
    "/api/v1/org/exports",
    "/api/v1/webhooks",
    "/api/v1/api-keys",
    "/api/v1/auth",
    "/api/v1/email",
    "/api/v1/internal",
    "/api/v1/portal",
    // Legacy twins, for completeness if a spec ever lists them again.
    "/api/attachments",
    "/api/org/exports",
    "/api/webhooks",
    "/api/api-keys",
    "/api/auth",
    "/api/email",
    "/api/internal",
    "/api/portal",
];

/// Capability namespace derived from an API path: the resource segment
/// after the (optional) version prefix — `/api/v1/tickets/...` and
/// `/api/tickets/...` both derive `tickets`.
pub(crate) fn capability_of(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/api/")?;
    let rest = rest.strip_prefix("v1/").unwrap_or(rest);
    let ns = rest.split('/').next()?;
    if ns.is_empty() {
        None
    } else {
        Some(ns.to_string())
    }
}

pub(crate) fn is_excluded(path: &str) -> bool {
    if EXCLUDED_PATH_PREFIXES.iter().any(|p| path.starts_with(p)) {
        return true;
    }
    // Attachment content rides many mount points (tickets, comments,
    // portal); file bytes and file listings never enter the agent
    // catalog regardless of where they hang.
    path.contains("/attachments")
}
