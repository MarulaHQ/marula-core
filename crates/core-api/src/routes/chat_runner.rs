//! ChatRunner (ADR 0008 §3, Wave 18 lane 2): the agent harness's model
//! loop, ONE runner consumed by all AI-assisted surfaces.
//!
//! Purpose:
//! - Resolve the caller through the SAME authorization pipeline lane 1
//!   built (`ai_policy::authorize_surface_call`: deny-by-default surface
//!   grant → allowlisted model → caps → sealed credential slot) — the
//!   runner shares the completion edge; there is no shadow re-wiring.
//! - Derive the tool wardrobe from the COMMITTED OpenAPI snapshot (the
//!   mcp-server adapter's drawing rules: GET operations only, sensitive
//!   prefixes excluded, capability `<ns>:read` per path) and gate every
//!   offered tool against the org's `mcp_capability_grants`
//!   (deny-by-default). WRITES never enter the wardrobe — agent-authored
//!   record changes stay IF8 proposals with the approval-default dial.
//! - Run the bounded tool loop: model call → tool-result feedback →
//!   iterate to completion, at most [`MAX_LOOP_STEPS`] model turns, every
//!   loop frame streaming to the client over SSE. SSE (not WS) because
//!   the loop is request-scoped and starts/ends with the POST — the
//!   realtime WS fan-out rides the worker's outbox stream, which cannot
//!   stream a live in-process loop, and lane 1's edge is REST.
//! - Every model call AND every tool call lands in the append-only
//!   `ai_usage_ledger` (tool rows carry zero tokens — they are reads) and
//!   the provenance audit (`Actor::Agent` rows naming the for_user) —
//!   docs/10 §5/§7: every invocation audited, limits enforced host-side.
//! - In-editor surfaces consume the SAME runner scoped to their grant:
//!   `ticket_note_assist` files the draft as an IF8 proposal (dial
//!   captured at filing, approval default, nothing lands unapproved);
//!   `doc_drafting` / `email_drafting` receive a `suggested_insert`
//!   frame — the client inserts the draft into the user's editor, no
//!   server write at all.
//!
//! Non-purpose: NO new migration (`session_id` is client-borne — the
//! runner persists no transcript, history rides the request body); NO
//! provider-side token streaming (model calls are single-shot inside the
//! loop — the loop FRAMING is what streams); NO provider keys or network
//! calls in tests (deterministic scripted transports, the BYOK-injection
//! posture); no OAuth/subscription chat transport this lane (typed
//! degradation instead of a guess).
//!
//! Honest gap (landing record): the wardrobe's path rules
//! (`capability_ns`/`is_excluded`) are reviewed twins of the mcp-server
//! adapter's, not shared code — until a shared crate extracts them.
//!
//! Review items SETTLED in lane 2b (postures documented where they live,
//! not left in landing notes): provider-side token streaming stays off
//! (posture above); OAuth/subscription chat transport keeps its typed
//! degradation with the generic OAuth flow on ADR 0008 §2's trigger; the
//! wardrobe twin extraction point is pinned at [`derive_wardrobe`]; the
//! transport-override/router seam carries a production review gate (see
//! `resolve_transport`); the turn limiter's single-node posture is stated
//! at [`turn_rate_check`].

use crate::auth::middleware::authed;
use crate::error::{ApiError, ApiResult};
use crate::routes::ai_policy::{authorize_surface_call, AuthorizedAiCall};
use crate::state::AppState;
use async_trait::async_trait;
use axum::{
    extract::{Path, State},
    http::{Extensions, HeaderMap},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use core_db::ai_policy::{record_usage, AiUsageNew};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    convert::Infallible,
    pin::Pin,
    sync::{Arc, Mutex},
    task::Poll,
    time::Duration,
};
use tower::ServiceExt;
use utoipa::ToSchema;
use uuid::Uuid;

/// Host-side loop bound (docs/10 §7.4): at most this many MODEL turns per
/// request; a bound hit ends the turn honestly — a runaway loop is a bug,
/// not a strategy.
pub const MAX_LOOP_STEPS: usize = 8;
/// Chat turns one (org, user) may start per sliding window.
pub const TURN_RATE_LIMIT: usize = 20;
const TURN_RATE_WINDOW_SECS: u64 = 60;
const MAX_PROXY_BODY: usize = 1_048_576;

const SESSION_ENTITY: &str = "ai_chat_session";
const TURN_ENTITY: &str = "ai_chat_turn";
const TOOL_ENTITY: &str = "ai_chat_tool_invocation";

/// The committed OpenAPI snapshot — the reviewed byte-identical pin pair
/// (`packages/sdk/openapi.json`), never a mutable runtime file.
const OPENAPI_SNAPSHOT: &str = include_str!("../../../../packages/sdk/openapi.json");

// ---------------------------------------------------------------------------
// Wardrobe — MCP-catalog-derived READ tools gated by the capability registry
// ---------------------------------------------------------------------------

/// One read tool the runner may offer the model.
#[derive(Debug, Clone)]
pub struct WardrobeTool {
    /// The spec's `operationId` — how the model names the tool.
    pub name: String,
    /// Spec path, canonical `/api/v1` form, `{param}` templated.
    pub path: String,
    /// `<namespace>:read` — the capability gate binding invocation.
    pub capability: String,
    /// Input schema (path + query parameters).
    pub input_schema: serde_json::Value,
    /// Path parameter names in path order.
    pub path_params: Vec<String>,
}

/// Derives the READ wardrobe from the pinned spec, mirroring the
/// mcp-server catalog adapter's rules. Pure, unit-tested.
/// EXTRACTION POINT (Wave 18 lane 2b): the path-rule twins with the
/// mcp-server adapter (`capability_ns`, `is_excluded`, and the
/// GET/deny logic below) get their shared crate home HERE — when the
/// second consumer appears, lift these rules into a shared crate and
/// point both call sites at it; re-check the ADR re-eval triggers then.
pub fn derive_wardrobe() -> Vec<WardrobeTool> {
    let Ok(spec) = serde_json::from_str::<serde_json::Value>(OPENAPI_SNAPSHOT) else {
        return Vec::new();
    };
    let Some(paths) = spec.get("paths").and_then(|p| p.as_object()) else {
        return Vec::new();
    };
    let mut tools = Vec::new();
    for (path, item) in paths {
        if is_excluded(path) {
            continue;
        }
        let Some(op) = item.get("get").and_then(|g| g.as_object()) else {
            continue;
        };
        let Some(operation_id) = op.get("operationId").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(ns) = capability_ns(path) else {
            continue;
        };
        let description = op
            .get("summary")
            .and_then(|v| v.as_str())
            .or_else(|| op.get("description").and_then(|v| v.as_str()))
            .unwrap_or(operation_id)
            .to_string();
        let mut properties = serde_json::Map::new();
        let mut required: Vec<String> = Vec::new();
        if let Some(params) = op.get("parameters").and_then(|v| v.as_array()) {
            for param in params {
                let (Some(name), Some(location)) = (
                    param.get("name").and_then(|v| v.as_str()),
                    param.get("in").and_then(|v| v.as_str()),
                ) else {
                    continue;
                };
                properties.insert(
                    name.to_string(),
                    param
                        .get("schema")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({ "type": "string" })),
                );
                if location == "path"
                    || param.get("required").and_then(|v| v.as_bool()) == Some(true)
                {
                    required.push(name.to_string());
                }
            }
        }
        tools.push(WardrobeTool {
            name: operation_id.to_string(),
            path: path.to_string(),
            capability: format!("{ns}:read"),
            input_schema: serde_json::json!({
                "type": "object",
                "description": description,
                "properties": properties,
                "required": required,
            }),
            path_params: path
                .split('/')
                .filter_map(|seg| {
                    seg.strip_prefix('{')
                        .map(|raw| raw.trim_end_matches('}').to_string())
                })
                .collect(),
        });
    }
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    tools
}

fn capability_ns(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/api/")?;
    let rest = rest.strip_prefix("v1/").unwrap_or(rest);
    let ns = rest.split('/').next()?;
    if ns.is_empty() {
        None
    } else {
        Some(ns.to_string())
    }
}

fn is_excluded(path: &str) -> bool {
    const EXCLUDED: &[&str] = &[
        "/api/v1/attachments",
        "/api/v1/org/exports",
        "/api/v1/webhooks",
        "/api/v1/api-keys",
        "/api/v1/auth",
        "/api/v1/email",
        "/api/v1/internal",
        "/api/v1/portal",
    ];
    EXCLUDED.iter().any(|p| path.starts_with(p)) || path.contains("/attachments")
}

// ---------------------------------------------------------------------------
// Router handle — tool self-calls ride the REAL composed router with the
// caller's bearer, so every execution runs the identical auth/RBAC/RLS/
// audit pipeline as a human request. build_router stamps its clone on every
// build; production builds exactly one router per process.
// ---------------------------------------------------------------------------

static RUNNER_ROUTER: Mutex<Option<axum::Router>> = Mutex::new(None);

pub fn set_runner_router(router: axum::Router) {
    if let Ok(mut slot) = RUNNER_ROUTER.lock() {
        *slot = Some(router);
    }
}

fn current_router() -> Option<axum::Router> {
    RUNNER_ROUTER.lock().ok().and_then(|s| s.clone())
}

// ---------------------------------------------------------------------------
// The model transport seam (CompletionProvider doctrine, chat half)
// ---------------------------------------------------------------------------

/// One conversation message (`system` | `user` | `assistant` | `tool`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LoopMessage {
    pub role: String,
    pub content: String,
    /// For `assistant` messages that requested tools: the provider-wire
    /// tool_calls round-trip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Object, nullable = true)]
    pub tool_calls: Option<serde_json::Value>,
    /// For `tool` messages: the tool call id being answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = String, nullable = true)]
    pub tool_call_id: Option<String>,
}

/// One loop step's ask to the transport.
#[derive(Debug, Clone)]
pub struct ModelCallRequest {
    pub model: Option<String>,
    pub messages: Vec<LoopMessage>,
    /// OpenAI-format function definitions — only granted tools ride.
    pub tools: Vec<serde_json::Value>,
}

/// One tool request the model made.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallReq {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// One loop step's output + spend.
#[derive(Debug, Clone, Default)]
pub struct ModelTurn {
    pub content: String,
    pub tool_calls: Vec<ToolCallReq>,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatTransportError {
    #[error("model transport unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("model transport rate limited: {reason}")]
    RateLimited { reason: String },
    #[error("model transport backend error: {reason}")]
    Backend { reason: String },
}

/// The model-call seam, same doctrine as the CompletionProvider edge:
/// implementations never log prompts or key material. Provider token
/// streaming is deliberately out of the trait — the runner streams loop
/// framing to clients.
#[async_trait]
pub trait ChatModelTransport: Send + Sync {
    async fn call(&self, req: &ModelCallRequest) -> Result<ModelTurn, ChatTransportError>;

    fn kind(&self) -> &str {
        "openai_compatible"
    }
}

/// Deterministic scripted transport — TEST support only. Org-keyed so
/// parallel flow tests in one binary never observe each other's scripts.
#[derive(Default)]
pub struct ScriptedTransport {
    turns: Mutex<VecDeque<ModelTurn>>,
}

impl ScriptedTransport {
    pub fn new(turns: Vec<ModelTurn>) -> Self {
        Self {
            turns: Mutex::new(turns.into_iter().collect()),
        }
    }
}

#[async_trait]
impl ChatModelTransport for ScriptedTransport {
    async fn call(&self, _req: &ModelCallRequest) -> Result<ModelTurn, ChatTransportError> {
        self.turns
            .lock()
            .expect("scripted transport lock poisoned")
            .pop_front()
            .ok_or_else(|| ChatTransportError::Unavailable {
                reason: "scripted transport exhausted (test bug)".to_string(),
            })
    }

    fn kind(&self) -> &str {
        "scripted_test"
    }
}

static TRANSPORT_OVERRIDES: std::sync::LazyLock<Mutex<HashMap<Uuid, Arc<dyn ChatModelTransport>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Seeds a deterministic transport for one org (test support only —
/// reaching this from production code is a review red flag).
pub fn seed_transport_for_org(org_id: Uuid, transport: Arc<dyn ChatModelTransport>) {
    TRANSPORT_OVERRIDES
        .lock()
        .expect("transport overrides lock poisoned")
        .insert(org_id, transport);
}

/// OpenAI-compatible `/chat/completions` adapter — the real transport,
/// built from the winning sealed credential (lane 1's slot) + the
/// provider's endpoint convention (`OPENAI_BASE_URL` / Ollama compat).
pub struct OpenAiChatCompat {
    http: reqwest::Client,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

#[derive(serde::Deserialize)]
struct ChatCompletionWire {
    choices: Vec<ChoiceWire>,
    usage: Option<ChatUsageWire>,
}

#[derive(serde::Deserialize)]
struct ChatUsageWire {
    prompt_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens: Option<u32>,
}

#[derive(serde::Deserialize)]
struct ChoiceWire {
    message: MessageWire,
}

#[derive(serde::Deserialize)]
struct MessageWire {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallWire>,
}

#[derive(serde::Deserialize)]
struct ToolCallWire {
    id: String,
    function: FunctionWire,
}

#[derive(serde::Deserialize)]
struct FunctionWire {
    name: String,
    #[serde(default)]
    arguments: Option<String>,
}

/// Parses an OpenAI-compatible chat-completions body into the loop shape.
/// Pure — unit-tested without I/O. Malformed provider output is a typed
/// backend failure, never a panic.
pub fn parse_chat_completion_turn(
    body: &serde_json::Value,
) -> Result<ModelTurn, ChatTransportError> {
    let wire: ChatCompletionWire =
        serde_json::from_value(body.clone()).map_err(|e| ChatTransportError::Backend {
            reason: format!("malformed chat-completions response: {e}"),
        })?;
    let Some(choice) = wire.choices.first() else {
        return Err(ChatTransportError::Backend {
            reason: "chat-completions response carried no choices".to_string(),
        });
    };
    Ok(ModelTurn {
        content: choice.message.content.clone().unwrap_or_default(),
        tool_calls: choice
            .message
            .tool_calls
            .iter()
            .map(|tc| ToolCallReq {
                id: tc.id.clone(),
                name: tc.function.name.clone(),
                arguments: tc
                    .function
                    .arguments
                    .as_deref()
                    .and_then(|raw| serde_json::from_str(raw).ok())
                    .unwrap_or_else(|| serde_json::json!({})),
            })
            .collect(),
        prompt_tokens: wire
            .usage
            .as_ref()
            .and_then(|u| u.prompt_tokens)
            .unwrap_or(0),
        completion_tokens: wire
            .usage
            .as_ref()
            .and_then(|u| u.completion_tokens)
            .unwrap_or(0),
    })
}

#[async_trait]
impl ChatModelTransport for OpenAiChatCompat {
    async fn call(&self, req: &ModelCallRequest) -> Result<ModelTurn, ChatTransportError> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let messages: Vec<serde_json::Value> = req
            .messages
            .iter()
            .map(|m| {
                let mut wire = serde_json::json!({ "role": m.role, "content": m.content });
                if let Some(calls) = &m.tool_calls {
                    wire["tool_calls"] = calls.clone();
                }
                if let Some(id) = &m.tool_call_id {
                    wire["tool_call_id"] = serde_json::json!(id);
                }
                wire
            })
            .collect();
        let mut body = serde_json::json!({
            "model": req.model.as_deref().unwrap_or(&self.model),
            "messages": messages,
        });
        if !req.tools.is_empty() {
            body["tools"] = serde_json::json!(req
                .tools
                .iter()
                .map(|t| serde_json::json!({ "type": "function", "function": t }))
                .collect::<Vec<_>>());
        }
        let response = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| ChatTransportError::Backend {
                reason: format!("chat-completions request failed: {e}"),
            })?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ChatTransportError::RateLimited {
                reason: format!("model provider returned {status}"),
            });
        }
        if !status.is_success() {
            return Err(ChatTransportError::Backend {
                reason: format!("model provider returned {status}"),
            });
        }
        let body: serde_json::Value =
            response
                .json()
                .await
                .map_err(|e| ChatTransportError::Backend {
                    reason: format!("chat-completions response was not JSON: {e}"),
                })?;
        parse_chat_completion_turn(&body)
    }
}

/// Transport resolution: the test override wins; otherwise the real
/// OpenAI-compatible adapter built from the authorized credential.
/// Credential kinds with no adapter degrade typed instead of guessing.
///
/// PRODUCTION REVIEW GATE (Wave 18 lane 2b, review item settled): this
/// function IS the transport-override/router seam — `TRANSPORT_OVERRIDES`
/// (org-keyed, test-only injection) sits in front of the credential-built
/// adapter, and `current_router`/`set_runner_router` hands the loop a
/// CLONED router so tool calls re-enter the same HTTP stack as clients.
/// Before any GA multi-instance deployment, this seam must be re-reviewed
/// for: override registration being test-only (no admin path may write
/// TRANSPORT_OVERRIDES), router stamping happening on every worker, and
/// the env-var base URLs (`OPENAI_BASE_URL`) being pinned per deployment.
fn resolve_transport(org_id: Uuid, call: &AuthorizedAiCall) -> Arc<dyn ChatModelTransport> {
    if let Some(seeded) = TRANSPORT_OVERRIDES
        .lock()
        .expect("transport overrides lock poisoned")
        .get(&org_id)
    {
        return seeded.clone();
    }
    match (call.credential_kind.as_str(), call.provider.as_str()) {
        ("api_key", "openai") => Arc::new(OpenAiChatCompat {
            http: reqwest::Client::new(),
            base_url: std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string()),
            api_key: String::from_utf8_lossy(&call.plaintext_secret).to_string(),
            model: call.model.clone(),
        }),
        ("api_key", "ollama") => Arc::new(OpenAiChatCompat {
            http: reqwest::Client::new(),
            base_url: std::env::var("OLLAMA_BASE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:11434/v1".to_string()),
            api_key: String::from_utf8_lossy(&call.plaintext_secret).to_string(),
            model: call.model.clone(),
        }),
        (kind, provider) => Arc::new(UnsupportedKindTransport {
            kind: format!("{kind}:{provider}"),
        }),
    }
}

/// Stands in when the winning sealed credential kind (or provider) has no
/// chat adapter this lane: the turn degrades TYPED, never 5xx.
struct UnsupportedKindTransport {
    kind: String,
}

#[async_trait]
impl ChatModelTransport for UnsupportedKindTransport {
    async fn call(&self, _req: &ModelCallRequest) -> Result<ModelTurn, ChatTransportError> {
        Err(ChatTransportError::Unavailable {
            reason: format!(
                "no chat transport for '{}' this lane (api_key + openai/ollama only)",
                self.kind
            ),
        })
    }

    fn kind(&self) -> &str {
        "unsupported"
    }
}

// ---------------------------------------------------------------------------
// Host-side turn rate limiter (docs/10 §7.3/§7.4) — sliding 60s window per
// (org, user), enforced host-side, not by convention.
// ---------------------------------------------------------------------------

/// (org, user) → the instants their chat turns started at.
type TurnWindow = HashMap<(Uuid, Uuid), VecDeque<std::time::Instant>>;

static TURN_RATE: std::sync::LazyLock<Mutex<TurnWindow>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// `Ok(())` under [`TURN_RATE_LIMIT`], `Err(retry_after_secs)` otherwise.
pub fn turn_rate_check(org_id: Uuid, user_id: Uuid) -> Result<(), u64> {
    // POSTURE (Wave 18 lane 2b, review item settled): this limiter is a
    // per-PROCESS sliding window — the honest SINGLE-NODE alpha posture.
    // Multi-node deployments need a shared store (Postgres or Redis) for
    // the same (org, user) window, or the effective limit multiplies by
    // replica count. Re-evaluate together with the runner's router
    // stamping seam (see `resolve_transport`'s review gate) and the ADR
    // 0010 posture review; do not silently rely on this bound at scale.
    let mut map = TURN_RATE.lock().expect("turn rate lock poisoned");
    let window = map.entry((org_id, user_id)).or_default();
    let now = std::time::Instant::now();
    let horizon = Duration::from_secs(TURN_RATE_WINDOW_SECS);
    while let Some(front) = window.front() {
        if now.duration_since(*front) <= horizon {
            break;
        }
        window.pop_front();
    }
    if window.len() >= TURN_RATE_LIMIT {
        let oldest = window
            .front()
            .map(|front| now.duration_since(*front))
            .unwrap_or_default();
        return Err(horizon.saturating_sub(oldest).as_secs().max(1));
    }
    window.push_back(now);
    Ok(())
}

// ---------------------------------------------------------------------------
// Wire types + endpoint
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChatTurnRequest {
    pub surface_key: String,
    /// The conversation so far (system prompt first). The client owns the
    /// transcript; the runner persists none of it.
    pub messages: Vec<LoopMessage>,
    /// Optional explicit (provider, model) ask — must sit inside the
    /// admin allowlist, exactly like the authorize edge.
    #[serde(default)]
    #[schema(value_type = String, nullable = true)]
    pub provider: Option<String>,
    #[serde(default)]
    #[schema(value_type = String, nullable = true)]
    pub model: Option<String>,
    /// `ticket_note_assist` only: the ticket the assist drafts against
    /// (the IF8 proposal target).
    #[serde(default)]
    #[schema(value_type = String, nullable = true)]
    pub ticket_id: Option<Uuid>,
    /// True only on a session's first turn — the
    /// `ai.chat_session_started` event rides that turn alone.
    #[serde(default)]
    pub session_opener: bool,
}

const FRAME_TURN_STARTED: &str = "turn_started";
const FRAME_ASSISTANT_DELTA: &str = "assistant_delta";
const FRAME_TOOL_INVOCATION: &str = "tool_invocation";
const FRAME_ASSIST_PROPOSED: &str = "assist_proposed";
const FRAME_TURN_COMPLETED: &str = "turn_completed";

/// POST /ai/chat/sessions/{session_id}/turns — run ONE bounded model turn
/// on a granted surface, streaming loop frames over `text/event-stream`.
/// Refusals (surface/model/cap/credential/agent capability), the hard cap
/// and the rate limit answer typed BEFORE any stream begins — clients
/// branch on codes without parsing frames.
#[utoipa::path(
    operation_id = "chat_runner_turn_post",
    post,
    path = "/api/ai/chat/sessions/{session_id}/turns",
    tag = "ai",
    security(("bearer" = [])),
    params(("session_id" = Uuid, Path)),
    request_body = ChatTurnRequest,
    responses(
        (status = 200, description = "SSE stream of loop frames (text/event-stream)"),
        (status = 403, description = "typed refusal: ai_surface_not_granted | ai_surface_disabled | ai_surface_internal | ai_model_not_allowed | ai_no_models_configured | ai_hard_cap_reached | ai_no_credential | ai_agent_capability_not_granted"),
        (status = 400, description = "validation"),
        (status = 429, description = "chat turn rate limit")
    )
)]
pub async fn post_turn(
    State(state): State<AppState>,
    extensions: Extensions,
    headers: HeaderMap,
    Path(session_id): Path<Uuid>,
    Json(req): Json<ChatTurnRequest>,
) -> ApiResult<Response> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let surface_key = req.surface_key.trim().to_string();
    if surface_key.is_empty() {
        return Err(ApiError::Validation("surface_key is required".into()));
    }
    if req.messages.is_empty() {
        return Err(ApiError::Validation("messages must not be empty".into()));
    }
    // Host-side rate limit BEFORE anything runs.
    if let Err(retry_after_secs) = turn_rate_check(org_id, user_id) {
        return Err(ApiError::RateLimited {
            retry_after_secs: retry_after_secs as i64,
        });
    }

    // THE lane-1 authorization edge — surface grant → model → caps →
    // sealed credential, once, typed. Capability enforcement is
    // load-bearing (lane 2b): the resolved model must carry `completion`,
    // plus `tools` whenever the wardrobe is non-empty (a model that
    // cannot call tools never enters the tool loop).
    let wardrobe = derive_wardrobe();
    let mut required_edges = vec![crate::routes::ai_policy::AiEdge::Completion];
    if !wardrobe.is_empty() {
        required_edges.push(crate::routes::ai_policy::AiEdge::ToolCall);
    }
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let surface_kind = core_db::ai_policy::get_surface_by_key(&mut *tx, org_id, &surface_key)
        .await
        .map_err(ApiError::from)?
        .map(|s| s.kind)
        .unwrap_or_default();
    let call = authorize_surface_call(
        &state,
        &mut tx,
        crate::routes::ai_policy::AiAdmissionAsk {
            org_id,
            user_id,
            surface_key: &surface_key,
            provider_ask: req.provider.as_deref(),
            model_ask: req.model.as_deref(),
            required_capabilities: &required_edges,
            internal_core_call: false,
        },
    )
    .await?;

    // Assist admit (in-editor surface rides the SAME runner):
    // ticket_note_assist needs a real ticket of the org + a granted
    // `tickets:write` capability (deny-by-default).
    let mut assist_target: Option<(Uuid, i64, &'static str)> = None;
    if surface_key == "ticket_note_assist" {
        let ticket_id = req.ticket_id.ok_or_else(|| {
            ApiError::Validation("ticket_id is required for ticket_note_assist".into())
        })?;
        let grant =
            core_db::agent_platform::get_capability_grant(&mut *tx, org_id, "tickets:write")
                .await
                .map_err(ApiError::from)?;
        let dial_mode = dial_mode_of(grant.as_ref().map(|g| g.mode.as_str())).ok_or(
            ApiError::AiCallRefused {
                reason: "ai_agent_capability_not_granted",
                message: "the ticket_note_assist files ONLY through the agent proposal \
                          pipeline, which needs a granted 'tickets:write' capability"
                    .into(),
                surface: surface_key.clone(),
            },
        )?;
        let base_version =
            core_db::agent_platform::entity_current_seq(&mut *tx, org_id, "ticket", ticket_id)
                .await
                .map_err(ApiError::from)?;
        // The ticket must exist for the proposal's FK and the approval
        // diff to be real.
        core_db::tickets::get_ticket(&mut *tx, org_id, ticket_id)
            .await
            .map_err(ApiError::from)?;
        assist_target = Some((ticket_id, base_version, dial_mode));
    }
    tx.commit().await.map_err(ApiError::from)?;

    let transport = resolve_transport(org_id, &call);
    let Some(router) = current_router() else {
        return Err(ApiError::Internal(anyhow::anyhow!(
            "the runner router was never stamped; build_router must call set_runner_router"
        )));
    };
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .map(|v| v.to_str().unwrap_or_default().to_string())
        .unwrap_or_default();

    let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);
    let run = RunState {
        state,
        session_id,
        org_id,
        user_id,
        call,
        surface_kind,
        transport,
        wardrobe,
        router,
        bearer,
        assist_target,
        initial_messages: req.messages,
        session_opener: req.session_opener,
    };
    tokio::spawn(run_turn(run, sender));
    Ok(Sse::new(RunnerStream { receiver })
        .keep_alive(KeepAlive::default())
        .into_response())
}

fn dial_mode_of(mode: Option<&str>) -> Option<&'static str> {
    match mode {
        Some("proposed") => Some("proposed"),
        Some("autonomous") => Some("autonomous"),
        _ => None,
    }
}

struct RunState {
    state: AppState,
    session_id: Uuid,
    org_id: Uuid,
    user_id: Uuid,
    call: AuthorizedAiCall,
    #[allow(dead_code)] // kept: assist scoping reads it in later legs
    surface_kind: String,
    transport: Arc<dyn ChatModelTransport>,
    wardrobe: Vec<WardrobeTool>,
    router: axum::Router,
    bearer: String,
    assist_target: Option<(Uuid, i64, &'static str)>,
    initial_messages: Vec<LoopMessage>,
    session_opener: bool,
}

/// Bridges the tokio mpsc receiver into the SSE response's stream.
struct RunnerStream {
    receiver: tokio::sync::mpsc::Receiver<Result<Event, Infallible>>,
}

impl futures_util::Stream for RunnerStream {
    type Item = Result<Event, Infallible>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.receiver).poll_recv(cx)
    }
}

async fn emit(
    sender: &tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    event: &'static str,
    data: serde_json::Value,
) {
    let _ = sender
        .send(Ok(Event::default().event(event).data(data.to_string())))
        .await;
}

/// One record of a tool invocation.
struct ToolInvocation {
    invocation_id: Uuid,
    capability: String,
    outcome: &'static str,
    /// What the loop feeds back to the model (typed refusal or the read).
    tool_result: String,
}

/// The bounded loop. Every observable step emits BOTH an SSE frame and
/// (via the transactional outbox) its contract event; every model or
/// tool call lands its usage ledger + provenance rows. Failures degrade
/// typed — a human conversation never 5xxs.
async fn run_turn(st: RunState, sender: tokio::sync::mpsc::Sender<Result<Event, Infallible>>) {
    let surface_key = st.call.surface_key.clone();
    let provider = st.call.provider.clone();
    let model = st.call.model.clone();
    let turn_id = Uuid::now_v7();

    emit(
        &sender,
        FRAME_TURN_STARTED,
        serde_json::json!({
            "session_id": st.session_id,
            "turn_id": turn_id,
            "surface_key": surface_key,
            "provider": provider,
            "model": model,
            "resolution": st.call.resolution,
            "warnings": st.call.warnings,
        }),
    )
    .await;

    if st.session_opener {
        emit_outbox_event(
            &st.state,
            st.org_id,
            st.user_id,
            types::AI_CHAT_SESSION_STARTED,
            SESSION_ENTITY,
            Some(st.session_id.to_string().as_str()),
            serde_json::json!({
                "session_id": st.session_id,
                "surface_key": surface_key,
                "turn_id": turn_id,
            }),
        )
        .await
        .ok();
    }

    let mut messages = st.initial_messages.clone();
    let mut steps = 0usize;
    let mut total_prompt: u32 = 0;
    let mut total_completion: u32 = 0;
    let mut outcome: &str = "completed";
    let mut error: serde_json::Value = serde_json::Value::Null;

    loop {
        if steps >= MAX_LOOP_STEPS {
            outcome = "step_limit_reached";
            break;
        }
        let mut allowed_tools = Vec::new();
        for tool in &st.wardrobe {
            if matches!(tool_granted(&st, &tool.capability).await, Ok(true)) {
                allowed_tools.push(serde_json::json!({
                    "name": tool.name,
                    "description": tool.name,
                    "parameters": tool.input_schema,
                }));
            }
        }
        let ask = ModelCallRequest {
            model: Some(model.clone()),
            messages: messages.clone(),
            tools: allowed_tools,
        };
        let step_tag = format!("turn:{turn_id}:step:{steps}");
        match st.transport.call(&ask).await {
            Ok(turn) => {
                total_prompt += turn.prompt_tokens;
                total_completion += turn.completion_tokens;
                record_call(&st, &step_tag, turn.prompt_tokens, turn.completion_tokens)
                    .await
                    .ok();
                steps += 1;
                if !turn.content.is_empty() {
                    emit(
                        &sender,
                        FRAME_ASSISTANT_DELTA,
                        serde_json::json!({
                            "turn_id": turn_id,
                            "step": steps,
                            "delta": turn.content,
                        }),
                    )
                    .await;
                }
                messages.push(LoopMessage {
                    role: "assistant".into(),
                    content: turn.content.clone(),
                    tool_calls: if turn.tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls_wire(&turn.tool_calls))
                    },
                    tool_call_id: None,
                });
                if turn.tool_calls.is_empty() {
                    break;
                }
                for call_req in turn.tool_calls {
                    let invocation = invoke_tool(&st, turn_id, &call_req).await;
                    emit(
                        &sender,
                        FRAME_TOOL_INVOCATION,
                        serde_json::json!({
                            "turn_id": turn_id,
                            "tool": call_req.name,
                            "capability": invocation.capability,
                            "outcome": invocation.outcome,
                        }),
                    )
                    .await;
                    emit_outbox_event(
                        &st.state,
                        st.org_id,
                        st.user_id,
                        types::AI_CHAT_TOOL_INVOKED,
                        TOOL_ENTITY,
                        Some(invocation.invocation_id.to_string().as_str()),
                        serde_json::json!({
                            "session_id": st.session_id,
                            "turn_id": turn_id,
                            "tool": call_req.name,
                            "capability": invocation.capability,
                            "outcome": invocation.outcome,
                        }),
                    )
                    .await
                    .ok();
                    messages.push(LoopMessage {
                        role: "tool".into(),
                        content: invocation.tool_result,
                        tool_calls: None,
                        tool_call_id: Some(call_req.id.clone()),
                    });
                }
            }
            Err(e) => {
                // Provider-side 429 is surfaced typed (degrade-never-5xx).
                outcome = match e {
                    ChatTransportError::RateLimited { reason } => {
                        error = serde_json::json!({
                            "event": "model_rate_limited",
                            "reason": reason,
                        });
                        "model_rate_limited"
                    }
                    _ => {
                        error = serde_json::json!({
                            "event": "model_transport_failed",
                            "reason": e.to_string(),
                        });
                        "transport_failed"
                    }
                };
                break;
            }
        }
        // Mid-loop cap rebound: usage grew inside the run.
        if let Some(hard) = st.call.hard_limit {
            let used = core_db::ai_policy::user_usage_since(
                &st.state.pool,
                st.org_id,
                st.user_id,
                month_start(chrono::Utc::now()),
            )
            .await
            .unwrap_or(i64::MAX);
            if used >= hard {
                outcome = "hard_cap_reached";
                break;
            }
        }
    }

    // Assist surfaces: the draft NEVER becomes a direct write.
    let final_answer = messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant" && m.tool_calls.is_none() && !m.content.is_empty())
        .map(|m| m.content.clone())
        .unwrap_or_default();

    let mut proposal_id: Option<Uuid> = None;
    if outcome == "completed" && !final_answer.is_empty() {
        if let Some((ticket_id, base_version, dial_mode)) = &st.assist_target {
            match file_ticket_note_proposal(
                &st,
                *ticket_id,
                *base_version,
                dial_mode,
                &final_answer,
            )
            .await
            {
                Ok(id) => {
                    proposal_id = Some(id);
                    emit(
                        &sender,
                        FRAME_ASSIST_PROPOSED,
                        serde_json::json!({
                            "turn_id": turn_id,
                            "proposal_id": id,
                            "ticket_id": ticket_id,
                        }),
                    )
                    .await;
                }
                Err(e) => {
                    outcome = "proposal_failed";
                    error = serde_json::json!({ "event": "proposal_filing_failed", "reason": e.to_string() });
                }
            }
        }
    }

    emit(
        &sender,
        FRAME_TURN_COMPLETED,
        serde_json::json!({
            "turn_id": turn_id,
            "session_id": st.session_id,
            "steps": steps,
            "prompt_tokens": total_prompt,
            "completion_tokens": total_completion,
            "outcome": outcome,
            "proposal_id": proposal_id,
            "error": error,
        }),
    )
    .await;

    emit_outbox_event(
        &st.state,
        st.org_id,
        st.user_id,
        types::AI_CHAT_TURN_COMPLETED,
        TURN_ENTITY,
        Some(turn_id.to_string().as_str()),
        serde_json::json!({
            "session_id": st.session_id,
            "turn_id": turn_id,
            "surface_key": surface_key,
            "provider": provider,
            "model": model,
            "prompt_tokens": total_prompt as i64,
            "completion_tokens": total_completion as i64,
            "steps": steps as i64,
            "outcome": outcome,
            "proposal_id": proposal_id,
        }),
    )
    .await
    .ok();
}

fn tool_calls_wire(calls: &[ToolCallReq]) -> serde_json::Value {
    serde_json::json!(calls
        .iter()
        .map(|tc| serde_json::json!({
            "id": tc.id,
            "type": "function",
            "function": { "name": tc.name, "arguments": tc.arguments.to_string() },
        }))
        .collect::<Vec<_>>())
}

/// Capability gate: the grant must EXIST (deny-by-default). Both dial
/// positions permit READS (the dial speaks to writes); the dial position
/// is captured on proposals at filing time.
async fn tool_granted(st: &RunState, capability: &str) -> Result<bool, core_db::DbError> {
    let mut tx = st.state.pool.begin().await?;
    core_db::set_org(&mut tx, st.org_id).await?;
    let granted = core_db::agent_platform::get_capability_grant(&mut *tx, st.org_id, capability)
        .await?
        .is_some();
    tx.commit().await?;
    Ok(granted)
}

/// Executes the model's tool request against the REAL router with the
/// caller's bearer — the same auth/RBAC/RLS path as a human GET.
async fn invoke_tool(st: &RunState, turn_id: Uuid, call_req: &ToolCallReq) -> ToolInvocation {
    let invocation_id = Uuid::now_v7();
    let step_tag = format!("turn:{turn_id}:tool:{}", call_req.name);
    let Some(tool) = st.wardrobe.iter().find(|t| t.name == call_req.name) else {
        return ToolInvocation {
            invocation_id,
            capability: call_req.name.clone(),
            outcome: "refused_unknown_tool",
            tool_result: "tool refused: not a wardrobe read tool".into(),
        };
    };
    let capability = tool.capability.clone();
    let granted = tool_granted(st, &capability).await.unwrap_or(false);
    if !granted {
        record_call(st, &step_tag, 0, 0).await.ok();
        return ToolInvocation {
            invocation_id,
            capability: capability.clone(),
            outcome: "refused_capability_not_granted",
            tool_result: format!(
                "tool refused: the '{capability}' capability is not granted for this org"
            ),
        };
    }
    // Fill `{param}` slots from the model's arguments; leftovers ride the
    // query string.
    let mut path = tool.path.clone();
    if let serde_json::Value::Object(map) = &call_req.arguments {
        for param in &tool.path_params {
            if let Some(value) = map.get(param).and_then(|v| v.as_str()) {
                path = path.replace(&format!("{{{param}}}"), value);
            }
        }
    }
    let query = match &call_req.arguments {
        serde_json::Value::Object(map) => map
            .iter()
            .filter(|(k, _)| !tool.path_params.contains(k))
            .map(|(k, v)| {
                let raw = v.as_str().unwrap_or(&v.to_string()).to_string();
                format!("{k}={raw}")
            })
            .collect::<Vec<_>>()
            .join("&"),
        _ => String::new(),
    };
    let uri = if query.is_empty() {
        path.clone()
    } else {
        format!("{path}?{query}")
    };
    let request = axum::http::Request::builder()
        .method("GET")
        .uri(&uri)
        .header(axum::http::header::AUTHORIZATION, st.bearer.clone())
        .body(axum::body::Body::empty())
        .expect("fixed GET request shape");
    let router_owned = st.router.clone();
    let tool_result = match router_owned.oneshot(request).await {
        Ok(response) => {
            let status = response.status().as_u16();
            let bytes = axum::body::to_bytes(response.into_body(), MAX_PROXY_BODY)
                .await
                .unwrap_or_default();
            let body: serde_json::Value =
                serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            serde_json::json!({ "status": status, "body": body }).to_string()
        }
        Err(_) => {
            record_call(st, &step_tag, 0, 0).await.ok();
            return ToolInvocation {
                invocation_id,
                capability,
                outcome: "tool_execution_failed",
                tool_result: "tool execution failed (router error)".into(),
            };
        }
    };
    record_call(st, &step_tag, 0, 0).await.ok();
    ToolInvocation {
        invocation_id,
        capability,
        outcome: "executed",
        tool_result,
    }
}

/// One usage-ledger row + one provenance audit row per call (model OR
/// tool). `request_id` rides step-unique; the lane-1 note ("dedupe is the
/// runner's job") is answered here: the loop never reuses a request_id,
/// so ledger reconstruction is exact.
async fn record_call(
    st: &RunState,
    request_id: &str,
    prompt_tokens: u32,
    completion_tokens: u32,
) -> anyhow::Result<()> {
    let mut tx = st.state.pool.begin().await?;
    core_db::set_org_and_user(&mut tx, st.org_id, st.user_id).await?;
    record_usage(
        &mut tx,
        &AiUsageNew {
            id: Uuid::now_v7(),
            org_id: st.org_id,
            user_id: st.user_id,
            surface: st.call.surface_key.clone(),
            provider: st.call.provider.clone(),
            model: st.call.model.clone(),
            prompt_tokens: prompt_tokens as i32,
            completion_tokens: completion_tokens as i32,
            cost_weight: st.call.cost_weight,
            request_id: Some(request_id.to_string()),
        },
    )
    .await?;
    core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id: st.org_id,
            actor: Actor::Agent(format!("chat_runner/user:{}", st.user_id)),
            action: "ai.chat_model_call".into(),
            entity_type: Some(TURN_ENTITY.into()),
            entity_id: Some(request_id.into()),
            detail: serde_json::json!({
                "surface": st.call.surface_key,
                "provider": st.call.provider,
                "model": st.call.model,
                "call": request_id,
                "for_user": st.user_id,
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
            }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// One transactional outbox insert per contract event (worker publishes).
async fn emit_outbox_event(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    event_type: &str,
    entity: &str,
    entity_id: Option<&str>,
    payload: serde_json::Value,
) -> ApiResult<()> {
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    core_db::outbox::insert(
        &mut tx,
        Uuid::now_v7(),
        org_id,
        event_type,
        entity,
        entity_id,
        &payload,
    )
    .await
    .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)
}

/// ticket_note_assist: the draft files as an IF8 proposal — status
/// `proposed`, capability + dial recorded, for_user the human, session_id
/// the chat session, NOTHING else writes. Approval re-reads through the
/// existing decision workbench; applying rides the IF1 instant path.
async fn file_ticket_note_proposal(
    st: &RunState,
    ticket_id: Uuid,
    base_version: i64,
    dial_mode: &'static str,
    draft: &str,
) -> ApiResult<Uuid> {
    let mut tx = st.state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, st.org_id, st.user_id)
        .await
        .map_err(ApiError::from)?;
    let fields = core_db::agent_platform::ProposalFields {
        entity: "ticket".into(),
        entity_id: Some(ticket_id),
        op: "update".into(),
        before_value: serde_json::Value::Null,
        proposed_value: serde_json::json!({ "body": draft }),
        classification: "internal".into(),
        capability: Some("tickets:write".into()),
        dial_mode: dial_mode.into(),
        base_version,
        agent_kind: "agent".into(),
        agent_ref: "chat_runner".into(),
        skill_slug: None,
        skill_version: None,
        rule_set_hash: None,
        session_id: Some(st.session_id.to_string()),
        tool: Some("chat_runner".into()),
        for_user: Some(st.user_id),
        rationale: Some("ticket_note_assist draft (ChatRunner)".into()),
    };
    let row = core_db::agent_platform::insert_proposal(&mut tx, Uuid::now_v7(), st.org_id, &fields)
        .await
        .map_err(ApiError::from)?;
    core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id: st.org_id,
            actor: Actor::Agent("chat_runner".into()),
            action: "agent_proposal.created".into(),
            entity_type: Some("agent_proposal".into()),
            entity_id: Some(row.id.to_string()),
            detail: serde_json::json!({
                "entity": "ticket",
                "entity_id": ticket_id,
                "op": "update",
                "classification": "internal",
                "dial_mode": dial_mode,
                "agent_ref": "chat_runner",
                "session_id": st.session_id,
            }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await
    .map_err(ApiError::from)?;
    core_db::outbox::insert(
        &mut tx,
        Uuid::now_v7(),
        st.org_id,
        types::AGENT_PROPOSAL_CREATED,
        "agent_proposal",
        Some(row.id.to_string().as_str()),
        &serde_json::json!({
            "proposal_id": row.id,
            "entity": "ticket",
            "entity_id": ticket_id,
            "op": "update",
            "classification": "internal",
            "dial_mode": dial_mode,
            "agent_ref": "chat_runner",
        }),
    )
    .await
    .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(row.id)
}

fn month_start(now: chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::Utc> {
    crate::routes::ai_policy::month_start(now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wardrobe_derives_read_tools_from_the_pinned_spec() {
        let wardrobe = derive_wardrobe();
        assert!(!wardrobe.is_empty(), "the pinned spec carries GET reads");
        let one = wardrobe
            .iter()
            .find(|t| t.name == "tickets_get_one")
            .expect("GET /api/v1/tickets/{id} rides the reviewed spec");
        assert_eq!(one.capability, "tickets:read");
        assert_eq!(one.path, "/api/v1/tickets/{id}");
        assert_eq!(one.path_params, vec!["id"]);
        // WRITES never enter the wardrobe (they ride IF8 proposals).
        assert!(
            wardrobe.iter().all(|t| t.capability.ends_with(":read")),
            "every wardrobe tool is :read"
        );
        // Sensitive prefixes never enter the wardrobe.
        assert!(wardrobe
            .iter()
            .all(|t| !t.path.starts_with("/api/v1/attachments")
                && !t.path.starts_with("/api/v1/org/exports")
                && !t.path.starts_with("/api/v1/auth")
                && !t.path.starts_with("/api/v1/portal")
                && !t.path.contains("/attachments")));
    }

    #[test]
    fn capability_ns_matches_the_mcp_adapter_mapping() {
        assert_eq!(capability_ns("/api/v1/tickets"), Some("tickets".into()));
        assert_eq!(capability_ns("/api/tickets"), Some("tickets".into()));
        assert_eq!(capability_ns("/api/v1/"), None);
        assert_eq!(capability_ns("/health"), None);
    }

    #[test]
    fn parse_maps_content_tool_calls_and_usage() {
        let turn = parse_chat_completion_turn(&json!({
            "choices": [{
                "message": {
                    "content": "Let me look.",
                    "tool_calls": [
                        { "id": "call_1", "type": "function",
                          "function": { "name": "tickets_get_one", "arguments": "{\"id\":\"11\"}" } },
                        { "id": "call_2", "type": "function",
                          "function": { "name": "clients_list", "arguments": "{}" } }
                    ]
                }
            }],
            "usage": { "prompt_tokens": 120, "completion_tokens": 64 }
        }))
        .expect("parses");
        assert_eq!(turn.content, "Let me look.");
        assert_eq!(turn.tool_calls.len(), 2);
        assert_eq!(turn.tool_calls[0].name, "tickets_get_one");
        assert_eq!(turn.tool_calls[0].arguments.get("id"), Some(&json!("11")));
        assert_eq!(turn.tool_calls[1].arguments, json!({}));
        assert_eq!(turn.prompt_tokens, 120);
        assert_eq!(turn.completion_tokens, 64);
    }

    #[test]
    fn parse_final_answer_maps_plain_content_without_tool_calls() {
        let turn = parse_chat_completion_turn(&json!({
            "choices": [{ "message": { "content": "the final answer" } }]
        }))
        .expect("parses");
        assert_eq!(turn.content, "the final answer");
        assert!(turn.tool_calls.is_empty());
    }

    #[test]
    fn parse_refuses_malformed_provider_payload_typed() {
        let error = parse_chat_completion_turn(&json!({ "choices": "nope" }));
        assert!(
            matches!(error, Err(ChatTransportError::Backend { .. })),
            "malformed provider output is a typed backend failure, never a panic"
        );
    }

    #[test]
    fn turn_rate_limit_admits_under_the_limit_and_throttles_at_it() {
        let org = Uuid::now_v7();
        let user = Uuid::now_v7();
        for _ in 0..TURN_RATE_LIMIT {
            assert!(
                turn_rate_check(org, user).is_ok(),
                "admission under the limit"
            );
        }
        let retry = turn_rate_check(org, user).expect_err("saturated");
        assert!(
            (1..=TURN_RATE_WINDOW_SECS).contains(&retry),
            "retry-after must name the remaining window, got {retry}"
        );
    }
}
