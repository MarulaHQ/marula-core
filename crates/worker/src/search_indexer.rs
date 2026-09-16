//! worker/search_indexer — event-driven auto-indexing for unified search
//! (TASKLIST 2.16).
//!
//! Purpose: consume the domain event stream and keep the search index in
//! step with tickets, comments, clients, sites and contacts. On every
//! relevant event the CURRENT row is re-fetched from Postgres (reads are
//! allowed here; §4.1 forbids only writes) and pushed to core-api's
//! internal effect surface — this process never writes the index (or any
//! other domain table) directly.
//!
//! Delivery semantics: the stream cursor advances only after a successful
//! flush, so failures replay on the next poll; upserts are keyed by
//! (org, kind, entity) and idempotent, making replays exactly-once at the
//! index level. Best-effort v1 like the notifier leg: a persistently down
//! core-api stalls THIS consumer only, never the relay.
//!
//! Non-purpose: no embeddings/vector ranking (Stage 3 grows that behind
//! the same sink seam), no backfills beyond live events. KB article rule
//! (Wave 15 lane 4): the staff-only unified index carries DRAFT and
//! PUBLISHED articles — a draft surfaces in staff search so the web KB
//! workbench's FTS preview ("Preview search" against /api/search) is
//! meaningful before publishing; ARCHIVED entries drop. Draft visibility
//! stays staff-only by construction: /api/search accepts human staff
//! principals only, and the portal KB search re-resolves from kb_articles
//! with its own published + client-visible gate (never this index).

use crate::config::WorkerConfig;
use core_db::search::SearchKind;
use core_domain::entities::KbArticleStatus;
use core_events::{Envelope, EVENT_STREAM};
use redis::streams::{StreamReadOptions, StreamReadReply};
use redis::AsyncCommands;
use serde::Serialize;
use sqlx::PgPool;
use std::collections::HashMap;
use tokio::sync::watch;
use uuid::Uuid;

/// Blocking-XREAD wait per poll; bounds shutdown latency.
const BLOCK_MS: usize = 2_000;
const COUNT: usize = 200;
const MAX_BACKOFF_MS: u64 = 5_000;

/// One entity whose indexed content may need refreshing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    Ticket(Uuid),
    Comment(Uuid),
    Client(Uuid),
    Site(Uuid),
    Contact(Uuid),
    /// Native KB article (TASKLIST Phase 3 native KB v1).
    Article(Uuid),
    /// Document whose CURRENT version is the searchable content
    /// (TASKLIST D4).
    Document(Uuid),
}

impl Target {
    pub fn kind(&self) -> SearchKind {
        match self {
            Target::Ticket(_) => SearchKind::Ticket,
            Target::Comment(_) => SearchKind::Comment,
            Target::Client(_) => SearchKind::Client,
            Target::Site(_) => SearchKind::Site,
            Target::Contact(_) => SearchKind::Contact,
            Target::Article(_) => SearchKind::KbArticle,
            Target::Document(_) => SearchKind::Document,
        }
    }

    pub fn entity_id(&self) -> Uuid {
        match self {
            Target::Ticket(id)
            | Target::Comment(id)
            | Target::Client(id)
            | Target::Site(id)
            | Target::Contact(id)
            | Target::Article(id)
            | Target::Document(id) => *id,
        }
    }
}

/// Maps an envelope onto an indexing target, or None when the event does
/// not affect searchable content. Pure — unit-tested without I/O.
/// A `kb_article.deleted` maps to the article target so the resolver's
/// NotFound leg drops the index entry.
pub fn match_event(envelope: &Envelope) -> Option<Target> {
    let id = envelope.entity_id.as_deref()?;
    let uuid = Uuid::parse_str(id).ok()?;
    Some(match envelope.event_type.as_str() {
        "ticket.created" | "ticket.updated" => Target::Ticket(uuid),
        "ticket.comment_added" => Target::Comment(uuid),
        "client.created" | "client.updated" => Target::Client(uuid),
        "site.created" => Target::Site(uuid),
        "contact.created" => Target::Contact(uuid),
        "kb_article.created" | "kb_article.updated" | "kb_article.deleted" => Target::Article(uuid),
        "document.created" | "document.updated" | "document.restored" => Target::Document(uuid),
        _ => return None,
    })
}

/// A single write the sink must apply. Serialized verbatim into the
/// internal effect surface's request bodies.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum IndexOp {
    Upsert {
        entity_kind: String,
        entity_id: Uuid,
        parent_id: Option<Uuid>,
        title: String,
        body: String,
    },
    Delete {
        entity_kind: String,
        entity_id: Uuid,
    },
}

fn upsert_op(
    kind: SearchKind,
    entity_id: Uuid,
    parent: Option<Uuid>,
    title: String,
    body: String,
) -> IndexOp {
    IndexOp::Upsert {
        entity_kind: kind.as_str().to_string(),
        entity_id,
        parent_id: parent,
        title,
        body,
    }
}

fn delete_op(kind: SearchKind, entity_id: Uuid) -> IndexOp {
    IndexOp::Delete {
        entity_kind: kind.as_str().to_string(),
        entity_id,
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

/// Re-fetches the current content for one target:
/// - `Ok(Some(op))` — an upsert with fresh content;
/// - `Ok(None)` — the source row is gone; the index entry must be deleted;
/// - `Err(_)` — transient failure; callers replay the whole batch rather
///   than risk deleting a good index row off a failed read.
pub async fn resolve_target(
    pool: &PgPool,
    org_id: Uuid,
    target: Target,
) -> core_db::DbResult<Option<IndexOp>> {
    // NotFound is data, not an error: it signals the delete leg. Every
    // other failure propagates so the batch replays.
    async fn or_gone<T>(result: core_db::DbResult<T>) -> core_db::DbResult<Option<T>> {
        match result {
            Ok(value) => Ok(Some(value)),
            Err(core_db::DbError::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    let op = match target {
        Target::Ticket(id) => {
            match or_gone(core_db::tickets::get_ticket(pool, org_id, id).await).await? {
                None => None,
                Some(t) => Some(upsert_op(
                    SearchKind::Ticket,
                    id,
                    None,
                    t.title,
                    t.body.unwrap_or_default(),
                )),
            }
        }
        Target::Comment(id) => {
            match or_gone(core_db::tickets::get_comment(pool, org_id, id).await).await? {
                None => None,
                Some(c) => Some(upsert_op(
                    SearchKind::Comment,
                    id,
                    Some(c.ticket_id),
                    truncate(&c.body, 80),
                    c.body,
                )),
            }
        }
        Target::Client(id) => {
            match or_gone(core_db::clients::get_client(pool, org_id, id).await).await? {
                None => None,
                Some(c) => Some(upsert_op(
                    SearchKind::Client,
                    id,
                    None,
                    c.name,
                    c.notes.unwrap_or_default(),
                )),
            }
        }
        Target::Site(id) => {
            match or_gone(core_db::clients::get_site(pool, org_id, id).await).await? {
                None => None,
                Some(s) => {
                    let body = [
                        s.address.as_deref(),
                        s.city.as_deref(),
                        s.region.as_deref(),
                        s.country.as_deref(),
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(", ");
                    Some(upsert_op(
                        SearchKind::Site,
                        id,
                        Some(s.client_id),
                        s.name,
                        body,
                    ))
                }
            }
        }
        Target::Contact(id) => {
            match or_gone(core_db::clients::get_contact(pool, org_id, id).await).await? {
                None => None,
                Some(c) => {
                    let body = [c.email.as_deref(), c.phone.as_deref(), c.title.as_deref()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" ");
                    Some(upsert_op(
                        SearchKind::Contact,
                        id,
                        Some(c.client_id),
                        c.name,
                        body,
                    ))
                }
            }
        }
        Target::Article(id) => {
            match or_gone(core_db::kb::get_article(pool, org_id, id).await).await? {
                None => None,
                // The staff-only /kb FTS preview (the web KB workbench's
                // "Preview search") uses this same unified index, so the
                // index carries DRAFT + PUBLISHED articles — a draft must
                // surface in staff search to make the preview meaningful
                // before publishing (Wave 15 lane 4 `p15-docs-closeout`).
                // Staff-only is guaranteed structurally: /api/search only
                // accepts human staff principals, and the portal KB search
                // re-resolves from kb_articles with the published +
                // client-visible gate — it never touches this index.
                // Archived = retired: the entry drops.
                Some(a) if a.status != KbArticleStatus::Archived => Some(upsert_op(
                    SearchKind::KbArticle,
                    id,
                    a.category_id,
                    a.title,
                    a.body,
                )),
                Some(_) => None,
            }
        }
        Target::Document(id) => {
            match or_gone(core_db::documents::get_document(pool, org_id, id).await).await? {
                None => None,
                Some(d) => {
                    // The document's searchable content is its CURRENT
                    // version; the head row always exists (created with
                    // version 1 and only ever appended to).
                    let head = core_db::documents::get_version(pool, org_id, id, d.current_version)
                        .await?;
                    Some(upsert_op(
                        SearchKind::Document,
                        id,
                        None,
                        head.title,
                        head.body,
                    ))
                }
            }
        }
    };
    Ok(op)
}

/// The write seam: core-api's internal effect surface in production; test
/// doubles record instead. Mirrors the repo's RPITIT trait style.
pub trait IndexSink: Send + Sync {
    /// Applies a batch of ops for one org (the HTTP surface wraps each
    /// route call in one RLS transaction).
    fn apply(
        &self,
        org_id: Uuid,
        ops: Vec<IndexOp>,
    ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send;
}

/// Production sink: POSTs batched ops to `/internal/search/{upsert,delete}`.
pub struct HttpIndexSink {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl HttpIndexSink {
    pub fn new(base_url: &str, token: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }
}

impl IndexSink for HttpIndexSink {
    async fn apply(&self, org_id: Uuid, ops: Vec<IndexOp>) -> anyhow::Result<()> {
        let (upserts, deletes): (Vec<&IndexOp>, Vec<&IndexOp>) = ops
            .iter()
            .partition(|op| matches!(op, IndexOp::Upsert { .. }));
        if !upserts.is_empty() {
            self.http
                .post(format!("{}/internal/search/upsert", self.base_url))
                .header("x-internal-token", &self.token)
                .json(&serde_json::json!({ "org_id": org_id, "entries": upserts }))
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await?
                .error_for_status()?;
        }
        if !deletes.is_empty() {
            self.http
                .post(format!("{}/internal/search/delete", self.base_url))
                .header("x-internal-token", &self.token)
                .json(&serde_json::json!({ "org_id": org_id, "entries": deletes }))
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await?
                .error_for_status()?;
        }
        Ok(())
    }
}

/// Runs the indexer until `shutdown` fires. Redis errors reconnect with
/// backoff; malformed entries are logged and skipped.
pub async fn run_search_indexer<S: IndexSink>(
    pool: PgPool,
    config: WorkerConfig,
    sink: S,
    mut shutdown: watch::Receiver<bool>,
) {
    let client = match redis::Client::open(config.redis_url.as_str()) {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(%error, "invalid redis url; search indexer exiting");
            return;
        }
    };
    let mut backoff_ms = 100_u64;
    loop {
        if *shutdown.borrow() {
            return;
        }
        match client.get_multiplexed_async_connection().await {
            Ok(mut conn) => {
                backoff_ms = 100;
                let broke = follow_stream(&pool, &mut conn, &sink, &mut shutdown).await;
                if broke.is_none() {
                    return;
                }
                tracing::warn!("search indexer stream connection ended; reconnecting");
            }
            Err(error) => {
                tracing::error!(%error, "search indexer redis connect failed");
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
        backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
    }
}

/// Streams events; returns None on shutdown, Some when the connection
/// dropped or a flush failed (the caller reconnects WITHOUT advancing the
/// cursor, so the failed batch replays against idempotent upserts).
async fn follow_stream<S: IndexSink>(
    pool: &PgPool,
    conn: &mut redis::aio::MultiplexedConnection,
    sink: &S,
    shutdown: &mut watch::Receiver<bool>,
) -> Option<()> {
    // `$` starts at the live tail: restarts skip history by design; a full
    // re-index job is Stage-3 scope alongside the vector leg.
    let mut cursor = "$".to_string();
    let options = StreamReadOptions::default().block(BLOCK_MS).count(COUNT);
    let keys = [EVENT_STREAM];
    loop {
        if *shutdown.borrow() {
            return None;
        }
        let ids = [cursor.as_str()];
        tokio::select! {
            _ = shutdown.changed() => return None,
            result = conn.xread_options::<_, _, StreamReadReply>(&keys, &ids, &options) => {
                match result {
                    Ok(reply) => {
                        let mut targets: Vec<(Uuid, Target)> = Vec::new();
                        for key in reply.keys {
                            for entry in key.ids {
                                cursor = entry.id.clone();
                                collect_entry(&mut targets, &entry);
                            }
                        }
                        if !targets.is_empty() {
                            if let Err(error) = flush(pool, sink, targets).await {
                                tracing::error!(%error, "search index flush failed; replaying batch");
                                return Some(());
                            }
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "search indexer xread failed");
                        return Some(());
                    }
                }
            }
        }
    }
}

/// Parses one stream entry into zero or more deduplicated targets.
fn collect_entry(targets: &mut Vec<(Uuid, Target)>, entry: &redis::streams::StreamId) {
    let Some(raw) = entry.get::<String>("data") else {
        tracing::warn!(entry_id = %entry.id, "stream entry missing data field");
        return;
    };
    let Ok(envelope) = serde_json::from_str::<Envelope>(&raw) else {
        tracing::warn!(entry_id = %entry.id, "unparseable event");
        return;
    };
    if let Some(target) = match_event(&envelope) {
        let pair = (envelope.org_id, target);
        if !targets.contains(&pair) {
            targets.push(pair);
        }
    }
}

/// Resolves every target against current rows and pushes per-org batches
/// through the sink. Duplicate targets collapse; `Ok(None)` resolves to a
/// delete so future source deletions keep the index honest.
/// Resolves targets and pushes per-org batches — exposed for the flow tests.
/// Duplicate (org, target) pairs collapse into one op: replays are free.
pub async fn flush<S: IndexSink>(
    pool: &PgPool,
    sink: &S,
    targets: Vec<(Uuid, Target)>,
) -> anyhow::Result<()> {
    let mut seen: std::collections::HashSet<(Uuid, Target)> = std::collections::HashSet::new();
    let mut per_org: HashMap<Uuid, Vec<IndexOp>> = HashMap::new();
    for (org_id, target) in targets {
        if !seen.insert((org_id, target.clone())) {
            continue;
        }
        let op = match resolve_target(pool, org_id, target.clone()).await {
            Ok(Some(op)) => Some(op),
            Ok(None) => Some(delete_op(target.kind(), target.entity_id())),
            Err(error) => return Err(error.into()),
        };
        if let Some(op) = op {
            per_org.entry(org_id).or_default().push(op);
        }
    }
    for (org_id, ops) in per_org {
        sink.apply(org_id, ops).await?;
    }
    Ok(())
}
