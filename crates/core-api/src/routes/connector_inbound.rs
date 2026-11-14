//! Inbound connector callbacks, the press registry, and the channel surfaces
//! (G7, G8, G9; ADR 0032 §"Access is three questions", ADR 0033).
//!
//! ## The four callback endpoints, and why they are unauthenticated
//!
//! ```text
//! POST /api/v1/connectors/slack/interactive   Slack block_actions / view_submission
//! POST /api/v1/connectors/slack/commands      Slack slash command (form-encoded)
//! POST /api/v1/connectors/teams/interactive   Bot Framework activity
//! POST /api/v1/connectors/teams/messages      messaging-extension query
//! ```
//!
//! None of them is behind the JWT middleware, because a channel has no Marula
//! session: **the vendor signature is the authentication** (Slack's HMAC over
//! the raw bytes, the Bot Framework's JWT). That makes two rules absolute:
//!
//! 1. **The body is read as bytes, before any parse.** A signature computed
//!    over re-serialised JSON is worthless — key order, spacing and escaping
//!    would all change the digest. Both handlers take `Bytes`.
//! 2. **A body field may select a candidate connection, never authenticate
//!    one.** The signing secret belongs to the connection, so the connection
//!    has to be found before the signature can be checked, and the only thing
//!    that names it is a body field (Slack `team.id`, Teams `recipient.id`).
//!    [`resolve_slack`] and [`resolve_teams`] therefore use that field only as a
//!    lookup key into [`core_db::connector_messaging::callback_candidates`], and
//!    proceed on a constant-time signature match. A forged selector picks a
//!    different secret and fails; it can never pick an org.
//!
//! Every authentication failure is the same typed 401 with the same sentence.
//! The check that actually failed goes into the ledger, never into the reply.
//!
//! ## Idempotency, and Slack's three-second rule
//!
//! Slack re-delivers a callback it did not see answered within three seconds,
//! up to three times, with byte-identical bodies. The work here is
//! deliberately cheap (a handful of indexed reads, then the ticket service the
//! UI already uses) so the answer is immediate; on top of that every press is
//! keyed by the SHA-256 of its own bytes and a replay returns the first
//! answer. The key is taken inside a transaction that holds an advisory lock
//! for the duration, so two duplicates cannot both pass "not handled yet" —
//! the second waits, then replays.
//!
//! ## The action registry
//!
//! [`ChannelAction`] is the unit a press resolves. One `static` per action, one
//! entry in [`registry`], and both vendor renderers walk the same list — so a
//! new action is one entry, not a new arm in two vendor match blocks. `take`,
//! `status:<category>` and `open` are the three shipped kinds; `open` is a link
//! (a press cannot reach it), and the status actions are a small closed set
//! bound to semantic categories rather than to board-specific status ids,
//! because a board names its own columns.
//!
//! ## What a press may do
//!
//! Nothing the presser could not do in the UI. The vendor identity is resolved
//! through `user_external_identities` to a Marula user, that user's principal is
//! rebuilt from their *current* roles, and the underlying action's own gate is
//! re-run (tickets:view, tickets:edit, board RBAC, the client's restriction
//! list). An unmapped or unauthorised press is a typed refusal the channel
//! renders — never a silent no-op, never an act performed as somebody else.
//!
//! ## The card
//!
//! One [`TicketCard`] from one builder, two renderers
//! ([`app_bridges::card::render_slack`], [`app_bridges::card::render_teams`]).
//! The card carries six facts and nothing else — no body, no notes — so it
//! cannot leak restricted content by construction, and a restricted client's
//! ticket is refused a share outright rather than redacted into a channel a
//! wider audience reads.
//!
//! Non-purpose: no vendor-side identity linking (a mapping is an admin act),
//! and no per-vendor action logic.

use crate::auth::middleware::authed;
use crate::error::{ApiError, ApiResult};
use crate::org_tx::{self, OrgTx};
use crate::state::AppState;
use app_bridges::card::{CardAction, TicketCard};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{header, Extensions, HeaderMap};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use core_db::connector_messaging::{
    self, ActionRecord, CallbackCandidate, Destination, TicketFacts,
};
use core_domain::connectors::ConnectionStatus;
use core_domain::entities::{Actor, AuditEvent, BoardStatus, Principal};
use core_domain::events::types;
use core_domain::policy::{check, Action as PolicyAction, BoardOp, Module, Permission};
use core_domain::status::TicketStatusCategory;
use core_domain::workflow::{self, TransitionEdge};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

use super::board_access::authorize_board_op;
use super::tickets::ChangeStatusRequest;
use super::ApiGeneration;

/// The vendor vocabulary this surface speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Slack,
    Teams,
}

impl Vendor {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Slack => "slack",
            Self::Teams => "teams",
        }
    }

    /// The `user_external_identities.identity_kind` a press of this vendor
    /// resolves through (0185's closed vocabulary).
    const fn identity_kind(self) -> &'static str {
        match self {
            Self::Slack => "slack_user",
            Self::Teams => "teams_user",
        }
    }
}

/// One authentication failure, for every vendor and every check.
///
/// The message is deliberately constant: it says the callback did not
/// authenticate and nothing else. Which check failed is in the ledger, where
/// only an operator can read it.
const AUTH_REFUSAL: &str = "the connector callback did not authenticate";

/// The unsealed credentials of one connection.
///
/// Keys, documented here because they are the connector's own contract:
/// Slack `signing_secret` (inbound) and `bot_token` (outbound);
/// Teams `bot_token` (outbound only — the inbound token is verified with
/// Microsoft's published keys, and the bot's app id is non-secret `settings`).
struct Credentials(Value);

impl Credentials {
    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

/// A connection the signature proved.
struct VerifiedConnection {
    org_id: Uuid,
    connection_id: Uuid,
    label: String,
}

/// The one answer an authentication failure gets.
fn auth_failure() -> Response {
    ApiError::Unauthorized(AUTH_REFUSAL.into()).into_response()
}

// ---- routing ---------------------------------------------------------------

/// The four unauthenticated callback endpoints.
///
/// Mounted inside the `public` half of the API surface, which is the same
/// half the alert ingestor and the Stripe receiver live on: no JWT middleware,
/// because the signature is the credential.
pub fn public_routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/connectors/slack/interactive"),
            post(slack_interactive),
        )
        .route(
            &format!("{base}/connectors/slack/commands"),
            post(slack_command),
        )
        .route(
            &format!("{base}/connectors/teams/interactive"),
            post(teams_interactive),
        )
        .route(
            &format!("{base}/connectors/teams/messages"),
            post(teams_messages),
        )
}

// ---- authentication --------------------------------------------------------

/// The digest that identifies one callback.
///
/// SHA-256 of the raw bytes, hex. Slack's retry re-sends the same bytes, so a
/// retry and its original collapse onto one ledger row. Two genuine presses of
/// the same button are different bytes (`action_ts`, `trigger_id`), so they stay
/// two presses.
fn callback_key(body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body);
    hex::encode(hasher.finalize())
}

/// The ledger key for a *failed authentication*, kept out of the press digest's
/// key space on purpose.
///
/// Two failures on the same bytes (Slack retrying an unanswered 401, an
/// operator fixing a secret while the retries arrive) must each be able to
/// record what happened, and — more importantly — a failed authentication must
/// never occupy the digest a later *genuine* delivery of the same bytes will
/// use: that would turn one transient refusal into "this press was already
/// refused". The digest is therefore over a domain-separated string.
fn auth_failure_key(body: &[u8], code: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"connector-callback-auth-failure:");
    hasher.update(code.as_bytes());
    hasher.update(b":");
    hasher.update(body);
    hex::encode(hasher.finalize())
}

/// Opens one candidate's sealed credential document.
fn credentials_of(state: &AppState, sealed: Option<&[u8]>) -> Result<Credentials, ApiError> {
    let Some(sealed) = sealed else {
        return Ok(Credentials(Value::Null));
    };
    let plaintext = crate::crypto::open(&state.config.secret_key, sealed)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("connector secret: {e}")))?;
    let value: Value = serde_json::from_slice(&plaintext).unwrap_or(Value::Null);
    Ok(Credentials(value))
}

/// The candidate list for a vendor selector, with `disabled` already excluded.
async fn candidates(
    state: &AppState,
    vendor: Vendor,
    selector: &str,
) -> ApiResult<Vec<CallbackCandidate>> {
    Ok(connector_messaging::callback_candidates(&state.pool, vendor.as_str(), selector).await?)
}

/// Records an authentication failure against the one connection it can be
/// attributed to, and answers the single 401.
///
/// Attribution is only possible when the selector resolved exactly one
/// candidate: with none there is no org to write against, and with several
/// there is no way to tell whose secret was presented. Silence is then the
/// honest answer — the alternative would be a ledger row accusing the wrong
/// tenant.
async fn audit_auth_failure(
    state: &AppState,
    vendor: Vendor,
    candidates: &[CallbackCandidate],
    code: &'static str,
    body: &[u8],
) -> Response {
    if let [one] = candidates {
        let org_id = one.r_org_id;
        let connection_id = one.r_id;
        let key = auth_failure_key(body, code);
        let write = org_tx::with_org_tx(state, org_id, Uuid::nil(), |mut ctx| async move {
            connector_messaging::insert_action(
                &mut ctx.tx,
                connector_messaging::NewAction {
                    id: Uuid::now_v7(),
                    org_id,
                    connection_id,
                    callback_key: &key,
                    action: "callback.auth",
                    outcome: "refused",
                    refusal_code: Some(code),
                    ticket_id: None,
                    ticket_number: None,
                    message_id: None,
                    channel_id: None,
                    vendor_user_id: None,
                    actor_user_id: None,
                    detail: &json!({ "vendor": vendor.as_str(), "check": code }),
                },
            )
            .await?;
            Ok((ctx, ()))
        })
        .await;
        if let Err(e) = write {
            tracing::warn!(error = %e, "connector callback auth refusal was not audited");
        }
    }
    auth_failure()
}

/// Verifies a Slack callback and returns the connection it authenticates.
async fn resolve_slack(
    state: &AppState,
    headers: &HeaderMap,
    body: &Bytes,
    selector: &str,
) -> Result<VerifiedConnection, Response> {
    let candidates = candidates(state, Vendor::Slack, selector)
        .await
        .map_err(|_| auth_failure())?;

    let timestamp = headers
        .get(app_bridges::slack_inbound::TIMESTAMP_HEADER)
        .and_then(|v| v.to_str().ok());
    let signature = headers
        .get(app_bridges::slack_inbound::SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok());
    let now = chrono::Utc::now().timestamp();
    let mut matched: Option<&CallbackCandidate> = None;

    for candidate in &candidates {
        let Ok(credentials) = credentials_of(state, candidate.r_secrets_encrypted.as_deref())
        else {
            // A sealed document that will not open is an operational fault, not
            // an authentication answer: refuse, and do not blame the caller.
            tracing::error!(
                connection = %candidate.r_id,
                "connector connection secret could not be unsealed"
            );
            continue;
        };
        let Some(secret) = credentials.get("signing_secret") else {
            continue;
        };
        if app_bridges::slack_inbound::verify(secret.as_bytes(), timestamp, signature, body, now)
            .is_ok()
        {
            match matched {
                None => matched = Some(candidate),
                // More than one connection in the same workspace signs the same
                // body. A multi-tenant Slack app shares one signing secret
                // across workspaces, so the secret cannot tell two tenants
                // apart — and guessing would attribute a press to the wrong
                // organisation. Refuse.
                Some(_) => {
                    return Err(audit_auth_failure(
                        state,
                        Vendor::Slack,
                        &candidates,
                        "connection_ambiguous",
                        body,
                    )
                    .await)
                }
            }
        }
    }

    if let Some(candidate) = matched {
        return Ok(VerifiedConnection {
            org_id: candidate.r_org_id,
            connection_id: candidate.r_id,
            label: candidate.r_label.clone(),
        });
    }

    let code = slack_refusal_code(state, &candidates, timestamp, signature, body, now);
    Err(audit_auth_failure(state, Vendor::Slack, &candidates, code, body).await)
}

/// The most specific failure the ledger can name, without repeating any of it
/// to the caller.
///
/// This re-runs the same checks the loop above did: the loop's answer is a
/// boolean by design, and the detail has to be gathered somewhere.
fn slack_refusal_code(
    state: &AppState,
    candidates: &[CallbackCandidate],
    timestamp: Option<&str>,
    signature: Option<&str>,
    body: &[u8],
    now: i64,
) -> &'static str {
    if candidates.is_empty() {
        return "connection_unknown";
    }
    for candidate in candidates {
        let Ok(credentials) = credentials_of(state, candidate.r_secrets_encrypted.as_deref())
        else {
            continue;
        };
        let Some(secret) = credentials.get("signing_secret") else {
            continue;
        };
        if let Err(err) =
            app_bridges::slack_inbound::verify(secret.as_bytes(), timestamp, signature, body, now)
        {
            return err.as_code();
        }
    }
    "slack_signing_secret_missing"
}

/// Verifies a Teams activity's Bot Framework token.
///
/// The token's audience is the bot's app id, so the candidate's non-secret
/// `settings.teams_bot_app_id` is what the token is validated against. The verifier
/// is injected ([`install_bot_auth`]) so a test never fetches Microsoft's keys.
async fn resolve_teams(
    state: &AppState,
    headers: &HeaderMap,
    body: &Bytes,
    selector: &str,
    service_url: Option<&str>,
) -> Result<VerifiedConnection, Response> {
    let candidates = candidates(state, Vendor::Teams, selector)
        .await
        .map_err(|_| auth_failure())?;

    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| {
            raw.strip_prefix("Bearer ")
                .or_else(|| raw.strip_prefix("bearer "))
        })
        .map(str::trim)
        .filter(|raw| !raw.is_empty());

    let auth = bot_auth();
    let mut last_error = "teams_token_missing";
    let mut matched: Option<&CallbackCandidate> = None;
    for candidate in &candidates {
        let Some(app_id) = candidate
            .r_settings
            .get("teams_bot_app_id")
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(token) = token else {
            last_error = "teams_token_missing";
            continue;
        };
        match auth.verify(token, app_id, service_url).await {
            // The same reasoning as the Slack door: a token that verifies for
            // two connections means the selector cannot tell two tenants
            // apart, and guessing would attribute the press to the wrong org.
            Ok(_) => match matched {
                None => matched = Some(candidate),
                Some(_) => {
                    return Err(audit_auth_failure(
                        state,
                        Vendor::Teams,
                        &candidates,
                        "connection_ambiguous",
                        body,
                    )
                    .await)
                }
            },
            Err(err) => last_error = err.as_code(),
        }
    }

    if let Some(candidate) = matched {
        return Ok(VerifiedConnection {
            org_id: candidate.r_org_id,
            connection_id: candidate.r_id,
            label: candidate.r_label.clone(),
        });
    }

    if candidates.is_empty() {
        last_error = "connection_unknown";
    }
    Err(audit_auth_failure(state, Vendor::Teams, &candidates, last_error, body).await)
}

// ---- the action registry ---------------------------------------------------

/// What one press did.
pub enum ActionOutcome {
    Applied { detail: Value, text: String },
    Refused { code: &'static str, text: String },
}

impl ActionOutcome {
    fn applied(text: impl Into<String>, detail: Value) -> Self {
        Self::Applied {
            detail,
            text: text.into(),
        }
    }

    fn refused(code: &'static str, text: impl Into<String>) -> Self {
        Self::Refused {
            code,
            text: text.into(),
        }
    }
}

/// Everything one action needs to act, assembled once by the route.
///
/// No transaction: the ticket transitions go through
/// `services::tickets::transition_status`, which owns its transaction and its
/// gates. Re-implementing that here — or running it inside a second one — is
/// exactly how a channel press would drift from the button in the UI.
pub struct ActionRun<'a> {
    pub state: &'a AppState,
    /// The presser, rebuilt from their current roles. A press never inherits
    /// the sharer's or the org's authority.
    pub principal: &'a Principal,
    pub facts: &'a TicketFacts,
    pub statuses: &'a [BoardStatus],
    pub edges: &'a [TransitionEdge],
    pub base_url: Option<String>,
}

/// One entry in the press registry.
///
/// Declared once, rendered by both vendors, resolved by id on the way back.
/// `link` returns a URL for actions that open something instead of acting;
/// those never reach [`apply`](ChannelAction::apply) because a card renders
/// them as a URL button.
///
/// `offered` and `link` take only the facts and the board's matrix, so the card
/// builder is a pure function over data — no `AppState`, no principal, and a
/// unit test can exercise every entry.
#[async_trait::async_trait]
pub trait ChannelAction: Send + Sync {
    /// The registry id, carried verbatim in both vendor payloads.
    fn id(&self) -> &'static str;
    /// The button label. Core never re-authors a vendor's label; this is the
    /// action's own name, and both products show it.
    fn label(&self) -> &'static str;
    /// Whether the action needs `tickets:edit` (and board work rights) rather
    /// than just reading the ticket.
    fn requires_edit(&self) -> bool {
        true
    }
    /// The URL this action opens, for `open`-shaped entries.
    fn link(&self, _facts: &TicketFacts, _base_url: Option<&str>) -> Option<String> {
        None
    }
    /// Whether this action is offered on the ticket right now.
    fn offered(
        &self,
        _facts: &TicketFacts,
        _statuses: &[BoardStatus],
        _edges: &[TransitionEdge],
    ) -> bool {
        true
    }
    async fn apply(&self, run: &mut ActionRun<'_>) -> ActionOutcome;
}

/// `take`: assign to the presser and start it, the way the ticket route does.
struct TakeAction;

#[async_trait::async_trait]
impl ChannelAction for TakeAction {
    fn id(&self) -> &'static str {
        "take"
    }

    fn label(&self) -> &'static str {
        "Take it"
    }

    fn offered(
        &self,
        facts: &TicketFacts,
        _statuses: &[BoardStatus],
        _edges: &[TransitionEdge],
    ) -> bool {
        // A closed ticket cannot be taken: the press would have to reopen it,
        // which is a decision for the UI, not a channel button.
        facts.status_category != "closed"
    }

    async fn apply(&self, run: &mut ActionRun<'_>) -> ActionOutcome {
        let user_id = run.principal.user.id;
        let facts = run.facts;

        // Start first, then record. The transition owns its transaction and its
        // gates (board RBAC, WIP cap, checklist), so reordering these would
        // either duplicate the gate or move the ticket after the ledger says
        // what happened. Both halves are idempotent: a retry that finds the
        // ticket already assigned and already in progress changes nothing.
        let mut started = None;
        let mut start_error = None;
        if facts.status_category != "in_progress" {
            match first_status_in(
                facts,
                run.statuses,
                run.edges,
                TicketStatusCategory::InProgress,
            ) {
                Some(status_id) => {
                    match transition_to(run.state, run.principal, facts.ticket_id, status_id).await
                    {
                        Ok(()) => started = Some(status_id),
                        Err(reason) => start_error = Some(reason),
                    }
                }
                None => {
                    start_error =
                        Some("this board has no in-progress column reachable from here".to_string())
                }
            }
        }

        // The assignment is the press: it goes through the same update the
        // ticket route uses, so `ticket.assigned` fires and the board's work
        // gate decides whether this presser may hold it.
        if let Err((code, message)) = assign_to(run.state, run.principal, facts, user_id).await {
            return ActionOutcome::refused(code, message);
        }

        let mut text = format!("Assigned {} to you.", facts.number);
        let mut detail = json!({
            "assignee_id": user_id,
            "from_assignee_id": facts.assignee_id,
            "from_status_id": facts.status_id,
            "status_moved": started.is_some(),
        });
        if let Some(status_id) = started {
            text.push_str(" Moved it to the in-progress column.");
            detail["to_status_id"] = json!(status_id);
        }
        if let Some(reason) = start_error {
            text.push_str(&format!(" It could not be started: {reason}"));
            detail["start_refused"] = json!(reason);
        }
        ActionOutcome::applied(text, detail)
    }
}

/// `status:<category>`: move the ticket to a status in one semantic category.
///
/// The category, not a status id, is the registry's vocabulary: a board names
/// its own columns, and a card built on one tenant must not carry another
/// tenant's status id.
struct StatusAction {
    id: &'static str,
    label: &'static str,
    category: TicketStatusCategory,
}

#[async_trait::async_trait]
impl ChannelAction for StatusAction {
    fn id(&self) -> &'static str {
        self.id
    }

    fn label(&self) -> &'static str {
        self.label
    }

    fn offered(
        &self,
        facts: &TicketFacts,
        statuses: &[BoardStatus],
        edges: &[TransitionEdge],
    ) -> bool {
        // A closed ticket is not reopened from a channel button: reopening
        // resets SLA bookkeeping and re-enters the board's gates, which is a
        // decision the ticket surface owns. `take` follows the same rule.
        if facts.status_category == "closed" {
            return false;
        }
        if facts.status_category == self.category.as_str() {
            return false;
        }
        first_status_in(facts, statuses, edges, self.category).is_some()
    }

    async fn apply(&self, run: &mut ActionRun<'_>) -> ActionOutcome {
        let Some(status_id) = first_status_in(run.facts, run.statuses, run.edges, self.category)
        else {
            return ActionOutcome::refused(
                "transition_refused",
                format!(
                    "This board has no {} column reachable from here.",
                    self.category
                ),
            );
        };
        match transition_to(run.state, run.principal, run.facts.ticket_id, status_id).await {
            Ok(()) => ActionOutcome::applied(
                format!("Moved {} to {}.", run.facts.number, self.category),
                json!({
                    "from_status_id": run.facts.status_id,
                    "to_status_id": status_id,
                    "category": self.category.as_str(),
                }),
            ),
            Err(reason) => ActionOutcome::refused("transition_refused", reason),
        }
    }
}

/// `open`: a deep link into the ticket. Rendered as a URL button, never
/// delivered as a press.
struct OpenAction;

#[async_trait::async_trait]
impl ChannelAction for OpenAction {
    fn id(&self) -> &'static str {
        "open"
    }

    fn label(&self) -> &'static str {
        "Open in Marula"
    }

    fn requires_edit(&self) -> bool {
        false
    }

    fn link(&self, facts: &TicketFacts, base_url: Option<&str>) -> Option<String> {
        base_url.map(|base| format!("{base}/tickets/{}", facts.ticket_id))
    }

    async fn apply(&self, _run: &mut ActionRun<'_>) -> ActionOutcome {
        // Unreachable through a callback: the renderers emit a URL button and
        // the vendor never posts it back. Answering instead of panicking keeps
        // a hand-crafted request from turning into a 500.
        ActionOutcome::refused("link_only", "That action opens a page; it has no press.")
    }
}

static TAKE: TakeAction = TakeAction;
static OPEN: OpenAction = OpenAction;
static IN_PROGRESS: StatusAction = StatusAction {
    id: "status:in_progress",
    label: "Start",
    category: TicketStatusCategory::InProgress,
};
static WAITING: StatusAction = StatusAction {
    id: "status:waiting",
    label: "Wait",
    category: TicketStatusCategory::Waiting,
};
static RESOLVE: StatusAction = StatusAction {
    id: "status:closed",
    label: "Resolve",
    category: TicketStatusCategory::Closed,
};

/// Every action a card can offer, in display order.
///
/// Adding an action is one `static` and one line here. Nothing in the Slack or
/// Teams paths changes.
pub fn registry() -> &'static [&'static dyn ChannelAction] {
    static ACTIONS: [&dyn ChannelAction; 5] = [&TAKE, &IN_PROGRESS, &WAITING, &RESOLVE, &OPEN];
    &ACTIONS
}

/// Resolves a registry id from either vendor's payload.
fn find_action(id: &str) -> Option<&'static dyn ChannelAction> {
    registry().iter().copied().find(|action| action.id() == id)
}

// ---- ticket operations the actions share -----------------------------------

/// The first status in `category` that is a legal next step from this ticket.
///
/// "First" is the board's own order (`sort_order`), so a press lands on the
/// column the UI would offer first. `validate_transition` is the same function
/// the ticket route uses, so a press cannot take an edge the matrix forbids.
fn first_status_in(
    facts: &TicketFacts,
    statuses: &[BoardStatus],
    edges: &[TransitionEdge],
    category: TicketStatusCategory,
) -> Option<Uuid> {
    statuses
        .iter()
        .filter(|status| status.category == category && status.id != facts.status_id)
        .find(|status| {
            workflow::validate_transition(statuses, edges, facts.status_id, status.id).is_ok()
        })
        .map(|status| status.id)
}

/// Applies a guarded status transition through the ticket route's own service.
async fn transition_to(
    state: &AppState,
    principal: &Principal,
    ticket_id: Uuid,
    status_id: Uuid,
) -> Result<(), String> {
    let req = ChangeStatusRequest {
        status_id,
        instant: Default::default(),
    };
    match crate::services::tickets::transition_status(state, principal, ticket_id, req).await {
        Ok(_) => Ok(()),
        Err(ApiError::Validation(message) | ApiError::Conflict(message)) => Err(message),
        Err(ApiError::Forbidden(message)) => Err(message),
        Err(ApiError::NotFound) => Err("the ticket is no longer available".into()),
        Err(other) => Err(other.to_string()),
    }
}

/// Assigns the ticket to `user_id` through the same update the ticket route uses.
///
/// Written as the presser: the board's work gate and the `tickets:edit` matrix
/// are re-checked inside, so an assignment cannot be made on behalf of somebody
/// who could not make it themselves.
async fn assign_to(
    state: &AppState,
    principal: &Principal,
    facts: &TicketFacts,
    user_id: Uuid,
) -> Result<(), (&'static str, String)> {
    if facts.assignee_id == Some(user_id) {
        return Ok(());
    }
    let org_id = principal.user.org_id;
    let result = org_tx::with_org_tx(state, org_id, principal.user.id, |mut ctx| async move {
        authorize_board_op(&mut ctx.tx, principal, facts.board_id, BoardOp::WorkTicket).await?;
        let before = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, facts.ticket_id).await?;
        core_db::tickets::update_ticket(
            &mut *ctx.tx,
            org_id,
            facts.ticket_id,
            core_db::tickets::TicketPatch {
                title: None,
                body: None,
                priority: None,
                assignee_id: Some(Some(user_id)),
                client_id: None,
                site_id: None,
                contact_id: None,
                due_at: None,
                type_id: None,
                category_id: None,
            },
        )
        .await?;
        let after = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, facts.ticket_id).await?;
        ctx.emit(
            types::TICKET_UPDATED,
            "ticket",
            Some(after.id.to_string().as_str()),
            &json!({
                "ticket_id": after.id,
                "board_id": after.board_id,
                "assignee_id": after.assignee_id,
            }),
        )
        .await?;
        if before.assignee_id != after.assignee_id {
            ctx.emit(
                types::TICKET_ASSIGNED,
                "ticket",
                Some(after.id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::TicketAssigned {
                    ticket_id: after.id,
                    board_id: after.board_id,
                    assignee_id: after.assignee_id,
                    assigned_by: Some(user_id),
                })
                .unwrap_or_default(),
            )
            .await?;
        }
        Ok((ctx, ()))
    })
    .await;
    match result {
        Ok(()) => Ok(()),
        Err(ApiError::Forbidden(message)) => Err(("permission_denied", message)),
        Err(ApiError::Validation(message) | ApiError::Conflict(message)) => {
            Err(("transition_refused", message))
        }
        Err(other) => Err(("action_failed", other.to_string())),
    }
}

// ---- the press pipeline ----------------------------------------------------

/// What one press produced, in the shape every endpoint answers with.
pub enum PressResult {
    /// The action ran. `text` is rendered ephemerally to the person who
    /// pressed, so they know it happened.
    Applied { text: String },
    /// Typed refusal. `code` goes to the ledger; `text` is what the channel
    /// shows, and it always names the next step.
    Refused { code: &'static str, text: String },
    /// A duplicate of an earlier press: the first answer, replayed.
    Replay(Box<ActionRecord>),
}

/// Where a press pointed, before identity or permission is known.
struct PressTarget {
    vendor_user_id: String,
    channel_id: Option<String>,
    message_ref: Option<String>,
}

/// Runs one press inside its idempotency window.
///
/// The transaction holds an advisory lock keyed on the callback digest for its
/// whole life, so a retry that arrives while the first is still running waits
/// and then replays instead of racing it. Every exit — applied, refused, or a
/// permission failure — writes exactly one ledger row.
async fn run_press(
    state: &AppState,
    connection: &VerifiedConnection,
    vendor: Vendor,
    key: &str,
    raw_action_id: &str,
    target: PressTarget,
) -> PressResult {
    let org_id = connection.org_id;
    let connection_id = connection.connection_id;
    let vendor_user_id = target.vendor_user_id;
    let channel_id = target.channel_id;
    let message_ref = target.message_ref;
    let action_label = raw_action_id.to_string();
    let base_url = web_base_url(state);

    let outcome = org_tx::with_org_tx(state, org_id, Uuid::nil(), |mut ctx| async move {
        // 1. The idempotency window.
        connector_messaging::lock_callback(&mut ctx.tx, org_id, connection_id, key).await?;
        if let Some(existing) =
            connector_messaging::action_by_callback(&mut ctx.tx, org_id, connection_id, key).await?
        {
            return Ok((ctx, PressResult::Replay(Box::new(existing))));
        }

        // 2. Which registry entry, and which message.
        let Some(action) = find_action(&action_label) else {
            record_ledger(
                &mut ctx,
                LedgerFields {
                    connection,
                    key,
                    action: &action_label,
                    outcome: "refused",
                    refusal_code: Some("unknown_action"),
                    ticket: None,
                    message: None,
                    channel_id: channel_id.as_deref(),
                    vendor_user_id: &vendor_user_id,
                    actor_user_id: None,
                    detail: json!({ "reason": "no registry entry with that id" }),
                },
            )
            .await?;
            return Ok((
                ctx,
                PressResult::Refused {
                    code: "unknown_action",
                    text: "That button belongs to a different app, or to a Marula version that no longer offers it."
                        .into(),
                },
            ));
        };

        // 3. The card this button lives in. Without it there is no ticket and
        //    no safe guess about which one was meant.
        let message = match (&channel_id, &message_ref) {
            (Some(channel), Some(message_ref)) => {
                connector_messaging::message_by_vendor(
                    &mut ctx.tx,
                    org_id,
                    connection_id,
                    channel,
                    message_ref,
                )
                .await?
            }
            _ => None,
        };
        let Some(message) = message else {
            record_ledger(
                &mut ctx,
                LedgerFields {
                    connection,
                    key,
                    action: action.id(),
                    outcome: "refused",
                    refusal_code: Some("message_unmapped"),
                    ticket: None,
                    message: None,
                    channel_id: channel_id.as_deref(),
                    vendor_user_id: &vendor_user_id,
                    actor_user_id: None,
                    detail: json!({
                        "reason": "the message is not in this connection's registry",
                        "message_ref": message_ref,
                    }),
                },
            )
            .await?;
            return Ok((
                ctx,
                PressResult::Refused {
                    code: "message_unmapped",
                    text: "This card was not posted by Marula, so there is no ticket behind the button."
                        .into(),
                },
            ));
        };

        let facts = connector_messaging::ticket_facts(&mut ctx.tx, org_id, message.ticket_id).await?;
        let Some(facts) = facts else {
            record_ledger(
                &mut ctx,
                LedgerFields {
                    connection,
                    key,
                    action: action.id(),
                    outcome: "refused",
                    refusal_code: Some("ticket_unreadable"),
                    ticket: None,
                    message: Some(message.id),
                    channel_id: Some(&message.channel_id),
                    vendor_user_id: &vendor_user_id,
                    actor_user_id: None,
                    detail: json!({ "reason": "the ticket the card was posted from is gone" }),
                },
            )
            .await?;
            return Ok((
                ctx,
                PressResult::Refused {
                    code: "ticket_unreadable",
                    text: "The ticket this card was posted from is no longer available.".into(),
                },
            ));
        };

        // 4. Who pressed, and may they do this?
        let actor = match resolve_presser(&mut ctx, connection, vendor, &vendor_user_id, &facts)
            .await?
        {
            Ok(actor) => actor,
            Err(refusal) => {
                record_ledger(
                    &mut ctx,
                    LedgerFields {
                        connection,
                        key,
                        action: action.id(),
                        outcome: "refused",
                        refusal_code: Some(refusal.code),
                        ticket: Some((facts.ticket_id, &facts.number)),
                        message: Some(message.id),
                        channel_id: Some(&message.channel_id),
                        vendor_user_id: &vendor_user_id,
                        actor_user_id: None,
                        detail: json!({ "reason": refusal.text }),
                    },
                )
                .await?;
                return Ok((
                    ctx,
                    PressResult::Refused {
                        code: refusal.code,
                        text: refusal.text,
                    },
                ));
            }
        };

        let statuses = core_db::boards::list_statuses(&mut *ctx.tx, facts.board_id).await?;
        let edges = core_db::boards::list_transitions(&mut *ctx.tx, facts.board_id).await?;
        let mut run = ActionRun {
            state,
            principal: &actor,
            facts: &facts,
            statuses: &statuses,
            edges: &edges,
            base_url,
        };

        // 5. The act.
        let result = action.apply(&mut run).await;
        let (ledger_outcome, refusal_code, detail, text) = match result {
            ActionOutcome::Applied { detail, text } => ("applied", None, detail, text),
            ActionOutcome::Refused { code, text } => ("refused", Some(code), json!({}), text),
        };
        record_ledger(
            &mut ctx,
            LedgerFields {
                connection,
                key,
                action: action.id(),
                outcome: ledger_outcome,
                refusal_code,
                ticket: Some((facts.ticket_id, &facts.number)),
                message: Some(message.id),
                channel_id: Some(&message.channel_id),
                vendor_user_id: &vendor_user_id,
                actor_user_id: Some(actor.user.id),
                detail,
            },
        )
        .await?;

        let result = match ledger_outcome {
            "applied" => PressResult::Applied { text },
            _ => PressResult::Refused {
                code: refusal_code.unwrap_or("action_failed"),
                text,
            },
        };
        Ok((ctx, result))
    })
    .await;

    outcome.unwrap_or_else(|e| {
        tracing::error!(error = %e, "connector press failed");
        PressResult::Refused {
            code: "action_failed",
            text: "Marula could not carry that out. Try again from the ticket.".into(),
        }
    })
}

/// A refusal that names its next step.
struct PressRefusal {
    code: &'static str,
    text: String,
}

/// Resolves the vendor identity and re-derives the presser's authority.
///
/// The order matters and is the security of the whole surface: identity first,
/// then the ticket's readability, then the action's own gate. A refusal that
/// came before the identity check would tell an unauthorised presser which
/// ticket exists.
async fn resolve_presser(
    ctx: &mut OrgTx<'_>,
    connection: &VerifiedConnection,
    vendor: Vendor,
    vendor_user_id: &str,
    facts: &TicketFacts,
) -> ApiResult<Result<Principal, PressRefusal>> {
    let org_id = connection.org_id;
    let Some(user_id) = connector_messaging::identity_for_vendor(
        &mut ctx.tx,
        org_id,
        connection.connection_id,
        vendor.identity_kind(),
        vendor_user_id,
    )
    .await?
    else {
        return Ok(Err(PressRefusal {
            code: "identity_unmapped",
            text: format!(
                "Your {} account is not linked to a Marula user yet. Ask an administrator to link it, then press again.",
                match vendor {
                    Vendor::Slack => "Slack",
                    Vendor::Teams => "Teams",
                }
            ),
        }));
    };

    let Some(principal) = principal_of(&mut ctx.tx, org_id, user_id).await? else {
        return Ok(Err(PressRefusal {
            code: "actor_inactive",
            text: "Your Marula account is not active, so this press cannot be performed.".into(),
        }));
    };

    if !check(
        &principal.permissions,
        Permission::Matrix {
            module: Module::Tickets,
            action: PolicyAction::View,
        },
    )
    .is_allowed()
    {
        return Ok(Err(PressRefusal {
            code: "permission_denied",
            text: "You do not have permission to read tickets in Marula.".into(),
        }));
    }
    if denied_by_client_restriction(&principal, facts) {
        return Ok(Err(PressRefusal {
            code: "ticket_unreadable",
            text: "That ticket belongs to a client you cannot read.".into(),
        }));
    }
    Ok(Ok(principal))
}

/// Whether the ticket's client restriction hides it from this principal.
///
/// The same rule the client route applies: a non-empty `restricted_to` list
/// admits only the listed users, and admins bypass it.
fn denied_by_client_restriction(principal: &Principal, facts: &TicketFacts) -> bool {
    !facts.restricted_to.is_empty()
        && !facts.restricted_to.contains(&principal.user.id)
        && !principal.is_owner_or_admin()
}

/// Rebuilds a principal from the user's *current* roles.
///
/// Reusing the session path's own assembly (`principal_for` over
/// `effective_roles_for_user`) means a press is evaluated exactly as the same
/// person clicking the button in the UI would be — including a role granted
/// through a directory group, and including a revocation that happened after
/// the card was posted.
async fn principal_of(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
) -> ApiResult<Option<Principal>> {
    let user = match core_db::users::get_user(&mut *conn, org_id, user_id).await {
        Ok(user) => user,
        Err(core_db::DbError::NotFound) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !user.is_active {
        return Ok(None);
    }
    let roles = core_db::users::effective_roles_for_user(&mut *conn, user_id, org_id).await?;
    Ok(Some(crate::auth::middleware::principal_for(user, roles)))
}

/// The fields one ledger row carries.
struct LedgerFields<'a> {
    connection: &'a VerifiedConnection,
    key: &'a str,
    action: &'a str,
    outcome: &'a str,
    refusal_code: Option<&'a str>,
    ticket: Option<(Uuid, &'a str)>,
    message: Option<Uuid>,
    channel_id: Option<&'a str>,
    vendor_user_id: &'a str,
    actor_user_id: Option<Uuid>,
    detail: Value,
}

/// Writes the one ledger row for a press, plus the org audit-stream row.
///
/// Both are written here, together, because they answer different questions and
/// a press that appears in one and not the other is worse than either alone:
/// the ledger carries the channel/message/vendor vocabulary, the audit stream
/// carries the actor and the record for the operator's trail.
async fn record_ledger(ctx: &mut OrgTx<'_>, fields: LedgerFields<'_>) -> ApiResult<ActionRecord> {
    let org_id = ctx.org_id;
    let row = connector_messaging::insert_action(
        &mut ctx.tx,
        connector_messaging::NewAction {
            id: Uuid::now_v7(),
            org_id,
            connection_id: fields.connection.connection_id,
            callback_key: fields.key,
            action: fields.action,
            outcome: fields.outcome,
            refusal_code: fields.refusal_code,
            ticket_id: fields.ticket.map(|(id, _)| id),
            ticket_number: fields.ticket.map(|(_, number)| number),
            message_id: fields.message,
            channel_id: fields.channel_id,
            vendor_user_id: Some(fields.vendor_user_id),
            actor_user_id: fields.actor_user_id,
            detail: &fields.detail,
        },
    )
    .await?;

    let actor = match fields.actor_user_id {
        Some(user) => Actor::Human(user),
        None => Actor::Connector(format!(
            "{}:{}",
            fields.connection.label, fields.connection.connection_id
        )),
    };
    core_db::audit::insert(
        &mut *ctx.tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor,
            action: format!("connector.action_{}", fields.outcome),
            entity_type: fields.ticket.map(|_| "ticket".to_string()),
            entity_id: fields.ticket.map(|(id, _)| id.to_string()),
            detail: json!({
                "connection_id": fields.connection.connection_id,
                "channel_id": fields.channel_id,
                "message_id": fields.message,
                "action": fields.action,
                "outcome": fields.outcome,
                "refusal_code": fields.refusal_code,
                "vendor_user_id": fields.vendor_user_id,
                "result": fields.detail,
            }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    Ok(row)
}

// ---- Slack endpoints -------------------------------------------------------

/// POST /api/v1/connectors/slack/interactive — a button press.
///
/// Answers 200 for everything that authenticated, because a 4xx/5xx only makes
/// Slack retry a callback we have already handled. An unauthenticated request
/// gets the one typed 401.
#[utoipa::path(
    operation_id = "connectors_slack_interactive",
    post,
    path = "/api/v1/connectors/slack/interactive",
    tag = "connectors",
    request_body(content = String, description = "Slack interactive payload (form-encoded `payload=` field)"),
    responses(
        (status = 200, description = "the press was handled, applied or refused; the body is the ephemeral reply"),
        (status = 401, description = "the signature did not verify")
    )
)]
pub async fn slack_interactive(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let pairs = app_bridges::slack_inbound::parse_form(&body);
    let Some(payload_text) = app_bridges::slack_inbound::form_field(&pairs, "payload") else {
        return auth_failure();
    };
    // Selecting a candidate, not authenticating: the parsed fields are only
    // used to find which connection's secret to try.
    let Ok(selector) = serde_json::from_str::<Value>(payload_text) else {
        return auth_failure();
    };
    let Some(team_id) = selector
        .get("team")
        .and_then(|t| t.get("id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 200)
    else {
        return auth_failure();
    };

    let connection = match resolve_slack(&state, &headers, &body, team_id).await {
        Ok(connection) => connection,
        Err(response) => return response,
    };

    // Authenticated. From here a malformed body is the caller's problem, and
    // the answer is still 200 with an ephemeral explanation so Slack stops
    // retrying.
    let Ok(interaction) =
        serde_json::from_str::<app_bridges::slack_inbound::SlackInteraction>(payload_text)
    else {
        return slack_text("Marula could not read that interaction.").into_response();
    };
    let Some(action) = interaction.actions.first() else {
        return slack_text("That interaction carried no button Marula knows.").into_response();
    };
    let Some(action_id) = app_bridges::card::registry_id_from_slack(&action.action_id) else {
        return slack_text("That button belongs to a different app.").into_response();
    };
    let Some(vendor_user_id) = interaction.user_id() else {
        return slack_text("Marula could not tell who pressed that button.").into_response();
    };

    let key = callback_key(&body);
    let result = run_press(
        &state,
        &connection,
        Vendor::Slack,
        &key,
        action_id,
        PressTarget {
            vendor_user_id: vendor_user_id.to_string(),
            channel_id: interaction.channel_id().map(str::to_string),
            message_ref: interaction.message_ts().map(str::to_string),
        },
    )
    .await;

    slack_text(&press_text(&result)).into_response()
}

/// POST /api/v1/connectors/slack/commands — `/marula T-1042`.
///
/// Resolves the number to a card, enforcing **the caller's own** permissions.
/// A ticket they cannot read is a typed refusal, never an empty card.
#[utoipa::path(
    operation_id = "connectors_slack_commands",
    post,
    path = "/api/v1/connectors/slack/commands",
    tag = "connectors",
    request_body(content = String, description = "Slack slash-command body (form-encoded)"),
    responses(
        (status = 200, description = "an ephemeral card, or the typed refusal text"),
        (status = 401, description = "the signature did not verify")
    )
)]
pub async fn slack_command(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let pairs = app_bridges::slack_inbound::parse_form(&body);
    let Some(command) = app_bridges::slack_inbound::SlackCommand::from_form(&pairs) else {
        return auth_failure();
    };
    let connection = match resolve_slack(&state, &headers, &body, &command.team_id).await {
        Ok(connection) => connection,
        Err(response) => return response,
    };

    let key = callback_key(&body);
    match command_card(
        &state,
        &connection,
        Vendor::Slack,
        &key,
        &command.user_id,
        &command.text,
    )
    .await
    {
        Ok(QueryOutcome::Card { card }) => {
            Json(app_bridges::card::render_slack(&card)).into_response()
        }
        Ok(QueryOutcome::Refused(text)) => slack_text(&text).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "slack command failed");
            auth_failure()
        }
    }
}

/// A slash command's body: an ephemeral message, which Slack shows only to the
/// caller and does not post into the channel.
fn slack_text(text: &str) -> Json<Value> {
    Json(json!({ "response_type": "ephemeral", "text": text }))
}

fn press_text(result: &PressResult) -> String {
    match result {
        PressResult::Applied { text } | PressResult::Refused { text, .. } => text.clone(),
        PressResult::Replay(row) => match row.outcome.as_str() {
            "applied" => "That press was already applied.".to_string(),
            _ => "That press was already refused.".to_string(),
        },
    }
}

// ---- Teams endpoints -------------------------------------------------------

/// POST /api/v1/connectors/teams/interactive — an Adaptive Card press.
#[utoipa::path(
    operation_id = "connectors_teams_interactive",
    post,
    path = "/api/v1/connectors/teams/interactive",
    tag = "connectors",
    request_body(content = String, description = "Bot Framework activity"),
    responses(
        (status = 200, description = "the press was handled; the body is the Bot Framework reply"),
        (status = 401, description = "the Bot Framework token did not verify")
    )
)]
pub async fn teams_interactive(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Ok(activity) = serde_json::from_slice::<app_bridges::teams_inbound::TeamsActivity>(&body)
    else {
        return auth_failure();
    };
    let Some(bot_app_id) = activity.bot_app_id().map(str::to_string) else {
        return auth_failure();
    };
    let connection = match resolve_teams(
        &state,
        &headers,
        &body,
        &bot_app_id,
        activity.service_url.as_deref(),
    )
    .await
    {
        Ok(connection) => connection,
        Err(response) => return response,
    };

    let Some(action_id) = activity.action_id() else {
        return Json(app_bridges::teams_inbound::activity_message(
            "That card did not carry an action Marula knows.",
        ))
        .into_response();
    };
    let Some(vendor_user_id) = activity.vendor_user_id() else {
        return Json(app_bridges::teams_inbound::activity_message(
            "Marula could not tell who pressed that button.",
        ))
        .into_response();
    };

    let key = callback_key(&body);
    let result = run_press(
        &state,
        &connection,
        Vendor::Teams,
        &key,
        action_id,
        PressTarget {
            vendor_user_id: vendor_user_id.to_string(),
            channel_id: activity.conversation_id().map(str::to_string),
            message_ref: activity.id.clone(),
        },
    )
    .await;

    Json(app_bridges::teams_inbound::activity_message(&press_text(
        &result,
    )))
    .into_response()
}

/// POST /api/v1/connectors/teams/messages — the messaging-extension query.
#[utoipa::path(
    operation_id = "connectors_teams_messages",
    post,
    path = "/api/v1/connectors/teams/messages",
    tag = "connectors",
    request_body(content = String, description = "Bot Framework messaging-extension query"),
    responses(
        (status = 200, description = "a card result, or a typed refusal message"),
        (status = 401, description = "the Bot Framework token did not verify")
    )
)]
pub async fn teams_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Ok(activity) = serde_json::from_slice::<app_bridges::teams_inbound::TeamsActivity>(&body)
    else {
        return auth_failure();
    };
    let Some(bot_app_id) = activity.bot_app_id().map(str::to_string) else {
        return auth_failure();
    };
    let connection = match resolve_teams(
        &state,
        &headers,
        &body,
        &bot_app_id,
        activity.service_url.as_deref(),
    )
    .await
    {
        Ok(connection) => connection,
        Err(response) => return response,
    };

    let query: app_bridges::teams_inbound::MessagingExtensionQuery =
        serde_json::from_value(activity.value.clone().unwrap_or(Value::Null)).unwrap_or_default();
    let Some(ticket_number) = query.ticket_number() else {
        return Json(app_bridges::teams_inbound::compose_extension_message(
            "Type a ticket number, for example T-1042.",
        ))
        .into_response();
    };
    let Some(vendor_user_id) = activity.vendor_user_id().map(str::to_string) else {
        return Json(app_bridges::teams_inbound::compose_extension_message(
            "Marula could not tell who is searching.",
        ))
        .into_response();
    };

    let key = callback_key(&body);
    let text = ticket_number.to_string();
    match command_card(
        &state,
        &connection,
        Vendor::Teams,
        &key,
        &vendor_user_id,
        &text,
    )
    .await
    {
        Ok(QueryOutcome::Card { card }) => {
            Json(app_bridges::teams_inbound::compose_extension_card(
                app_bridges::card::render_teams(&card),
            ))
            .into_response()
        }
        Ok(QueryOutcome::Refused(text)) => {
            Json(app_bridges::teams_inbound::compose_extension_message(&text)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "teams messaging extension failed");
            auth_failure()
        }
    }
}

// ---- the slash command / query pipeline ------------------------------------

/// The answer a channel query produces.
pub enum QueryOutcome {
    Card { card: TicketCard },
    Refused(String),
}

/// Resolves a typed ticket number into a card **for the caller**, and records
/// the query.
///
/// Every refusal here is a sentence the channel shows. There is deliberately no
/// path that answers an empty card: an empty result is indistinguishable from
/// "no such ticket", which is the silent no-op G9 forbids.
async fn command_card(
    state: &AppState,
    connection: &VerifiedConnection,
    vendor: Vendor,
    key: &str,
    vendor_user_id: &str,
    raw_number: &str,
) -> ApiResult<QueryOutcome> {
    let org_id = connection.org_id;
    let connection_id = connection.connection_id;
    let number = normalise_number(raw_number);
    let action = "card.query";
    let base_url = web_base_url(state);

    org_tx::with_org_tx(state, org_id, Uuid::nil(), |mut ctx| async move {
        connector_messaging::lock_callback(&mut ctx.tx, org_id, connection_id, key).await?;
        // A retry of the same query replays the first answer and writes
        // nothing: one callback, one ledger row. A refused lookup can be
        // answered verbatim; an applied one needs the card rebuilt, which the
        // code below does without touching the ledger.
        let replayed = if let Some(existing) =
            connector_messaging::action_by_callback(&mut ctx.tx, org_id, connection_id, key).await?
        {
            if existing.outcome == "refused" {
                let text = existing
                    .detail
                    .get("refusal_text")
                    .and_then(Value::as_str)
                    .unwrap_or("That lookup was refused.")
                    .to_string();
                return Ok((ctx, QueryOutcome::Refused(text)));
            }
            Some(existing)
        } else {
            None
        };

        let refuse = |ctx: OrgTx<'static>, text: String| async move {
            let out: ApiResult<(OrgTx<'static>, QueryOutcome)> =
                Ok((ctx, QueryOutcome::Refused(text)));
            out
        };

        if number.is_empty() {
            return refuse(ctx, "Type a ticket number, for example T-1042.".to_string()).await;
        }

        let ticket_id = match Uuid::parse_str(&number) {
            Ok(uuid) => Some(uuid),
            Err(_) => connector_messaging::ticket_id_by_number(&mut ctx.tx, org_id, &number).await?,
        };
        let not_found = format!("No ticket {number} in this organisation.");
        let Some(ticket_id) = ticket_id else {
            if replayed.is_none() {
                record_ledger(
                    &mut ctx,
                    LedgerFields {
                        connection,
                        key,
                        action,
                        outcome: "refused",
                        refusal_code: Some("ticket_unreadable"),
                        ticket: None,
                        message: None,
                        channel_id: None,
                        vendor_user_id,
                        actor_user_id: None,
                        detail: json!({ "reason": "no ticket with that number", "number": number, "refusal_text": not_found }),
                    },
                )
                .await?;
            }
            return refuse(ctx, not_found).await;
        };

        let Some(facts) = connector_messaging::ticket_facts(&mut ctx.tx, org_id, ticket_id).await?
        else {
            return refuse(ctx, not_found).await;
        };

        let principal =
            resolve_presser(&mut ctx, connection, vendor, vendor_user_id, &facts).await?;
        let principal = match principal {
            Ok(principal) => principal,
            Err(refusal) => {
                let text = refusal.text;
                if replayed.is_none() {
                    record_ledger(
                        &mut ctx,
                        LedgerFields {
                            connection,
                            key,
                            action,
                            outcome: "refused",
                            refusal_code: Some(refusal.code),
                            ticket: Some((facts.ticket_id, &facts.number)),
                            message: None,
                            channel_id: None,
                            vendor_user_id,
                            actor_user_id: None,
                            detail: json!({ "reason": text, "refusal_text": text }),
                        },
                    )
                    .await?;
                }
                return refuse(ctx, text).await;
            }
        };

        let statuses = core_db::boards::list_statuses(&mut *ctx.tx, facts.board_id).await?;
        let edges = core_db::boards::list_transitions(&mut *ctx.tx, facts.board_id).await?;
        let card = build_card(
            &facts,
            &statuses,
            &edges,
            base_url.as_deref(),
            None,
        );
        if replayed.is_none() {
            record_ledger(
                &mut ctx,
                LedgerFields {
                    connection,
                    key,
                    action,
                    outcome: "applied",
                    refusal_code: None,
                    ticket: Some((facts.ticket_id, &facts.number)),
                    message: None,
                    channel_id: None,
                    vendor_user_id,
                    actor_user_id: Some(principal.user.id),
                    detail: json!({ "query": number }),
                },
            )
            .await?;
        }
        Ok((ctx, QueryOutcome::Card { card }))
    })
    .await
}

/// Normalises a typed ticket number: trimmed, upper-cased, internal whitespace
/// dropped. `t-1042` and `T-1042` are the same ticket.
fn normalise_number(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase()
}

// ---- the card --------------------------------------------------------------

/// Builds the one card both products render.
///
/// Actions come from the registry and are filtered by whether they are offered
/// on this ticket right now, so the card never shows a button whose only
/// possible answer is a refusal. Pure over its inputs: no database, no state.
fn build_card(
    facts: &TicketFacts,
    statuses: &[BoardStatus],
    edges: &[TransitionEdge],
    base_url: Option<&str>,
    footer: Option<String>,
) -> TicketCard {
    let mut actions: Vec<CardAction> = Vec::new();
    for action in registry() {
        if !action.offered(facts, statuses, edges) {
            continue;
        }
        match action.link(facts, base_url) {
            Some(url) => actions.push(CardAction::link(action.id(), action.label(), url)),
            None => actions.push(CardAction::press(action.id(), action.label())),
        }
    }
    TicketCard {
        number: facts.number.clone(),
        title: facts.title.clone(),
        client: facts.client_name.clone(),
        status: facts.status_name.clone(),
        priority: facts.priority.to_uppercase(),
        assignee: facts.assignee_name.clone(),
        url: base_url.map(|base| format!("{base}/tickets/{}", facts.ticket_id)),
        actions,
        footer,
    }
}

/// The deployment's web base, or `None` when it is not a browser-reachable URL.
fn web_base_url(state: &AppState) -> Option<String> {
    let raw = state.config.web_base_url.trim().trim_end_matches('/');
    if raw.starts_with("http://") || raw.starts_with("https://") {
        Some(raw.to_string())
    } else {
        None
    }
}

// ---- share from the ticket (G8) --------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct ShareTicketRequest {
    #[schema(value_type = uuid::Uuid)]
    pub connection_id: Uuid,
    /// The binding's vendor channel id, from the destination picker. Absent
    /// means the connection's first enabled destination.
    pub channel_ref: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SharedChannelView {
    pub binding_id: String,
    pub label: String,
    pub external_id: String,
    pub kind: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ShareTicketResponse {
    pub message_id: String,
    pub channel: SharedChannelView,
    /// The card exactly as it was posted, so the dialog can show what the
    /// channel will see.
    #[schema(value_type = Object)]
    pub card: Value,
}

/// POST /api/v1/tickets/{id}/share — post the ticket card into a bound channel.
///
/// Three rules, in this order:
///
/// 1. the caller must be able to read the ticket (module gate + client
///    restriction) — the same gate the ticket page uses;
/// 2. a ticket belonging to a **restricted client** is refused outright. The
///    channel's audience is not the ticket's audience, and redacting a card is
///    a promise that ages badly; refusing is the honest answer;
/// 3. the connection must have an enabled destination. A connection with no
///    binding cannot be a share target, and the answer says so instead of
///    failing at the vendor.
#[utoipa::path(
    operation_id = "tickets_share",
    post,
    path = "/api/v1/tickets/{id}/share",
    tag = "tickets",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ShareTicketRequest,
    responses(
        (status = 200, body = ShareTicketResponse),
        (status = 404, description = "ticket not visible in scope"),
        (status = 409, description = "the channel refused the card"),
        (status = 422, description = "no destination, or a restricted client"),
        (status = 503, description = "the channel endpoint is unreachable")
    )
)]
pub async fn share_ticket(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(ticket_id): Path<Uuid>,
    Json(req): Json<ShareTicketRequest>,
) -> ApiResult<Json<ShareTicketResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let principal = authed.principal;
    let org_id = principal.user.org_id;

    // The connection decides the vendor and the destination, so both are read
    // first. RLS scopes them to the org; the module gate comes after.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let connection =
        core_db::connector_connections::get_connection(&mut tx, org_id, req.connection_id)
            .await?
            .ok_or(ApiError::NotFound)?;
    let destinations =
        connector_messaging::list_destinations(&mut tx, org_id, connection.id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let Some(destination) = pick_destination(&destinations, req.channel_ref.as_deref()) else {
        return Err(ApiError::Validation(
            "this connection has no enabled channel destination; add one in its settings first"
                .into(),
        ));
    };
    let Some(vendor) = vendor_for_binding(&destination.binding_kind) else {
        return Err(ApiError::Validation(format!(
            "`{}` destinations cannot carry a ticket card",
            destination.binding_kind
        )));
    };

    if connection.status == ConnectionStatus::Disabled {
        return Err(ApiError::Validation(
            "this connection is disabled; enable it before sharing through it".into(),
        ));
    }

    // Read + restriction gate, in the transaction the ticket page would use.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let facts = connector_messaging::ticket_facts(&mut tx, org_id, ticket_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if !check(
        &principal.permissions,
        Permission::Matrix {
            module: Module::Tickets,
            action: PolicyAction::View,
        },
    )
    .is_allowed()
    {
        return Err(ApiError::NotFound);
    }
    if !facts.restricted_to.is_empty() {
        return Err(ApiError::Validation(
            "this ticket belongs to a restricted client; a channel audience is wider than the ticket's, so it cannot be shared"
                .into(),
        ));
    }
    let statuses = core_db::boards::list_statuses(&mut *tx, facts.board_id).await?;
    let edges = core_db::boards::list_transitions(&mut *tx, facts.board_id).await?;
    let base_url = web_base_url(&state);
    let footer = format!("Shared by {}", principal.user.display_name);
    let card = build_card(&facts, &statuses, &edges, base_url.as_deref(), Some(footer));
    let sealed = connection_secrets(&mut tx, connection.id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let credentials = Credentials(
        sealed
            .map(|bytes| {
                crate::crypto::open(&state.config.secret_key, &bytes)
                    .ok()
                    .and_then(|plain| serde_json::from_slice::<Value>(&plain).ok())
                    .unwrap_or(Value::Null)
            })
            .unwrap_or(Value::Null),
    );

    let post = outbound_post(vendor, &connection, &destination, &credentials, &card)?;
    let posted = transport().post(&post).await.map_err(|err| match err {
        TransportError::Refused(detail) => ApiError::Conflict(format!(
            "the {} channel refused the card: {detail}",
            vendor.as_str()
        )),
        TransportError::Unreachable(detail) => ApiError::Unavailable(format!(
            "the {} channel is unreachable: {detail}",
            vendor.as_str()
        )),
        TransportError::BadConfig(detail) => ApiError::Validation(detail),
    })?;

    // The registry row is written after the vendor accepted the message,
    // because its whole purpose is to name a message that exists. A share whose
    // post failed leaves nothing a press could resolve to.
    let mut ctx = org_tx::begin(&state, org_id, principal.user.id).await?;
    let message = connector_messaging::insert_message(
        &mut ctx.tx,
        connector_messaging::NewChannelMessage {
            id: Uuid::now_v7(),
            org_id,
            connection_id: connection.id,
            binding_id: destination.id,
            ticket_id,
            channel_id: &destination.external_id,
            message_id: &posted.message_id,
            shared_by: Some(principal.user.id),
            shared_client_id: facts.client_id,
        },
    )
    .await?;
    core_db::audit::insert(
        &mut *ctx.tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(principal.user.id),
            action: "ticket.shared".into(),
            entity_type: Some("ticket".into()),
            entity_id: Some(ticket_id.to_string()),
            detail: json!({
                "connection_id": connection.id,
                "binding_id": destination.id,
                "channel_id": destination.external_id,
                "channel_label": destination.label,
                "message_id": posted.message_id,
            }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    ctx.commit().await?;

    Ok(Json(ShareTicketResponse {
        message_id: message.message_id,
        channel: SharedChannelView {
            binding_id: destination.id.to_string(),
            label: destination.label,
            external_id: destination.external_id,
            kind: destination.binding_kind,
        },
        card: match vendor {
            Vendor::Slack => app_bridges::card::render_slack(&card),
            Vendor::Teams => app_bridges::card::render_teams(&card),
        },
    }))
}

/// The connection's sealed document, if it has one.
async fn connection_secrets(
    conn: &mut sqlx::PgConnection,
    connection_id: Uuid,
) -> ApiResult<Option<Vec<u8>>> {
    // `get_connection`'s column list deliberately omits the sealed blob, so
    // this is the one narrow read that selects it.
    let row = sqlx::query("SELECT secrets_encrypted FROM connector_connections WHERE id = $1")
        .bind(connection_id)
        .fetch_optional(conn)
        .await?;
    Ok(row.and_then(|row| {
        sqlx::Row::try_get::<Option<Vec<u8>>, _>(&row, "secrets_encrypted")
            .ok()
            .flatten()
    }))
}

fn pick_destination(
    destinations: &[Destination],
    channel_ref: Option<&str>,
) -> Option<Destination> {
    match channel_ref {
        Some(reference) => destinations
            .iter()
            .find(|destination| destination.external_id == reference)
            .cloned(),
        None => destinations.first().cloned(),
    }
}

fn vendor_for_binding(binding_kind: &str) -> Option<Vendor> {
    match binding_kind {
        "slack_channel" => Some(Vendor::Slack),
        "teams_conversation" => Some(Vendor::Teams),
        _ => None,
    }
}

// ---- outbound transport ----------------------------------------------------

/// One card delivery, described without a client so it can be replayed in a
/// test.
pub struct CardPost {
    pub vendor: Vendor,
    pub url: String,
    pub token: Option<String>,
    pub body: Value,
}

impl CardPost {
    /// The lead line, for a log. Never the token, never the body.
    fn describe(&self) -> String {
        format!("{} {}", self.vendor.as_str(), self.url)
    }
}

/// The vendor's id for the message it accepted (Slack `ts`, Teams activity id).
pub struct PostedMessage {
    pub message_id: String,
}

pub enum TransportError {
    /// The vendor answered and rejected the card. Its own words are kept.
    Refused(String),
    /// The endpoint could not be reached, or answered 5xx.
    Unreachable(String),
    /// The connection cannot post (no token, no service URL): a configuration
    /// answer, not a vendor one.
    BadConfig(String),
}

/// Delivery behind a trait, so the share flow is testable without a vendor.
///
/// The production implementation is [`HttpTransport`]; a test installs a
/// recorder with [`install_transport`] and asserts what would have been posted.
#[async_trait::async_trait]
pub trait ChannelTransport: Send + Sync {
    async fn post(&self, post: &CardPost) -> Result<PostedMessage, TransportError>;
}

static TRANSPORT: std::sync::OnceLock<std::sync::Arc<dyn ChannelTransport>> =
    std::sync::OnceLock::new();

/// Replaces the transport for this process. Test-only: production never calls
/// it, and the real implementation has no configuration that could point it at
/// anything but the documented endpoints.
pub fn install_transport(transport: std::sync::Arc<dyn ChannelTransport>) {
    let _ = TRANSPORT.set(transport);
}

fn transport() -> std::sync::Arc<dyn ChannelTransport> {
    TRANSPORT
        .get()
        .cloned()
        .unwrap_or_else(|| std::sync::Arc::new(HttpTransport))
}

/// The documented endpoints, behind the egress guard.
#[derive(Default)]
pub struct HttpTransport;

#[async_trait::async_trait]
impl ChannelTransport for HttpTransport {
    async fn post(&self, post: &CardPost) -> Result<PostedMessage, TransportError> {
        // Slack accepts the call at one fixed documented URL, so nothing in a
        // connection can point Core elsewhere. Teams' service URL comes from
        // the destination, and the guard screens it before a socket opens.
        let guarded = crate::outbound::guard(&post.url)
            .await
            .map_err(|denial| TransportError::BadConfig(format!("address refused: {denial}")))?;
        let client = guarded
            .client(std::time::Duration::from_secs(10))
            .map_err(|denial| TransportError::BadConfig(format!("address refused: {denial}")))?;

        let mut request = client.post(&post.url).json(&post.body);
        if let Some(token) = &post.token {
            request = request.bearer_auth(token);
        }
        tracing::info!(target = %post.describe(), "posting connector card");
        let response = request
            .send()
            .await
            .map_err(|e| TransportError::Unreachable(e.to_string()))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(if status.is_server_error() {
                TransportError::Unreachable(format!("HTTP {}", status.as_u16()))
            } else {
                TransportError::Refused(format!("HTTP {}: {}", status.as_u16(), truncate(&text)))
            });
        }
        let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        match post.vendor {
            Vendor::Slack => {
                // Slack answers 200 with `ok: false` for a refused message; a
                // success status alone is not a success.
                if body.get("ok").and_then(Value::as_bool) != Some(true) {
                    let error = body
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown_error");
                    return Err(TransportError::Refused(error.to_string()));
                }
                let ts = body.get("ts").and_then(Value::as_str).ok_or_else(|| {
                    TransportError::Refused("the response carried no message ts".into())
                })?;
                Ok(PostedMessage {
                    message_id: ts.to_string(),
                })
            }
            Vendor::Teams => {
                let id = body.get("id").and_then(Value::as_str).ok_or_else(|| {
                    TransportError::Refused("the response carried no activity id".into())
                })?;
                Ok(PostedMessage {
                    message_id: id.to_string(),
                })
            }
        }
    }
}

fn truncate(raw: &str) -> String {
    raw.chars().take(300).collect()
}

/// Builds the delivery for one destination from the connection's credentials.
fn outbound_post(
    vendor: Vendor,
    connection: &core_db::connector_connections::Connection,
    destination: &Destination,
    credentials: &Credentials,
    card: &TicketCard,
) -> ApiResult<CardPost> {
    match vendor {
        Vendor::Slack => {
            if credentials.get("bot_token").is_none() {
                return Err(ApiError::Validation(
                    "this Slack connection has no bot token stored, so it cannot post a card"
                        .into(),
                ));
            }
            let rendered = app_bridges::card::render_slack(card);
            // `chat.postMessage` wants the channel beside the blocks, not inside
            // the rendered envelope.
            let mut body = json!({
                "channel": destination.external_id,
                "text": rendered.get("text").cloned().unwrap_or(Value::Null),
                "blocks": rendered.get("blocks").cloned().unwrap_or(Value::Null),
            });
            if let Some(team) = destination.external_parent.as_deref() {
                // Harmless and explicit: Slack validates the pair, which is
                // what stops a channel id from one workspace resolving in
                // another.
                body["team_id"] = json!(team);
            }
            Ok(CardPost {
                vendor,
                url: "https://slack.com/api/chat.postMessage".into(),
                token: credentials.get("bot_token").map(str::to_string),
                body,
            })
        }
        Vendor::Teams => {
            if credentials.get("bot_token").is_none() {
                return Err(ApiError::Validation(
                    "this Teams connection has no bot token stored, so it cannot post a card"
                        .into(),
                ));
            }
            let service_url = destination
                .vendor_ids
                .get("service_url")
                .and_then(Value::as_str)
                .or_else(|| {
                    connection
                        .settings
                        .get("service_url")
                        .and_then(Value::as_str)
                })
                .ok_or_else(|| {
                    ApiError::Validation(
                        "this Teams destination has no Bot Framework service URL recorded".into(),
                    )
                })?;
            let conversation = encode_path_segment(&destination.external_id);
            Ok(CardPost {
                vendor,
                url: format!(
                    "{}/v3/conversations/{conversation}/activities",
                    service_url.trim_end_matches('/')
                ),
                token: credentials.get("bot_token").map(str::to_string),
                body: app_bridges::card::render_teams(card),
            })
        }
    }
}

/// Percent-encodes one path segment (RFC 3986 unreserved bytes kept).
///
/// A Teams conversation id contains `;`, `=` and sometimes `@`, none of which
/// may rewrite the path.
fn encode_path_segment(raw: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(raw.len());
    for byte in raw.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(*byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

// ---- the Teams verifier seam ----------------------------------------------

static BOT_AUTH: std::sync::OnceLock<
    std::sync::Arc<dyn app_bridges::teams_inbound::BotFrameworkAuth>,
> = std::sync::OnceLock::new();

/// Replaces the Bot Framework verifier for this process. Test-only: the
/// production verifier fetches Microsoft's published keys, and no test may
/// reach the network.
pub fn install_bot_auth(auth: std::sync::Arc<dyn app_bridges::teams_inbound::BotFrameworkAuth>) {
    let _ = BOT_AUTH.set(auth);
}

fn bot_auth() -> std::sync::Arc<dyn app_bridges::teams_inbound::BotFrameworkAuth> {
    BOT_AUTH.get().cloned().unwrap_or_else(|| {
        std::sync::Arc::new(app_bridges::teams_inbound::OpenIdBotFrameworkAuth::default())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(category: &str) -> TicketFacts {
        TicketFacts {
            ticket_id: Uuid::from_u128(1),
            number: "T-1042".into(),
            title: "Printer down".into(),
            board_id: Uuid::from_u128(2),
            status_id: Uuid::from_u128(3),
            status_name: "New".into(),
            status_category: category.into(),
            priority: "p2".into(),
            client_id: None,
            client_name: Some("Acme".into()),
            restricted_to: Vec::new(),
            assignee_id: None,
            assignee_name: None,
            updated_at: chrono::Utc::now(),
        }
    }

    fn status(id: u128, category: TicketStatusCategory) -> BoardStatus {
        BoardStatus {
            id: Uuid::from_u128(id),
            board_id: Uuid::from_u128(2),
            org_id: Uuid::from_u128(9),
            name: category.to_string(),
            category,
            sort_order: id as i32,
            is_entry: false,
            color: None,
            wip_limit: None,
            client_visible: true,
        }
    }

    fn principal(roles: Vec<core_domain::entities::Role>) -> Principal {
        Principal {
            user: core_domain::entities::User {
                id: Uuid::from_u128(70),
                org_id: Uuid::from_u128(9),
                email: "tech@example.test".into(),
                display_name: "Tech".into(),
                timezone: "UTC".into(),
                is_active: true,
                mfa_totp_secret: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            roles,
            permissions: Vec::new(),
        }
    }

    /// The registry is data: ids are unique, and the shipped kinds resolve.
    #[test]
    fn the_registry_has_unique_ids_and_the_three_shipped_kinds() {
        let mut ids: Vec<&str> = registry().iter().map(|a| a.id()).collect();
        ids.sort_unstable();
        let unique = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), unique, "registry ids must be unique");
        assert!(find_action("take").is_some());
        assert!(find_action("open").is_some());
        assert!(find_action("status:in_progress").is_some());
        assert!(find_action("status:closed").is_some());
        assert!(find_action("nonsense").is_none());
    }

    /// A status action is offered only when a legal edge to that category
    /// exists, so the card never shows a button whose answer is a refusal.
    #[test]
    fn status_actions_follow_the_transition_matrix() {
        let statuses = vec![
            status(3, TicketStatusCategory::Open),
            status(4, TicketStatusCategory::InProgress),
            status(5, TicketStatusCategory::Closed),
        ];
        let open = facts("open");
        // No explicit matrix: the semantic-category floor governs, and
        // open → in_progress is allowed.
        assert!(IN_PROGRESS.offered(&open, &statuses, &[]));
        // A closed ticket offers no status moves at all: closed → open is
        // prohibited by the floor, and closed → in_progress is not a floor.
        let closed = facts("closed");
        assert!(!IN_PROGRESS.offered(&closed, &statuses, &[]));
        // Already in the category: not offered again.
        let mut in_progress = facts("in_progress");
        in_progress.status_id = Uuid::from_u128(4);
        assert!(!IN_PROGRESS.offered(&in_progress, &statuses, &[]));
    }

    /// `take` is not offered on a closed ticket: it would have to reopen it.
    #[test]
    fn take_is_not_offered_on_a_closed_ticket() {
        let statuses = vec![status(3, TicketStatusCategory::Open)];
        assert!(TAKE.offered(&facts("open"), &statuses, &[]));
        assert!(!TAKE.offered(&facts("closed"), &statuses, &[]));
    }

    /// A restricted client's ticket is refused a share by the same rule the
    /// client route applies, and an admin is not exempt from the *share*
    /// refusal (that check lives on the handler, above this predicate).
    #[test]
    fn a_restricted_client_ticket_is_hidden_from_an_unlisted_user() {
        let mut restricted = facts("open");
        restricted.restricted_to = vec![Uuid::from_u128(77)];
        assert!(denied_by_client_restriction(
            &principal(Vec::new()),
            &restricted
        ));
        restricted.restricted_to.clear();
        assert!(!denied_by_client_restriction(
            &principal(Vec::new()),
            &restricted
        ));
    }

    #[test]
    fn ticket_numbers_normalise() {
        assert_eq!(normalise_number(" t-1042 "), "T-1042");
        assert_eq!(normalise_number("T - 1042"), "T-1042");
        assert_eq!(normalise_number("   "), "");
    }

    #[test]
    fn conversation_ids_cannot_rewrite_the_path() {
        assert_eq!(encode_path_segment("a;b=c/d"), "a%3Bb%3Dc%2Fd");
        assert_eq!(
            encode_path_segment("19:abc@thread.skype"),
            "19%3Aabc%40thread.skype"
        );
    }

    /// Both products get the same action set from one registry walk.
    ///
    /// `build_card` is a pure function over facts and the board's matrix, so
    /// this needs no database and no `AppState` — which is the property that
    /// keeps the card builder testable.
    #[test]
    fn the_card_carries_the_offered_actions_and_no_body() {
        let statuses = vec![
            status(3, TicketStatusCategory::Open),
            status(4, TicketStatusCategory::InProgress),
        ];
        let facts = facts("open");
        let card = build_card(
            &facts,
            &statuses,
            &[],
            Some("https://marula.test"),
            Some("Shared by Tech".into()),
        );
        assert_eq!(card.number, "T-1042");
        assert_eq!(card.priority, "P2");
        assert_eq!(card.footer.as_deref(), Some("Shared by Tech"));
        assert!(card
            .url
            .as_deref()
            .unwrap()
            .ends_with(&facts.ticket_id.to_string()));
        let ids: Vec<&str> = card.actions.iter().map(|a| a.id.as_str()).collect();
        assert!(ids.contains(&"take"));
        assert!(ids.contains(&"open"));
        assert!(ids.contains(&"status:in_progress"));
        // The card holds six facts and nothing else: a title is a fact, and a
        // body never reaches the card at all.
        let team_card = app_bridges::card::render_teams(&card).to_string();
        assert!(team_card.contains("T-1042"));
        assert!(!team_card.contains("reception"));
    }

    #[test]
    fn callback_keys_are_stable_for_the_same_bytes_and_differ_otherwise() {
        let one = callback_key(b"payload=%7B%7D");
        assert_eq!(one, callback_key(b"payload=%7B%7D"));
        assert_ne!(one, callback_key(b"payload=%7B%22a%22%7D"));
        assert_eq!(one.len(), 64);
    }
}
