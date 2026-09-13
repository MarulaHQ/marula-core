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
//! `meridian_events`. ONE dedicated listener connection per core-api
//! process ([`run_listener`], sqlx `PgListener` — never a per-request
//! connection) receives notifications at commit time and fans frames out
//! through the process-global [`hub`]. Delivery is at-least-once from the
//! client's perspective: a reconnecting client rehydrates from its
//! snapshot read model and dedupes by the per-entity monotonic `seq` the
//! frames carry (the ledger makes replay/duplicate frames idempotent).
//!
//! Frames are org-filtered at fan-out (a subscriber NEVER sees events from
//! another org — the hub drops or routes by the frame's `org_id`, never at
//! subscribe time). Per-connection buffers are bounded; a slow consumer
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
//! [`PRESENCE_CHANNEL`] (`meridian_presence`, no new infra dependency —
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
pub const NOTIFY_CHANNEL: &str = "meridian_events";

/// The dedicated Postgres NOTIFY channel for cross-replica presence
/// relay (Wave 13 lane 10). Payload is one heartbeat's identity; see
/// [`HeartbeatNotify`]. No migration needed — NOTIFY channels are
/// runtime-only.
pub const PRESENCE_CHANNEL: &str = "meridian_presence";

/// Maximum number of outbox events replayed on `Last-Event-ID` resume.
/// Past this bound the client is too stale for a replay to be useful —
/// it rehydrates from snapshots instead (documented cutoff).
pub const REPLAY_BUFFER: usize = 256;

/// Connection cap per user — bounds fan-out memory per identity.
pub const MAX_CONNECTIONS_PER_USER: usize = 8;

/// Per-subscriber frame buffer. A full buffer means the consumer stopped
/// reading: drop it (client resyncs from snapshots — the ledger exists for
/// exactly this) rather than blocking the listener or growing memory.
const CHANNEL_BUFFER: usize = 64;

fn connection_cap() -> usize {
    static CAP: OnceLock<usize> = OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("MERIDIAN_SSE_MAX_CONNECTIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|v| *v > 0)
            .unwrap_or(MAX_CONNECTIONS_PER_USER)
    })
}

fn replay_buffer() -> usize {
    static BUF: OnceLock<usize> = OnceLock::new();
    *BUF.get_or_init(|| {
        std::env::var("MERIDIAN_SSE_REPLAY_BUFFER")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|v| *v > 0)
            .unwrap_or(REPLAY_BUFFER)
    })
}

fn presence_ttl() -> Duration {
    static TTL: OnceLock<Duration> = OnceLock::new();
    *TTL.get_or_init(|| {
        std::env::var("MERIDIAN_PRESENCE_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|v| *v > 0)
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(90))
    })
}

/// One stream frame — the SSE `data:` JSON. Contract-event frames carry
/// exactly `{type, org_id, entity, entity_id, seq, at}` (the wire contract
/// of the notify payload, `type` = the contract event type); presence
/// frames carry `type: "presence"` plus the current `viewers` roster and
/// are EPHEMERAL — never contract events, never in the trail.
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

/// The parsed notify payload shape (migration 0068).
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
}

#[derive(Debug)]
struct Subscriber {
    user_id: Uuid,
    /// `Some((entity, entity_id))` — only frames for that entity instance.
    /// `None` — org-wide (all frames in the org).
    scope: Option<(String, String)>,
    tx: mpsc::Sender<Arc<StreamFrame>>,
}

/// The process-global fan-out hub. `Mutex` (not async) keeps publish a
/// non-blocking, allocation-light operation off the listener hot path.
pub struct Hub {
    inner: Mutex<HubInner>,
}

#[derive(Default)]
struct HubInner {
    orgs: HashMap<Uuid, Vec<Subscriber>>,
    per_user: HashMap<(Uuid, Uuid), usize>,
}

fn scope_matches(sub: &Subscriber, frame: &StreamFrame) -> bool {
    match &sub.scope {
        None => true,
        Some((entity, entity_id)) => {
            frame.entity == *entity && frame.entity_id.as_deref().is_some_and(|id| id == entity_id)
        }
    }
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
    /// frame only ever reaches subscribers of its own org. Slow/closed
    /// consumers are dropped (never block the listener).
    pub fn publish(&self, frame: StreamFrame) {
        let arc = Arc::new(frame);
        let org = arc.org_id;
        let mut dropped: Vec<Uuid> = Vec::new();
        let mut inner = self.inner.lock().unwrap();
        if let Some(subs) = inner.orgs.get_mut(&org) {
            subs.retain(|s| {
                if !scope_matches(s, &arc) {
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
/// connection per process receives every `meridian_events` notification at
/// commit time and fans it out through the hub, AND every
/// `meridian_presence` relayed heartbeat (cross-replica presence,
/// Wave 13 lane 10). Reconnects with backoff; missed notifications during
/// a disconnect are covered by the client's snapshot hydration on
/// reconnect (at-least-once over the ledger) and by the next heartbeat.
pub async fn run_listener(database_url: String) {
    let mut backoff = Duration::from_millis(250);
    loop {
        match listener_session(&database_url).await {
            Ok(()) => {
                // Unreachable: listener_session only returns on error.
                backoff = Duration::from_millis(250);
            }
            Err(e) => {
                tracing::warn!(error = %e, "event listener disconnected; reconnecting");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(10));
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
                Ok(n) => hub().publish(StreamFrame {
                    id: Some(n.id),
                    event_type: n.event_type,
                    org_id: n.org_id,
                    entity: n.entity,
                    entity_id: n.entity_id,
                    seq: n.seq,
                    at: n.at,
                    viewers: None,
                }),
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
/// read back from the authoritative trail (the outbox IS the buffer).
#[derive(sqlx::FromRow)]
struct ReplayRow {
    id: Uuid,
    event_type: String,
    entity: String,
    entity_id: Option<String>,
    occurred_at: DateTime<Utc>,
    seq: Option<i64>,
}

/// Replays the events committed AFTER `last_event_id` for `org_id`
/// (optionally narrowed to one entity instance), from the outbox — the
/// authoritative trail, so replay is exact and the ledger's seq dedupe
/// makes it idempotent against live frames. Bounds:
///
/// * Unknown cursor (never seen, or a presence-frame uuid): no replay.
/// * More than [`replay_buffer()`] events after the cursor: no replay —
///   the client fell past the buffer and rehydrates from snapshots.
///
/// Returns the frames in commit order with their ids attached, ready to
/// be streamed as SSE events with the `id:` field set, plus whether the
/// cursor fell PAST the replay buffer (so the endpoint can tell the
/// client apart from a plain "nothing missed").
pub async fn replay_after(
    pool: &sqlx::PgPool,
    org_id: Uuid,
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
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let Some(cursor_at) = cursor_at else {
        return (Vec::new(), false);
    };

    let limit = replay_buffer() as i64 + 1;
    // seq is joined exactly the way migration 0068's trigger does, minus
    // the text->uuid cast hazard: entity_write_seqs.entity_id is uuid, so
    // compare uuid::text — a non-uuid outbox entity_id simply finds no
    // counter and degrades to seq: null (same as the trigger).
    let rows = match &scope {
        Some((entity, entity_id)) => {
            sqlx::query_as::<_, ReplayRow>(
                r#"SELECT o.id, o.event_type, o.entity, o.entity_id, o.occurred_at,
                      (SELECT w.seq FROM entity_write_seqs w
                       WHERE w.org_id = o.org_id AND w.entity = o.entity
                         AND w.entity_id::text = o.entity_id) AS seq
               FROM outbox_events o
               WHERE o.org_id = $1 AND o.occurred_at > $2
                 AND o.entity = $3 AND o.entity_id = $4
               ORDER BY o.occurred_at, o.id
               LIMIT $5"#,
            )
            .bind(org_id)
            .bind(cursor_at)
            .bind(entity)
            .bind(entity_id)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        None => {
            sqlx::query_as::<_, ReplayRow>(
                r#"SELECT o.id, o.event_type, o.entity, o.entity_id, o.occurred_at,
                      (SELECT w.seq FROM entity_write_seqs w
                       WHERE w.org_id = o.org_id AND w.entity = o.entity
                         AND w.entity_id::text = o.entity_id) AS seq
               FROM outbox_events o
               WHERE o.org_id = $1 AND o.occurred_at > $2
               ORDER BY o.occurred_at, o.id
               LIMIT $3"#,
            )
            .bind(org_id)
            .bind(cursor_at)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
    };
    let Ok(rows) = rows else {
        tracing::warn!("resume replay query failed; client falls back to rehydration");
        return (Vec::new(), false);
    };
    if rows.len() as i64 > replay_buffer() as i64 {
        // Cutoff: too much history between cursor and now. Serve nothing
        // (a partial replay that skips events would be worse than none) —
        // the client's full snapshot rehydration is the recovery path.
        tracing::info!(org_id = %org_id, "resume cursor past replay buffer; skipping replay");
        return (Vec::new(), true);
    }
    (
        rows.into_iter()
            .map(|r| StreamFrame {
                id: Some(r.id),
                event_type: r.event_type,
                org_id,
                entity: r.entity,
                entity_id: r.entity_id,
                seq: r.seq,
                at: r.occurred_at,
                viewers: None,
            })
            .collect(),
        false,
    )
}
