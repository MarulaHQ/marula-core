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

/// The derived MCP read surface (row A29), generated from the snapshot plus
/// `packages/sdk/mcp-catalog.config.json`. Core-api reads the same artifact
/// for its scope vocabulary and api-key allowance, so the adapter and the
/// server cannot disagree about what exists.
pub const MCP_CATALOG: &str = include_str!("../../../packages/sdk/mcp-catalog.json");

/// Parses the embedded snapshot.
pub fn openapi_spec() -> serde_json::Value {
    serde_json::from_str(OPENAPI_SNAPSHOT).expect("committed openapi.json parses")
}

/// Parses the embedded derived catalog.
pub fn mcp_catalog_spec() -> serde_json::Value {
    serde_json::from_str(MCP_CATALOG).expect("committed mcp-catalog.json parses")
}

// Inclusion, sensitive-prefix exclusion and capability assignment live in
// the generated artifact (packages/sdk/mcp-catalog.json); the adapter no
// longer keeps its own copy of the rules.
