//! mcp-server — MCP protocol adapter over the public API.
//!
//! Purpose: expose capability-scoped domain tools over the Model Context
//! Protocol, derived mechanically from the OpenAPI surface + capability
//! registry at build time — zero hand-written tools, zero business logic
//! (docs/10-agent-platform.md §1). Reads land with MCP v1 (Phase 3); tool
//! writes are Stage 4–5 behind layered rules evaluation.
//!
//! Non-purpose: no direct database access (it calls core-api exclusively),
//! no custom tool implementations, no auth decisions of its own (delegates
//! to API keys/capability scopes).
//!
//! Status: scaffolded, unimplemented — TASKLIST Phase 3 (MCP server crate
//! v1 line). Home reserved so the boundary stays documented before code.
