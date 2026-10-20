//! Live-delivery half of the instant-first contract (Wave 12 lane 12).
//!
//! Purpose: push contract events to connected clients the moment their
//! transaction commits, plus ephemeral presence — the two halves the
//! correctness path (IF1–IF3) cannot deliver on its own. Big-team
//! concurrency needs both: "someone else is working on this ticket" is
//! presence; "their note just landed, reconcile" is the event stream.
//!
//! Pipeline: every mutation's outbox row (the authoritative trail) fires a
//! same-transaction `pg_notify` (migration 0068) on channel
//! `marula_events`. ONE dedicated listener connection per core-api
//! process ([`run_listener`], sqlx `PgListener` — never a per-request
//! connection) receives notifications at commit time and fans frames out
//! through the process-global [`hub`]. Delivery is at-least-once from the
//! client's perspective: a reconnecting client rehydrates from its
//! snapshot read model and dedupes by the per-entity monotonic `seq` the
//! frames carry (the ledger makes replay/duplicate frames idempotent).
//!
//! Frames are org-filtered at fan-out (a subscriber NEVER sees events from
//! another org — the hub drops or routes by the frame's `org_id`, never at
//! subscribe time). The per-user families (`notification.*`,
//! `notification_prefs.*`, `timer.*`) are narrower than the org: their payload
//! names one user, [`recipient_from`]
//! reads that id onto the frame, and the hub delivers such a frame only to
//! that user's subscribers. Replay applies the same predicate, so a
//! reconnecting member cannot read a gap the live stream withheld.
//! Per-connection buffers are bounded; a slow consumer
//! whose buffer fills is DROPPED (stream ends, client reconnects and
//! resyncs from snapshots) rather than buffering unbounded or blocking the
//! listener. Connections per user are capped.
//!
//! Presence ([`presence`]) is EPHEMERAL process memory with a TTL: it is
//! never written to the database, never enters the event trail, and never
//! registers in the contract registry — presence transitions ride the
//! stream as lightweight frames with `type: "presence"`, a distinct kind
//! from the typed contract events (see `routes::events`). Multi-replica
//! story (Wave 13 lane 10): every heartbeat is relayed to the OTHER
//! core-api replicas over the dedicated pg NOTIFY channel
//! [`PRESENCE_CHANNEL`] (`marula_presence`, no new infra dependency —
//! same LISTEN fan-out as contract events). Each replica applies the
//! relayed heartbeat into its local store, so every replica converges on
//! the same roster, TTLs it identically, and publishes the
//! `type: "presence"` transition frame to its own subscribers with the
//! frame shape IDENTICAL to the documented contract. The originating
//! replica's own NOTIFY round-trips back as a harmless no-op (the entry
//! already exists with the same editing flag — no duplicate frame).
//!
//! Resume (Wave 13 lane 10): every frame carries its event uuid as the
//! SSE `id:` field (contract events use the outbox row's uuid; presence
//! frames get a fresh per-frame uuid — replaying ephemeral presence is
//! meaningless, so a presence cursor simply finds no outbox row and
//! degrades to no replay). On reconnect a client sends `Last-Event-ID`;
//! the server replays the events committed AFTER that cursor from the
//! outbox ([`replay_after`]), bounded by [`REPLAY_BUFFER`]. The ledger
//! dedupes by seq, so replayed frames are idempotent alongside live
//! frames. Cutoff: when more than [`REPLAY_BUFFER`] events separate the
//! cursor from now, NO replay is served — the client falls back to its
//! full snapshot rehydration (the pre-resume behavior); an unknown cursor
//! id degrades the same way.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

/// The dedicated Postgres NOTIFY channel fed by migration 0068's trigger.
pub const NOTIFY_CHANNEL: &str = "marula_events";

/// The dedicated Postgres NOTIFY channel for cross-replica presence
/// relay (Wave 13 lane 10). Payload is one heartbeat's identity; see
/// [`HeartbeatNotify`]. No migration needed — NOTIFY channels are
/// runtime-only.
pub const PRESENCE_CHANNEL: &str = "marula_presence";

/// Maximum number of outbox events replayed on `Last-Event-ID` resume.
/// Past this bound the client is too stale for a replay to be useful —
/// it rehydrates from snapshots instead (documented cutoff).
pub const REPLAY_BUFFER: usize = 256;

/// How many raw outbox rows a resume scan may read for each frame the replay
/// buffer will serve. A per-user frame can sit behind other members' rows, so
/// the scan gets headroom over the buffer; four pages is the ceiling, and past
/// it the honest answer is "cannot prove completeness" (see [`replay_after`]).
const REPLAY_SCAN_FACTOR: usize = 4;

/// Connection cap per user — bounds fan-out memory per identity.
pub const MAX_CONNECTIONS_PER_USER: usize = 8;

/// Per-subscriber frame buffer. A full buffer means the consumer stopped
/// reading: drop it (client resyncs from snapshots — the ledger exists for
/// exactly this) rather than blocking the listener or growing memory.
const CHANNEL_BUFFER: usize = 64;

fn connection_cap() -> usize {
    static CAP: OnceLock<usize> = OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("MARULA_SSE_MAX_CONNECTIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|v| *v > 0)
            .unwrap_or(MAX_CONNECTIONS_PER_USER)
    })
}

fn replay_buffer() -> usize {
    static BUF: OnceLock<usize> = OnceLock::new();
    *BUF.get_or_init(|| {
        std::env::var("MARULA_SSE_REPLAY_BUFFER")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|v| *v > 0)
            .unwrap_or(REPLAY_BUFFER)
    })
}

fn presence_ttl() -> Duration {
    static TTL: OnceLock<Duration> = OnceLock::new();
    *TTL.get_or_init(|| {
        std::env::var("MARULA_PRESENCE_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|v| *v > 0)
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(90))
    })
}

/// One stream frame — the SSE `data:` JSON. Contract-event frames carry
/// exactly `{type, org_id, entity, entity_id, seq, at}` (the wire contract
/// of the notify payload, `type` = the contract event type), plus
/// `recipient_user_id` on the per-user families; presence frames carry
/// `type: "presence"` plus the current `viewers` roster and are EPHEMERAL
/// — never contract events, never in the trail.
#[derive(Debug, Clone, Serialize)]
pub struct StreamFrame {
    /// Contract event type (e.g. `ticket.updated`) or `"presence"`.
    #[serde(rename = "type")]
    pub event_type: String,
    pub org_id: Uuid,
    pub entity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    /// Per-entity monotonic ledger seq (`null` for events emitted outside
    /// the instant path — clients treat a null seq as always-apply).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
    pub at: DateTime<Utc>,
    /// The one user this frame is addressed to, for the per-user event
    /// families ([`PER_USER_EVENT_FAMILIES`]) whose payload names a
    /// `user_id`. The hub delivers such a frame ONLY to that user's
    /// subscribers — the recipient is what the deleted socket leg's
    /// `user:<id>` channels carried. `None` keeps the org-wide behaviour:
    /// every subscriber in the org sees it.
    ///
    /// Skipped when absent, so org-scoped frames keep the exact wire shape
    /// they had before this field existed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient_user_id: Option<Uuid>,
    /// Presence roster — present ONLY on `type: "presence"` frames.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewers: Option<Vec<PresenceViewer>>,
    /// The event uuid carried as the SSE `id:` field (Wave 13 lane 10).
    /// Contract events carry the outbox row's uuid; presence frames a
    /// per-frame uuid. `serde(skip)` — the JSON wire contract above is
    /// UNCHANGED; the uuid rides the SSE envelope only.
    #[serde(skip)]
    pub id: Option<Uuid>,
}

/// One presence viewer on an entity (roster + roster frames).
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct PresenceViewer {
    pub user_id: Uuid,
    pub name: String,
    #[schema(value_type = String)]
    pub at: DateTime<Utc>,
    /// True while the user is actively editing (vs. just viewing).
    pub editing: bool,
}

/// The parsed notify payload shape (migration 0068, plus the recipient added
/// by migration 0141). `user_id` is the payload's own `user_id` field when it
/// had one — present for the per-user families, `null` otherwise — and it is
/// the ONLY part of the envelope body the NOTIFY carries. The body stays in
/// the outbox: a whole payload on the channel would hand every subscriber's
/// frame to every replica and can exceed the 8000-byte NOTIFY limit.
#[derive(Debug, Clone, serde::Deserialize)]
struct NotifyFrame {
    id: Uuid,
    org_id: Uuid,
    #[serde(rename = "type")]
    event_type: String,
    entity: String,
    entity_id: Option<String>,
    seq: Option<i64>,
    at: DateTime<Utc>,
    /// The payload's `user_id`, verbatim. Deliberately not `Option<Uuid>`:
    /// a payload whose `user_id` is not a uuid must not fail deserialization
    /// and drop the whole frame, so the id stays untyped here and
    /// [`recipient_from`] decides whether it is one.
    #[serde(default)]
    user_id: Option<serde_json::Value>,
}

/// The event families whose events belong to one named user rather than to
/// the whole org.
///
/// This is an allow-list of type prefixes, not "any payload that happens to
/// carry a `user_id`", because other families name a user too: ticket and
/// appointment payloads carry assignees and technicians, and those events are
/// still org news. `notification_prefs.updated` is separate from the
/// `notification.` prefix but belongs here: it carries one user's quiet hours
/// and channel flags. A new per-user family is a one-line addition here, in
/// the single place the decision is written down.
const PER_USER_EVENT_FAMILIES: [&str; 3] = ["notification.", "notification_prefs.", "timer."];

/// The recipient of one event, read from its envelope payload — or `None` for
/// the org-wide families.
///
/// Both paths that build a [`StreamFrame`] call this: the live listener (which
/// passes the `user_id` subset its NOTIFY carries) and [`replay_after`] (which
/// passes the stored payload), so a live frame and its replayed twin can never
/// disagree about who the event was for.
pub fn recipient_from(event_type: &str, payload: &serde_json::Value) -> Option<Uuid> {
    if !PER_USER_EVENT_FAMILIES
        .iter()
        .any(|family| event_type.starts_with(family))
    {
        return None;
    }
    payload
        .get("user_id")
        .and_then(|value| value.as_str())
        .and_then(|value| Uuid::parse_str(value).ok())
}

/// Both predicates a frame must pass to reach a subscriber: the entity scope
/// (when the subscriber asked for one entity instance) and the recipient (when
/// the frame is addressed to one user). A frame addressed to another user does
/// not reach this subscriber even if its entity scope matches, because the
/// recipient is the narrower of the two guarantees.
fn frame_visible(
    user_id: Uuid,
    scope: Option<&(String, String)>,
    entity: &str,
    entity_id: Option<&str>,
    recipient: Option<Uuid>,
) -> bool {
    let recipient_matches = recipient.is_none_or(|recipient| recipient == user_id);
    recipient_matches && scope_matches(scope, entity, entity_id)
}

fn scope_matches(scope: Option<&(String, String)>, entity: &str, entity_id: Option<&str>) -> bool {
    match scope {
        None => true,
        Some((scoped_entity, scoped_id)) => {
            entity == scoped_entity && entity_id.is_some_and(|id| id == scoped_id)
        }
    }
}

/// The process-global fan-out hub. `Mutex` (not async) keeps publish a
/// non-blocking, allocation-light operation off the listener hot path.
pub struct Hub {
    inner: Mutex<HubInner>,
}

#[derive(Debug)]
struct Subscriber {
    user_id: Uuid,
    /// `Some((entity, entity_id))` — only frames for that entity instance.
    /// `None` — org-wide (all frames in the org).
    scope: Option<(String, String)>,
    tx: mpsc::Sender<Arc<StreamFrame>>,
}

#[derive(Default)]
struct HubInner {
    orgs: HashMap<Uuid, Vec<Subscriber>>,
    per_user: HashMap<(Uuid, Uuid), usize>,
}

/// The hub's delivery predicate for one subscriber: the recipient filter and
/// the entity scope, both of which must hold.
fn subscriber_visible(sub: &Subscriber, frame: &StreamFrame) -> bool {
    frame_visible(
        sub.user_id,
        sub.scope.as_ref(),
        &frame.entity,
        frame.entity_id.as_deref(),
        frame.recipient_user_id,
    )
}

impl Hub {
    fn new() -> Self {
        Self {
            inner: Mutex::new(HubInner::default()),
        }
    }

    /// Registers a subscriber for `org_id`, optionally scoped to one
    /// entity instance. Rejects (None) when the per-user connection cap is
    /// hit — bounded fan-out memory per identity.
    pub fn subscribe(
        &self,
        org_id: Uuid,
        user_id: Uuid,
        scope: Option<(String, String)>,
    ) -> Option<mpsc::Receiver<Arc<StreamFrame>>> {
        let (tx, rx) = mpsc::channel(CHANNEL_BUFFER);
        let mut inner = self.inner.lock().unwrap();
        // Lazily reclaim connections whose client went away, so the cap
        // counts live subscribers, not leaked streams.
        Self::prune_closed(&mut inner);
        let count = inner.per_user.entry((org_id, user_id)).or_default();
        if *count >= connection_cap() {
            return None;
        }
        *count += 1;
        inner
            .orgs
            .entry(org_id)
            .or_default()
            .push(Subscriber { user_id, scope, tx });
        Some(rx)
    }

    fn prune_closed(inner: &mut HubInner) {
        inner.orgs.retain(|org, subs| {
            subs.retain(|s| {
                let live = !s.tx.is_closed();
                if !live {
                    if let Some(c) = inner.per_user.get_mut(&(*org, s.user_id)) {
                        *c -= 1;
                        if *c == 0 {
                            inner.per_user.remove(&(*org, s.user_id));
                        }
                    }
                }
                live
            });
            !subs.is_empty()
        });
    }

    /// Fan-out to one org's subscribers. Org filtering happens HERE — the
    /// frame only ever reaches subscribers of its own org. A frame addressed
    /// to one user ([`StreamFrame::recipient_user_id`]) reaches only that
    /// user's subscribers, then the entity scope still applies on top; an
    /// unaddressed frame keeps the org-wide behaviour.
    ///
    /// The recipient filter belongs on the server, not in the client: a
    /// subscriber that receives another member's notification can read its
    /// type and entity id, and a client is in no position to promise it will
    /// ignore them. Slow/closed consumers are dropped (never block the
    /// listener).
    pub fn publish(&self, frame: StreamFrame) {
        let arc = Arc::new(frame);
        let org = arc.org_id;
        let mut dropped: Vec<Uuid> = Vec::new();
        let mut inner = self.inner.lock().unwrap();
        if let Some(subs) = inner.orgs.get_mut(&org) {
            subs.retain(|s| {
                if !subscriber_visible(s, &arc) {
                    return true;
                }
                match s.tx.try_send(Arc::clone(&arc)) {
                    Ok(()) => true,
                    // Slow consumer: buffer full — drop, client resyncs.
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        dropped.push(s.user_id);
                        false
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        dropped.push(s.user_id);
                        false
                    }
                }
            });
            if subs.is_empty() {
                inner.orgs.remove(&org);
            }
        }
        for user in dropped {
            if let Some(c) = inner.per_user.get_mut(&(org, user)) {
                *c -= 1;
                if *c == 0 {
                    inner.per_user.remove(&(org, user));
                }
            }
        }
    }

    /// Live subscriber count for tests/telemetry.
    #[cfg(test)]
    pub fn subscriber_count(&self, org_id: Uuid) -> usize {
        self.inner
            .lock()
            .unwrap()
            .orgs
            .get(&org_id)
            .map_or(0, Vec::len)
    }
}

pub fn hub() -> &'static Hub {
    static HUB: OnceLock<Hub> = OnceLock::new();
    HUB.get_or_init(Hub::new)
}

// --- ephemeral presence ------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PresenceKey {
    org_id: Uuid,
    entity: String,
    entity_id: String,
}

#[derive(Debug, Clone)]
struct PresenceEntry {
    name: String,
    at: DateTime<Utc>,
    editing: bool,
}

/// In-process ephemeral presence: who is viewing/editing what, TTL-expired.
/// Never persisted, never in the event trail, never a contract event.
pub struct PresenceStore {
    inner: Mutex<HashMap<PresenceKey, HashMap<Uuid, PresenceEntry>>>,
}

pub fn presence() -> &'static PresenceStore {
    static STORE: OnceLock<PresenceStore> = OnceLock::new();
    STORE.get_or_init(|| PresenceStore {
        inner: Mutex::new(HashMap::new()),
    })
}

fn roster_of(entries: &HashMap<Uuid, PresenceEntry>) -> Vec<PresenceViewer> {
    let mut viewers: Vec<PresenceViewer> = entries
        .iter()
        .map(|(user_id, e)| PresenceViewer {
            user_id: *user_id,
            name: e.name.clone(),
            at: e.at,
            editing: e.editing,
        })
        .collect();
    viewers.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.user_id.cmp(&b.user_id)));
    viewers
}

impl PresenceStore {
    fn prune(bucket: &mut HashMap<Uuid, PresenceEntry>, now: DateTime<Utc>) {
        let ttl = presence_ttl();
        bucket.retain(|_, e| now.signed_duration_since(e.at).to_std().unwrap_or_default() < ttl);
    }

    /// Shared upsert: applies one heartbeat (local or relayed from
    /// another replica) at `hb.at`, prunes, and reports whether it was a
    /// TRANSITION (new viewer or editing-flag flip) — transitions are
    /// what get published as `type: "presence"` frames. A re-applied
    /// heartbeat with an unchanged editing flag (e.g. the origin's own
    /// NOTIFY round-trip) refreshes `at` silently — no duplicate frame.
    fn apply_heartbeat(&self, hb: &HeartbeatNotify) -> (Vec<PresenceViewer>, bool) {
        let HeartbeatNotify {
            org_id,
            user_id,
            name,
            entity,
            entity_id,
            editing,
            at,
        } = hb;
        let editing = *editing;
        let at = *at;
        let user_id = *user_id;
        let key = PresenceKey {
            org_id: *org_id,
            entity: entity.clone(),
            entity_id: entity_id.clone(),
        };
        let mut transition = false;
        let viewers = {
            let mut inner = self.inner.lock().unwrap();
            let bucket = inner.entry(key).or_default();
            Self::prune(bucket, at);
            match bucket.get_mut(&user_id) {
                // STALE (last-write-wins): a relayed heartbeat can
                // round-trip AFTER newer state for the same viewer exists
                // locally — most importantly the ORIGIN's own NOTIFY
                // arriving back after its local apply (and after any
                // newer editing flip). Re-applying it would regress the
                // editing flag and re-publish a phantom transition, so
                // anything not strictly newer than the stored entry is a
                // no-op.
                Some(entry) if entry.at >= at => {}
                Some(entry) if entry.editing == editing => {
                    entry.at = at;
                    entry.name = name.clone();
                }
                Some(entry) => {
                    entry.at = at;
                    entry.name = name.clone();
                    entry.editing = editing;
                    transition = true;
                }
                None => {
                    bucket.insert(
                        user_id,
                        PresenceEntry {
                            name: name.clone(),
                            at,
                            editing,
                        },
                    );
                    transition = true;
                }
            }
            roster_of(bucket)
        };
        (viewers, transition)
    }

    fn publish_transition(
        &self,
        org_id: Uuid,
        entity: String,
        entity_id: String,
        at: DateTime<Utc>,
        viewers: Vec<PresenceViewer>,
    ) {
        hub().publish(StreamFrame {
            event_type: "presence".into(),
            org_id,
            entity,
            entity_id: Some(entity_id),
            seq: None,
            at,
            recipient_user_id: None,
            viewers: Some(viewers),
            id: Some(Uuid::now_v7()),
        });
    }

    /// Upserts a heartbeat. On a transition (new viewer, or editing flag
    /// flip) publishes a lightweight presence frame to the org's stream
    /// subscribers. Returns the current roster.
    pub fn heartbeat(
        &self,
        org_id: Uuid,
        user_id: Uuid,
        name: String,
        entity: String,
        entity_id: String,
        editing: bool,
    ) -> Vec<PresenceViewer> {
        let now = Utc::now();
        let hb = HeartbeatNotify {
            org_id,
            user_id,
            name,
            entity: entity.clone(),
            entity_id: entity_id.clone(),
            editing,
            at: now,
        };
        let (viewers, transition) = self.apply_heartbeat(&hb);
        if transition {
            self.publish_transition(org_id, entity, entity_id, now, viewers.clone());
        }
        viewers
    }

    /// Applies a heartbeat RELAYED from another replica (off
    /// [`PRESENCE_CHANNEL`] by [`run_listener`]). Anchors the entry at the
    /// ORIGIN's heartbeat timestamp so every replica TTLs identically,
    /// and publishes the identical transition frame to this replica's
    /// subscribers. Frame shape matches [`Self::heartbeat`] exactly.
    pub fn apply_remote_heartbeat(&self, hb: HeartbeatNotify) {
        let (viewers, transition) = self.apply_heartbeat(&hb);
        if transition {
            self.publish_transition(
                hb.org_id,
                hb.entity.clone(),
                hb.entity_id.clone(),
                hb.at,
                viewers,
            );
        }
    }

    /// Current roster with expired entries pruned.
    pub fn roster(&self, org_id: Uuid, entity: &str, entity_id: &str) -> Vec<PresenceViewer> {
        let now = Utc::now();
        let mut inner = self.inner.lock().unwrap();
        let Some(bucket) = inner.get_mut(&PresenceKey {
            org_id,
            entity: entity.to_string(),
            entity_id: entity_id.to_string(),
        }) else {
            return Vec::new();
        };
        Self::prune(bucket, now);
        roster_of(bucket)
    }
}

/// One relayed heartbeat on [`PRESENCE_CHANNEL`] — the identity needed for
/// every replica to converge its local store and TTL identically.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HeartbeatNotify {
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub entity: String,
    pub entity_id: String,
    pub editing: bool,
    pub at: DateTime<Utc>,
}

/// Relays one heartbeat to all core-api replicas over pg NOTIFY. Never
/// blocks the heartbeat response: callers fire-and-forget (log-only on
/// failure — the local store is already updated, and a missed relay only
/// delays cross-replica convergence by one heartbeat).
pub async fn publish_heartbeat_notify(
    pool: &sqlx::PgPool,
    hb: &HeartbeatNotify,
) -> Result<(), sqlx::Error> {
    let payload = serde_json::to_string(hb).unwrap_or_default();
    sqlx::query("SELECT pg_notify($1, $2)")
        .bind(PRESENCE_CHANNEL)
        .bind(payload)
        .execute(pool)
        .await?;
    Ok(())
}

// --- listener ----------------------------------------------------------------

/// Long-running dedicated LISTEN connection. Never per-request: ONE
/// connection per process receives every `marula_events` notification at
/// commit time and fans it out through the hub, AND every
/// `marula_presence` relayed heartbeat (cross-replica presence,
/// Wave 13 lane 10). Reconnects with backoff; missed notifications during
/// a disconnect are covered by the client's snapshot hydration on
/// reconnect (at-least-once over the ledger) and by the next heartbeat.
pub async fn run_listener(database_url: String) {
    let policy = core_domain::retry::policy(core_domain::retry::Subsystem::Relay);
    let mut failed_passes = 0_u32;
    loop {
        match listener_session(&database_url).await {
            Ok(()) => {
                // Unreachable: listener_session only returns on error.
                failed_passes = 0;
            }
            Err(e) => {
                tracing::warn!(error = %e, "event listener disconnected; reconnecting");
                failed_passes = failed_passes.saturating_add(1);
                tokio::time::sleep(policy.delay(failed_passes, core_domain::retry::sample())).await;
            }
        }
    }
}

async fn listener_session(database_url: &str) -> Result<(), sqlx::Error> {
    let mut listener = sqlx::postgres::PgListener::connect(database_url).await?;
    listener.listen(NOTIFY_CHANNEL).await?;
    listener.listen(PRESENCE_CHANNEL).await?;
    tracing::info!(
        channels = %format!("{NOTIFY_CHANNEL} + {PRESENCE_CHANNEL}"),
        "contract-event + presence-relay listener connected"
    );
    loop {
        let notification = listener.recv().await?;
        match notification.channel() {
            NOTIFY_CHANNEL => match serde_json::from_str::<NotifyFrame>(notification.payload()) {
                Ok(n) => {
                    // The recipient comes from the same derivation the replay
                    // path uses. The NOTIFY ships only the payload's `user_id`
                    // (migration 0141), never the body, so the subset handed
                    // over here is all `recipient_from` reads.
                    let notified_user = serde_json::json!({ "user_id": n.user_id });
                    let recipient_user_id = recipient_from(&n.event_type, &notified_user);
                    let frame = StreamFrame {
                        id: Some(n.id),
                        event_type: n.event_type,
                        org_id: n.org_id,
                        entity: n.entity,
                        entity_id: n.entity_id,
                        seq: n.seq,
                        at: n.at,
                        recipient_user_id,
                        viewers: None,
                    };
                    // Reference cache (Wave 52 lane P3): the same event that
                    // reaches subscribers drops the cached values it changed,
                    // so a stale read cannot outlive its mutation by more than
                    // the time to fan this frame out.
                    crate::cache_invalidation::invalidate_for(&frame);
                    hub().publish(frame);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "malformed notify payload on {NOTIFY_CHANNEL}")
                }
            },
            PRESENCE_CHANNEL => {
                match serde_json::from_str::<HeartbeatNotify>(notification.payload()) {
                    Ok(hb) => presence().apply_remote_heartbeat(hb),
                    Err(e) => {
                        tracing::warn!(error = %e, "malformed relayed heartbeat on {PRESENCE_CHANNEL}")
                    }
                }
            }
            _ => {}
        }
    }
}

// --- Last-Event-ID resume replay ---------------------------------------------

/// A replayable outbox row — exactly the identity a live frame carries,
/// read back from the authoritative trail (the outbox IS the buffer). The
/// payload rides along because it is where a per-user event's recipient
/// lives; the scan reads at most `replay_buffer() * REPLAY_SCAN_FACTOR` rows
/// before it declares the gap unprovable.
#[derive(sqlx::FromRow)]
struct ReplayRow {
    id: Uuid,
    event_type: String,
    entity: String,
    entity_id: Option<String>,
    occurred_at: DateTime<Utc>,
    seq: Option<i64>,
    payload: serde_json::Value,
}

/// One page of replay rows after `after` (a `(occurred_at, id)` cursor), in
/// commit order. The keyset comparison is what lets a page boundary fall
/// anywhere in the gap without skipping or repeating a row: an outbox row's
/// id is unique, and `(occurred_at, id)` is a total order even when several
/// events share a timestamp.
async fn fetch_replay_page(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    after: (DateTime<Utc>, Uuid),
    scope: Option<&(String, String)>,
    limit: i64,
) -> Result<Vec<ReplayRow>, sqlx::Error> {
    // seq is joined exactly the way migration 0068's trigger does, minus
    // the text->uuid cast hazard: entity_write_seqs.entity_id is uuid, so
    // compare uuid::text — a non-uuid outbox entity_id simply finds no
    // counter and degrades to seq: null (same as the trigger).
    match scope {
        Some((entity, entity_id)) => {
            sqlx::query_as::<_, ReplayRow>(
                r#"SELECT o.id, o.event_type, o.entity, o.entity_id, o.occurred_at, o.payload,
                      (SELECT w.seq FROM entity_write_seqs w
                       WHERE w.org_id = o.org_id AND w.entity = o.entity
                         AND w.entity_id::text = o.entity_id) AS seq
               FROM outbox_events o
               WHERE o.org_id = $1 AND (o.occurred_at, o.id) > ($2, $3)
                 AND o.entity = $4 AND o.entity_id = $5
               ORDER BY o.occurred_at, o.id
               LIMIT $6"#,
            )
            .bind(org_id)
            .bind(after.0)
            .bind(after.1)
            .bind(entity)
            .bind(entity_id)
            .bind(limit)
            .fetch_all(&mut *tx)
            .await
        }
        None => {
            sqlx::query_as::<_, ReplayRow>(
                r#"SELECT o.id, o.event_type, o.entity, o.entity_id, o.occurred_at, o.payload,
                      (SELECT w.seq FROM entity_write_seqs w
                       WHERE w.org_id = o.org_id AND w.entity = o.entity
                         AND w.entity_id::text = o.entity_id) AS seq
               FROM outbox_events o
               WHERE o.org_id = $1 AND (o.occurred_at, o.id) > ($2, $3)
               ORDER BY o.occurred_at, o.id
               LIMIT $4"#,
            )
            .bind(org_id)
            .bind(after.0)
            .bind(after.1)
            .bind(limit)
            .fetch_all(&mut *tx)
            .await
        }
    }
}

/// Replays the events committed AFTER `last_event_id` for `org_id`
/// (optionally narrowed to one entity instance), from the outbox — the
/// authoritative trail, so replay is exact and the ledger's seq dedupe
/// makes it idempotent against live frames. Bounds, all measured over the
/// frames this caller would actually receive:
///
/// * Unknown cursor (never seen, or a presence-frame uuid): no replay.
/// * More than [`replay_buffer()`] visible frames after the cursor: no replay
///   — the client fell past the buffer and rehydrates from snapshots.
/// * The scan cannot prove it reached the end of the gap within
///   `replay_buffer() * REPLAY_SCAN_FACTOR` raw rows: no replay, same
///   rehydration path. A partial replay that silently skips history is worse
///   than none.
///
/// The scan pages by `(occurred_at, id)` and filters each row through
/// [`frame_visible`] BEFORE counting it. That order is the point: a member
/// with a handful of own events behind a page of other members' traffic gets
/// those events replayed, and busy org traffic alone never trips the cutoff
/// for everyone. The family allow-list stays in Rust; SQL only copies the
/// raw rows.
///
/// Each frame carries its recipient derived by [`recipient_from`], the same
/// function the live listener uses; rows addressed to a user other than
/// `user_id` are skipped, because replay bypasses the hub and would
/// otherwise hand a reconnecting member the frames the hub just filtered
/// out of the live stream.
///
/// Returns the frames in commit order with their ids attached, ready to
/// be streamed as SSE events with the `id:` field set, plus whether the
/// cursor fell PAST the replay buffer (so the endpoint can tell the
/// client apart from a plain "nothing missed").
///
/// RLS scope (Wave 18 lane 2b): `outbox_events` and `entity_write_seqs`
/// are RLS-gated, so the replay MUST ride an org-bound tx — the caller
/// opens it and runs `set_org_and_user` first. The old raw-pool read was
/// silently RLS-filtered to empty (the exact unscoped-pool-read class
/// `check-rls-pool-reads.sh` exists for).
pub async fn replay_after(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    scope: Option<(String, String)>,
    last_event_id: &str,
) -> (Vec<StreamFrame>, bool) {
    let Ok(cursor) = Uuid::parse_str(last_event_id) else {
        return (Vec::new(), false);
    };
    // The cursor must resolve to an outbox row IN THIS ORG. A miss (too
    // old, foreign org, or an ephemeral presence uuid) degrades to no
    // replay — the client's snapshot rehydration covers it.
    let cursor_at: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT occurred_at FROM outbox_events WHERE id = $1 AND org_id = $2")
            .bind(cursor)
            .bind(org_id)
            .fetch_optional(&mut *tx)
            .await
            .ok()
            .flatten();
    let Some(cursor_at) = cursor_at else {
        return (Vec::new(), false);
    };

    let buffer = replay_buffer();
    let page_size = buffer as i64;
    let raw_budget = buffer.saturating_mul(REPLAY_SCAN_FACTOR) as i64;
    let mut frames: Vec<StreamFrame> = Vec::new();
    let mut raw_read: i64 = 0;
    let mut after = (cursor_at, cursor);
    loop {
        let rows = match fetch_replay_page(&mut *tx, org_id, after, scope.as_ref(), page_size).await
        {
            Ok(rows) => rows,
            Err(_) => {
                // A failed page cannot prove the gap was scanned to its end.
                // Report the cutoff instead of an empty "nothing missed".
                tracing::warn!("resume replay query failed; client falls back to rehydration");
                return (Vec::new(), true);
            }
        };
        let fetched = rows.len() as i64;
        if fetched == 0 {
            // The scan reached the end of the gap with every page proven.
            return (frames, false);
        }
        raw_read += fetched;
        for row in rows {
            after = (row.occurred_at, row.id);
            let recipient_user_id = recipient_from(&row.event_type, &row.payload);
            // Replay bypasses the hub, so it applies the hub's own
            // predicate rather than trusting the caller to filter.
            if !frame_visible(
                user_id,
                scope.as_ref(),
                &row.entity,
                row.entity_id.as_deref(),
                recipient_user_id,
            ) {
                continue;
            }
            frames.push(StreamFrame {
                id: Some(row.id),
                event_type: row.event_type,
                org_id,
                entity: row.entity,
                entity_id: row.entity_id,
                seq: row.seq,
                at: row.occurred_at,
                recipient_user_id,
                viewers: None,
            });
            if frames.len() > buffer {
                // Too much for this caller to take as a delta. Serve nothing
                // (a partial replay that skips events would be worse than
                // none) — the client's full snapshot rehydration is the
                // recovery path.
                tracing::info!(org_id = %org_id, "resume cursor past replay buffer; skipping replay");
                return (Vec::new(), true);
            }
        }
        if fetched < page_size {
            return (frames, false);
        }
        if raw_read >= raw_budget {
            tracing::info!(
                org_id = %org_id,
                raw_read,
                "resume scan hit its raw budget before the end of the gap; skipping replay"
            );
            return (Vec::new(), true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn frame(org: Uuid, entity: &str, entity_id: &str, recipient: Option<Uuid>) -> StreamFrame {
        StreamFrame {
            event_type: "notification.created".into(),
            org_id: org,
            entity: entity.into(),
            entity_id: Some(entity_id.into()),
            seq: Some(1),
            at: Utc::now(),
            recipient_user_id: recipient,
            viewers: None,
            id: Some(Uuid::new_v4()),
        }
    }

    /// Drains one subscriber without blocking: `Some` when the frame arrived.
    fn took(rx: &mut mpsc::Receiver<Arc<StreamFrame>>) -> Option<Arc<StreamFrame>> {
        match rx.try_recv() {
            Ok(frame) => Some(frame),
            Err(mpsc::error::TryRecvError::Empty) => None,
            Err(mpsc::error::TryRecvError::Disconnected) => None,
        }
    }

    #[test]
    fn a_user_addressed_frame_reaches_that_user_and_not_the_other_subscriber() {
        let hub = Hub::new();
        let org = Uuid::new_v4();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let mut rx_a = hub.subscribe(org, a, None).expect("a subscribes");
        let mut rx_b = hub.subscribe(org, b, None).expect("b subscribes");
        assert_eq!(hub.subscriber_count(org), 2, "two members, two streams");

        hub.publish(frame(org, "notification", "n1", Some(a)));

        assert_eq!(took(&mut rx_a).map(|f| f.recipient_user_id), Some(Some(a)));
        assert!(took(&mut rx_b).is_none(), "the other member saw nothing");
    }

    #[test]
    fn the_same_frame_without_a_recipient_reaches_every_subscriber() {
        let hub = Hub::new();
        let org = Uuid::new_v4();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let mut rx_a = hub.subscribe(org, a, None).expect("a subscribes");
        let mut rx_b = hub.subscribe(org, b, None).expect("b subscribes");

        hub.publish(frame(org, "ticket", "t1", None));

        assert!(took(&mut rx_a).is_some(), "org news reaches a");
        assert!(took(&mut rx_b).is_some(), "org news reaches b");
    }

    #[test]
    fn the_recipient_and_the_entity_scope_both_have_to_hold() {
        let hub = Hub::new();
        let org = Uuid::new_v4();
        let (a, scoped) = (Uuid::new_v4(), Uuid::new_v4());
        let mut rx_a = hub.subscribe(org, a, None).expect("a subscribes");
        let mut rx_scoped = hub
            .subscribe(org, scoped, Some(("ticket".into(), "t1".into())))
            .expect("scoped subscribes");

        // Addressed to `a`, but for a different entity instance: the scoped
        // subscriber's entity predicate fails even though it is not the
        // addressed user's concern either way.
        hub.publish(frame(org, "ticket", "t2", Some(a)));

        assert!(took(&mut rx_a).is_some(), "a receives its own frame");
        assert!(
            took(&mut rx_scoped).is_none(),
            "a scope on t1 never sees t2, recipient or not"
        );

        // Addressed to `a` but inside the scoped subscriber's entity: the
        // scope holds and the recipient does not, so it stays out.
        hub.publish(frame(org, "ticket", "t1", Some(a)));
        assert!(
            took(&mut rx_scoped).is_none(),
            "another user's frame stays out even inside the scope"
        );
        assert!(took(&mut rx_a).is_some());

        // Addressed to the scoped subscriber AND inside its scope: both
        // predicates hold, so it arrives.
        hub.publish(frame(org, "ticket", "t1", Some(scoped)));
        let delivered = took(&mut rx_scoped).expect("in-scope frame arrives");
        assert_eq!(delivered.entity_id.as_deref(), Some("t1"));
    }

    #[test]
    fn recipient_from_reads_the_user_id_of_the_per_user_families_only() {
        let user = Uuid::new_v4();
        let payload = json!({ "user_id": user, "kind": "mention" });

        assert_eq!(
            recipient_from("notification.created", &payload),
            Some(user),
            "notification.created names its recipient"
        );
        assert_eq!(
            recipient_from("notification.read", &payload),
            Some(user),
            "a read action belongs to the reader too"
        );
        assert_eq!(
            recipient_from("timer.started", &payload),
            Some(user),
            "every timer.* family names its user"
        );
        assert_eq!(recipient_from("timer.resumed", &payload), Some(user));
        assert_eq!(recipient_from("timer.switched", &payload), Some(user));
        assert_eq!(
            recipient_from("ticket.updated", &payload),
            None,
            "another family carrying user_id stays org news"
        );
        assert_eq!(recipient_from("appointment.updated", &payload), None);
        assert_eq!(
            recipient_from("notification_prefs.updated", &payload),
            Some(user),
            "preference changes carry one user's quiet hours and flags"
        );
        assert_eq!(
            recipient_from("notification.created", &json!({ "kind": "mention" })),
            None,
            "a per-user family without a user_id has no recipient"
        );
        assert_eq!(
            recipient_from("notification.created", &json!({ "user_id": "not-a-uuid" })),
            None
        );
    }

    #[test]
    fn an_org_scoped_frame_serializes_without_the_recipient_key() {
        let org = Uuid::new_v4();
        let wire = serde_json::to_value(frame(org, "ticket", "t1", None)).expect("serialize");
        assert!(
            wire.get("recipient_user_id").is_none(),
            "the unaddressed wire shape is unchanged: {wire}"
        );

        let wire =
            serde_json::to_value(frame(org, "notification", "n1", Some(org))).expect("serialize");
        assert_eq!(
            wire.get("recipient_user_id").and_then(|v| v.as_str()),
            Some(org.to_string().as_str())
        );
    }
}
