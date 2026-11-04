//! mcp-server — MCP adapter over the public API, built on `rmcp`.
//!
//! Purpose: expose the org's capability-granted READ surface over the Model
//! Context Protocol. The tool catalog derives MECHANICALLY from the committed
//! OpenAPI snapshot (`packages/sdk/openapi.json`): every safe read operation
//! (`GET` without sensitive paths) becomes one MCP tool whose input schema is
//! that operation's query parameters — zero hand-written tools, zero business
//! logic. The adapter holds no database: every tool call proxies core-api with
//! the caller's own session.
//!
//! Protocol work belongs to the official SDK (`rmcp` 3.x), which implements
//! the stable 2026-07-28 revision and remains compatible with 2025-11-25 and
//! earlier. Framing, version negotiation and transport semantics are the
//! SDK's; this crate owns the catalog, the capability filter and the proxy.
//!
//! Two transports:
//! - stdio JSON-RPC (the canonical MCP transport), the caller's token in
//!   `MARULA_API_TOKEN`;
//! - Streamable HTTP at `/mcp`, the caller's token in the request's
//!   `Authorization: Bearer` header.
//!
//! Three surfaces, one rule:
//!
//! - **tools** — every safe read operation in the snapshot;
//! - **resources** — `marula://tickets/{id}`, `marula://clients/{id}` and
//!   `marula://documents/{id}`, each resolving through the same catalog tool as
//!   the equivalent call, so the path, the capability and the core-api check
//!   are the same on both surfaces;
//! - **prompts** — in-code workflow templates, listed only when the caller can
//!   perform the first read the workflow needs.
//!
//! Tasks are not advertised: the revision defines them as an extension with
//! its own server-side lifecycle, and nothing here needs to poll one yet.
//!
//! Capability registry: a capability slug the org has NOT granted is
//! deny-by-default — a tool, resource template or prompt whose capability is
//! not granted is hidden from its list method and refuses a direct call.
//! Core-api's RBAC still decides every read, defense in depth.
//!
//! Write tools: intentionally ABSENT in v1. Agent-authored record changes
//! enter through the IF8 proposal pipeline (`POST /api/agent-proposals`,
//! accepted from `x-api-key` callers) and apply only after human approval
//! through the IF1 path — never through a direct MCP write tool.
//!
//! Non-purpose: no direct database access (it calls core-api exclusively), no
//! custom tool implementations, no adapter credential.

pub mod api;
pub mod catalog;
pub mod http;
pub mod marula;
pub mod prompts;
pub mod resources;
pub mod stdio;

pub use marula::MarulaServer;

/// Default core-api base URL when `MARULA_API_BASE` is unset.
pub const DEFAULT_API_BASE: &str = "http://127.0.0.1:8080";

/// The committed OpenAPI snapshot, embedded at build time: the tool
/// catalog is pinned to the reviewed spec, never to a mutable runtime
/// file (the binary does not trust its CWD).
pub const OPENAPI_SNAPSHOT: &str = include_str!("../../../packages/sdk/openapi.json");

/// The derived MCP read surface (row A29), generated from the snapshot plus
/// `packages/sdk/mcp-catalog.config.json`. Core-api reads the same artifact for
/// its capability registry, so the adapter and the server cannot disagree about
/// what exists.
pub const MCP_CATALOG: &str = include_str!("../../../packages/sdk/mcp-catalog.json");

/// Parses the embedded snapshot.
pub fn openapi_spec() -> serde_json::Value {
    serde_json::from_str(OPENAPI_SNAPSHOT).expect("committed openapi.json parses")
}

/// Parses the embedded derived catalog.
pub fn mcp_catalog_spec() -> serde_json::Value {
    serde_json::from_str(MCP_CATALOG).expect("committed mcp-catalog.json parses")
}
