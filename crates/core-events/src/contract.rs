//! The event-evolution contract (policy §2, the private build tracker U2/U4).
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

/// Deterministic calendar date, January n (n in 1..=28).
fn sd(day: u32) -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2024, 1, day).expect("fixed sample date")
}

/// Representative active monthly recurring contract for billing samples.
fn sample_contract() -> Contract {
    Contract {
        id: su(23),
        org_id: su(2),
        client_id: su(3),
        number: "C-2026-001".into(),
        name: "Managed IT retainer".into(),
        contract_type: ContractType::Recurring,
        status: ContractStatus::Active,
        currency: "USD".into(),
        effective_from: sd(1),
        effective_until: None,
        billing_frequency: Some(BillingFrequency::Monthly),
        billing_anchor: Some(sd(1)),
        amount_minor: Some(150_000),
        hourly_rate_minor: None,
        included_minutes: None,
        overage_rate_minor: None,
        unit_price_minor: None,
        unit_basis: None,
        incident_price_minor: None,
        incident_priority: None,
        po_number: None,
        notes: None,
        created_at: st(1),
        updated_at: st(1),
    }
}

/// Representative invoice-run roll-up for invoicing samples.
fn sample_invoice_run() -> InvoiceRun {
    InvoiceRun {
        id: su(31),
        org_id: su(2),
        number: "RUN-000007".into(),
        period_from: sd(1),
        period_to: sd(28),
        invoice_count: 1,
        total_minor: 150_000,
        currency: Some("USD".into()),
        due_in_days: 30,
        created_by: Some(su(4)),
        created_at: st(2),
    }
}

/// Representative issued invoice for invoicing samples.
fn sample_invoice() -> Invoice {
    Invoice {
        id: su(32),
        org_id: su(2),
        run_id: Some(su(31)),
        client_id: su(3),
        contract_id: su(23),
        number: "INV-000042".into(),
        status: InvoiceStatus::Issued,
        currency: "USD".into(),
        period_from: Some(sd(1)),
        period_to: Some(sd(28)),
        issued_at: sd(28),
        due_on: sd(28),
        total_minor: 150_000,
        total_paid_minor: 0,
        fx_rate_used: None,
        fx_rate_date: None,
        fx_source: None,
        base_total_minor: None,
        dunning_state: DunningState::None,
        dunning_updated_at: None,
        dunning_note: None,
        po_number: None,
        notes: None,
        created_by: Some(su(4)),
        created_at: st(2),
        updated_at: st(2),
    }
}

/// Representative payment receipt for invoicing samples.
fn sample_invoice_payment() -> InvoicePayment {
    InvoicePayment {
        id: su(33),
        org_id: su(2),
        invoice_id: su(32),
        amount_minor: 150_000,
        currency: "USD".into(),
        fx_rate_used: None,
        fx_rate_date: None,
        fx_source: None,
        base_amount_minor: None,
        provider: "none".into(),
        provider_ref: Some("local-approval".into()),
        notes: None,
        received_at: st(3),
        created_by: Some(su(4)),
    }
}

/// Representative org tag for the workflow-construct samples.
fn sample_tag() -> Tag {
    Tag {
        id: su(41),
        org_id: su(2),
        name: "customer-site".into(),
        color: Some("#0ea5e9".into()),
        created_by: Some(su(5)),
        created_at: st(1),
        updated_at: st(1),
    }
}

/// Representative time category for the Wave 39 taxonomy samples.
fn sample_time_category() -> core_domain::entities::TimeCategory {
    core_domain::entities::TimeCategory {
        id: su(46),
        org_id: su(2),
        slug: "onsite".into(),
        label: "On-site".into(),
        description: Some("Work at the client location".into()),
        default_billable: true,
        requires_unbillable_reason: false,
        rate_card_affinity: true,
        position: 10,
        retired: false,
        created_by: Some(su(5)),
        created_at: st(1),
        updated_at: st(1),
    }
}

/// Representative ticket cost line for the workflow-construct samples.
fn sample_cost_line() -> TicketCostLine {
    TicketCostLine {
        id: su(45),
        org_id: su(2),
        ticket_id: su(1),
        kind: CostLineKind::Parts,
        description: "Replacement PSU".into(),
        quantity: 1.0,
        unit_cost_minor: 8_900,
        currency: "USD".into(),
        billable: true,
        fx_rate_used: None,
        fx_rate_date: None,
        fx_source: None,
        base_amount_minor: None,
        invoice_line_id: None,
        created_by: Some(su(5)),
        created_at: st(1),
        updated_at: st(1),
    }
}

/// Representative ticket expense for the workflow-construct samples.
fn sample_expense() -> TicketExpense {
    TicketExpense {
        id: su(47),
        org_id: su(2),
        ticket_id: su(1),
        description: "Courier for replacement hardware".into(),
        amount_minor: 2_400,
        currency: "USD".into(),
        billable: true,
        fx_rate_used: None,
        fx_rate_date: None,
        fx_source: None,
        base_amount_minor: None,
        invoice_line_id: None,
        incurred_by: Some(su(5)),
        incurred_at: st(1),
        created_at: st(1),
        updated_at: st(1),
    }
}

/// Representative PM schedule for the workflow-construct samples.
fn sample_pm_schedule() -> PmSchedule {
    PmSchedule {
        id: su(48),
        org_id: su(2),
        name: "Monthly backup restore check".into(),
        board_id: su(2),
        title: "Backup restore verification".into(),
        body: Some("Run the documented restore drill.".into()),
        priority: Priority::P3,
        client_id: Some(su(3)),
        site_id: None,
        assignee_id: Some(su(4)),
        cadence: core_domain::cadence::Cadence::Monthly,
        time_of_day: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        day_of_week: None,
        day_of_month: Some(1),
        timezone: "UTC".into(),
        enabled: true,
        next_run_at: st(2),
        last_run_at: None,
        last_error: None,
        created_by: su(5),
        created_at: st(1),
        updated_at: st(1),
    }
}

/// Representative milestone phase of the sample project.
fn sample_phase() -> ProjectPhase {
    ProjectPhase {
        id: su(25),
        org_id: su(2),
        project_id: su(24),
        name: "Discovery milestone".into(),
        description: None,
        sort_order: 1,
        status: PhaseStatus::Completed,
        is_milestone: true,
        milestone_amount_minor: Some(150_000),
        milestone_currency: Some("USD".into()),
        // The sample phase is a COMPLETED milestone, so it carries no
        // planned window: the event contract's sample must not look like a
        // dated Gantt bar, or a consumer building a timeline from the
        // sample would draw a schedule nobody planned.
        start_date: None,
        end_date: None,
        completed_at: Some(st(2)),
        created_at: st(1),
        updated_at: st(2),
    }
}

/// Representative completed task inside the sample phase.
fn sample_task() -> ProjectTask {
    ProjectTask {
        id: su(26),
        org_id: su(2),
        project_id: su(24),
        phase_id: Some(su(25)),
        title: "Migration dry run".into(),
        description: None,
        status: TaskStatus::Completed,
        assignee_id: Some(su(4)),
        sort_order: 1,
        is_milestone: false,
        milestone_amount_minor: None,
        milestone_currency: None,
        due_at: None,
        completed_at: Some(st(2)),
        created_at: st(1),
        updated_at: st(2),
    }
}

/// Representative active milestone-billed project.
fn sample_project() -> Project {
    Project {
        id: su(24),
        org_id: su(2),
        client_id: Some(su(3)),
        name: "ERP rollout".into(),
        description: None,
        status: ProjectStatus::Active,
        billing_method: BillingMethod::Milestone,
        budget_minor: None,
        currency: None,
        start_date: Some(sd(1)),
        due_date: None,
        billing_contract_id: None,
        created_at: st(1),
        updated_at: st(1),
    }
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
    Appointment, AppointmentKind, AppointmentStatus, BillingFrequency, BillingMethod, BoardGroup,
    BoardStatus, CatalogItem, ChecklistItem, Contract, ContractStatus, ContractType, CostLineKind,
    DealStatus, DunningState, Invoice, InvoicePayment, InvoiceRun, InvoiceStatus, NotificationKind,
    NotificationPreference, PhaseStatus, PipelineDeal, PipelineStage, PmSchedule, Project,
    ProjectPhase, ProjectStatus, ProjectTask, Quote, QuoteStatus, StageType, Tag, TaskStatus,
    TicketComment, TicketCostLine, TicketExpense, TicketLinkType, TimeEntry,
};
use core_domain::events::{
    types, AgentSkillEnabledPayload, AlertAcknowledged, AlertIngested, AlertResolved,
    AppointmentCancelled, AppointmentCreated, AppointmentRequestApproved,
    AppointmentRequestCancelled, AppointmentRequestCreated, AppointmentRequestDeclined,
    AppointmentUpdated, AttachmentAdded, AttachmentDeleted, AttachmentLinked,
    AttachmentQuarantined, AvailabilityChanged, BoardFavoriteToggled, BoardGroupCreated,
    BoardGroupRemoved, BoardGroupUpdated, BoardMembershipRemoved, BoardMembershipUpdated,
    BoardStatusAdded, BoardStatusRemoved, BoardStatusUpdated, BoardTransitionsUpdated,
    CalendarExclusionChanged, CalendarSyncCompleted, CalendarSyncConnectionUpdated,
    CatalogItemArchived, CatalogItemCreated, CatalogItemUpdated, ClientCreated, ClientUpdated,
    ClientWorkflowOverrideCleared, ClientWorkflowOverrideSet, ConfigItemCreated,
    ConfigItemMergeUndone, ConfigItemMerged, ConfigItemUpdated, ContactCreated, ContactUpdated,
    ContractCreated, ContractRemoved, ContractUpdated, CreditNoteApplied, CreditNoteIssued,
    CreditNoteRefunded, DispatchQueueUpdated, DispatchStatusUpdated, DocumentAttachmentAdded,
    DocumentAttachmentDeleted, FxBaseCurrencyChanged, FxRateRecorded, IdentityBundleInstalled,
    IdentityBundleRevoked, IdentityBundleUpdated, ImportBatchFinished, ImportBatchStarted,
    InvoiceCreated, InvoiceDunningChanged, InvoicePaymentRecorded, InvoiceRunCompleted,
    InvoiceVoided, LabelTemplateCreated, LabelTemplateUpdated, LoanIssued, LoanReturned,
    MeetingTypeDeleted, MigrationPushSessionFinished, MigrationPushSessionStarted,
    NotificationCreated, OnCallChanged, PipelineDealCreated, PipelineDealStageChanged,
    PipelineDealUpdated, PipelineStageCreated, PipelineStageUpdated, PluginCatalogEntryCreated,
    PluginInstallRevoked, PluginInstallUpdated, PluginInstalled, PluginPublisherRevoked,
    PluginPublisherVerified, PluginRatingRecorded, PluginSettingsUpdated,
    PluginStagedImportApproved, PluginStagedImportCreated, PluginStagedImportRefused,
    PluginUpdatePending, PmScheduleChanged, PmScheduleDeleted, PodVisibilityUpdated,
    ProjectCreated, ProjectMilestoneCompleted, ProjectPhaseCompleted, ProjectPhaseCreated,
    ProjectPhaseUpdated, ProjectTaskCompleted, ProjectTaskCreated, ProjectTaskUpdated,
    ProjectTemplateRemoved, ProjectTemplateUpdated, ProjectUpdated, ProjectViewRemoved,
    ProjectViewUpdated, PurchaseBillPosted, PurchaseOrderCreated, PurchaseOrderStatusChanged,
    PurchaseOrderUpdated, PurchaseReceiptRecorded, QuoteAccepted, QuoteConverted, QuoteCreated,
    QuoteDeclined, QuoteExpired, QuoteSent, QuoteUpdated, RoleForked, RoleRestored,
    SchedulingLinkCreated, SchedulingLinkRevoked, SchedulingLinkScheduled, SchedulingLinkViewed,
    ScorecardSettingsUpdated, SecretFolderCreated, SecretFolderDeleted, SecretFolderForked,
    SecretFolderUpdated, SecretGrantDeleted, SecretGrantUpdated, SecretRecordCreated,
    SecretRecordDeleted, SecretRecordUpdated, SecretRevealed, SecretRotationDue,
    SecretShareLinkClaimed, SecretShareLinkCreated, SignOffConsentCreated, SignOffCreated,
    SignOffRevoked, SignOffSuperseded, SiteCreated, SiteUpdated, StockItemArchived,
    StockItemCreated, StockItemUpdated, StockLevelLow, StockMovementRecorded, TagCreated,
    TagDeleted, TagUpdated, TemplateCreated, TemplateRemoved, TemplateUpdated, TicketAssigned,
    TicketBoardMoved, TicketChecklistUpdated, TicketCommentAdded, TicketCostLineAdded,
    TicketCostLineRemoved, TicketCreated, TicketExpenseRecorded, TicketExpenseRemoved,
    TicketLinkChanged, TicketMerged, TicketReverted, TicketStatusChanged, TicketTagsChanged,
    TimeCategoryCreated, TimeCategoryUpdated, TimeEntryApprovalApproved, TimeEntryApprovalRejected,
    TimeEntryApprovalSubmitted, TimeEntryCreated, TimeEntryFixed, TimerPaused, TimerStarted,
    TimerStopped, ToolCheckinCompleted, ToolCheckoutCompleted, ToolKitCreated, ToolKitUpdated,
    TravelDraftSubmitted, UserActivated, UserDeactivated, UserInvited, UserRoleChanged,
    VendorCreated, VendorUpdated,
};
use core_domain::money::{CurrencyCode, Money};
use core_domain::policy::BoardRole;
use core_domain::status::{NoteKind, Priority, TicketStatusCategory};
use core_domain::workflow::TransitionEdge;

/// Every payload Marula publishes on the event stream today (envelope
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
                number: "MAR-1001".into(),
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
                board_id: su(2),
                assignee_id: Some(su(4)),
                occurred_at: st(2),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_BOARD_MOVED,
        entity: "ticket",
        variant: "default",
        shape: typed_shape!(
            TicketBoardMoved,
            TicketBoardMoved {
                ticket_id: su(1),
                number: "MAR-1001".into(),
                from_board_id: su(2),
                to_board_id: su(6),
                from_status: "triage".into(),
                to_status: "intake".into(),
                from_category: TicketStatusCategory::Open,
                to_category: TicketStatusCategory::Open,
                sla_reset: false,
                moved_by: Some(su(5)),
                occurred_at: st(3),
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
                board_id: su(2),
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
        event_type: types::TICKET_REVERTED,
        entity: "ticket",
        variant: "default",
        shape: typed_shape!(
            TicketReverted,
            TicketReverted {
                ticket_id: su(1),
                board_id: su(2),
                assignee_id: Some(su(4)),
                reverted_seq: 4,
                fields: vec!["title".into()],
                whole_record: false,
                reverted_by: Some(su(5)),
                occurred_at: st(4),
            }
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
                    // (Wave 17 lane 11) user-written fixture note: no
                    // machine provenance — workflow_meta stays unset.
                    kind: NoteKind::Internal,
                    body: "Routed through on-call.".into(),
                    created_at: st(3),
                    updated_at: None,
                    workflow_meta: None,
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
                    na: false,
                    sort_order: 0,
                    checked_by: None,
                    checked_at: None,
                    template_item_id: Some(su(9)),
                }],
                updated_by: Some(su(5)),
                board_id: su(2),
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
        event_type: types::CSAT_SURVEY_SENT,
        entity: "csat_survey",
        variant: "default",
        shape: untyped_shape!(
            [
                "ticket_id",
                "client_id",
                "status",
                "skip_reason",
                "occurred_at"
            ],
            serde_json::json!({
                "ticket_id": su(1).to_string(),
                "client_id": su(2).to_string(),
                "status": "sent",
                "skip_reason": serde_json::Value::Null,
                "occurred_at": st(9).to_rfc3339(),
            })
        ),
    },
    PublishedEvent {
        event_type: types::CSAT_RESPONSE_RECORDED,
        entity: "csat_survey",
        variant: "default",
        shape: untyped_shape!(
            [
                "ticket_id",
                "client_id",
                "assignee_id",
                "score",
                "occurred_at"
            ],
            serde_json::json!({
                "ticket_id": su(1).to_string(),
                "client_id": su(2).to_string(),
                "assignee_id": su(3).to_string(),
                "score": 5,
                "occurred_at": st(9).to_rfc3339(),
            })
        ),
    },
    PublishedEvent {
        event_type: types::REPLY_SNIPPET_CREATED,
        entity: "reply_snippet",
        variant: "default",
        shape: untyped_shape!(
            ["snippet_id", "name", "boards"],
            serde_json::json!({
                "snippet_id": su(1).to_string(),
                "name": "Rotate the logs",
                "boards": 0,
            })
        ),
    },
    PublishedEvent {
        event_type: types::REPLY_SNIPPET_UPDATED,
        entity: "reply_snippet",
        variant: "default",
        shape: untyped_shape!(
            ["snippet_id", "name"],
            serde_json::json!({
                "snippet_id": su(1).to_string(),
                "name": "Rotate the logs",
            })
        ),
    },
    PublishedEvent {
        event_type: types::REPLY_SNIPPET_FORKED,
        entity: "reply_snippet",
        variant: "default",
        shape: untyped_shape!(
            ["snippet_id", "from_snippet_id", "name"],
            serde_json::json!({
                "snippet_id": su(1).to_string(),
                "from_snippet_id": su(2).to_string(),
                "name": "Rotate the logs (copy)",
            })
        ),
    },
    PublishedEvent {
        event_type: types::REPLY_SNIPPET_REMOVED,
        entity: "reply_snippet",
        variant: "default",
        shape: untyped_shape!(
            ["snippet_id", "name"],
            serde_json::json!({
                "snippet_id": su(1).to_string(),
                "name": "Rotate the logs",
            })
        ),
    },
    PublishedEvent {
        event_type: types::JOURNEY_START_POINT_SET,
        entity: "journey_start_point",
        variant: "default",
        shape: untyped_shape!(
            ["start_point_id", "user_id", "day", "kind", "site_id"],
            serde_json::json!({
                "start_point_id": su(1).to_string(),
                "user_id": su(2).to_string(),
                "day": "2026-01-09",
                "kind": "home",
                "site_id": serde_json::Value::Null,
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
    // -- migration connector (the private build tracker D3) ---------------------------------
    PublishedEvent {
        event_type: types::MIGRATION_PUSH_SESSION_STARTED,
        entity: "migration_push_session",
        variant: "default",
        shape: typed_shape!(
            MigrationPushSessionStarted,
            MigrationPushSessionStarted {
                session_id: su(34),
                batch_id: su(12),
                entity_kind: "client".into(),
                conflict_policy: "update".into(),
                label: "connectwise-clients-2026".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::MIGRATION_PUSH_SESSION_FINISHED,
        entity: "migration_push_session",
        variant: "default",
        shape: typed_shape!(
            MigrationPushSessionFinished,
            MigrationPushSessionFinished {
                session_id: su(34),
                batch_id: su(12),
                entity_kind: "client".into(),
                chunks_committed: 6,
                rows_total: 120,
                rows_created: 90,
                rows_updated: 18,
                rows_skipped: 7,
                rows_failed: 5,
            }
        ),
    },
    // -- dispatch v2 (the private build tracker Phase 3, Wave 9 lane 9) ----------------------
    PublishedEvent {
        event_type: types::DISPATCH_STATUS_UPDATED,
        entity: "appointment",
        variant: "default",
        shape: typed_shape!(
            DispatchStatusUpdated,
            DispatchStatusUpdated {
                appointment_id: su(40),
                status: "en_route".into(),
                note: Some("Left the depot".into()),
                source: "connector".into(),
                actor_id: su(41),
                occurred_at: st(4),
            }
        ),
    },
    PublishedEvent {
        event_type: types::DISPATCH_QUEUE_UPDATED,
        entity: "ticket",
        variant: "default",
        shape: typed_shape!(
            DispatchQueueUpdated,
            DispatchQueueUpdated {
                ticket_id: su(12),
                rank: 3,
                cleared: false,
            }
        ),
    },
    PublishedEvent {
        event_type: types::TRAVEL_DRAFT_SUBMITTED,
        entity: "travel_draft",
        variant: "default",
        shape: typed_shape!(
            TravelDraftSubmitted,
            TravelDraftSubmitted {
                draft_id: su(42),
                ticket_id: su(13),
                user_id: su(4),
                time_entry_id: su(43),
                distance: 42.5,
                distance_unit: "km".into(),
                vehicle: "van".into(),
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
    // -- org user management (Wave 11 lane 2) -------------------------------
    PublishedEvent {
        event_type: types::USER_INVITED,
        entity: "user",
        variant: "default",
        shape: typed_shape!(
            UserInvited,
            UserInvited {
                user_id: su(4),
                email: "dana@acme.example".into(),
                display_name: "Dana Reyes".into(),
                role: "technician".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::USER_ACTIVATED,
        entity: "user",
        variant: "default",
        shape: typed_shape!(UserActivated, UserActivated { user_id: su(4) }),
    },
    PublishedEvent {
        event_type: types::USER_DEACTIVATED,
        entity: "user",
        variant: "default",
        shape: typed_shape!(UserDeactivated, UserDeactivated { user_id: su(4) }),
    },
    PublishedEvent {
        event_type: types::USER_ROLE_CHANGED,
        entity: "user",
        variant: "default",
        shape: typed_shape!(
            UserRoleChanged,
            UserRoleChanged {
                user_id: su(4),
                from_role: Some("technician".into()),
                to_role: "dispatcher".into(),
            }
        ),
    },
    // -- role directory + preset lifecycle (Wave 13 lane 6, RB1) ------------
    PublishedEvent {
        event_type: types::ROLE_FORKED,
        entity: "role",
        variant: "default",
        shape: typed_shape!(
            RoleForked,
            RoleForked {
                role_id: su(30),
                name: "Dispatcher (custom)".into(),
                from_builtin: Some("dispatcher".into()),
            }
        ),
    },
    PublishedEvent {
        event_type: types::ROLE_RESTORED,
        entity: "role",
        variant: "default",
        shape: typed_shape!(
            RoleRestored,
            RoleRestored {
                role_id: su(30),
                builtin: "dispatcher".into(),
            }
        ),
    },
    // -- template registry (Wave 13 lane 5, KB3/KB4) ------------------------
    PublishedEvent {
        event_type: types::TEMPLATE_CREATED,
        entity: "template",
        variant: "default",
        shape: typed_shape!(
            TemplateCreated,
            TemplateCreated {
                template_id: su(31),
                kind: "email".into(),
                slug: "ticket_first_response".into(),
                from_builtin: Some("ticket_first_response".into()),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TEMPLATE_UPDATED,
        entity: "template",
        variant: "default",
        shape: typed_shape!(
            TemplateUpdated,
            TemplateUpdated {
                template_id: su(31),
                kind: "email".into(),
                slug: "ticket_first_response".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TEMPLATE_REMOVED,
        entity: "template",
        variant: "default",
        shape: typed_shape!(
            TemplateRemoved,
            TemplateRemoved {
                template_id: su(31),
                kind: "note".into(),
                slug: "org_note_snippet".into(),
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
                    ticket_id: Some(su(1)),
                    phase_id: None,
                    user_id: su(4),
                    started_at: st(4),
                    ended_at: st(6),
                    minutes: 120,
                    billable: true,
                    notes: Some("On-site visit".into()),
                    work_type: Some("onsite".into()),
                    timer_session_id: Some(su(15)),
                    approval_status: None,
                    approval_reason_code: None,
                    unbillable_reason: None,
                    created_at: st(6),
                },
            }
        ),
    },
    // Wave 43 lane 14 gate sweep: `time_entry.fixed` has a real emission
    // site (`routes/approvals.rs` calls ctx.emit with the TimeEntryFixed
    // typed shape) and has had one since the fix flow shipped. The constant
    // was declared but never registered, so the conformance suite was red
    // for a type the product already publishes. Registered with its golden.
    PublishedEvent {
        event_type: types::TIME_ENTRY_FIXED,
        entity: "time_entry",
        variant: "default",
        shape: typed_shape!(
            TimeEntryFixed,
            TimeEntryFixed {
                entry_id: su(17),
                fixed_by: su(4),
                description_changed: false,
                minutes: 120,
                fixed_at: st(7),
            }
        ),
    },
    // Wave 39 lane 1 (p39-time-categories, migration 0110): the org's
    // semantic time taxonomy. Payloads carry the full category row.
    PublishedEvent {
        event_type: types::TIME_CATEGORY_CREATED,
        entity: "time_category",
        variant: "default",
        shape: typed_shape!(
            TimeCategoryCreated,
            TimeCategoryCreated {
                category: sample_time_category(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TIME_CATEGORY_UPDATED,
        entity: "time_category",
        variant: "default",
        shape: typed_shape!(
            TimeCategoryUpdated,
            TimeCategoryUpdated {
                category: sample_time_category(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TIME_ENTRY_APPROVAL_SUBMITTED,
        entity: "time_entry",
        variant: "default",
        shape: typed_shape!(
            TimeEntryApprovalSubmitted,
            TimeEntryApprovalSubmitted {
                entry_id: su(17),
                ticket_id: Some(su(1)),
                entry_user_id: su(4),
                submitted_by: su(4),
                submitted_at: st(7),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TIME_ENTRY_APPROVAL_APPROVED,
        entity: "time_entry",
        variant: "default",
        shape: typed_shape!(
            TimeEntryApprovalApproved,
            TimeEntryApprovalApproved {
                entry_id: su(17),
                approved_by: su(2),
                decided_at: st(8),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TIME_ENTRY_APPROVAL_REJECTED,
        entity: "time_entry",
        variant: "default",
        shape: typed_shape!(
            TimeEntryApprovalRejected,
            TimeEntryApprovalRejected {
                entry_id: su(17),
                rejected_by: su(2),
                reason_code: "missing_notes".into(),
                note: Some("add the ticket reference".into()),
                decided_at: st(8),
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
            serde_json::json!({ "name": "Marula MSP", "ticket_prefix": "MER" })
        ),
    },
    PublishedEvent {
        event_type: types::POD_VISIBILITY_UPDATED,
        entity: "org",
        variant: "default",
        shape: typed_shape!(
            PodVisibilityUpdated,
            PodVisibilityUpdated {
                config: serde_json::json!({ "tasks": { "read_only": false } }),
                updated_by: Some(su(5)),
            }
        ),
    },
    // -- BYOK sealed keys (Wave 9 adapters lane) ------------------------------
    // Presence-only payloads: secret bytes never leave the sealed store,
    // so the contract floor is provider + boolean flags.
    PublishedEvent {
        event_type: types::AI_BYOK_UPDATED,
        entity: "byok_secret",
        variant: "default",
        shape: untyped_shape!(
            ["provider", "has_key"],
            serde_json::json!({ "provider": "openai", "has_key": true })
        ),
    },
    PublishedEvent {
        event_type: types::AI_BYOK_DELETED,
        entity: "byok_secret",
        variant: "default",
        shape: untyped_shape!(["provider"], serde_json::json!({ "provider": "openai" })),
    },
    // -- AI availability matrix (Wave 18 lane 1, ADR 0008) --------------------
    // Every admin mutation of the matrix is audited through these typed
    // contract events. Credential payloads are presence-only: secret
    // bytes never leave the sealed store.
    PublishedEvent {
        event_type: types::AI_SURFACE_GRANT_UPDATED,
        entity: "ai_surface_grant",
        variant: "default",
        shape: untyped_shape!(
            ["surface_key", "grantee_kind", "role_id", "user_id"],
            serde_json::json!({
                "surface_key": "general_chat",
                "grantee_kind": "user",
                "role_id": serde_json::Value::Null,
                "user_id": su(5).to_string(),
            })
        ),
    },
    PublishedEvent {
        event_type: types::AI_SURFACE_GRANT_REVOKED,
        entity: "ai_surface_grant",
        variant: "default",
        shape: untyped_shape!(
            ["grant_id", "surface_key"],
            serde_json::json!({
                "grant_id": su(7).to_string(),
                "surface_key": "general_chat",
            })
        ),
    },
    PublishedEvent {
        event_type: types::AI_MODEL_CATALOG_UPDATED,
        entity: "ai_model_catalog",
        variant: "default",
        shape: untyped_shape!(
            ["provider", "model", "enabled"],
            serde_json::json!({ "provider": "openai", "model": "gpt-4.1", "enabled": true })
        ),
    },
    PublishedEvent {
        event_type: types::AI_MODEL_CREDENTIAL_UPDATED,
        entity: "ai_model_credential",
        variant: "default",
        shape: untyped_shape!(
            ["credential_id", "provider", "kind", "owner_present"],
            serde_json::json!({
                "credential_id": su(8).to_string(),
                "provider": "openai",
                "kind": "api_key",
                "owner_present": false,
            })
        ),
    },
    PublishedEvent {
        event_type: types::AI_MODEL_CREDENTIAL_REVOKED,
        entity: "ai_model_credential",
        variant: "default",
        shape: untyped_shape!(
            ["credential_id", "provider", "kind"],
            serde_json::json!({
                "credential_id": su(8).to_string(),
                "provider": "openai",
                "kind": "api_key",
            })
        ),
    },
    PublishedEvent {
        event_type: types::AI_CAP_UPDATED,
        entity: "ai_cap",
        variant: "default",
        shape: untyped_shape!(
            [
                "scope",
                "user_present",
                "period",
                "has_soft_limit",
                "has_hard_limit"
            ],
            serde_json::json!({
                "scope": "org",
                "user_present": false,
                "period": "month",
                "has_soft_limit": true,
                "has_hard_limit": false,
            })
        ),
    },
    PublishedEvent {
        event_type: types::AI_MODEL_PICK_UPDATED,
        entity: "ai_model_pick",
        variant: "default",
        shape: untyped_shape!(
            ["surface_key", "scope", "provider", "model"],
            serde_json::json!({
                "surface_key": "general_chat",
                "scope": "user",
                "provider": "openai",
                "model": "gpt-4.1",
            })
        ),
    },
    PublishedEvent {
        event_type: types::AI_CHAT_SESSION_STARTED,
        entity: "ai_chat_session",
        variant: "default",
        shape: untyped_shape!(
            ["session_id", "surface_key", "user_id"],
            serde_json::json!({
                "session_id": "01914f6a-4d1f-7f22-8d1a-1f18a1c6e9a1",
                "surface_key": "general_chat",
                "user_id": "6b6a0f2a-9a03-4a4a-9d4e-1f18a1c6e9a1",
            })
        ),
    },
    PublishedEvent {
        // Wave 18 lane 2b (p18-ai-gaps): the plugin-surface registration
        // leg emits this on list/register/enable/disable — the golden
        // registration was missed at emission time (caught by the Wave 23
        // lane 3 conformance run).
        event_type: types::AI_PLUGIN_SURFACE_UPDATED,
        entity: "ai_plugin_surface",
        variant: "default",
        shape: untyped_shape!(
            ["key", "label", "enabled", "inserted"],
            serde_json::json!({
                "key": "acme-fs",
                "label": "Acme file store",
                "enabled": true,
                "inserted": true,
            })
        ),
    },
    PublishedEvent {
        event_type: types::AI_CHAT_TURN_COMPLETED,
        entity: "ai_chat_turn",
        variant: "default",
        shape: untyped_shape!(
            [
                "session_id",
                "turn_id",
                "surface_key",
                "provider",
                "model",
                "prompt_tokens",
                "completion_tokens",
                "steps",
                "outcome"
            ],
            serde_json::json!({
                "session_id": "01914f6a-4d1f-7f22-8d1a-1f18a1c6e9a1",
                "turn_id": "01914f6a-4d1f-7f22-8d1a-1f18a1c6e9b2",
                "surface_key": "general_chat",
                "provider": "openai",
                "model": "gpt-4.1",
                "prompt_tokens": 120,
                "completion_tokens": 64,
                "steps": 1,
                "outcome": "completed",
            })
        ),
    },
    PublishedEvent {
        event_type: types::AI_CHAT_TOOL_INVOKED,
        entity: "ai_chat_tool_invocation",
        variant: "default",
        shape: untyped_shape!(
            ["session_id", "turn_id", "tool", "capability", "outcome"],
            serde_json::json!({
                "session_id": "01914f6a-4d1f-7f22-8d1a-1f18a1c6e9a1",
                "turn_id": "01914f6a-4d1f-7f22-8d1a-1f18a1c6e9b2",
                "tool": "tickets_get_ticket",
                "capability": "tickets:read",
                "outcome": "executed",
            })
        ),
    },
    PublishedEvent {
        // Wave 19 lane 3 straggler: `p19-vault-scale` conformance run —
        // emission sites (org master switch + per-surface config) shipped
        // in p18-ai-closeout without a registry registration.
        event_type: types::AI_DECISION_STATE_CHANGED,
        entity: "ai_features",
        variant: "master_switch",
        shape: untyped_shape!(
            ["org_id", "features_enabled"],
            serde_json::json!({
                "org_id": su(2).to_string(),
                "features_enabled": true,
            })
        ),
    },
    PublishedEvent {
        event_type: types::AI_DECISION_STATE_CHANGED,
        entity: "ai_decision_surface_config",
        variant: "surface_config",
        shape: untyped_shape!(
            ["surface_key", "mode", "enabled"],
            serde_json::json!({
                "surface_key": "decision_triage",
                "mode": "shadow",
                "enabled": false,
            })
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
    // -- portal v2 appointment requests ---------------------------------------
    // (Portal v2 lane: the request is an intent; approval additionally
    // emits the standard `appointment.created` above.)
    PublishedEvent {
        event_type: types::APPOINTMENT_REQUEST_CREATED,
        entity: "appointment_request",
        variant: "default",
        shape: typed_shape!(
            AppointmentRequestCreated,
            AppointmentRequestCreated {
                request: sample_appointment_request(
                    core_domain::entities::AppointmentRequestStatus::Pending
                ),
            }
        ),
    },
    PublishedEvent {
        event_type: types::APPOINTMENT_REQUEST_APPROVED,
        entity: "appointment_request",
        variant: "default",
        shape: typed_shape!(
            AppointmentRequestApproved,
            AppointmentRequestApproved {
                request_id: su(45),
                appointment_id: su(19),
                client_id: su(3),
                decided_by: su(5),
                decided_at: st(11),
            }
        ),
    },
    PublishedEvent {
        event_type: types::APPOINTMENT_REQUEST_DECLINED,
        entity: "appointment_request",
        variant: "default",
        shape: typed_shape!(
            AppointmentRequestDeclined,
            AppointmentRequestDeclined {
                request_id: su(45),
                client_id: su(3),
                decided_by: su(5),
                note: Some("That week is fully committed; please pick another day.".into()),
                decided_at: st(11),
            }
        ),
    },
    PublishedEvent {
        event_type: types::APPOINTMENT_REQUEST_CANCELLED,
        entity: "appointment_request",
        variant: "default",
        shape: typed_shape!(
            AppointmentRequestCancelled,
            AppointmentRequestCancelled {
                request_id: su(45),
                client_id: su(3),
                cancelled_by_portal_user_id: su(46),
                cancelled_at: st(11),
            }
        ),
    },
    // -- scheduling links (Wave 11 lane 4) -------------------------------------
    // (The raw token never appears in any event — only its SHA-256 hash
    // is stored; `scheduling_link.scheduled` co-emits the standard
    // `appointment.created`.)
    PublishedEvent {
        event_type: types::SCHEDULING_LINK_CREATED,
        entity: "scheduling_link",
        variant: "default",
        shape: typed_shape!(
            SchedulingLinkCreated,
            SchedulingLinkCreated {
                link: sample_scheduling_link(core_domain::entities::SchedulingLinkStatus::Active),
                created_by: su(5),
            }
        ),
    },
    PublishedEvent {
        event_type: types::SCHEDULING_LINK_VIEWED,
        entity: "scheduling_link",
        variant: "default",
        shape: typed_shape!(
            SchedulingLinkViewed,
            SchedulingLinkViewed {
                link_id: su(47),
                viewed_at: st(13),
                view_count: 1,
            }
        ),
    },
    PublishedEvent {
        event_type: types::SCHEDULING_LINK_SCHEDULED,
        entity: "scheduling_link",
        variant: "default",
        shape: typed_shape!(
            SchedulingLinkScheduled,
            SchedulingLinkScheduled {
                link_id: su(45),
                appointment_id: su(19),
                booker_name: Some("Dana Fischer".into()),
                booker_email: Some("dana@northwind.example".into()),
                scheduled_at: st(15),
            }
        ),
    },
    PublishedEvent {
        event_type: types::SCHEDULING_LINK_REVOKED,
        entity: "scheduling_link",
        variant: "default",
        shape: typed_shape!(
            SchedulingLinkRevoked,
            SchedulingLinkRevoked {
                link_id: su(45),
                revoked_by: su(5),
                revoked_at: st(16),
            }
        ),
    },
    // -- meeting-type hard delete (Wave 17 lane 13) ---------------------------
    PublishedEvent {
        event_type: types::MEETING_TYPE_DELETED,
        entity: "meeting_type",
        variant: "default",
        shape: typed_shape!(
            MeetingTypeDeleted,
            MeetingTypeDeleted {
                meeting_type_id: su(46),
                name: "On-site assessment".into(),
                deleted_by: su(5),
                deleted_at: st(17),
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
    // -- two-way calendar sync (Wave 14 lane 2) --------------------------------
    // (The raw outbound feed token never appears in any event — only its
    // SHA-256 hash is stored.)
    PublishedEvent {
        event_type: types::CALENDAR_SYNC_CONNECTION_UPDATED,
        entity: "calendar_sync_connection",
        variant: "default",
        shape: typed_shape!(
            CalendarSyncConnectionUpdated,
            CalendarSyncConnectionUpdated {
                connection_id: su(49),
                user_id: su(4),
                provider: "ical".into(),
                direction: "two_way".into(),
                status: "active".into(),
                last_error: None,
                occurred_at: st(16),
            }
        ),
    },
    PublishedEvent {
        event_type: types::CALENDAR_SYNC_COMPLETED,
        entity: "calendar_sync_connection",
        variant: "default",
        shape: typed_shape!(
            CalendarSyncCompleted,
            CalendarSyncCompleted {
                connection_id: su(49),
                user_id: su(4),
                imported: 3,
                updated: 1,
                released: 2,
                overlaps: 1,
                duplicates: 0,
                synced_at: st(17),
            }
        ),
    },
    // -- coaching scorecards (Wave 14 lane 3) ---------------------------------
    PublishedEvent {
        event_type: types::SCORECARD_SETTINGS_UPDATED,
        entity: "scorecard_settings",
        variant: "default",
        shape: typed_shape!(
            ScorecardSettingsUpdated,
            ScorecardSettingsUpdated {
                user_id: su(4),
                gamification_opt_in: true,
                occurred_at: st(18),
            }
        ),
    },
    // -- plugin runtime v2 + marketplace registry (Wave 14 lane 5) -----------
    // (Grant lists are capability KEYS only; the resolved permissions stay
    // server-side and are always re-derived from the existing matrix.)
    PublishedEvent {
        event_type: types::PLUGIN_CATALOG_ENTRY_CREATED,
        entity: "plugin_catalog_entry",
        variant: "default",
        shape: typed_shape!(
            PluginCatalogEntryCreated,
            PluginCatalogEntryCreated {
                catalog_id: su(50),
                slug: "ticket-summarizer".into(),
                name: "Ticket Summarizer".into(),
                publisher: "marula".into(),
                version: "1.0.0".into(),
                interface_version: 1,
                capabilities: vec!["tickets.read".into()],
                created_at: st(18),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PLUGIN_INSTALLED,
        entity: "plugin_install",
        variant: "default",
        shape: typed_shape!(
            PluginInstalled,
            PluginInstalled {
                install_id: su(51),
                catalog_id: su(50),
                slug: "ticket-summarizer".into(),
                version: "1.0.0".into(),
                granted_capabilities: vec!["tickets.read".into()],
                installed_at: st(19),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PLUGIN_INSTALL_UPDATED,
        entity: "plugin_install",
        variant: "default",
        shape: typed_shape!(
            PluginInstallUpdated,
            PluginInstallUpdated {
                install_id: su(51),
                catalog_id: su(50),
                slug: "ticket-summarizer".into(),
                version: "1.0.0".into(),
                status: "enabled".into(),
                granted_capabilities: vec!["tickets.read".into(), "tickets.write".into()],
                occurred_at: st(20),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PLUGIN_INSTALL_REVOKED,
        entity: "plugin_install",
        variant: "default",
        shape: typed_shape!(
            PluginInstallRevoked,
            PluginInstallRevoked {
                install_id: su(51),
                catalog_id: su(50),
                slug: "ticket-summarizer".into(),
                reason: Some("upstream vulnerability".into()),
                revoked_at: st(21),
            }
        ),
    },
    // -- the identity-bundle artifact lifecycle (lane p52-identity-bundle) ----
    PublishedEvent {
        event_type: types::IDENTITY_BUNDLE_INSTALLED,
        entity: "identity_bundle_install",
        variant: "default",
        shape: typed_shape!(
            IdentityBundleInstalled,
            IdentityBundleInstalled {
                install_id: su(55),
                name: "corp-okta".into(),
                version: "1.4.0".into(),
                interface_version: 1,
                provider_kinds: vec!["oidc".into()],
                artifact_digest: "ab".repeat(32),
                // Installs land DISABLED; the sample says so, because a
                // consumer must not assume approval switched anything on.
                status: "disabled".into(),
                occurred_at: st(22),
            }
        ),
    },
    PublishedEvent {
        event_type: types::IDENTITY_BUNDLE_UPDATED,
        entity: "identity_bundle_install",
        variant: "default",
        shape: typed_shape!(
            IdentityBundleUpdated,
            IdentityBundleUpdated {
                install_id: su(55),
                name: "corp-okta".into(),
                version: "1.4.0".into(),
                status: "enabled".into(),
                jit_enabled: false,
                occurred_at: st(23),
            }
        ),
    },
    PublishedEvent {
        event_type: types::IDENTITY_BUNDLE_REVOKED,
        entity: "identity_bundle_install",
        variant: "default",
        shape: typed_shape!(
            IdentityBundleRevoked,
            IdentityBundleRevoked {
                install_id: su(55),
                name: "corp-okta".into(),
                version: "1.4.0".into(),
                reason: Some("tenant decommissioned the IdP".into()),
                revoked_at: st(24),
            }
        ),
    },
    // -- ADR 0009 plugin & skill distribution (Wave 16 lane 7) ---------------
    PublishedEvent {
        event_type: types::PLUGIN_STAGED_IMPORT_CREATED,
        entity: "plugin_staged_item",
        variant: "default",
        shape: typed_shape!(
            PluginStagedImportCreated,
            PluginStagedImportCreated {
                staged_item_id: su(52),
                artifact_kind: "plugin".into(),
                source_kind: "local_zip".into(),
                source_url: "local://zip-import".into(),
                fetched_ref: None,
                artifact_digest: "aa".repeat(32),
                declared_capabilities: vec!["tickets.read".into()],
                uploaded_by: su(53),
                uploaded_at: st(19),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PLUGIN_STAGED_IMPORT_APPROVED,
        entity: "plugin_staged_item",
        variant: "default",
        shape: typed_shape!(
            PluginStagedImportApproved,
            PluginStagedImportApproved {
                staged_item_id: su(52),
                reviewed_by: su(54),
                review_note: None,
                artifact_kind: "plugin".into(),
                adopted_install_id: Some(su(51)),
                adopted_skill_id: None,
                granted_capabilities: vec!["tickets.read".into()],
                approved_at: st(20),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PLUGIN_STAGED_IMPORT_REFUSED,
        entity: "plugin_staged_item",
        variant: "default",
        shape: typed_shape!(
            PluginStagedImportRefused,
            PluginStagedImportRefused {
                staged_item_id: su(52),
                reviewed_by: su(54),
                review_note: Some("digest did not verify".into()),
                refused_at: st(20),
            }
        ),
    },
    // -- Marketplace v2 (Wave 22): verified publishers + ratings ------------
    PublishedEvent {
        event_type: types::PLUGIN_PUBLISHER_VERIFIED,
        entity: "plugin_publisher_verification",
        variant: "default",
        shape: typed_shape!(
            PluginPublisherVerified,
            PluginPublisherVerified {
                verification_id: Some(su(55)),
                publisher: "github.com/marula-labs".into(),
                display_name: "Marula Labs".into(),
                verified_by: su(54),
                verified_at: st(20),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PLUGIN_PUBLISHER_REVOKED,
        entity: "plugin_publisher_verification",
        variant: "default",
        shape: typed_shape!(
            PluginPublisherRevoked,
            PluginPublisherRevoked {
                verification_id: Some(su(55)),
                publisher: "github.com/marula-labs".into(),
                revoked_by: su(54),
                revoked_at: st(21),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PLUGIN_RATING_RECORDED,
        entity: "plugin_rating",
        variant: "default",
        shape: typed_shape!(
            PluginRatingRecorded,
            PluginRatingRecorded {
                source_kind: "marketplace_registry".into(),
                source_url: "registry://ticket-summarizer".into(),
                artifact_digest: "aa".repeat(32),
                rating: 4,
                comment: "Solid summarizer, unchecks cues".into(),
                rated_by: su(53),
                rated_at: st(20),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PLUGIN_UPDATE_PENDING,
        entity: "plugin_install",
        variant: "default",
        shape: typed_shape!(
            PluginUpdatePending,
            PluginUpdatePending {
                install_id: su(51),
                catalog_id: su(50),
                slug: "ticket-summarizer".into(),
                current_version: "1.0.0".into(),
                pending_version: "1.1.0".into(),
                pending_digest: "bb".repeat(32),
                added_capabilities: vec!["financial.read".into()],
                removed_capabilities: Vec::new(),
                occurred_at: st(20),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PLUGIN_SETTINGS_UPDATED,
        entity: "plugin_install",
        variant: "default",
        shape: typed_shape!(
            PluginSettingsUpdated,
            PluginSettingsUpdated {
                install_id: su(51),
                slug: "ticket-summarizer".into(),
                version: "1.0.0".into(),
                config_keys: vec!["tone".into()],
                updated_by: su(54),
                occurred_at: st(20),
            }
        ),
    },
    // -- sign-offs (Wave 43 track 2 lane 12) --------------------------------
    PublishedEvent {
        event_type: types::SIGN_OFF_CREATED,
        entity: "sign_off",
        variant: "default",
        shape: typed_shape!(
            SignOffCreated,
            SignOffCreated {
                sign_off_id: su(60),
                ticket_id: su(1),
                attachment_id: Some(su(61)),
                typed_agreement: false,
                signer_name: "Dana Whitfield".into(),
                signer_kind: "customer".into(),
                witnessed_by_user_id: Some(su(5)),
                captured_at: st(20),
                consent_version: 1,
                sha256: "cc".repeat(32),
                size_bytes: 18_432,
                supersedes_sign_off_id: None,
            }
        ),
    },
    PublishedEvent {
        event_type: types::SIGN_OFF_REVOKED,
        entity: "sign_off",
        variant: "default",
        shape: typed_shape!(
            SignOffRevoked,
            SignOffRevoked {
                sign_off_id: su(60),
                revocation_id: su(62),
                ticket_id: su(1),
                reason: "Customer disputed the work, re-sign requested".into(),
                seq: 1,
                revoked_by_user_id: su(5),
            }
        ),
    },
    PublishedEvent {
        event_type: types::SIGN_OFF_SUPERSEDED,
        entity: "sign_off",
        variant: "default",
        shape: typed_shape!(
            SignOffSuperseded,
            SignOffSuperseded {
                sign_off_id: su(60),
                ticket_id: su(1),
                corrected_by_sign_off_id: su(64),
            }
        ),
    },
    PublishedEvent {
        event_type: types::SIGN_OFF_CONSENT_CREATED,
        entity: "sign_off_consent",
        variant: "default",
        shape: typed_shape!(
            SignOffConsentCreated,
            SignOffConsentCreated {
                consent_id: su(63),
                version: 2,
                body: "I confirm the work described above was completed to my satisfaction.".into(),
                created_by: su(5),
            }
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
    // The scan gate's refusal (Wave 52 lane P6a, migration 0139). No
    // attachment id exists to name — nothing was admitted — so `sha256` is
    // the object's identity in the payload, exactly as it is in the
    // `attachment_scans` primary key.
    PublishedEvent {
        event_type: types::ATTACHMENT_QUARANTINED,
        entity: "attachment",
        variant: "default",
        shape: typed_shape!(
            AttachmentQuarantined,
            AttachmentQuarantined {
                sha256: "aa".repeat(32),
                filename: "invoice-scan.pdf".into(),
                content_type: "application/pdf".into(),
                size_bytes: 44_180,
                scanner: "clamd".into(),
                detail: Some("stream: Eicar-Test-Signature FOUND".into()),
                ticket_id: Some(su(1)),
                uploaded_by_user_id: Some(su(5)),
            }
        ),
    },
    // -- documents attachments (Wave 17 lane 13, migration 0089) --------------
    PublishedEvent {
        event_type: types::DOCUMENT_ATTACHMENT_ADDED,
        entity: "document_attachment",
        variant: "default",
        shape: typed_shape!(
            DocumentAttachmentAdded,
            DocumentAttachmentAdded {
                attachment_id: su(21),
                org_id: su(2),
                document_id: su(22),
                filename: "network-diagram.png".into(),
                content_type: "image/png".into(),
                size_bytes: 12_345,
                sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
                uploaded_by: Some(su(5)),
            }
        ),
    },
    PublishedEvent {
        event_type: types::DOCUMENT_ATTACHMENT_DELETED,
        entity: "document_attachment",
        variant: "default",
        shape: typed_shape!(
            DocumentAttachmentDeleted,
            DocumentAttachmentDeleted {
                attachment_id: su(21),
                document_id: su(22),
                object_removed: true,
            }
        ),
    },
    // -- knowledge base & documents ---------------------------------------
    PublishedEvent {
        event_type: types::KB_ARTICLE_CREATED,
        entity: "kb_article",
        variant: "default",
        shape: untyped_shape!(
            ["article_id", "status"],
            serde_json::json!({
                "article_id": su(21),
                "status": "published",
            })
        ),
    },
    PublishedEvent {
        event_type: types::KB_ARTICLE_UPDATED,
        entity: "kb_article",
        variant: "default",
        shape: untyped_shape!(
            ["article_id", "status"],
            serde_json::json!({
                "article_id": su(21),
                "status": "archived",
            })
        ),
    },
    PublishedEvent {
        event_type: types::KB_ARTICLE_DELETED,
        entity: "kb_article",
        variant: "default",
        shape: untyped_shape!(
            ["article_id"],
            serde_json::json!({
                "article_id": su(21),
            })
        ),
    },
    PublishedEvent {
        event_type: types::DOCUMENT_CREATED,
        entity: "document",
        variant: "default",
        shape: untyped_shape!(
            ["document_id", "version"],
            serde_json::json!({
                "document_id": su(22),
                "version": 1,
            })
        ),
    },
    PublishedEvent {
        event_type: types::DOCUMENT_UPDATED,
        entity: "document",
        variant: "default",
        shape: untyped_shape!(
            ["document_id", "version"],
            serde_json::json!({
                "document_id": su(22),
                "version": 2,
            })
        ),
    },
    PublishedEvent {
        event_type: types::DOCUMENT_RESTORED,
        entity: "document",
        variant: "default",
        shape: untyped_shape!(
            ["document_id", "version", "restored_from_version"],
            serde_json::json!({
                "document_id": su(22),
                "version": 3,
                "restored_from_version": 1,
            })
        ),
    },
    // -- asset taxonomy (Wave 11 lane 5, migration 0054) ------------------
    // Typed asset instances on the docs-platform; the MCP/connector seam
    // mirrors an external DMS from these.
    PublishedEvent {
        event_type: types::ASSET_CREATED,
        entity: "asset",
        variant: "default",
        shape: untyped_shape!(
            ["asset_id", "client_id"],
            serde_json::json!({
                "asset_id": su(25),
                "client_id": su(1),
            })
        ),
    },
    PublishedEvent {
        event_type: types::ASSET_UPDATED,
        entity: "asset",
        variant: "default",
        shape: untyped_shape!(
            ["asset_id"],
            serde_json::json!({
                "asset_id": su(25),
            })
        ),
    },
    PublishedEvent {
        event_type: types::ASSET_DELETED,
        entity: "asset",
        variant: "default",
        shape: untyped_shape!(
            ["asset_id"],
            serde_json::json!({
                "asset_id": su(25),
            })
        ),
    },
    // -- docs-platform layout + vault + links (lane 7, 0053; registered
    // by lane 5 — the constants shipped with emission sites but missed
    // their contract registration, which blocked the conformance suite).
    PublishedEvent {
        event_type: types::DOCUMENT_LAYOUT_CREATED,
        entity: "document_layout",
        variant: "default",
        shape: untyped_shape!(
            ["layout_id", "slug"],
            serde_json::json!({
                "layout_id": su(27),
                "slug": "server",
            })
        ),
    },
    PublishedEvent {
        event_type: types::DOCUMENT_LAYOUT_UPDATED,
        entity: "document_layout",
        variant: "default",
        shape: untyped_shape!(
            ["layout_id", "slug"],
            serde_json::json!({
                "layout_id": su(27),
                "slug": "server",
            })
        ),
    },
    PublishedEvent {
        event_type: types::DOCUMENT_LAYOUT_REMOVED,
        entity: "document_layout",
        variant: "default",
        shape: untyped_shape!(
            ["layout_id", "slug"],
            serde_json::json!({
                "layout_id": su(27),
                "slug": "server",
            })
        ),
    },
    PublishedEvent {
        event_type: types::PASSWORD_CREATED,
        entity: "password",
        variant: "default",
        shape: untyped_shape!(
            ["password_id", "client_id", "title"],
            serde_json::json!({
                "password_id": su(28),
                "client_id": su(1),
                "title": "Router admin",
            })
        ),
    },
    PublishedEvent {
        event_type: types::PASSWORD_UPDATED,
        entity: "password",
        variant: "default",
        shape: untyped_shape!(
            ["password_id", "secret_rotated"],
            serde_json::json!({
                "password_id": su(28),
                "secret_rotated": true,
            })
        ),
    },
    PublishedEvent {
        event_type: types::PASSWORD_DELETED,
        entity: "password",
        variant: "default",
        shape: untyped_shape!(
            ["password_id", "title"],
            serde_json::json!({
                "password_id": su(28),
                "title": "Router admin",
            })
        ),
    },
    PublishedEvent {
        event_type: types::PASSWORD_REVEALED,
        entity: "password",
        variant: "default",
        shape: untyped_shape!(
            ["password_id", "title"],
            serde_json::json!({
                "password_id": su(28),
                "title": "Router admin",
            })
        ),
    },
    PublishedEvent {
        event_type: types::LINK_ADDED,
        entity: "link",
        variant: "default",
        shape: untyped_shape!(
            ["from_kind", "from_id", "to_kind", "to_id"],
            serde_json::json!({
                "from_kind": "document",
                "from_id": su(22),
                "to_kind": "ticket",
                "to_id": su(2),
            })
        ),
    },
    PublishedEvent {
        event_type: types::LINK_REMOVED,
        entity: "link",
        variant: "default",
        shape: untyped_shape!(
            ["link_id"],
            serde_json::json!({
                "link_id": su(29),
            })
        ),
    },
    // -- contracts & billing ----------------------------------------------
    PublishedEvent {
        event_type: types::CONTRACT_CREATED,
        entity: "contract",
        variant: "default",
        shape: typed_shape!(
            ContractCreated,
            ContractCreated {
                contract: sample_contract(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::CONTRACT_UPDATED,
        entity: "contract",
        variant: "default",
        shape: typed_shape!(
            ContractUpdated,
            ContractUpdated {
                contract: sample_contract(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::CONTRACT_REMOVED,
        entity: "contract",
        variant: "default",
        shape: typed_shape!(
            ContractRemoved,
            ContractRemoved {
                contract_id: su(23),
                number: "C-2026-001".into(),
            }
        ),
    },
    // -- invoicing (native invoicing v1, Wave 9) ---------------------------
    PublishedEvent {
        event_type: types::INVOICE_RUN_COMPLETED,
        entity: "invoice_run",
        variant: "default",
        shape: typed_shape!(
            InvoiceRunCompleted,
            InvoiceRunCompleted {
                run: sample_invoice_run(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::INVOICE_CREATED,
        entity: "invoice",
        variant: "default",
        shape: typed_shape!(
            InvoiceCreated,
            InvoiceCreated {
                invoice: sample_invoice(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::INVOICE_VOIDED,
        entity: "invoice",
        variant: "default",
        shape: typed_shape!(
            InvoiceVoided,
            InvoiceVoided {
                invoice_id: su(32),
                number: "INV-000042".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::INVOICE_PAYMENT_RECORDED,
        entity: "invoice_payment",
        variant: "default",
        shape: typed_shape!(
            InvoicePaymentRecorded,
            InvoicePaymentRecorded {
                payment: sample_invoice_payment(),
                invoice_status: "paid".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::INVOICE_DUNNING_CHANGED,
        entity: "invoice",
        variant: "default",
        shape: typed_shape!(
            InvoiceDunningChanged,
            InvoiceDunningChanged {
                invoice_id: su(32),
                number: "INV-000042".into(),
                from_state: "none".into(),
                to_state: "overdue".into(),
                note: Some("Balance past due 30 days".into()),
                changed_at: st(4),
            }
        ),
    },
    // -- credit notes (Wave 11 backend-gaps lane) ---------------------------
    PublishedEvent {
        event_type: types::CREDIT_NOTE_ISSUED,
        entity: "credit_note",
        variant: "default",
        shape: typed_shape!(
            CreditNoteIssued,
            CreditNoteIssued {
                credit_note_id: su(33),
                number: "CRN-000012".into(),
                client_id: su(3),
                invoice_id: Some(su(32)),
                currency: "USD".into(),
                total_minor: 25_000,
                reason: Some("Goodwill credit".into()),
            }
        ),
    },
    PublishedEvent {
        event_type: types::CREDIT_NOTE_APPLIED,
        entity: "credit_note",
        variant: "default",
        shape: typed_shape!(
            CreditNoteApplied,
            CreditNoteApplied {
                credit_note_id: su(33),
                credit_note_number: "CRN-000012".into(),
                invoice_id: su(32),
                amount_minor: 10_000,
                currency: "USD".into(),
                applied_minor: 10_000,
                remaining_minor: 15_000,
            }
        ),
    },
    PublishedEvent {
        event_type: types::CREDIT_NOTE_REFUNDED,
        entity: "credit_note",
        variant: "default",
        shape: typed_shape!(
            CreditNoteRefunded,
            CreditNoteRefunded {
                credit_note_id: su(33),
                credit_note_number: "CRN-000012".into(),
                amount_minor: 5_000,
                currency: "USD".into(),
                refunded_minor: 5_000,
                remaining_minor: 10_000,
            }
        ),
    },
    // -- catalog / quotes / sales pipeline (Wave 12 lane 2) -----------------
    PublishedEvent {
        event_type: types::CATALOG_ITEM_ARCHIVED,
        entity: "catalog_item",
        variant: "default",
        shape: typed_shape!(
            CatalogItemArchived,
            CatalogItemArchived {
                item_id: su(37),
                name: "Managed workstation".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::CATALOG_ITEM_CREATED,
        entity: "catalog_item",
        variant: "default",
        shape: typed_shape!(
            CatalogItemCreated,
            CatalogItemCreated {
                item: sample_catalog_item(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::CATALOG_ITEM_UPDATED,
        entity: "catalog_item",
        variant: "default",
        shape: typed_shape!(
            CatalogItemUpdated,
            CatalogItemUpdated {
                item: sample_catalog_item(),
            }
        ),
    },
    // -- stock + consumables (Wave 32 lane 1, migration 0102) ---------------
    PublishedEvent {
        event_type: types::STOCK_ITEM_CREATED,
        entity: "stock_item",
        variant: "default",
        shape: typed_shape!(
            StockItemCreated,
            StockItemCreated {
                item: sample_stock_item(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::STOCK_ITEM_UPDATED,
        entity: "stock_item",
        variant: "default",
        shape: typed_shape!(
            StockItemUpdated,
            StockItemUpdated {
                item: sample_stock_item(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::STOCK_ITEM_ARCHIVED,
        entity: "stock_item",
        variant: "default",
        shape: typed_shape!(
            StockItemArchived,
            StockItemArchived {
                item_id: su(60),
                name: "Cat6 patch cable box".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::STOCK_MOVEMENT_RECORDED,
        entity: "stock_movement",
        variant: "default",
        shape: typed_shape!(
            StockMovementRecorded,
            StockMovementRecorded {
                movement: sample_stock_movement(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::STOCK_LEVEL_LOW,
        entity: "stock_level",
        variant: "default",
        shape: typed_shape!(
            StockLevelLow,
            StockLevelLow {
                org_id: su(2),
                stock_item_id: su(60),
                stock_location_id: su(62),
                qty_milli: 800,
                reorder_point_milli: 1_000,
                item_name: "Cat6 patch cable box".into(),
                ticket_id: None,
                occurred_at: st(3),
            }
        ),
    },
    // -- tools + toolboxes + client loans (Wave 32 lane 3, migration 0104) ---
    PublishedEvent {
        event_type: types::TOOL_KIT_CREATED,
        entity: "tool_kit",
        variant: "default",
        shape: typed_shape!(
            ToolKitCreated,
            ToolKitCreated {
                kit: sample_tool_kit(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TOOL_KIT_UPDATED,
        entity: "tool_kit",
        variant: "default",
        shape: typed_shape!(
            ToolKitUpdated,
            ToolKitUpdated {
                kit: sample_tool_kit(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TOOL_CHECKOUT_COMPLETED,
        entity: "tool_checkout",
        variant: "default",
        shape: typed_shape!(
            ToolCheckoutCompleted,
            ToolCheckoutCompleted {
                checkout: sample_tool_checkout(),
                lines: sample_tool_checkout_lines(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TOOL_CHECKIN_COMPLETED,
        entity: "tool_checkout",
        variant: "default",
        shape: typed_shape!(
            ToolCheckinCompleted,
            ToolCheckinCompleted {
                checkout: sample_tool_checkout(),
                lines: sample_tool_checkout_lines(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::LOAN_ISSUED,
        entity: "client_loan",
        variant: "default",
        shape: typed_shape!(
            LoanIssued,
            LoanIssued {
                loan: sample_client_loan(),
                lines: sample_client_loan_lines(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::LOAN_RETURNED,
        entity: "client_loan",
        variant: "default",
        shape: typed_shape!(
            LoanReturned,
            LoanReturned {
                loan: sample_client_loan(),
                lines: sample_client_loan_lines(),
            }
        ),
    },
    // -- label templates (Wave 32 lane 5, migration 0105) --------------------
    PublishedEvent {
        event_type: types::LABEL_TEMPLATE_CREATED,
        entity: "label_template",
        variant: "default",
        shape: typed_shape!(
            LabelTemplateCreated,
            LabelTemplateCreated {
                template: sample_label_template(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::LABEL_TEMPLATE_UPDATED,
        entity: "label_template",
        variant: "default",
        shape: typed_shape!(
            LabelTemplateUpdated,
            LabelTemplateUpdated {
                template: sample_label_template(),
            }
        ),
    },
    // -- purchasing + vendors (Wave 32 lane 2, migration 0103) --------------
    PublishedEvent {
        event_type: types::VENDOR_CREATED,
        entity: "vendor",
        variant: "default",
        shape: typed_shape!(
            VendorCreated,
            VendorCreated {
                vendor: sample_vendor(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::VENDOR_UPDATED,
        entity: "vendor",
        variant: "default",
        shape: typed_shape!(
            VendorUpdated,
            VendorUpdated {
                vendor: sample_vendor(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PURCHASE_ORDER_CREATED,
        entity: "purchase_order",
        variant: "default",
        shape: typed_shape!(
            PurchaseOrderCreated,
            PurchaseOrderCreated {
                order: sample_purchase_order(),
                lines: vec![sample_purchase_order_line()],
            }
        ),
    },
    PublishedEvent {
        event_type: types::PURCHASE_ORDER_UPDATED,
        entity: "purchase_order",
        variant: "default",
        shape: typed_shape!(
            PurchaseOrderUpdated,
            PurchaseOrderUpdated {
                order: sample_purchase_order(),
                lines: vec![sample_purchase_order_line()],
            }
        ),
    },
    PublishedEvent {
        event_type: types::PURCHASE_ORDER_STATUS_CHANGED,
        entity: "purchase_order",
        variant: "default",
        shape: typed_shape!(
            PurchaseOrderStatusChanged,
            PurchaseOrderStatusChanged {
                order_id: su(71),
                number: "PO-000123".into(),
                from: "partially_received".into(),
                to: "closed".into(),
                close_kind: Some("short".into()),
                occurred_at: st(5),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PURCHASE_RECEIPT_RECORDED,
        entity: "purchase_receipt",
        variant: "default",
        shape: typed_shape!(
            PurchaseReceiptRecorded,
            PurchaseReceiptRecorded {
                receipt: sample_purchase_receipt(),
                lines: vec![sample_purchase_receipt_line()],
            }
        ),
    },
    PublishedEvent {
        event_type: types::PURCHASE_BILL_POSTED,
        entity: "purchase_bill",
        variant: "default",
        shape: typed_shape!(
            PurchaseBillPosted,
            PurchaseBillPosted {
                bill: sample_purchase_bill(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::CLIENT_WORKFLOW_OVERRIDE_CLEARED,
        entity: "client",
        variant: "default",
        shape: typed_shape!(
            ClientWorkflowOverrideCleared,
            ClientWorkflowOverrideCleared {
                client_id: su(3),
                key: "sla.policy".into(),
                cleared_by: Some(su(4)),
                occurred_at: st(3),
            }
        ),
    },
    PublishedEvent {
        event_type: types::CLIENT_WORKFLOW_OVERRIDE_SET,
        entity: "client",
        variant: "default",
        shape: typed_shape!(
            ClientWorkflowOverrideSet,
            ClientWorkflowOverrideSet {
                client_id: su(3),
                key: "sla.policy".into(),
                state: "set".into(),
                value: Some(serde_json::Value::String(su(30).to_string())),
                changed_by: Some(su(4)),
                occurred_at: st(2),
            }
        ),
    },
    PublishedEvent {
        event_type: types::CONFIG_ITEM_CREATED,
        entity: "config_item",
        variant: "default",
        shape: typed_shape!(
            ConfigItemCreated,
            ConfigItemCreated {
                config_item_id: su(60),
                org_id: su(2),
                client_id: su(3),
                source_link_id: su(61),
                origin: "feed".into(),
                source_system: "ninjaone".into(),
                occurred_at: st(3),
            }
        ),
    },
    PublishedEvent {
        event_type: types::CONFIG_ITEM_MERGED,
        entity: "config_item",
        variant: "default",
        shape: typed_shape!(
            ConfigItemMerged,
            ConfigItemMerged {
                primary_config_item_id: su(60),
                merged_config_item_id: su(62),
                org_id: su(2),
                candidate_id: Some(su(63)),
                confidence: Some(0.95),
                reason: "serial match SN-9911".into(),
                auto: true,
                merged_by: Some(su(5)),
                occurred_at: st(3),
            }
        ),
    },
    PublishedEvent {
        event_type: types::CONFIG_ITEM_MERGE_UNDONE,
        entity: "config_item",
        variant: "default",
        shape: typed_shape!(
            ConfigItemMergeUndone,
            ConfigItemMergeUndone {
                primary_config_item_id: su(60),
                restored_config_item_id: su(62),
                org_id: su(2),
                merge_id: su(64),
                undone_by: Some(su(5)),
                occurred_at: st(3),
            }
        ),
    },
    PublishedEvent {
        event_type: types::CONFIG_ITEM_UPDATED,
        entity: "config_item",
        variant: "default",
        shape: typed_shape!(
            ConfigItemUpdated,
            ConfigItemUpdated {
                config_item_id: su(60),
                org_id: su(2),
                source_link_id: su(61),
                change: "feed_sync".into(),
                fields_changed: vec!["ip_address".into()],
                contradictions: vec![],
                occurred_at: st(3),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PIPELINE_DEAL_CREATED,
        entity: "pipeline_deal",
        variant: "default",
        shape: typed_shape!(
            PipelineDealCreated,
            PipelineDealCreated {
                deal: sample_pipeline_deal(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PIPELINE_DEAL_STAGE_CHANGED,
        entity: "pipeline_deal",
        variant: "default",
        shape: typed_shape!(
            PipelineDealStageChanged,
            PipelineDealStageChanged {
                deal_id: su(42),
                name: "Workstation rollout".into(),
                from_stage_id: Some(su(41)),
                to_stage_id: su(43),
                to_stage_type: StageType::Won,
                status: DealStatus::Won,
                amount_minor: 27_000,
                currency: "USD".into(),
                actor_id: Some(su(4)),
                occurred_at: st(5),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PIPELINE_DEAL_UPDATED,
        entity: "pipeline_deal",
        variant: "default",
        shape: typed_shape!(
            PipelineDealUpdated,
            PipelineDealUpdated {
                deal: sample_pipeline_deal(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PIPELINE_STAGE_CREATED,
        entity: "pipeline_stage",
        variant: "default",
        shape: typed_shape!(
            PipelineStageCreated,
            PipelineStageCreated {
                stage: sample_pipeline_stage(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PIPELINE_STAGE_UPDATED,
        entity: "pipeline_stage",
        variant: "default",
        shape: typed_shape!(
            PipelineStageUpdated,
            PipelineStageUpdated {
                stage: sample_pipeline_stage(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::QUOTE_ACCEPTED,
        entity: "quote",
        variant: "default",
        shape: typed_shape!(
            QuoteAccepted,
            QuoteAccepted {
                quote_id: su(40),
                number: "Q-000012".into(),
                client_id: su(3),
                currency: "USD".into(),
                total_minor: 27_000,
                decided_at: st(4),
            }
        ),
    },
    PublishedEvent {
        event_type: types::QUOTE_CONVERTED,
        entity: "quote",
        variant: "default",
        shape: typed_shape!(
            QuoteConverted,
            QuoteConverted {
                quote_id: su(40),
                quote_number: "Q-000012".into(),
                client_id: su(3),
                contract_id: su(43),
                invoice_id: su(44),
                currency: "USD".into(),
                total_minor: 27_000,
                converted_at: st(6),
            }
        ),
    },
    PublishedEvent {
        event_type: types::QUOTE_CREATED,
        entity: "quote",
        variant: "default",
        shape: typed_shape!(
            QuoteCreated,
            QuoteCreated {
                quote: sample_quote(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::QUOTE_DECLINED,
        entity: "quote",
        variant: "default",
        shape: typed_shape!(
            QuoteDeclined,
            QuoteDeclined {
                quote_id: su(40),
                number: "Q-000012".into(),
                client_id: su(3),
                decided_at: st(4),
            }
        ),
    },
    PublishedEvent {
        event_type: types::QUOTE_EXPIRED,
        entity: "quote",
        variant: "default",
        shape: typed_shape!(
            QuoteExpired,
            QuoteExpired {
                quote_id: su(40),
                number: "Q-000012".into(),
                client_id: su(3),
                decided_at: st(5),
            }
        ),
    },
    PublishedEvent {
        event_type: types::QUOTE_SENT,
        entity: "quote",
        variant: "default",
        shape: typed_shape!(
            QuoteSent,
            QuoteSent {
                quote_id: su(40),
                number: "Q-000012".into(),
                client_id: su(3),
                currency: "USD".into(),
                total_minor: 27_000,
                valid_until: sd(28),
                sent_at: st(3),
            }
        ),
    },
    PublishedEvent {
        event_type: types::QUOTE_UPDATED,
        entity: "quote",
        variant: "default",
        shape: typed_shape!(
            QuoteUpdated,
            QuoteUpdated {
                quote: sample_quote(),
            }
        ),
    },
    // -- currency & FX (ADR 0002, Wave 12 lane 10) --------------------------
    PublishedEvent {
        event_type: types::FX_RATE_RECORDED,
        entity: "fx_rate",
        variant: "default",
        shape: typed_shape!(
            FxRateRecorded,
            FxRateRecorded {
                rate_id: su(34),
                org_id: su(1),
                base_currency: "USD".into(),
                quote_currency: "ZAR".into(),
                as_of_date: chrono::NaiveDate::from_ymd_opt(2026, 9, 12).unwrap(),
                rate: "17.850000000000".into(),
                source: "manual".into(),
                captured_at: st(4),
            }
        ),
    },
    PublishedEvent {
        event_type: types::FX_BASE_CURRENCY_CHANGED,
        entity: "fx",
        variant: "default",
        shape: typed_shape!(
            FxBaseCurrencyChanged,
            FxBaseCurrencyChanged {
                org_id: su(1),
                from: "USD".into(),
                to: "EUR".into(),
                changed_at: st(4),
            }
        ),
    },
    // -- projects ----------------------------------------------------------
    PublishedEvent {
        event_type: types::PROJECT_CREATED,
        entity: "project",
        variant: "default",
        shape: typed_shape!(
            ProjectCreated,
            ProjectCreated {
                project: sample_project(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PROJECT_UPDATED,
        entity: "project",
        variant: "default",
        shape: typed_shape!(ProjectUpdated, ProjectUpdated { project_id: su(24) }),
    },
    PublishedEvent {
        event_type: types::PROJECT_PHASE_CREATED,
        entity: "project_phase",
        variant: "default",
        shape: typed_shape!(
            ProjectPhaseCreated,
            ProjectPhaseCreated {
                phase: sample_phase(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PROJECT_PHASE_UPDATED,
        entity: "project_phase",
        variant: "default",
        shape: typed_shape!(
            ProjectPhaseUpdated,
            ProjectPhaseUpdated {
                phase_id: su(25),
                project_id: su(24),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PROJECT_PHASE_COMPLETED,
        entity: "project_phase",
        variant: "default",
        shape: typed_shape!(
            ProjectPhaseCompleted,
            ProjectPhaseCompleted {
                phase: sample_phase(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PROJECT_TASK_CREATED,
        entity: "project_task",
        variant: "default",
        shape: typed_shape!(
            ProjectTaskCreated,
            ProjectTaskCreated {
                task: sample_task(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PROJECT_TASK_UPDATED,
        entity: "project_task",
        variant: "default",
        shape: typed_shape!(
            ProjectTaskUpdated,
            ProjectTaskUpdated {
                task_id: su(26),
                project_id: su(24),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PROJECT_TASK_COMPLETED,
        entity: "project_task",
        variant: "default",
        shape: typed_shape!(
            ProjectTaskCompleted,
            ProjectTaskCompleted {
                task: sample_task(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PROJECT_MILESTONE_COMPLETED,
        entity: "project",
        variant: "default",
        shape: typed_shape!(
            ProjectMilestoneCompleted,
            ProjectMilestoneCompleted {
                org_id: su(2),
                project_id: su(24),
                phase_id: Some(su(25)),
                task_id: None,
                name: "Discovery milestone".into(),
                amount: Some(Money {
                    minor: 150_000,
                    currency: CurrencyCode::new("USD").expect("valid sample currency"),
                }),
                billing_method: "milestone".into(),
                completed_at: st(2),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PROJECT_VIEW_UPDATED,
        entity: "project_view",
        variant: "default",
        shape: typed_shape!(
            ProjectViewUpdated,
            ProjectViewUpdated {
                view_id: su(27),
                project_id: Some(su(24)),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PROJECT_VIEW_REMOVED,
        entity: "project_view",
        variant: "default",
        shape: typed_shape!(ProjectViewRemoved, ProjectViewRemoved { view_id: su(27) }),
    },
    PublishedEvent {
        event_type: types::PROJECT_TEMPLATE_UPDATED,
        entity: "project_template",
        variant: "default",
        shape: typed_shape!(
            ProjectTemplateUpdated,
            ProjectTemplateUpdated {
                template_id: su(28),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PROJECT_TEMPLATE_REMOVED,
        entity: "project_template",
        variant: "default",
        shape: typed_shape!(
            ProjectTemplateRemoved,
            ProjectTemplateRemoved {
                template_id: su(28),
            }
        ),
    },
    // -- workflow constructs (Wave 11 lane 9, migration 0056) --------------
    PublishedEvent {
        event_type: types::TAG_CREATED,
        entity: "tag",
        variant: "default",
        shape: typed_shape!(TagCreated, TagCreated { tag: sample_tag() }),
    },
    PublishedEvent {
        event_type: types::TAG_UPDATED,
        entity: "tag",
        variant: "default",
        shape: typed_shape!(TagUpdated, TagUpdated { tag: sample_tag() }),
    },
    PublishedEvent {
        event_type: types::TAG_DELETED,
        entity: "tag",
        variant: "default",
        shape: typed_shape!(
            TagDeleted,
            TagDeleted {
                tag_id: su(41),
                name: "customer-site".into(),
                applied_count: 3,
            }
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_TAGS_CHANGED,
        entity: "ticket",
        variant: "default",
        shape: typed_shape!(
            TicketTagsChanged,
            TicketTagsChanged {
                ticket_id: su(1),
                board_id: su(2),
                tag_id: su(41),
                applied: true,
                changed_by: Some(su(5)),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_MERGED,
        entity: "ticket",
        variant: "default",
        shape: typed_shape!(
            TicketMerged,
            TicketMerged {
                source_id: su(1),
                source_number: "MAR-1001".into(),
                survivor_id: su(42),
                survivor_number: "MAR-1002".into(),
                comments_folded: 4,
                time_entries_folded: 2,
                time_minutes_folded: 95,
                cost_lines_folded: 1,
                expenses_folded: 1,
                children_relinked: 1,
                conflicts: vec!["priority".into()],
                merged_by: Some(su(5)),
                occurred_at: st(5),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_LINKED,
        entity: "ticket",
        variant: "default",
        shape: typed_shape!(
            TicketLinkChanged,
            TicketLinkChanged {
                from_ticket_id: su(1),
                to_ticket_id: su(42),
                link_type: TicketLinkType::ParentChild.as_str().into(),
                linked: true,
                changed_by: Some(su(5)),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_UNLINKED,
        entity: "ticket",
        variant: "default",
        shape: typed_shape!(
            TicketLinkChanged,
            TicketLinkChanged {
                from_ticket_id: su(1),
                to_ticket_id: su(42),
                link_type: TicketLinkType::MajorIncident.as_str().into(),
                linked: false,
                changed_by: Some(su(5)),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_COST_LINE_ADDED,
        entity: "ticket_cost_line",
        variant: "default",
        shape: typed_shape!(
            TicketCostLineAdded,
            TicketCostLineAdded {
                line: sample_cost_line(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_COST_LINE_UPDATED,
        entity: "ticket_cost_line",
        variant: "default",
        shape: typed_shape!(
            TicketCostLineAdded,
            TicketCostLineAdded {
                line: sample_cost_line(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_COST_LINE_REMOVED,
        entity: "ticket_cost_line",
        variant: "default",
        shape: typed_shape!(
            TicketCostLineRemoved,
            TicketCostLineRemoved {
                line_id: su(45),
                ticket_id: su(1),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_EXPENSE_RECORDED,
        entity: "ticket_expense",
        variant: "default",
        shape: typed_shape!(
            TicketExpenseRecorded,
            TicketExpenseRecorded {
                expense: sample_expense(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_EXPENSE_UPDATED,
        entity: "ticket_expense",
        variant: "default",
        shape: typed_shape!(
            TicketExpenseRecorded,
            TicketExpenseRecorded {
                expense: sample_expense(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::TICKET_EXPENSE_REMOVED,
        entity: "ticket_expense",
        variant: "default",
        shape: typed_shape!(
            TicketExpenseRemoved,
            TicketExpenseRemoved {
                expense_id: su(47),
                ticket_id: su(1),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PM_SCHEDULE_CREATED,
        entity: "pm_schedule",
        variant: "default",
        shape: typed_shape!(
            PmScheduleChanged,
            PmScheduleChanged {
                schedule: sample_pm_schedule(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PM_SCHEDULE_UPDATED,
        entity: "pm_schedule",
        variant: "default",
        shape: typed_shape!(
            PmScheduleChanged,
            PmScheduleChanged {
                schedule: sample_pm_schedule(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::PM_SCHEDULE_DELETED,
        entity: "pm_schedule",
        variant: "default",
        shape: typed_shape!(
            PmScheduleDeleted,
            PmScheduleDeleted {
                schedule_id: su(48),
                name: "Quarterly fleet audit".into(),
            }
        ),
    },
    // -- report definitions ------------------------------------------------
    PublishedEvent {
        event_type: types::REPORT_DEFINITION_CREATED,
        entity: "report_definition",
        variant: "default",
        shape: untyped_shape!(
            ["definition_id", "name", "kind", "scope"],
            serde_json::json!({
                "definition_id": su(29),
                "name": "Tickets by status",
                "kind": "tickets",
                "scope": "org",
            })
        ),
    },
    PublishedEvent {
        event_type: types::REPORT_DEFINITION_UPDATED,
        entity: "report_definition",
        variant: "default",
        shape: untyped_shape!(
            ["definition_id", "name", "kind", "scope"],
            serde_json::json!({
                "definition_id": su(29),
                "name": "Tickets by status",
                "kind": "tickets",
                "scope": "org",
            })
        ),
    },
    PublishedEvent {
        event_type: types::REPORT_DEFINITION_DELETED,
        entity: "report_definition",
        variant: "default",
        shape: untyped_shape!(
            ["definition_id", "name"],
            serde_json::json!({
                "definition_id": su(29),
                "name": "Tickets by status",
            })
        ),
    },
    // -- scheduled report delivery ------------------------------------------
    PublishedEvent {
        event_type: types::REPORT_SCHEDULE_CREATED,
        entity: "report_schedule",
        variant: "default",
        shape: untyped_shape!(
            [
                "schedule_id",
                "definition_id",
                "name",
                "cadence",
                "next_run_at"
            ],
            serde_json::json!({
                "schedule_id": su(30),
                "definition_id": su(29),
                "name": "Weekly ops digest",
                "cadence": "weekly",
                "next_run_at": "2026-01-05T08:00:00Z",
            })
        ),
    },
    PublishedEvent {
        event_type: types::REPORT_SCHEDULE_UPDATED,
        entity: "report_schedule",
        variant: "default",
        shape: untyped_shape!(
            [
                "schedule_id",
                "definition_id",
                "name",
                "cadence",
                "next_run_at"
            ],
            serde_json::json!({
                "schedule_id": su(30),
                "definition_id": su(29),
                "name": "Weekly ops digest",
                "cadence": "monthly",
                "next_run_at": "2026-02-01T08:00:00Z",
            })
        ),
    },
    PublishedEvent {
        event_type: types::REPORT_SCHEDULE_DELETED,
        entity: "report_schedule",
        variant: "default",
        shape: untyped_shape!(
            ["schedule_id", "name"],
            serde_json::json!({
                "schedule_id": su(30),
                "name": "Weekly ops digest",
            })
        ),
    },
    // -- agent platform (IF8 proposals, skills, capability registry) ---------
    PublishedEvent {
        event_type: types::AGENT_PROPOSAL_CREATED,
        entity: "agent_proposal",
        variant: "default",
        shape: untyped_shape!(
            [
                "proposal_id",
                "entity",
                "entity_id",
                "op",
                "classification",
                "dial_mode",
                "agent_ref",
                "skill_slug",
                "skill_version"
            ],
            serde_json::json!({
                "proposal_id": su(34),
                "entity": "ticket",
                "entity_id": su(1),
                "op": "update",
                "classification": "internal",
                "dial_mode": "proposed",
                "agent_ref": "api-key:00000000-0000-0000-0000-000000000035",
                "skill_slug": "triage-drafter",
                "skill_version": 2,
            })
        ),
    },
    PublishedEvent {
        event_type: types::AGENT_PROPOSAL_APPROVED,
        entity: "agent_proposal",
        variant: "default",
        shape: untyped_shape!(
            [
                "proposal_id",
                "entity",
                "entity_id",
                "approved_by",
                "applied"
            ],
            serde_json::json!({
                "proposal_id": su(34),
                "entity": "ticket",
                "entity_id": su(1),
                "approved_by": su(5),
                "applied": true,
            })
        ),
    },
    PublishedEvent {
        event_type: types::AGENT_PROPOSAL_DECLINED,
        entity: "agent_proposal",
        variant: "default",
        shape: untyped_shape!(
            ["proposal_id", "entity", "declined_by"],
            serde_json::json!({
                "proposal_id": su(34),
                "entity": "ticket",
                "declined_by": su(5),
            })
        ),
    },
    PublishedEvent {
        event_type: types::AGENT_PROPOSAL_WITHDRAWN,
        entity: "agent_proposal",
        variant: "default",
        shape: untyped_shape!(
            ["proposal_id", "entity", "withdrawn_by"],
            serde_json::json!({
                "proposal_id": su(34),
                "entity": "ticket",
                "withdrawn_by": "api-key:00000000-0000-0000-0000-000000000035",
            })
        ),
    },
    PublishedEvent {
        event_type: types::AGENT_SKILL_CREATED,
        entity: "agent_skill",
        variant: "default",
        shape: untyped_shape!(
            ["skill_id", "slug", "version", "created_by"],
            serde_json::json!({
                "skill_id": su(36),
                "slug": "triage-drafter",
                "version": 1,
                "created_by": su(5),
            })
        ),
    },
    PublishedEvent {
        event_type: types::AGENT_SKILL_UPDATED,
        entity: "agent_skill",
        variant: "default",
        shape: untyped_shape!(
            ["skill_id", "slug", "version", "created_by"],
            serde_json::json!({
                "skill_id": su(36),
                "slug": "triage-drafter",
                "version": 2,
                "created_by": su(5),
            })
        ),
    },
    PublishedEvent {
        event_type: types::MCP_CAPABILITY_UPDATED,
        entity: "mcp_capability",
        variant: "granted",
        shape: untyped_shape!(
            ["capability", "mode", "granted_by"],
            serde_json::json!({
                "capability": "tickets:read",
                "mode": "proposed",
                "granted_by": su(5),
            })
        ),
    },
    PublishedEvent {
        event_type: types::MCP_CAPABILITY_UPDATED,
        entity: "mcp_capability",
        variant: "revoked",
        shape: untyped_shape!(
            ["capability", "granted_by"],
            serde_json::json!({
                "capability": "tickets:read",
                "granted_by": su(5),
            })
        ),
    },
    PublishedEvent {
        event_type: types::BOARD_UPDATED,
        entity: "board",
        variant: "default",
        shape: untyped_shape!(
            ["board_id", "name"],
            serde_json::json!({
                "board_id": su(2),
                "name": "Field Services",
            })
        ),
    },
    // -- KB rich content (Wave 13 lane 4, migration 0069; ADR 0004) --------
    // oEmbed allowlist registry + embed instances + linked external
    // documents. Untyped additive JSON (KB pattern): the row is the read
    // model; payloads carry identity + the field the consumer most often
    // needs without a fetch.
    PublishedEvent {
        event_type: types::KB_EMBED_PROVIDER_CREATED,
        entity: "kb_embed_provider",
        variant: "default",
        shape: untyped_shape!(
            ["provider_id", "slug", "is_system"],
            serde_json::json!({
                "provider_id": su(70),
                "slug": "youtube",
                "is_system": true,
            })
        ),
    },
    PublishedEvent {
        event_type: types::KB_EMBED_PROVIDER_UPDATED,
        entity: "kb_embed_provider",
        variant: "default",
        shape: untyped_shape!(
            ["provider_id", "slug", "enabled"],
            serde_json::json!({
                "provider_id": su(70),
                "slug": "youtube",
                "enabled": false,
            })
        ),
    },
    PublishedEvent {
        event_type: types::KB_EMBED_PROVIDER_DELETED,
        entity: "kb_embed_provider",
        variant: "default",
        shape: untyped_shape!(
            ["provider_id"],
            serde_json::json!({
                "provider_id": su(70),
            })
        ),
    },
    PublishedEvent {
        event_type: types::KB_EMBED_CREATED,
        entity: "kb_embed",
        variant: "default",
        shape: untyped_shape!(
            ["embed_id", "article_id", "provider_slug"],
            serde_json::json!({
                "embed_id": su(71),
                "article_id": su(21),
                "provider_slug": "youtube",
            })
        ),
    },
    PublishedEvent {
        event_type: types::KB_EMBED_DELETED,
        entity: "kb_embed",
        variant: "default",
        shape: untyped_shape!(
            ["embed_id", "article_id"],
            serde_json::json!({
                "embed_id": su(71),
                "article_id": su(21),
            })
        ),
    },
    PublishedEvent {
        event_type: types::KB_EXTERNAL_DOCUMENT_CREATED,
        entity: "kb_external_document",
        variant: "default",
        shape: untyped_shape!(
            ["external_document_id", "article_id", "url_kind"],
            serde_json::json!({
                "external_document_id": su(72),
                "article_id": su(21),
                "url_kind": "sharepoint",
            })
        ),
    },
    PublishedEvent {
        event_type: types::KB_EXTERNAL_DOCUMENT_UPDATED,
        entity: "kb_external_document",
        variant: "default",
        shape: untyped_shape!(
            ["external_document_id"],
            serde_json::json!({
                "external_document_id": su(72),
            })
        ),
    },
    PublishedEvent {
        event_type: types::KB_EXTERNAL_DOCUMENT_DELETED,
        entity: "kb_external_document",
        variant: "default",
        shape: untyped_shape!(
            ["external_document_id"],
            serde_json::json!({
                "external_document_id": su(72),
            })
        ),
    },
    PublishedEvent {
        event_type: types::KB_EXTERNAL_DOCUMENT_LINK_CHECKED,
        entity: "kb_external_document",
        variant: "default",
        shape: untyped_shape!(
            ["external_document_id", "status"],
            serde_json::json!({
                "external_document_id": su(72),
                "status": "ok",
            })
        ),
    },
    // -- native secrets vault (Wave 16 lane 3; contract-registered Wave 16
    // lane 9 — replaces the temporary RESERVED heal) --------------------------
    PublishedEvent {
        event_type: types::SECRET_FOLDER_CREATED,
        entity: "secret_folder",
        variant: "default",
        shape: typed_shape!(
            SecretFolderCreated,
            SecretFolderCreated {
                folder_id: su(72),
                client_id: None,
                slug: "api-keys".into(),
                restricted: true,
            }
        ),
    },
    PublishedEvent {
        event_type: types::SECRET_FOLDER_CREATED,
        entity: "secret_folder",
        variant: "fork",
        shape: typed_shape!(
            SecretFolderForked,
            SecretFolderForked {
                client_id: Some(su(3)),
                forked_from: su(72),
                slug: "api-keys".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::SECRET_FOLDER_UPDATED,
        entity: "secret_folder",
        variant: "default",
        shape: typed_shape!(
            SecretFolderUpdated,
            SecretFolderUpdated {
                folder_id: su(72),
                slug: "api-keys".into(),
                restricted: true,
            }
        ),
    },
    PublishedEvent {
        event_type: types::SECRET_FOLDER_DELETED,
        entity: "secret_folder",
        variant: "default",
        shape: typed_shape!(
            SecretFolderDeleted,
            SecretFolderDeleted {
                folder_id: su(72),
                slug: "api-keys".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::SECRET_RECORD_CREATED,
        entity: "secret_record",
        variant: "default",
        shape: typed_shape!(
            SecretRecordCreated,
            SecretRecordCreated {
                record_id: su(73),
                client_id: None,
                folder_id: su(72),
                title: "PagerDuty service key".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::SECRET_RECORD_UPDATED,
        entity: "secret_record",
        variant: "default",
        shape: typed_shape!(
            SecretRecordUpdated,
            SecretRecordUpdated {
                record_id: su(73),
                fields_rotated: true,
            }
        ),
    },
    PublishedEvent {
        event_type: types::SECRET_RECORD_DELETED,
        entity: "secret_record",
        variant: "default",
        shape: typed_shape!(
            SecretRecordDeleted,
            SecretRecordDeleted {
                record_id: su(73),
                title: "PagerDuty service key".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::SECRET_REVEALED,
        entity: "secret_record",
        variant: "default",
        shape: typed_shape!(
            SecretRevealed,
            SecretRevealed {
                record_id: su(73),
                title: "PagerDuty service key".into(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::SECRET_GRANT_UPDATED,
        entity: "secret_grant",
        variant: "default",
        shape: typed_shape!(
            SecretGrantUpdated,
            SecretGrantUpdated {
                grant_id: su(74),
                role_id: su(6),
                folder_id: su(72),
                can_read: true,
                can_edit: true,
            }
        ),
    },
    PublishedEvent {
        event_type: types::SECRET_GRANT_DELETED,
        entity: "secret_grant",
        variant: "default",
        shape: typed_shape!(
            SecretGrantDeleted,
            SecretGrantDeleted {
                role_id: su(6),
                folder_id: su(72),
            }
        ),
    },
    // -- Wave 19 lane 3 (secrets product): rotation reminder + one-time
    //    share links — ids/titles only, NEVER tokens or secret material --
    PublishedEvent {
        event_type: types::SECRET_ROTATION_DUE,
        entity: "secret_record",
        variant: "default",
        shape: typed_shape!(
            SecretRotationDue,
            SecretRotationDue {
                subject_kind: "secret_record".into(),
                subject_id: su(73),
                title: "PagerDuty service key".into(),
                rotate_at: st(900).to_rfc3339(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::SECRET_SHARE_LINK_CREATED,
        entity: "secret_share_link",
        variant: "default",
        shape: typed_shape!(
            SecretShareLinkCreated,
            SecretShareLinkCreated {
                share_id: su(75),
                subject_kind: "secret_record".into(),
                subject_id: su(73),
                title: "PagerDuty service key".into(),
                expires_at: st(1000).to_rfc3339(),
            }
        ),
    },
    PublishedEvent {
        event_type: types::SECRET_SHARE_LINK_CLAIMED,
        entity: "secret_share_link",
        variant: "default",
        shape: typed_shape!(
            SecretShareLinkClaimed,
            SecretShareLinkClaimed {
                share_id: su(75),
                subject_kind: "secret_record".into(),
                subject_id: su(73),
                title: "PagerDuty service key".into(),
            }
        ),
    },
    // -- agent skills (0040 lane payloads, typed Wave 16 lane 9) --------------
    PublishedEvent {
        event_type: types::AGENT_SKILL_UPDATED,
        entity: "agent_skill",
        variant: "enabled",
        shape: typed_shape!(
            AgentSkillEnabledPayload,
            AgentSkillEnabledPayload {
                skill_id: su(36),
                slug: "triage-drafter".into(),
                version: 2,
                enabled: true,
                updated_by: su(5),
            }
        ),
    },
    // -- alert ingestor ------------------------------------------------------
    PublishedEvent {
        event_type: types::ALERT_INGESTED,
        entity: "alert",
        variant: "default",
        shape: typed_shape!(
            AlertIngested,
            AlertIngested {
                alert_id: su(30),
                org_id: su(2),
                source_id: su(31),
                source_ref: "acme-rmm".into(),
                dedup_key: "evt-9182".into(),
                correlation_key: "srv-01/disk-c".into(),
                title: "Disk usage critical on SRV-01".into(),
                severity: "critical".into(),
                outcome: "collapsed".into(),
                ticket_id: Some(su(1)),
                alert_count: 7,
                occurred_at: st(3),
            }
        ),
    },
    // -- alert triage (Wave 17 lane 13, migration 0089) -----------------------
    PublishedEvent {
        event_type: types::ALERT_ACKNOWLEDGED,
        entity: "alert",
        variant: "default",
        shape: typed_shape!(
            AlertAcknowledged,
            AlertAcknowledged {
                alert_id: su(30),
                org_id: su(2),
                acknowledged_by: su(5),
                acknowledged_at: st(4),
            }
        ),
    },
    PublishedEvent {
        event_type: types::ALERT_RESOLVED,
        entity: "alert",
        variant: "default",
        shape: typed_shape!(
            AlertResolved,
            AlertResolved {
                alert_id: su(30),
                org_id: su(2),
                resolved_by: su(5),
                resolved_at: st(5),
            }
        ),
    },
    // -- Wave 47 lane A8 (per-org custom domains) ----------------------------
    // The custom-domain lifecycle emits `org.custom_domain_changed` with an
    // inline payload whose only always-present key is `action`
    // (`set` / `verified` / `cleared`); `domain` rides along whenever a domain
    // is in play. Untyped because the route builds the object inline.
    PublishedEvent {
        event_type: types::ORG_CUSTOM_DOMAIN_CHANGED,
        entity: "org",
        variant: "default",
        shape: untyped_shape!(
            ["action"],
            serde_json::json!({ "action": "set", "domain": "support.example.com" })
        ),
    },
    // -- Wave 47 lane A7 (org suspension notice) -----------------------------
    // The suspension flag emits `org.suspension_changed` with an inline
    // payload whose only always-present key is `action` (`suspended` /
    // `cleared`); `reason` and `notify_after_secs` ride along when the flag is
    // set. Untyped because the route builds the object inline. The flag is
    // informational — no consumer may treat this event as a reason to refuse a
    // request.
    PublishedEvent {
        event_type: types::ORG_SUSPENSION_CHANGED,
        entity: "org",
        variant: "default",
        shape: untyped_shape!(
            ["action"],
            serde_json::json!({
                "action": "suspended",
                "reason": "billing_lapsed",
                "notify_after_secs": 604800,
            })
        ),
    },
    // -- RAG vector namespace (Wave 45 lane 1) -------------------------------
    // Cross-lane registration (Wave 47 lane A8): the RAG lane emitted both
    // types with inline payloads (its payload structs are private to
    // `crates/core-api/src/routes/rag.rs`) but never registered a contract
    // shape, so `every_event_type_constant_is_published_or_reserved` failed
    // on main. Registered as untyped shapes matching the emitted key sets,
    // which is the correct form while the payload structs stay private; the
    // RAG lane should switch these to typed shapes if the structs move to
    // `core-domain`.
    PublishedEvent {
        event_type: types::RAG_CHUNKS_INGESTED,
        entity: "rag_namespace",
        variant: "default",
        shape: untyped_shape!(
            ["namespace", "total", "inserted", "updated", "unchanged"],
            serde_json::json!({
                "namespace": "kb:default",
                "total": 3,
                "inserted": 2,
                "updated": 1,
                "unchanged": 0,
            })
        ),
    },
    PublishedEvent {
        event_type: types::RAG_SOURCE_DELETED,
        entity: "rag_deletion_receipt",
        variant: "default",
        shape: untyped_shape!(
            ["receipt"],
            serde_json::json!({
                "receipt": {
                    "receipt_id": su(90),
                    "org_id": su(2),
                    "namespace": "kb:default",
                    "source_kind": "document",
                    "source_id": "doc-1",
                    "deleted_chunks": 4,
                    "content_hashes": ["sha256:abc"],
                    "deleted_by": su(5),
                    "deleted_at": st(9),
                }
            })
        ),
    },
];

/// Representative catalog item for catalog samples (Wave 12 lane 2).
fn sample_catalog_item() -> CatalogItem {
    CatalogItem {
        id: su(37),
        org_id: su(2),
        name: "Managed workstation".into(),
        sku: Some("WS-MGD".into()),
        description: None,
        unit: "month".into(),
        currency: "USD".into(),
        unit_price_minor: 9_900,
        cost_minor: 4_000,
        taxable: true,
        category: Some("Managed services".into()),
        group_name: None,
        active: true,
        created_by: Some(su(4)),
        created_at: st(1),
        updated_at: st(1),
    }
}

/// Representative stock item for stock samples (Wave 32 lane 1).
fn sample_stock_item() -> core_domain::entities::StockItem {
    use core_domain::entities::StockItem;
    StockItem {
        id: su(60),
        org_id: su(2),
        name: "Cat6 patch cable box".into(),
        sku: Some("CAT6-BOX".into()),
        catalog_item_id: None,
        part_number: Some("C6-BOX-025".into()),
        kind: "consumable".into(),
        is_tool: false,
        unit: "box".into(),
        currency: "USD".into(),
        last_unit_cost_minor: Some(12_000),
        active: true,
        created_by: Some(su(4)),
        created_at: st(1),
        updated_at: st(1),
    }
}

/// Representative stock movement for stock samples (Wave 32 lane 1).
fn sample_stock_movement() -> core_domain::entities::StockMovement {
    use core_domain::entities::StockMovement;
    StockMovement {
        id: su(61),
        org_id: su(2),
        stock_item_id: su(60),
        kind: "receive".into(),
        from_location_id: None,
        to_location_id: Some(su(62)),
        qty_milli: 3_000,
        cost_minor: Some(12_000),
        currency: None,
        fx_rate_used: None,
        fx_rate_date: None,
        fx_source: None,
        base_cost_minor: None,
        serial_id: None,
        client_id: None,
        ticket_id: None,
        carrier: None,
        tracking_ref: None,
        note: None,
        created_by: Some(su(4)),
        created_at: st(3),
    }
}

/// Representative vendor for purchasing samples (Wave 32 lane 2).
fn sample_vendor() -> core_domain::entities::Vendor {
    use core_domain::entities::Vendor;
    Vendor {
        id: su(70),
        org_id: su(2),
        name: "CDW Canada".into(),
        code: Some("CDW".into()),
        email: Some("orders@cdw.example".into()),
        phone: Some("+1-800-555-0100".into()),
        website: Some("https://www.cdw.example".into()),
        account_number: Some("ACCT-7788".into()),
        default_currency: "USD".into(),
        notes: None,
        active: true,
        created_by: Some(su(4)),
        created_at: st(1),
        updated_at: st(1),
    }
}

/// Representative purchase order (ordered, one line pending) for samples.
fn sample_purchase_order() -> core_domain::entities::PurchaseOrder {
    use core_domain::entities::{PurchaseOrder, PurchaseOrderStatus};
    PurchaseOrder {
        id: su(71),
        org_id: su(2),
        vendor_id: su(70),
        number: "PO-000123".into(),
        status: PurchaseOrderStatus::Ordered,
        currency: "USD".into(),
        vendor_reference: Some("CDW-88213".into()),
        project_id: Some(su(35)),
        notes: None,
        ordered_at: Some(st(2)),
        expected_at: Some(sd(28)),
        closed_at: None,
        close_kind: None,
        cancelled_at: None,
        cancel_reason: None,
        created_by: Some(su(4)),
        created_at: st(1),
        updated_at: st(2),
    }
}

/// Representative purchase order line for purchasing samples.
fn sample_purchase_order_line() -> core_domain::entities::PurchaseOrderLine {
    use core_domain::entities::{PurchaseOrderLine, ReceiptState};
    PurchaseOrderLine {
        id: su(72),
        org_id: su(2),
        purchase_order_id: su(71),
        stock_item_id: Some(su(60)),
        catalog_item_id: None,
        description: "Cat6 patch cable box".into(),
        qty_ordered_milli: 10_000,
        qty_received_milli: 0,
        unit_cost_minor: 12_000,
        receipt_state: ReceiptState::Pending,
        position: 0,
        created_at: st(1),
        updated_at: st(1),
    }
}

/// Representative receipt document for purchasing samples.
fn sample_purchase_receipt() -> core_domain::entities::PurchaseReceipt {
    use core_domain::entities::PurchaseReceipt;
    PurchaseReceipt {
        id: su(73),
        org_id: su(2),
        purchase_order_id: su(71),
        location_id: su(62),
        carrier: Some("UPS".into()),
        tracking_ref: Some("1Z999AA10123456784".into()),
        note: None,
        received_at: st(3),
        created_by: Some(su(4)),
        created_at: st(3),
    }
}

/// Representative receipt line for purchasing samples.
fn sample_purchase_receipt_line() -> core_domain::entities::PurchaseReceiptLine {
    use core_domain::entities::PurchaseReceiptLine;
    PurchaseReceiptLine {
        id: su(74),
        org_id: su(2),
        receipt_id: su(73),
        po_line_id: su(72),
        qty_milli: 4_000,
        movement_id: Some(su(61)),
        created_at: st(3),
    }
}

/// Representative posted vendor bill for purchasing samples.
fn sample_purchase_bill() -> core_domain::entities::PurchaseBill {
    use core_domain::entities::PurchaseBill;
    PurchaseBill {
        id: su(75),
        org_id: su(2),
        purchase_order_id: su(71),
        bill_number: Some("INV-55621".into()),
        amount_minor: 48_000,
        currency: "USD".into(),
        fx_rate_used: None,
        fx_rate_date: None,
        fx_source: None,
        base_amount_minor: None,
        posted_at: Some(st(4)),
        created_by: Some(su(4)),
        created_at: st(3),
        updated_at: st(4),
    }
}

/// Representative toolbox/kit for tool samples (Wave 32 lane 3).
fn sample_tool_kit() -> core_domain::entities::ToolKit {
    use core_domain::entities::ToolKit;
    ToolKit {
        id: su(70),
        org_id: su(2),
        name: "Electrical toolbox".into(),
        code: Some("TB-ELEC-1".into()),
        note: None,
        active: true,
        created_by: Some(su(4)),
        created_at: st(1),
        updated_at: st(1),
    }
}

/// Representative label template for label samples (Wave 32 lane 5).
fn sample_label_template() -> core_domain::entities::LabelTemplate {
    use core_domain::entities::LabelTemplate;
    LabelTemplate {
        id: su(80),
        org_id: su(2),
        name: "Serialized unit — 2×1".into(),
        purpose: "serialized_unit".into(),
        size_preset: "zebra_2x1".into(),
        width_tenths_mm: None,
        height_tenths_mm: None,
        dpi: 203,
        barcode_token: Some("serial".into()),
        include_qr: true,
        fields: vec!["name".into(), "serial".into(), "location".into()],
        is_default: true,
        active: true,
        created_by: Some(su(4)),
        created_at: st(1),
        updated_at: st(1),
    }
}

/// Representative checkout header for tool samples (Wave 32 lane 3).
fn sample_tool_checkout() -> core_domain::entities::ToolCheckout {
    use core_domain::entities::ToolCheckout;
    ToolCheckout {
        id: su(71),
        org_id: su(2),
        holder_user_id: su(4),
        kit_id: Some(su(70)),
        from_location_id: su(62),
        due_at: st(3),
        notes: Some("Van kit for the north route".into()),
        status: "open".into(),
        returned_at: None,
        return_notes: None,
        checked_out_by: Some(su(4)),
        created_at: st(3),
        updated_at: st(3),
    }
}

/// One returned + one outstanding line: the shape covers both states.
fn sample_tool_checkout_lines() -> Vec<core_domain::entities::ToolCheckoutLine> {
    use core_domain::entities::ToolCheckoutLine;
    vec![
        ToolCheckoutLine {
            id: su(72),
            org_id: su(2),
            checkout_id: su(71),
            stock_item_id: su(60),
            serial_id: su(63),
            movement_out_id: su(64),
            movement_in_id: Some(su(65)),
            returned_at: Some(st(4)),
            return_condition: Some("good".into()),
            return_notes: None,
            created_at: st(3),
            updated_at: st(4),
        },
        ToolCheckoutLine {
            id: su(73),
            org_id: su(2),
            checkout_id: su(71),
            stock_item_id: su(60),
            serial_id: su(66),
            movement_out_id: su(64),
            movement_in_id: None,
            returned_at: None,
            return_condition: None,
            return_notes: None,
            created_at: st(3),
            updated_at: st(3),
        },
    ]
}

/// Representative client loan for loan samples (Wave 32 lane 3).
fn sample_client_loan() -> core_domain::entities::ClientLoan {
    use core_domain::entities::ClientLoan;
    ClientLoan {
        id: su(74),
        org_id: su(2),
        client_id: su(3),
        contract_id: None,
        agreement_ref: Some("AGR-2026-014".into()),
        out_at: st(3),
        expected_return_at: st(3).date_naive(),
        returned_at: None,
        status: "open".into(),
        is_lease: true,
        lease_cycle: Some("monthly".into()),
        lease_cost_minor: Some(4_900),
        lease_currency: Some("USD".into()),
        notes: None,
        return_notes: None,
        issued_by: Some(su(4)),
        created_at: st(3),
        updated_at: st(3),
    }
}

/// One stock-sourced + one asset-sourced line.
fn sample_client_loan_lines() -> Vec<core_domain::entities::ClientLoanLine> {
    use core_domain::entities::ClientLoanLine;
    vec![
        ClientLoanLine {
            id: su(75),
            org_id: su(2),
            loan_id: su(74),
            stock_item_id: Some(su(60)),
            serial_id: Some(su(63)),
            device_asset_id: Some(su(67)),
            movement_out_id: Some(su(64)),
            movement_in_id: None,
            returned_at: None,
            return_condition: None,
            return_notes: None,
            created_at: st(3),
            updated_at: st(3),
        },
        ClientLoanLine {
            id: su(76),
            org_id: su(2),
            loan_id: su(74),
            stock_item_id: None,
            serial_id: None,
            device_asset_id: Some(su(68)),
            movement_out_id: None,
            movement_in_id: None,
            returned_at: None,
            return_condition: None,
            return_notes: None,
            created_at: st(3),
            updated_at: st(3),
        },
    ]
}

/// Representative pipeline stage for sales-pipeline samples.
fn sample_pipeline_stage() -> PipelineStage {
    PipelineStage {
        id: su(41),
        org_id: su(2),
        name: "Proposal sent".into(),
        position: 2,
        stage_type: StageType::Open,
        active: true,
        created_at: st(1),
        updated_at: st(1),
    }
}

/// Representative open deal for sales-pipeline samples.
fn sample_pipeline_deal() -> PipelineDeal {
    PipelineDeal {
        id: su(42),
        org_id: su(2),
        client_id: su(3),
        quote_id: Some(su(40)),
        stage_id: su(41),
        name: "Workstation rollout".into(),
        amount_minor: 27_000,
        currency: "USD".into(),
        status: DealStatus::Open,
        expected_close_on: Some(sd(28)),
        owner_user_id: Some(su(4)),
        notes: None,
        closed_at: None,
        created_by: Some(su(4)),
        created_at: st(2),
        updated_at: st(2),
    }
}

/// Representative draft quote for quote-builder samples.
fn sample_quote() -> Quote {
    Quote {
        id: su(40),
        org_id: su(2),
        number: "Q-000012".into(),
        client_id: su(3),
        status: QuoteStatus::Draft,
        currency: "USD".into(),
        issued_on: sd(1),
        valid_until: sd(28),
        notes: None,
        terms: Some("Net 30 from acceptance".into()),
        subtotal_minor: 25_000,
        tax_rate_bps: 800,
        tax_minor: 2_000,
        total_minor: 27_000,
        sent_at: None,
        decided_at: None,
        converted_contract_id: None,
        converted_invoice_id: None,
        converted_at: None,
        created_by: Some(su(4)),
        created_at: st(2),
        updated_at: st(2),
    }
}

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
        wip_limit: None,
        client_visible: true,
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
        scheduling_link_id: None,
        travel_from: None,
        travel_distance_meters: None,
        travel_provider: None,
        travel_quality: None,
        created_at: st(7),
        updated_at: st(7),
    }
}

/// Representative portal appointment request (Portal v2 samples).
fn sample_appointment_request(
    status: core_domain::entities::AppointmentRequestStatus,
) -> core_domain::entities::AppointmentRequest {
    core_domain::entities::AppointmentRequest {
        id: su(45),
        org_id: su(0),
        client_id: su(3),
        portal_user_id: su(46),
        user_id: su(4),
        title: "Request: quarterly security review".into(),
        starts_at: st(12),
        ends_at: st(14),
        kind: AppointmentKind::Remote,
        notes: Some("Please walk us through the new backup reports.".into()),
        status,
        decision_note: None,
        decided_by: None,
        decided_at: None,
        appointment_id: None,
        created_at: st(9),
        updated_at: st(9),
    }
}

fn sample_scheduling_link(
    status: core_domain::entities::SchedulingLinkStatus,
) -> core_domain::entities::SchedulingLink {
    core_domain::entities::SchedulingLink {
        id: su(47),
        org_id: su(0),
        meeting_type_id: su(48),
        label: Some("Quarterly review — 45 minutes".into()),
        ticket_id: Some(su(80)),
        status,
        expires_at: Some(st(17)),
        first_viewed_at: None,
        view_count: 0,
        scheduled_at: None,
        scheduled_appointment_id: None,
        created_by: Some(su(5)),
        created_at: st(9),
        updated_at: st(9),
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
    // Wave 43 lane 14 gate sweep: `webhook.ingested` is an AUDIT-ACTION name
    // (`routes/webhook_receiver.rs` writes it into audit_events with the
    // connector actor), not a domain-event stream emission, so it is not a
    // published payload shape. If an envelope ever rides the stream under
    // this type, it moves to PUBLISHED_EVENTS with its golden.
    types::WEBHOOK_INGESTED,
    // Wave 11 lane 7 (docs-platform) emission-site types are PUBLISHED
    // now (typed shapes + goldens registered; the lane-9 temporary
    // reservations were dropped 2026-09-10 by the backend-gaps lane).
    // Wave 16 lane 3 (0078 secret vault) emission-site types were
    // REGISTERED as PublishedEvents (typed shapes + goldens) by the
    // Wave 16 lane 9 straggler sweep — the temporary reservations above
    // (nine secret.* constants) dropped there.
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
