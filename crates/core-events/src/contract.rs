//! The event-evolution contract (policy §2, TASKLIST U2/U4).
//!
//! Purpose: the machine-checked index of every published domain payload —
//! event type → wire shape — plus the retired-type co-emit rail. The
//! conformance suite (`core-events/tests/event_conformance.rs`) enforces
//! three promises against this table and the golden fixtures in
//! `fixtures/events/v1/`:
//!
//! 1. **Unknown-field tolerance** — consumers must be able to deserialize
//!    any golden payload (and the envelope) with fields we have not thought
//!    of yet. This forbids `deny_unknown_fields` on contract types forever.
//! 2. **Additive-only payloads** — a golden set is committed per event type
//!    (+ variant) + version. Dropping or renaming a field makes the golden
//!    fail to deserialize (required fields) or shrinks the struct's key set
//!    below the golden's (Option fields) — either way the suite fails.
//!    Additions stay legal; regenerating goldens is a reviewed,
//!    release-notes-carrying act (policy §3a).
//! 3. **Retired types co-emit for one generation** — a retirement is only
//!    legal with a registered replacement, a sane generation window
//!    (§3: a subsequent major; pre-GA 0.x may step a minor), and goldens
//!    retained until the deprecation clock (U4) allows removal.
//!
//! Non-purpose: nothing here touches Redis/Postgres; this is contract data
//! and pure helpers only.

use crate::Envelope;
use core_domain::deprecation::{generation_ok, Version};
use serde::{Deserialize, Serialize};

/// The payload contract pinned by every golden fixture and envelope today.
pub const PAYLOAD_SCHEMA_V1: &str = "core-domain/v1";

/// Golden fixtures live here, relative to the repo root — one file per
/// (event type, entity, variant). See `fixtures/events/README.md`.
pub const FIXTURES_DIR: &str = "fixtures/events/v1";

/// A committed golden fixture document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenFixture {
    pub event_type: String,
    pub entity: String,
    pub variant: String,
    pub payload_schema_ref: String,
    /// Workspace version that last (re)generated this golden. A commit that
    /// regenerates must carry a "Breaking & deprecated" release-notes entry
    /// when the diff removes or renames keys (policy §3a).
    pub pinned_in: String,
    pub payload: serde_json::Value,
}

/// A payload that is a serde serialization of one core-domain struct.
/// `sample` materializes a deterministic representative instance (for
/// golden generation and key-set comparison); `accept` proves a wire
/// document still deserializes into the struct.
#[derive(Clone, Copy)]
pub struct TypedShape {
    pub sample: fn() -> serde_json::Value,
    pub accept: fn(&serde_json::Value) -> Result<(), String>,
}

/// A payload assembled ad hoc at the emission site (`serde_json::json!`).
/// The contract floor is `required_keys`: every one of them must survive
/// any future edit of the payload. The sample pins today's full shape.
#[derive(Clone, Copy)]
pub struct UntypedShape {
    pub required_keys: &'static [&'static str],
    pub sample: fn() -> serde_json::Value,
}

#[derive(Clone)]
pub enum Shape {
    Typed(TypedShape),
    Untyped(UntypedShape),
}

impl Shape {
    /// Today's representative wire document (deterministic).
    pub fn sample(&self) -> serde_json::Value {
        match self {
            Shape::Typed(t) => (t.sample)(),
            Shape::Untyped(u) => (u.sample)(),
        }
    }

    /// Does a wire document still deserialize into the published shape?
    pub fn accept(&self, wire: &serde_json::Value) -> Result<(), String> {
        match self {
            Shape::Typed(t) => (t.accept)(wire),
            // Untyped payloads have no consumer-side struct to decode into;
            // validity is key presence, enforced by the suite against the
            // golden + required keys.
            Shape::Untyped(u) => {
                let obj = wire
                    .as_object()
                    .ok_or_else(|| "payload must be a JSON object".to_string())?;
                let missing: Vec<&str> = u
                    .required_keys
                    .iter()
                    .filter(|k| !obj.contains_key(**k))
                    .copied()
                    .collect();
                if missing.is_empty() {
                    Ok(())
                } else {
                    Err(format!("missing required keys: {missing:?}"))
                }
            }
        }
    }

    /// Top-level payload keys of the representative sample.
    pub fn sample_keys(&self) -> Vec<String> {
        match self.sample() {
            serde_json::Value::Object(map) => map.keys().cloned().collect(),
            _ => vec![],
        }
    }
}

/// One published (event type, entity, variant) payload. Entities and
/// variants exist because a few event types carry different (each
/// individually stable) payloads depending on the entity acted on or the
/// lifecycle step — e.g. `sla.calendar_updated` on create vs. delete.
pub struct PublishedEvent {
    pub event_type: &'static str,
    pub entity: &'static str,
    /// `"default"` or a short discriminator; distinguishes multiple
    /// payload shapes under one event type.
    pub variant: &'static str,
    pub shape: Shape,
}

impl PublishedEvent {
    /// Fixture filename for this entry under [`FIXTURES_DIR`].
    pub fn fixture_file(&self) -> String {
        let base = self.event_type.replace('.', "_");
        if self.variant == "default" {
            format!("{base}.json")
        } else {
            format!("{base}__{}.json", self.variant)
        }
    }
}

// ---------------------------------------------------------------------------
// Sample builders — deterministic UUIDs and timestamps so goldens are
// byte-stable across runs and machines.
// ---------------------------------------------------------------------------

/// Deterministic UUID #n (never random in samples).
fn su(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(n)
}

/// Deterministic timestamp #n (2023-11-14T22:13:20Z + n seconds).
fn st(n: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(1_700_000_000 + n, 0).expect("fixed sample timestamp")
}

macro_rules! typed_shape {
    ($ty:ty, $sample:expr) => {{
        fn sample() -> serde_json::Value {
            serde_json::to_value($sample).expect("event payload sample serializes")
        }
        fn accept(v: &serde_json::Value) -> Result<(), String> {
            serde_json::from_value::<$ty>(v.clone())
                .map(|_| ())
                .map_err(|e| format!("{}: {e}", core::any::type_name::<$ty>()))
        }
        $crate::contract::Shape::Typed($crate::contract::TypedShape { sample, accept })
    }};
}

macro_rules! untyped_shape {
    ([$($k:expr),* $(,)?], $sample:expr) => {{
        fn sample() -> serde_json::Value {
            $sample
        }
        $crate::contract::Shape::Untyped($crate::contract::UntypedShape {
            required_keys: &[$($k),*],
            sample,
        })
    }};
}

use core_domain::entities::{
    Appointment, AppointmentKind, AppointmentStatus, BoardGroup, BoardStatus, ChecklistItem,
    NotificationKind, NotificationPreference, TicketComment, TimeEntry,
};
use core_domain::events::{
    types, AppointmentCancelled, AppointmentCreated, AppointmentUpdated, AttachmentAdded,
    AttachmentDeleted, AttachmentLinked, AvailabilityChanged, BoardFavoriteToggled,
    BoardGroupCreated, BoardGroupRemoved, BoardGroupUpdated, BoardMembershipRemoved,
    BoardMembershipUpdated, BoardStatusAdded, BoardStatusRemoved, BoardStatusUpdated,
    BoardTransitionsUpdated, CalendarExclusionChanged, ClientCreated, ClientUpdated,
    ContactCreated, ContactUpdated, ImportBatchFinished, ImportBatchStarted, NotificationCreated,
    OnCallChanged, SiteCreated, SiteUpdated, TicketAssigned, TicketChecklistUpdated,
    TicketCommentAdded, TicketCreated, TicketStatusChanged, TimeEntryCreated, TimerPaused,
    TimerStarted, TimerStopped,
};
use core_domain::policy::BoardRole;
use core_domain::status::{NoteKind, Priority, TicketStatusCategory};
use core_domain::workflow::TransitionEdge;

/// Every payload Meridian publishes on the event stream today (envelope
/// `payload_schema_ref` = [`PAYLOAD_SCHEMA_V1`]). Registering an emission
/// here is the review gate for putting a new payload on the wire.
pub static PUBLISHED_EVENTS: &[PublishedEvent] = &[
    // -- tickets ----------------------------------------------------------
    PublishedEvent {
        event_type: types::TICKET_CREATED,
        entity: "ticket",
        variant: "default",
        shape: typed_shape!(
            TicketCreated,
            TicketCreated {
                ticket_id: su(1),
                number: "MER-1001".into(),
                board_id: su(2),
                title: "VPN down at main office".into(),
                priority: Priority::P3,
                client_id: Some(su(3)),
                assignee_id: Some(su(4)),
                created_at: st(1),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_STATUS_CHANGED,
        entity: "ticket",
        variant: "default",
        shape: typed_shape!(
            TicketStatusChanged,
            TicketStatusChanged {
                ticket_id: su(1),
                from_status: "triage".into(),
                to_status: "waiting-on-client".into(),
                from_category: TicketStatusCategory::Open,
                to_category: TicketStatusCategory::Waiting,
                changed_by: Some(su(5)),
                occurred_at: st(2),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_ASSIGNED,
        entity: "ticket",
        variant: "default",
        shape: typed_shape!(
            TicketAssigned,
            TicketAssigned {
                ticket_id: su(1),
                assignee_id: Some(su(4)),
                assigned_by: Some(su(5)),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_ASSIGNED,
        entity: "ticket",
        variant: "via_automation",
        shape: untyped_shape!(
            ["ticket_id", "assignee_id", "assigned_by", "via_automation"],
            serde_json::json!({
                "ticket_id": su(1).to_string(),
                "assignee_id": su(4).to_string(),
                "assigned_by": serde_json::Value::Null,
                "via_automation": true,
            })
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_COMMENT_ADDED,
        entity: "ticket_comment",
        variant: "default",
        shape: typed_shape!(
            TicketCommentAdded,
            TicketCommentAdded {
                comment: TicketComment {
                    id: su(6),
                    org_id: su(0),
                    ticket_id: su(1),
                    author_id: Some(su(5)),
                    kind: NoteKind::Internal,
                    body: "Routed through on-call.".into(),
                    created_at: st(3),
                    updated_at: None,
                },
                mentioned_user_ids: vec![su(7)],
            }
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_COMMENT_ADDED,
        entity: "ticket_comment",
        variant: "via_automation",
        shape: untyped_shape!(
            ["comment", "mentioned_user_ids", "via_automation"],
            serde_json::json!({
                "comment": {
                    "id": su(6).to_string(),
                    "org_id": su(0).to_string(),
                    "ticket_id": su(1).to_string(),
                    "author_id": serde_json::Value::Null,
                    "kind": "internal",
                    "body": "Automation applied this note.",
                    "created_at": st(3).to_rfc3339(),
                    "updated_at": serde_json::Value::Null,
                },
                "mentioned_user_ids": [],
                "via_automation": true,
            })
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_CHECKLIST_UPDATED,
        entity: "ticket",
        variant: "default",
        shape: typed_shape!(
            TicketChecklistUpdated,
            TicketChecklistUpdated {
                ticket_id: su(1),
                items: vec![ChecklistItem {
                    id: su(8),
                    org_id: su(0),
                    ticket_id: su(1),
                    text: "Confirm backup restore".into(),
                    checked: false,
                    sort_order: 0,
                    checked_by: None,
                    checked_at: None,
                    template_item_id: Some(su(9)),
                }],
                updated_by: Some(su(5)),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_UPDATED,
        entity: "ticket",
        variant: "default",
        shape: untyped_shape!(
            ["ticket_id"],
            serde_json::json!({ "ticket_id": su(1).to_string() })
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_UPDATED,
        entity: "ticket",
        variant: "via_automation",
        shape: untyped_shape!(
            ["ticket_id", "via_automation"],
            serde_json::json!({ "ticket_id": su(1).to_string(), "via_automation": true })
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_CLOSED,
        entity: "ticket",
        variant: "default",
        shape: untyped_shape!(
            ["ticket_id", "resolved_at"],
            serde_json::json!({
                "ticket_id": su(1).to_string(),
                "resolved_at": st(9).to_rfc3339(),
            })
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_FIELD_SET,
        entity: "ticket",
        variant: "default",
        shape: untyped_shape!(
            ["ticket_id", "field_id", "value"],
            serde_json::json!({
                "ticket_id": su(1).to_string(),
                "field_id": su(22).to_string(),
                "value": serde_json::json!(6.5),
            })
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_FIELD_CLEARED,
        entity: "ticket",
        variant: "default",
        shape: untyped_shape!(
            ["ticket_id", "field_id"],
            serde_json::json!({
                "ticket_id": su(1).to_string(),
                "field_id": su(22).to_string(),
            })
        ),
    },
    PublishedEvent {
        event_type: types::CUSTOM_FIELD_DEF_CREATED,
        entity: "custom_field_def",
        variant: "default",
        shape: untyped_shape!(
            ["field_id", "board_id", "name", "kind", "required"],
            serde_json::json!({
                "field_id": su(22).to_string(),
                "board_id": su(2).to_string(),
                "name": "On-site hours",
                "kind": "number",
                "required": false,
            })
        ),
    },
    PublishedEvent {
        event_type: types::CUSTOM_FIELD_DEF_UPDATED,
        entity: "custom_field_def",
        variant: "default",
        shape: untyped_shape!(
            ["field_id", "board_id", "name"],
            serde_json::json!({
                "field_id": su(22).to_string(),
                "board_id": su(2).to_string(),
                "name": "On-site minutes",
            })
        ),
    },
    PublishedEvent {
        event_type: types::CUSTOM_FIELD_DEF_REMOVED,
        entity: "custom_field_def",
        variant: "default",
        shape: untyped_shape!(
            ["field_id", "board_id"],
            serde_json::json!({
                "field_id": su(22).to_string(),
                "board_id": su(2).to_string(),
            })
        ),
    },
    // -- clients / sites / contacts ---------------------------------------
    PublishedEvent {
        event_type: types::CLIENT_CREATED,
        entity: "client",
        variant: "default",
        shape: typed_shape!(
            ClientCreated,
            ClientCreated {
                client_id: su(3),
                name: "Acme Manufacturing".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::CLIENT_UPDATED,
        entity: "client",
        variant: "default",
        shape: typed_shape!(
            ClientUpdated,
            ClientUpdated {
                client_id: su(3),
                name: "Acme Manufacturing Co.".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::SITE_CREATED,
        entity: "site",
        variant: "default",
        shape: typed_shape!(
            SiteCreated,
            SiteCreated {
                site_id: su(10),
                client_id: su(3),
                name: "Plant 2".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::SITE_UPDATED,
        entity: "site",
        variant: "default",
        shape: typed_shape!(
            SiteUpdated,
            SiteUpdated {
                site_id: su(10),
                client_id: su(3),
                name: "Plant 2 — Warehouse".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::CONTACT_CREATED,
        entity: "contact",
        variant: "default",
        shape: typed_shape!(
            ContactCreated,
            ContactCreated {
                contact_id: su(11),
                client_id: su(3),
                name: "Dana Reyes".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::CONTACT_UPDATED,
        entity: "contact",
        variant: "default",
        shape: typed_shape!(
            ContactUpdated,
            ContactUpdated {
                contact_id: su(11),
                client_id: su(3),
                name: "Dana Reyes-Ortiz".into(),
            }
        ),
    },
    // -- import framework --------------------------------------------------
    PublishedEvent {
        event_type: types::IMPORT_BATCH_STARTED,
        entity: "import_batch",
        variant: "default",
        shape: typed_shape!(
            ImportBatchStarted,
            ImportBatchStarted {
                batch_id: su(12),
                entity_kind: "clients".into(),
                conflict_policy: "update".into(),
                label: "acme-2026-q3.csv".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::IMPORT_BATCH_FINISHED,
        entity: "import_batch",
        variant: "default",
        shape: typed_shape!(
            ImportBatchFinished,
            ImportBatchFinished {
                batch_id: su(12),
                entity_kind: "clients".into(),
                rows_total: 120,
                rows_created: 90,
                rows_updated: 18,
                rows_skipped: 7,
                rows_failed: 5,
            }
        ),
    },
    // -- boards & workflow -------------------------------------------------
    PublishedEvent {
        event_type: types::BOARD_CREATED,
        entity: "board",
        variant: "default",
        shape: untyped_shape!(
            ["board_id", "name"],
            serde_json::json!({ "board_id": su(2).to_string(), "name": "Service Desk" })
        ),
    },
    PublishedEvent {
        event_type: types::BOARD_STATUS_ADDED,
        entity: "board_status",
        variant: "default",
        shape: typed_shape!(
            BoardStatusAdded,
            BoardStatusAdded {
                board_id: su(2),
                status: sample_status("Waiting on client"),
            }
        ),
    },
    PublishedEvent {
        event_type: types::BOARD_STATUS_UPDATED,
        entity: "board_status",
        variant: "default",
        shape: typed_shape!(
            BoardStatusUpdated,
            BoardStatusUpdated {
                board_id: su(2),
                status: sample_status("Waiting on customer"),
            }
        ),
    },
    PublishedEvent {
        event_type: types::BOARD_STATUS_REMOVED,
        entity: "board_status",
        variant: "default",
        shape: typed_shape!(
            BoardStatusRemoved,
            BoardStatusRemoved {
                board_id: su(2),
                status_id: su(13),
                name: "Waiting on client".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::BOARD_TRANSITIONS_UPDATED,
        entity: "board",
        variant: "default",
        shape: typed_shape!(
            BoardTransitionsUpdated,
            BoardTransitionsUpdated {
                board_id: su(2),
                edges: vec![TransitionEdge {
                    from_status_id: su(13),
                    to_status_id: su(14),
                }],
            }
        ),
    },
    PublishedEvent {
        event_type: types::BOARD_CHECKLIST_TEMPLATE_UPDATED,
        entity: "board",
        variant: "default",
        shape: untyped_shape!(
            [
                "board_id",
                "template_id",
                "item_count",
                "require_checklist_close"
            ],
            serde_json::json!({
                "board_id": su(2).to_string(),
                "template_id": su(21).to_string(),
                "item_count": 4,
                "require_checklist_close": true,
            })
        ),
    },
    PublishedEvent {
        event_type: types::BOARD_CHECKLIST_TEMPLATE_UPDATED,
        entity: "board",
        variant: "deleted",
        shape: untyped_shape!(
            ["board_id", "deleted"],
            serde_json::json!({ "board_id": su(2).to_string(), "deleted": true })
        ),
    },
    PublishedEvent {
        event_type: types::BOARD_GROUP_CREATED,
        entity: "board_group",
        variant: "default",
        shape: typed_shape!(
            BoardGroupCreated,
            BoardGroupCreated {
                group: BoardGroup {
                    id: su(18),
                    org_id: su(0),
                    owner_id: su(5),
                    name: "Client work".into(),
                    position: 1,
                    created_at: st(7),
                },
            }
        ),
    },
    PublishedEvent {
        event_type: types::BOARD_GROUP_UPDATED,
        entity: "board_group",
        variant: "default",
        shape: typed_shape!(
            BoardGroupUpdated,
            BoardGroupUpdated {
                group_id: su(18),
                name: "Client work — Q3".into(),
                position: 2,
            }
        ),
    },
    PublishedEvent {
        event_type: types::BOARD_GROUP_UPDATED,
        entity: "board",
        variant: "board_ref",
        shape: untyped_shape!(
            ["board_id", "group_id"],
            serde_json::json!({
                "board_id": su(2).to_string(),
                "group_id": su(18).to_string(),
            })
        ),
    },
    PublishedEvent {
        event_type: types::BOARD_GROUP_REMOVED,
        entity: "board_group",
        variant: "default",
        shape: typed_shape!(BoardGroupRemoved, BoardGroupRemoved { group_id: su(18) }),
    },
    PublishedEvent {
        event_type: types::BOARD_MEMBERSHIP_UPDATED,
        entity: "board_member",
        variant: "default",
        shape: typed_shape!(
            BoardMembershipUpdated,
            BoardMembershipUpdated {
                board_id: su(2),
                user_id: su(4),
                role: BoardRole::Member,
            }
        ),
    },
    PublishedEvent {
        event_type: types::BOARD_MEMBERSHIP_REMOVED,
        entity: "board_member",
        variant: "default",
        shape: typed_shape!(
            BoardMembershipRemoved,
            BoardMembershipRemoved {
                board_id: su(2),
                user_id: su(4),
            }
        ),
    },
    PublishedEvent {
        event_type: types::BOARD_FAVORITE_TOGGLED,
        entity: "board_favorite",
        variant: "default",
        shape: typed_shape!(
            BoardFavoriteToggled,
            BoardFavoriteToggled {
                user_id: su(4),
                board_id: su(2),
                favorite: true,
            }
        ),
    },
    // -- timers & time -----------------------------------------------------
    PublishedEvent {
        event_type: types::TIMER_STARTED,
        entity: "timer",
        variant: "default",
        shape: typed_shape!(
            TimerStarted,
            TimerStarted {
                session_id: su(15),
                user_id: su(4),
                ticket_id: su(1),
                started_at: st(4),
                switched_from: Some(su(16)),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TIMER_PAUSED,
        entity: "timer",
        variant: "default",
        shape: typed_shape!(
            TimerPaused,
            TimerPaused {
                session_id: su(15),
                user_id: su(4),
                ticket_id: su(1),
                reason: Some("lunch".into()),
                elapsed_secs: 1800,
            }
        ),
    },
    PublishedEvent {
        event_type: types::TIMER_RESUMED,
        entity: "timer",
        variant: "default",
        shape: untyped_shape!(
            ["session_id", "user_id"],
            serde_json::json!({
                "session_id": su(15).to_string(),
                "user_id": su(4).to_string(),
            })
        ),
    },
    PublishedEvent {
        event_type: types::TIMER_SWITCHED,
        entity: "timer",
        variant: "default",
        shape: untyped_shape!(
            ["session_id", "user_id"],
            serde_json::json!({
                "session_id": su(16).to_string(),
                "user_id": su(4).to_string(),
            })
        ),
    },
    PublishedEvent {
        event_type: types::TIMER_STOPPED,
        entity: "timer",
        variant: "default",
        shape: typed_shape!(
            TimerStopped,
            TimerStopped {
                session_id: su(15),
                user_id: su(4),
                ticket_id: su(1),
                total_secs: 7200,
                time_entry_id: Some(su(17)),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TIME_ENTRY_CREATED,
        entity: "time_entry",
        variant: "default",
        shape: typed_shape!(
            TimeEntryCreated,
            TimeEntryCreated {
                entry: TimeEntry {
                    id: su(17),
                    org_id: su(0),
                    ticket_id: su(1),
                    user_id: su(4),
                    started_at: st(4),
                    ended_at: st(6),
                    minutes: 120,
                    billable: true,
                    notes: Some("On-site visit".into()),
                    work_type: Some("onsite".into()),
                    timer_session_id: Some(su(15)),
                    created_at: st(6),
                },
            }
        ),
    },
    // -- notifications & email ---------------------------------------------
    PublishedEvent {
        event_type: types::NOTIFICATION_CREATED,
        entity: "notification",
        variant: "default",
        shape: typed_shape!(
            NotificationCreated,
            NotificationCreated {
                user_id: su(5),
                kind: "mention".into(),
                ticket_id: Some(su(1)),
                title: "Alice mentioned you".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::NOTIFICATION_CREATED,
        entity: "notification",
        variant: "via_automation",
        shape: untyped_shape!(
            ["user_id", "kind", "ticket_id", "title", "via_automation"],
            serde_json::json!({
                "user_id": su(5).to_string(),
                "kind": "mention",
                "ticket_id": su(1).to_string(),
                "title": "Automation updated your ticket",
                "via_automation": true,
            })
        ),
    },
    PublishedEvent {
        event_type: types::NOTIFICATION_READ,
        entity: "notification",
        variant: "default",
        shape: untyped_shape!(
            ["user_id", "ids", "count"],
            serde_json::json!({
                "user_id": su(5).to_string(),
                "ids": [su(23).to_string()],
                "count": 1,
            })
        ),
    },
    PublishedEvent {
        event_type: types::NOTIFICATION_PREFS_UPDATED,
        entity: "notification_preferences",
        variant: "default",
        shape: typed_shape!(
            NotificationPreference,
            NotificationPreference {
                user_id: su(5),
                kind: NotificationKind::Mention,
                in_app: true,
                email: false,
                quiet_from: Some("22:00".into()),
                quiet_until: Some("07:00".into()),
                timezone: "Europe/Berlin".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::EMAIL_MESSAGE_RECORDED,
        entity: "email_message",
        variant: "default",
        shape: untyped_shape!(
            [
                "message_id",
                "direction",
                "outcome",
                "ticket_id",
                "comment_id"
            ],
            serde_json::json!({
                "message_id": su(24).to_string(),
                "direction": "outbound",
                "outcome": "queued",
                "ticket_id": su(1).to_string(),
                "comment_id": serde_json::Value::Null,
            })
        ),
    },
    // -- SLA & automations --------------------------------------------------
    PublishedEvent {
        event_type: types::SLA_ESCALATION_FIRED,
        entity: "ticket",
        variant: "default",
        shape: untyped_shape!(
            ["ticket_id", "target", "trigger", "step_id", "title"],
            serde_json::json!({
                "ticket_id": su(1).to_string(),
                "target": "assignee",
                "trigger": "breach",
                "step_id": su(25).to_string(),
                "title": "SLA at risk: VPN down at main office",
            })
        ),
    },
    PublishedEvent {
        event_type: types::SLA_CALENDAR_UPDATED,
        entity: "sla_calendar",
        variant: "default",
        shape: untyped_shape!(
            ["calendar_id", "name"],
            serde_json::json!({
                "calendar_id": su(26).to_string(),
                "name": "US business hours",
            })
        ),
    },
    PublishedEvent {
        event_type: types::SLA_CALENDAR_UPDATED,
        entity: "sla_calendar",
        variant: "patched",
        shape: untyped_shape!(
            ["calendar_id"],
            serde_json::json!({ "calendar_id": su(26).to_string() })
        ),
    },
    PublishedEvent {
        event_type: types::SLA_CALENDAR_UPDATED,
        entity: "sla_calendar",
        variant: "deleted",
        shape: untyped_shape!(
            ["calendar_id", "deleted"],
            serde_json::json!({
                "calendar_id": su(26).to_string(),
                "deleted": true,
            })
        ),
    },
    PublishedEvent {
        event_type: types::SLA_POLICY_UPDATED,
        entity: "sla_policy",
        variant: "default",
        shape: untyped_shape!(["ladder_size"], serde_json::json!({ "ladder_size": 3 })),
    },
    PublishedEvent {
        event_type: types::AUTOMATION_TRIGGERED,
        entity: "automation_rule",
        variant: "default",
        shape: untyped_shape!(
            ["rule_id", "ticket_id", "actions_applied", "via_automation"],
            serde_json::json!({
                "rule_id": su(27).to_string(),
                "ticket_id": su(1).to_string(),
                "actions_applied": 2,
                "via_automation": true,
            })
        ),
    },
    PublishedEvent {
        event_type: types::AUTOMATION_RULE_UPDATED,
        entity: "automation_rule",
        variant: "default",
        shape: untyped_shape!(
            ["rule_id", "trigger_event"],
            serde_json::json!({
                "rule_id": su(27).to_string(),
                "trigger_event": "ticket.created",
            })
        ),
    },
    PublishedEvent {
        event_type: types::AUTOMATION_RULE_UPDATED,
        entity: "automation_rule",
        variant: "minimal",
        shape: untyped_shape!(
            ["rule_id"],
            serde_json::json!({ "rule_id": su(27).to_string() })
        ),
    },
    PublishedEvent {
        event_type: types::AUTOMATION_RULE_UPDATED,
        entity: "automation_rule",
        variant: "deleted",
        shape: untyped_shape!(["deleted"], serde_json::json!({ "deleted": true })),
    },
    // -- settings & org ------------------------------------------------------
    PublishedEvent {
        event_type: types::SETTING_UPDATED,
        entity: "setting",
        variant: "default",
        shape: untyped_shape!(
            ["scope", "key", "value"],
            serde_json::json!({
                "scope": "org",
                "key": "notifications.digest_hour",
                "value": 18,
            })
        ),
    },
    PublishedEvent {
        event_type: types::SETTING_UPDATED,
        entity: "setting",
        variant: "user_scope",
        shape: untyped_shape!(
            ["scope", "user_id", "key", "value"],
            serde_json::json!({
                "scope": "user",
                "user_id": su(5).to_string(),
                "key": "ui.density",
                "value": "compact",
            })
        ),
    },
    PublishedEvent {
        event_type: types::ORG_UPDATED,
        entity: "org",
        variant: "default",
        shape: untyped_shape!(
            ["name", "ticket_prefix"],
            serde_json::json!({ "name": "Meridian MSP", "ticket_prefix": "MER" })
        ),
    },
    // -- portal ---------------------------------------------------------------
    PublishedEvent {
        event_type: types::PORTAL_USER_CREATED,
        entity: "portal_user",
        variant: "default",
        shape: untyped_shape!(
            ["portal_user_id", "client_id", "email", "role", "status"],
            serde_json::json!({
                "portal_user_id": su(28).to_string(),
                "client_id": su(3).to_string(),
                "email": "dana@acme.example",
                "role": "contact",
                "status": "active",
            })
        ),
    },
    PublishedEvent {
        event_type: types::PORTAL_USER_UPDATED,
        entity: "portal_user",
        variant: "default",
        shape: untyped_shape!(
            ["portal_user_id", "client_id", "role", "status"],
            serde_json::json!({
                "portal_user_id": su(28).to_string(),
                "client_id": su(3).to_string(),
                "role": "it_manager",
                "status": "active",
            })
        ),
    },
    PublishedEvent {
        event_type: types::PORTAL_USER_DELETED,
        entity: "portal_user",
        variant: "default",
        shape: untyped_shape!(
            ["portal_user_id"],
            serde_json::json!({ "portal_user_id": su(28).to_string() })
        ),
    },
    PublishedEvent {
        event_type: types::BOARD_PORTAL_EXPOSURE_UPDATED,
        entity: "board_portal_exposure",
        variant: "default",
        shape: untyped_shape!(
            ["board_id", "client_id", "exposed", "visible_status_ids"],
            serde_json::json!({
                "board_id": su(2).to_string(),
                "client_id": su(3).to_string(),
                "exposed": true,
                "visible_status_ids": [su(13).to_string()],
            })
        ),
    },
    // -- webhooks (test-fire rides the domain stream) -------------------------
    PublishedEvent {
        event_type: types::WEBHOOK_TEST,
        entity: "webhook",
        variant: "default",
        shape: untyped_shape!(
            ["source", "fired_by", "note"],
            serde_json::json!({
                "source": "test_fire",
                "fired_by": su(5).to_string(),
                "note": "Manual test fire from the integrations settings page.",
            })
        ),
    },
    // -- scheduling -------------------------------------------------------------
    PublishedEvent {
        event_type: types::APPOINTMENT_CREATED,
        entity: "appointment",
        variant: "default",
        shape: typed_shape!(
            AppointmentCreated,
            AppointmentCreated {
                appointment: sample_appointment(AppointmentStatus::Scheduled),
            }
        ),
    },
    PublishedEvent {
        event_type: types::APPOINTMENT_UPDATED,
        entity: "appointment",
        variant: "default",
        shape: typed_shape!(
            AppointmentUpdated,
            AppointmentUpdated {
                appointment: sample_appointment(AppointmentStatus::Scheduled),
                rescheduled: true,
            }
        ),
    },
    PublishedEvent {
        event_type: types::APPOINTMENT_CANCELLED,
        entity: "appointment",
        variant: "default",
        shape: typed_shape!(
            AppointmentCancelled,
            AppointmentCancelled {
                appointment_id: su(19),
                user_id: su(4),
                cancelled_by: Some(su(5)),
            }
        ),
    },
    PublishedEvent {
        event_type: types::AVAILABILITY_UPDATED,
        entity: "availability",
        variant: "default",
        shape: typed_shape!(AvailabilityChanged, AvailabilityChanged { user_id: su(4) }),
    },
    PublishedEvent {
        event_type: types::ON_CALL_UPDATED,
        entity: "on_call",
        variant: "default",
        shape: typed_shape!(OnCallChanged, OnCallChanged { user_id: su(4) }),
    },
    PublishedEvent {
        event_type: types::CALENDAR_EXCLUSION_UPDATED,
        entity: "calendar_exclusion",
        variant: "default",
        shape: typed_shape!(
            CalendarExclusionChanged,
            CalendarExclusionChanged { user_id: su(4) }
        ),
    },
    // -- attachments ----------------------------------------------------------
    PublishedEvent {
        event_type: types::ATTACHMENT_ADDED,
        entity: "attachment",
        variant: "default",
        shape: typed_shape!(
            AttachmentAdded,
            AttachmentAdded {
                attachment_id: su(20),
                kind: "ticket".into(),
                ticket_id: Some(su(1)),
                ticket_comment_id: Some(su(6)),
                email_message_id: None,
                filename: "firewall-photo.jpg".into(),
                content_type: "image/jpeg".into(),
                size_bytes: 204_800,
                sha256: "aa".repeat(32),
                uploaded_by_user_id: Some(su(5)),
            }
        ),
    },
    PublishedEvent {
        event_type: types::ATTACHMENT_LINKED,
        entity: "attachment",
        variant: "default",
        shape: typed_shape!(
            AttachmentLinked,
            AttachmentLinked {
                attachment_id: su(20),
                ticket_comment_id: su(6),
                ticket_id: su(1),
            }
        ),
    },
    PublishedEvent {
        event_type: types::ATTACHMENT_DELETED,
        entity: "attachment",
        variant: "default",
        shape: typed_shape!(
            AttachmentDeleted,
            AttachmentDeleted {
                attachment_id: su(20),
                ticket_id: Some(su(1)),
                object_removed: false,
            }
        ),
    },
];

fn sample_status(name: &str) -> BoardStatus {
    BoardStatus {
        id: su(13),
        board_id: su(2),
        org_id: su(0),
        name: name.into(),
        category: TicketStatusCategory::Waiting,
        sort_order: 3,
        is_entry: false,
        color: Some("#3b82f6".into()),
    }
}

fn sample_appointment(status: AppointmentStatus) -> Appointment {
    Appointment {
        id: su(19),
        org_id: su(0),
        user_id: su(4),
        ticket_id: Some(su(1)),
        client_id: Some(su(3)),
        site_id: Some(su(10)),
        title: "On-site: swap firewall".into(),
        starts_at: st(8),
        ends_at: st(10),
        kind: AppointmentKind::OnSite,
        status,
        travel_minutes: 35,
        notes: None,
        created_at: st(7),
        updated_at: st(7),
    }
}

/// Event type constants that exist but have NO published payload yet.
/// Reserved so the completeness check can tell "forgotten registration"
/// from "declared ahead of use". Move out of here (and add goldens) the
/// moment an emission site ships.
pub static RESERVED_EVENT_TYPES: &[&str] = &[
    types::ORG_CREATED,
    types::USER_CREATED,
    types::TIME_ENTRY_UPDATED,
];

/// A retired event type (policy §2: "retired event types keep emitting
/// alongside their replacements for one generation"). Registering here is
/// the ONLY legal path to stop publishing an event type outright; until
/// `removal_target`, publishers must emit BOTH types.
pub struct RetiredEvent {
    /// The old type still being emitted during the window.
    pub event_type: &'static str,
    /// The replacement consumers should migrate to — must be published.
    pub replacement: &'static str,
    pub retired_in: Version,
    pub removal_target: Version,
}

/// Empty today: nothing has ever been retired. The first retirement
/// registers here AND in `deprecations.json` (kind `event`) — the
/// conformance suite cross-checks both registries.
pub static RETIRED_EVENTS: &[RetiredEvent] = &[];

/// Pure validation of the retired registry (fixture retention is checked
/// by the suite, which owns file access).
pub fn retired_registry_errors() -> Vec<String> {
    let mut errors = Vec::new();
    let published: std::collections::BTreeSet<&str> =
        PUBLISHED_EVENTS.iter().map(|e| e.event_type).collect();
    for r in RETIRED_EVENTS {
        if r.replacement == r.event_type {
            errors.push(format!(
                "retired event `{}` cannot be its own replacement",
                r.event_type
            ));
        }
        if !published.contains(r.replacement) {
            errors.push(format!(
                "retired event `{}`: replacement `{}` is not a published event type",
                r.event_type, r.replacement
            ));
        }
        if !generation_ok(&r.retired_in, &r.removal_target) {
            errors.push(format!(
                "retired event `{}`: removal_target {} is not a later generation than retired_in {}",
                r.event_type, r.removal_target, r.retired_in
            ));
        }
    }
    errors
}

/// Distinct event type strings in the published registry.
pub fn published_event_types() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = PUBLISHED_EVENTS.iter().map(|e| e.event_type).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// All registry entries for one event type (may span entities/variants).
pub fn entries_for_type(event_type: &str) -> Vec<&'static PublishedEvent> {
    PUBLISHED_EVENTS
        .iter()
        .filter(move |e| e.event_type == event_type)
        .collect()
}

/// Repo root, resolved from this crate's manifest dir (workspace member
/// two levels down).
pub fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

pub fn fixture_path(file: &str) -> std::path::PathBuf {
    repo_root().join(FIXTURES_DIR).join(file)
}

/// Loads and parses one committed golden fixture.
pub fn load_fixture(file: &str) -> Result<GoldenFixture, String> {
    let path = fixture_path(file);
    let raw =
        std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("parsing {}: {e}", path.display()))
}

impl Envelope {
    /// The U2 co-emit rule, as code: while an event type is retired
    /// ([`RETIRED_EVENTS`]), publishers emit the replacement envelope AND
    /// the retired type. This derives the retired envelope from the
    /// replacement: same org, entity, payload, schema ref, and occurred_at;
    /// fresh id; retired type string. Emit both in the same transaction.
    ///
    /// ```
    /// use core_events::Envelope;
    /// # fn demo(base: &Envelope) {
    /// let legacy = base.co_emit_as("legacy.type");
    /// assert_eq!(legacy.payload, base.payload);
    /// assert_ne!(legacy.id, base.id);
    /// # }
    /// ```
    pub fn co_emit_as(&self, retired_type: &str) -> Self {
        let mut retired = self.clone();
        retired.id = uuid::Uuid::now_v7();
        retired.event_type = retired_type.to_string();
        retired
    }
}
