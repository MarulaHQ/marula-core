//! The workflow node type registry (ADR 0016 decision 5; Wave 43 lane 13).
//!
//! This module is the single source of truth the whole workflow foundation
//! derives from. Each node type declares, in Rust: an id, a label, an
//! agent-oriented description, an `idempotent` flag, a JSON Schema for its
//! inputs, an output schema, the event types it can produce (`emits[]`),
//! its audience and the executor path it rides. Those declarations generate
//! the canvas palette, the schema-driven properties panel, the API types
//! (through `utoipa`, so the OpenAPI pin IS the artefact the LLM reads),
//! the plugin-action picker and the LLM tool catalogue. There is no second
//! registry in TypeScript — that is the load-bearing rule (c).
//!
//! What lives here, and what does not:
//!   * the node CATALOGUE (`catalogue` submodule) — every first-party node;
//!   * the registry API — palette, agent catalogue, param validation, the
//!     static loop refusal and the retry refusal lane 15 depends on;
//!   * the plugin node contract (`PluginNodeContract`) — the six fields a
//!     plugin-contributed entry must carry (A2);
//!   * the drift guard between `AutomationAction` variants and registry
//!     entries (`AUTOMATION_ACTION_NODES` + its conformance tests).
//!
//! What does NOT live here: execution (lane 15), the document model
//! (lane 15), the run ledger (lane 14) and any I/O — core-domain carries
//! none.

use std::borrow::Cow;
use std::fmt;

use serde_json::Value;
use thiserror::Error;

use crate::error::DomainError;

/// The declaration macro (B). Each entry becomes a named `const` AND a
/// member of the registry's single table, so an entry cannot exist in one
/// place without the other. New `AutomationAction` variants are held to
/// the same table by [`AUTOMATION_ACTION_NODES`]' conformance tests.
///
/// Defined before the `catalogue` module so the catalogue's declarations
/// invoke it in their own scope — one macro, one table, no parallel
/// registration path.
macro_rules! declare_nodes {
    ( $( $(#[$meta:meta])* $name:ident : $node:expr ),* $(,)? ) => {
        $(
            $(#[$meta])*
            pub const $name: NodeType = $node;
        )*
        /// Every declared node, in declaration order. Tier A first is
        /// NOT assumed — consumers sort (see [`crate::node_registry::palette`]).
        pub fn all() -> &'static [NodeType] {
            &[ $($name),* ]
        }
    };
}

pub mod catalogue;

/// Which part of the workflow surface a node belongs to. The palette groups
/// by family; the properties panel and the LLM tool catalogue both filter
/// on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum NodeFamily {
    /// A workflow starts from one of these; the trigger catalogue is the
    /// event-type registry (Wave 43 lane 2), not a hardcoded list.
    Trigger,
    /// The Decision Engine node (`decision.ask`) and the two AI edges the
    /// catalogue offers.
    Decision,
    /// Conditions: pure predicates over the subject. They answer a
    /// question; `flow.branch` and `flow.switch` decide where the answer
    /// sends the run.
    Condition,
    /// Effects — Tier A wrappers, the two Tier B generic nodes, the Tier C
    /// plugin nodes.
    Action,
    /// Control flow: waits, parking, joins, iteration, branching, error
    /// handling, termination.
    Control,
    /// Value nodes — literal, reference, math, template. The vocabulary a
    /// workflow cannot be written without.
    Value,
}

impl NodeFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trigger => "trigger",
            Self::Decision => "decision",
            Self::Condition => "condition",
            Self::Action => "action",
            Self::Control => "control",
            Self::Value => "value",
        }
    }
}

/// The catalogue's three tiers (docs/reference/workflow-node-catalogue.md).
/// Tier A is always visible and first in search; Tier B is the domain long
/// tail reached through the generic operation nodes; Tier C is
/// plugin-contributed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum NodeTier {
    A,
    B,
    C,
}

impl NodeTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
            Self::C => "c",
        }
    }
}

/// Who a node is for. Decides palette visibility against the LLM tool
/// catalogue — a power user and the authoring model should not see the
/// same list. A plugin entry that declares nothing stays `human` until it
/// says otherwise (an undeclared node never auto-joins the model's view).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum NodeAudience {
    Human,
    Ai,
    Both,
}

impl NodeAudience {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Ai => "ai",
            Self::Both => "both",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "human" => Some(Self::Human),
            "ai" => Some(Self::Ai),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    /// True when the audience includes the authoring model.
    pub fn includes_ai(self) -> bool {
        matches!(self, Self::Ai | Self::Both)
    }

    /// True when the audience includes a human author on the canvas.
    pub fn includes_human(self) -> bool {
        matches!(self, Self::Human | Self::Both)
    }
}

/// The plugin node contract's `kind` (A2): what a plugin-contributed entry
/// does. Derived from the entry's surface (`automation_action` → action,
/// `read` → read, `automation_trigger` → trigger); the UI kinds
/// (`widget`, `report_render`, `field_validator`) are not workflow nodes
/// and never become one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PluginNodeKind {
    /// Write: Marula tells the external system something.
    Action,
    /// Read: Marula asks the external system a question. Guardrails are
    /// stricter than a write's (ADR 0017 §5).
    Read,
    /// Trigger: the plugin declares the event type it can produce.
    Trigger,
}

impl PluginNodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Action => "action",
            Self::Read => "read",
            Self::Trigger => "trigger",
        }
    }

    /// The node kind a `plugin_surface_entries.surface` value maps onto,
    /// or `None` for the UI kinds, which are not workflow nodes.
    pub fn from_surface(surface: &str) -> Option<Self> {
        match surface {
            "automation_action" => Some(Self::Action),
            "read" => Some(Self::Read),
            "automation_trigger" => Some(Self::Trigger),
            _ => None,
        }
    }
}

/// The AI edges the catalogue offers. The three internal surfaces and the
/// tool-calling edge are refused by the registry (see `catalogue`), because
/// tool calling inside a workflow reintroduces the agentic loop ADR 0016
/// forbids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AiEdge {
    /// `decision.ask` — the Decision Engine node.
    Decision,
    /// `ai.transcription` — transcribe an attachment.
    Transcription,
    /// `ai.completion` — free-text completion behind an explicit grant.
    Completion,
}

impl AiEdge {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Decision => "ai.decision",
            Self::Transcription => "ai.transcription",
            Self::Completion => "ai.completion",
        }
    }
}

/// The executor path a node rides (A3): all three tiers derive from this
/// one registry. Tier A nodes are thin wrappers that validate their
/// arguments and then call the SAME executor path as `domain.update`;
/// Tier B IS that generic path; Tier C is the plugin surface executor.
/// The paths are declared here; the executor itself is lane 15's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExecutorPath {
    /// Engine-internal: a trigger resolves an envelope into a run.
    Trigger,
    /// Pure predicate, no effect.
    Condition,
    /// Value production, no effect.
    Value,
    /// Control flow, including the parking nodes (a parked run holds no
    /// worker — the queue does the waking).
    Control,
    /// A decision/AI edge, gated by the shared admission path.
    Ai(AiEdge),
    /// A Tier A thin wrapper over one closed domain operation. The
    /// operation carries the entity and the event the wrapper emits, so
    /// a wrapper and `domain.update` can never drift.
    DomainWrapper(DomainOp),
    /// Tier B: the generic update path the Tier A wrappers call.
    DomainUpdate,
    /// Tier B: the generic read path. Read-only and rate-capped.
    DomainQuery,
    /// Tier C: a plugin-contributed entry, re-read at execution time.
    Plugin(PluginNodeKind),
}

impl ExecutorPath {
    /// The stable name of the path, for the catalogue digest and for any
    /// API view that names the path. `Ai(..)` and `Plugin(..)` carry their
    /// edge/kind in the name, so two different paths never collide.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trigger => "trigger",
            Self::Condition => "condition",
            Self::Value => "value",
            Self::Control => "control",
            Self::Ai(edge) => edge.as_str(),
            Self::DomainWrapper(op) => op.as_str(),
            Self::DomainUpdate => "domain.update",
            Self::DomainQuery => "domain.query",
            Self::Plugin(kind) => match kind {
                PluginNodeKind::Action => "plugin.action",
                PluginNodeKind::Read => "plugin.read",
                PluginNodeKind::Trigger => "plugin.trigger",
            },
        }
    }
}

/// One closed domain operation. Every Tier A action wrapper maps onto
/// exactly one of these, and `domain.update`'s operation selector is the
/// same vocabulary — which is what keeps the registry a single source of
/// truth instead of ~150 hand-written action nodes. The executor resolves
/// the operation to the org-scoped service path; this enum only NAMES it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DomainOp {
    // tickets
    TicketCreate,
    TicketUpdate,
    TicketTransition,
    TicketAssign,
    TicketComment,
    TicketInternalNote,
    TicketSetPriority,
    TicketSetTags,
    TicketAddChecklistItem,
    TicketCompleteChecklistItem,
    TicketLink,
    TicketMerge,
    TicketMoveToBoard,
    TicketSetField,
    // time
    TimeEntryLog,
    TimeEntryUpdate,
    TimeEntrySubmitForApproval,
    TimerStart,
    TimerStop,
    // clients
    ClientCreate,
    ClientUpdate,
    ClientAddNote,
    ContactCreate,
    ContactUpdate,
    SiteCreate,
    // money
    QuoteCreate,
    QuoteSend,
    QuoteConvert,
    InvoiceCreate,
    InvoiceRecordPayment,
    InvoiceVoid,
    CreditNoteIssue,
    ContractCreate,
    // scheduling
    AppointmentCreate,
    AppointmentUpdate,
    AppointmentCancel,
    DispatchAssign,
    DispatchUpdateStatus,
    AppointmentRequestApprove,
    AppointmentRequestDecline,
    // communication
    EmailSend,
    NotificationSend,
    SmsSend,
    CsatRequest,
    PortalNotify,
    // field service
    AlertAcknowledge,
    AlertResolve,
    TravelDraftCreate,
    SignOffCapture,
    SignOffFile,
    // assets
    StockItemCreate,
    StockItemMove,
    ToolCheckout,
    ToolCheckin,
    LoanIssue,
    LoanReturn,
    PurchaseOrderCreate,
    PurchaseOrderReceive,
    PurchaseOrderPostBill,
    // records
    DocumentCreate,
    DocumentAttach,
    AttachmentUpload,
    KbArticleCreate,
    KbArticleUpdate,
    SecretRecord,
    // projects
    ProjectCreate,
    ProjectPhaseComplete,
    ProjectTaskCreate,
    ProjectTaskComplete,
}

impl DomainOp {
    pub const ALL: [DomainOp; 69] = [
        Self::TicketCreate,
        Self::TicketUpdate,
        Self::TicketTransition,
        Self::TicketAssign,
        Self::TicketComment,
        Self::TicketInternalNote,
        Self::TicketSetPriority,
        Self::TicketSetTags,
        Self::TicketAddChecklistItem,
        Self::TicketCompleteChecklistItem,
        Self::TicketLink,
        Self::TicketMerge,
        Self::TicketMoveToBoard,
        Self::TicketSetField,
        Self::TimeEntryLog,
        Self::TimeEntryUpdate,
        Self::TimeEntrySubmitForApproval,
        Self::TimerStart,
        Self::TimerStop,
        Self::ClientCreate,
        Self::ClientUpdate,
        Self::ClientAddNote,
        Self::ContactCreate,
        Self::ContactUpdate,
        Self::SiteCreate,
        Self::QuoteCreate,
        Self::QuoteSend,
        Self::QuoteConvert,
        Self::InvoiceCreate,
        Self::InvoiceRecordPayment,
        Self::InvoiceVoid,
        Self::CreditNoteIssue,
        Self::ContractCreate,
        Self::AppointmentCreate,
        Self::AppointmentUpdate,
        Self::AppointmentCancel,
        Self::DispatchAssign,
        Self::DispatchUpdateStatus,
        Self::AppointmentRequestApprove,
        Self::AppointmentRequestDecline,
        Self::EmailSend,
        Self::NotificationSend,
        Self::SmsSend,
        Self::CsatRequest,
        Self::PortalNotify,
        Self::AlertAcknowledge,
        Self::AlertResolve,
        Self::TravelDraftCreate,
        Self::SignOffCapture,
        Self::SignOffFile,
        Self::StockItemCreate,
        Self::StockItemMove,
        Self::ToolCheckout,
        Self::ToolCheckin,
        Self::LoanIssue,
        Self::LoanReturn,
        Self::PurchaseOrderCreate,
        Self::PurchaseOrderReceive,
        Self::PurchaseOrderPostBill,
        Self::DocumentCreate,
        Self::DocumentAttach,
        Self::AttachmentUpload,
        Self::KbArticleCreate,
        Self::KbArticleUpdate,
        Self::SecretRecord,
        Self::ProjectCreate,
        Self::ProjectPhaseComplete,
        Self::ProjectTaskCreate,
        Self::ProjectTaskComplete,
    ];

    /// The stable selector name `domain.update`'s `operation` input uses
    /// and the OpenAPI operation enum derives from.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TicketCreate => "ticket.create",
            Self::TicketUpdate => "ticket.update",
            Self::TicketTransition => "ticket.transition",
            Self::TicketAssign => "ticket.assign",
            Self::TicketComment => "ticket.comment",
            Self::TicketInternalNote => "ticket.internal_note",
            Self::TicketSetPriority => "ticket.set_priority",
            Self::TicketSetTags => "ticket.set_tags",
            Self::TicketAddChecklistItem => "ticket.add_checklist_item",
            Self::TicketCompleteChecklistItem => "ticket.complete_checklist_item",
            Self::TicketLink => "ticket.link",
            Self::TicketMerge => "ticket.merge",
            Self::TicketMoveToBoard => "ticket.move_to_board",
            Self::TicketSetField => "ticket.set_field",
            Self::TimeEntryLog => "time_entry.log",
            Self::TimeEntryUpdate => "time_entry.update",
            Self::TimeEntrySubmitForApproval => "time_entry.submit_for_approval",
            Self::TimerStart => "timer.start",
            Self::TimerStop => "timer.stop",
            Self::ClientCreate => "client.create",
            Self::ClientUpdate => "client.update",
            Self::ClientAddNote => "client.add_note",
            Self::ContactCreate => "contact.create",
            Self::ContactUpdate => "contact.update",
            Self::SiteCreate => "site.create",
            Self::QuoteCreate => "quote.create",
            Self::QuoteSend => "quote.send",
            Self::QuoteConvert => "quote.convert",
            Self::InvoiceCreate => "invoice.create",
            Self::InvoiceRecordPayment => "invoice.record_payment",
            Self::InvoiceVoid => "invoice.void",
            Self::CreditNoteIssue => "credit_note.issue",
            Self::ContractCreate => "contract.create",
            Self::AppointmentCreate => "appointment.create",
            Self::AppointmentUpdate => "appointment.update",
            Self::AppointmentCancel => "appointment.cancel",
            Self::DispatchAssign => "dispatch.assign",
            Self::DispatchUpdateStatus => "dispatch.update_status",
            Self::AppointmentRequestApprove => "appointment_request.approve",
            Self::AppointmentRequestDecline => "appointment_request.decline",
            Self::EmailSend => "email.send",
            Self::NotificationSend => "notification.send",
            Self::SmsSend => "sms.send",
            Self::CsatRequest => "csat.request",
            Self::PortalNotify => "portal.notify",
            Self::AlertAcknowledge => "alert.acknowledge",
            Self::AlertResolve => "alert.resolve",
            Self::TravelDraftCreate => "travel_draft.create",
            Self::SignOffCapture => "sign_off.capture",
            Self::SignOffFile => "sign_off.file",
            Self::StockItemCreate => "stock_item.create",
            Self::StockItemMove => "stock_item.move",
            Self::ToolCheckout => "tool.checkout",
            Self::ToolCheckin => "tool.checkin",
            Self::LoanIssue => "loan.issue",
            Self::LoanReturn => "loan.return",
            Self::PurchaseOrderCreate => "purchase_order.create",
            Self::PurchaseOrderReceive => "purchase_order.receive",
            Self::PurchaseOrderPostBill => "purchase_order.post_bill",
            Self::DocumentCreate => "document.create",
            Self::DocumentAttach => "document.attach",
            Self::AttachmentUpload => "attachment.upload",
            Self::KbArticleCreate => "kb_article.create",
            Self::KbArticleUpdate => "kb_article.update",
            Self::SecretRecord => "secret.record",
            Self::ProjectCreate => "project.create",
            Self::ProjectPhaseComplete => "project.phase.complete",
            Self::ProjectTaskCreate => "project.task.create",
            Self::ProjectTaskComplete => "project.task.complete",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.as_str() == value)
    }

    /// The entity the operation writes. `domain.query`'s entity filter and
    /// the properties panel both read this.
    pub fn entity(self) -> &'static str {
        match self {
            Self::TicketCreate
            | Self::TicketUpdate
            | Self::TicketTransition
            | Self::TicketAssign
            | Self::TicketComment
            | Self::TicketInternalNote
            | Self::TicketSetPriority
            | Self::TicketSetTags
            | Self::TicketAddChecklistItem
            | Self::TicketCompleteChecklistItem
            | Self::TicketLink
            | Self::TicketMerge
            | Self::TicketMoveToBoard
            | Self::TicketSetField => "ticket",
            Self::TimeEntryLog
            | Self::TimeEntryUpdate
            | Self::TimeEntrySubmitForApproval => "time_entry",
            Self::TimerStart | Self::TimerStop => "timer",
            Self::ClientCreate | Self::ClientUpdate | Self::ClientAddNote => "client",
            Self::ContactCreate | Self::ContactUpdate => "contact",
            Self::SiteCreate => "site",
            Self::QuoteCreate | Self::QuoteSend | Self::QuoteConvert => "quote",
            Self::InvoiceCreate | Self::InvoiceRecordPayment | Self::InvoiceVoid => "invoice",
            Self::CreditNoteIssue => "credit_note",
            Self::ContractCreate => "contract",
            Self::AppointmentCreate | Self::AppointmentUpdate | Self::AppointmentCancel => {
                "appointment"
            }
            Self::DispatchAssign | Self::DispatchUpdateStatus => "dispatch",
            Self::AppointmentRequestApprove | Self::AppointmentRequestDecline => {
                "appointment_request"
            }
            Self::EmailSend => "email",
            Self::NotificationSend | Self::PortalNotify => "notification",
            Self::SmsSend => "sms",
            Self::CsatRequest => "csat",
            Self::AlertAcknowledge | Self::AlertResolve => "alert",
            Self::TravelDraftCreate => "travel_draft",
            Self::SignOffCapture | Self::SignOffFile => "sign_off",
            Self::StockItemCreate | Self::StockItemMove => "stock_item",
            Self::ToolCheckout | Self::ToolCheckin => "tool",
            Self::LoanIssue | Self::LoanReturn => "loan",
            Self::PurchaseOrderCreate
            | Self::PurchaseOrderReceive
            | Self::PurchaseOrderPostBill => "purchase_order",
            Self::DocumentCreate | Self::DocumentAttach => "document",
            Self::AttachmentUpload => "attachment",
            Self::KbArticleCreate | Self::KbArticleUpdate => "kb_article",
            Self::SecretRecord => "secret",
            Self::ProjectCreate => "project",
            Self::ProjectPhaseComplete => "project_phase",
            Self::ProjectTaskCreate | Self::ProjectTaskComplete => "project_task",
        }
    }

    /// The domain event the operation produces — the emit seam the static
    /// loop refusal reads. Empty when the operation writes without a
    /// dedicated event of its own today (recorded per op in the catalogue;
    /// the loop refusal is conservative by construction: an op with no
    /// declared event can never be statically linked to a wait).
    pub const fn emitted_event(self) -> &'static str {
        match self {
            Self::TicketCreate => "ticket.created",
            Self::TicketUpdate => "ticket.updated",
            Self::TicketTransition => "ticket.status_changed",
            Self::TicketAssign => "ticket.assigned",
            Self::TicketComment | Self::TicketInternalNote => "ticket.comment_added",
            Self::TicketSetPriority => "ticket.updated",
            Self::TicketSetTags => "ticket.tags_changed",
            Self::TicketAddChecklistItem | Self::TicketCompleteChecklistItem => {
                "ticket.checklist_updated"
            }
            Self::TicketLink => "ticket.linked",
            Self::TicketMerge => "ticket.merged",
            Self::TicketMoveToBoard => "ticket.board_moved",
            Self::TicketSetField => "ticket.field_set",
            Self::TimeEntryLog => "time_entry.created",
            Self::TimeEntryUpdate => "time_entry.updated",
            Self::TimeEntrySubmitForApproval => "time_entry.approval_submitted",
            Self::TimerStart => "timer.started",
            Self::TimerStop => "timer.stopped",
            Self::ClientCreate => "client.created",
            Self::ClientUpdate | Self::ClientAddNote => "client.updated",
            Self::ContactCreate => "contact.created",
            Self::ContactUpdate => "contact.updated",
            Self::SiteCreate => "site.created",
            Self::QuoteCreate => "quote.created",
            Self::QuoteSend => "quote.sent",
            Self::QuoteConvert => "quote.converted",
            Self::InvoiceCreate => "invoice.created",
            Self::InvoiceRecordPayment => "invoice.payment_recorded",
            Self::InvoiceVoid => "invoice.voided",
            Self::CreditNoteIssue => "credit_note.issued",
            Self::ContractCreate => "contract.created",
            Self::AppointmentCreate => "appointment.created",
            Self::AppointmentUpdate => "appointment.updated",
            Self::AppointmentCancel => "appointment.cancelled",
            Self::DispatchAssign | Self::DispatchUpdateStatus => "dispatch.status_updated",
            Self::AppointmentRequestApprove => "appointment_request.approved",
            Self::AppointmentRequestDecline => "appointment_request.declined",
            Self::EmailSend => "email.message_recorded",
            Self::NotificationSend | Self::PortalNotify => "notification.created",
            Self::CsatRequest => "csat.survey_sent",
            Self::AlertAcknowledge => "alert.acknowledged",
            Self::AlertResolve => "alert.resolved",
            Self::TravelDraftCreate => "travel_draft.submitted",
            Self::StockItemCreate => "stock.item_created",
            Self::StockItemMove => "stock.movement_recorded",
            Self::ToolCheckout => "tool.checkout_completed",
            Self::ToolCheckin => "tool.checkin_completed",
            Self::LoanIssue => "loan.issued",
            Self::LoanReturn => "loan.returned",
            Self::PurchaseOrderCreate => "purchase.order_created",
            Self::PurchaseOrderReceive => "purchase.receipt_recorded",
            Self::PurchaseOrderPostBill => "purchase.bill_posted",
            Self::DocumentCreate => "document.created",
            Self::DocumentAttach => "document_attachment.added",
            Self::AttachmentUpload => "attachment.added",
            Self::KbArticleCreate => "kb_article.created",
            Self::KbArticleUpdate => "kb_article.updated",
            Self::SecretRecord => "secret.record_created",
            Self::ProjectCreate => "project.created",
            Self::ProjectPhaseComplete => "project.phase_completed",
            Self::ProjectTaskCreate => "project.task_created",
            Self::ProjectTaskComplete => "project.task_completed",
            // No dedicated event today: the write is observable through the
            // entity's own stream, and the loop refusal must stay
            // conservative (an unlinkable op can never deadlock a wait).
            Self::SmsSend | Self::SignOffCapture | Self::SignOffFile => "",
        }
    }

    /// Whether repeating the operation is safe. The rule the matrix
    /// follows, stated so a future node inherits it: CREATIONS, SENDS,
    /// MESSAGES and LEDGER MOVEMENTS are not safe to repeat (a second
    /// `stock_item.move` is a second movement, a second `email.send` is a
    /// second email); STATE SETTERS and READS are safe (setting a field
    /// twice leaves it set). `decision.ask`-shaped calls are declared
    /// false separately — they spend money and may answer differently.
    pub const fn idempotent(self) -> bool {
        !matches!(
            self,
            Self::TicketCreate
                | Self::TicketComment
                | Self::TicketInternalNote
                | Self::TicketAddChecklistItem
                | Self::TicketMerge
                | Self::TimeEntryLog
                | Self::TimerStart
                | Self::ClientCreate
                | Self::ClientAddNote
                | Self::ContactCreate
                | Self::SiteCreate
                | Self::QuoteCreate
                | Self::QuoteSend
                | Self::QuoteConvert
                | Self::InvoiceCreate
                | Self::InvoiceRecordPayment
                | Self::CreditNoteIssue
                | Self::ContractCreate
                | Self::AppointmentCreate
                | Self::AppointmentRequestApprove
                | Self::AppointmentRequestDecline
                | Self::EmailSend
                | Self::NotificationSend
                | Self::SmsSend
                | Self::CsatRequest
                | Self::PortalNotify
                | Self::TravelDraftCreate
                | Self::SignOffCapture
                | Self::SignOffFile
                | Self::StockItemCreate
                | Self::StockItemMove
                | Self::ToolCheckout
                | Self::ToolCheckin
                | Self::LoanIssue
                | Self::PurchaseOrderCreate
                | Self::PurchaseOrderReceive
                | Self::PurchaseOrderPostBill
                | Self::DocumentCreate
                | Self::DocumentAttach
                | Self::AttachmentUpload
                | Self::KbArticleCreate
                | Self::SecretRecord
                | Self::ProjectCreate
                | Self::ProjectTaskCreate
        )
    }
}

/// One node type declaration — the atom everything derives from.
///
/// Schemas are `&'static str` JSON Schema (the declared subset — see
/// [`validate_against_schema`]) so the whole registry is a `const` table
/// with no allocation and no second representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct NodeType {
    pub id: &'static str,
    pub label: &'static str,
    pub agent_description: &'static str,
    pub family: NodeFamily,
    pub tier: NodeTier,
    pub idempotent: bool,
    pub audience: NodeAudience,
    pub executor: ExecutorPath,
    /// JSON Schema (declared subset) the node's params validate against.
    pub input_schema: &'static str,
    /// JSON Schema (declared subset) of the node's output, so downstream
    /// nodes can reference returned fields.
    pub output_schema: &'static str,
    /// Event types this node can produce — the seam lane 15's static loop
    /// refusal reads. Empty for reads, conditions and values.
    pub emits: &'static [&'static str],
}

impl NodeType {
    /// The declaration entry point. Schemas default to "any object"; the
    /// chainable setters below fill the rest, so a declaration reads as
    /// one statement.
    pub const fn new(
        id: &'static str,
        label: &'static str,
        agent_description: &'static str,
        family: NodeFamily,
        tier: NodeTier,
        executor: ExecutorPath,
    ) -> Self {
        Self {
            id,
            label,
            agent_description,
            family,
            tier,
            idempotent: true,
            audience: NodeAudience::Both,
            executor,
            input_schema: r#"{"type":"object"}"#,
            output_schema: r#"{"type":"object"}"#,
            emits: &[],
        }
    }

    pub const fn idempotent(mut self, idempotent: bool) -> Self {
        self.idempotent = idempotent;
        self
    }

    pub const fn audience(mut self, audience: NodeAudience) -> Self {
        self.audience = audience;
        self
    }

    pub const fn input(mut self, schema: &'static str) -> Self {
        self.input_schema = schema;
        self
    }

    pub const fn output(mut self, schema: &'static str) -> Self {
        self.output_schema = schema;
        self
    }

    pub const fn emits(mut self, emits: &'static [&'static str]) -> Self {
        self.emits = emits;
        self
    }
}

/// The declaration trait (B). Anything that can name a node type
/// implements it — the catalogue's const table does, and the plugin
/// contract adapter (`PluginNodeContract`) does for Tier C, so a
/// plugin-contributed entry plugs into the same interface the first-party
/// nodes use instead of a parallel path.
pub trait NodeDeclaration {
    fn declaration(&self) -> Cow<'_, NodeType>;
}

/// A first-party declaration IS its own node type; no adapter needed.
impl NodeDeclaration for NodeType {
    fn declaration(&self) -> Cow<'_, NodeType> {
        Cow::Borrowed(self)
    }
}

/// Registry errors. Typed so lane 14's retry decision and lane 15's
/// static refusal can match instead of string-parsing.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NodeRegistryError {
    #[error("unknown node type {id:?}")]
    UnknownNode { id: String },
    #[error("node {id:?} params violate the declared input schema: {detail}")]
    SchemaViolation { id: String, detail: String },
    #[error("flow.retry must refuse to wrap {id:?}: the node declares idempotent=false and repeating it is not safe")]
    RetryRefused { id: String },
    #[error("static loop refusal: node {waiting:?} waits for {event:?}, which node {producing:?} in the same workflow can produce")]
    LoopRefusal {
        waiting: String,
        event: String,
        producing: String,
    },
    #[error("downstream reference refused: node {id:?} declares no output schema, so nothing it returns can be referenced")]
    NoOutputSchema { id: String },
}

impl From<NodeRegistryError> for DomainError {
    fn from(e: NodeRegistryError) -> Self {
        DomainError::Validation(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// The registry API — everything downstream lanes consume
// ---------------------------------------------------------------------------

/// Every declared node.
pub fn all() -> &'static [NodeType] {
    catalogue::all()
}

/// The catalogue release. Bump it whenever a node is added, renamed or
/// removed, or a declaration's schemas, idempotence or emit list change:
/// a workflow document validated against one release must be able to say
/// which release that was, and two builds claiming the same version must
/// not be able to differ silently — [`catalogue_digest`] is what makes
/// that detectable.
pub const CATALOGUE_VERSION: &str = "1.0.0";

/// A digest over every declaration, in declaration order. Covers every
/// field that changes how a document validates or executes: id, label,
/// agent description, family, tier, idempotence, audience, executor path,
/// both schemas and the emit list. Computed rather than hand-maintained,
/// so it cannot go stale the way a comment can.
pub fn catalogue_digest() -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(CATALOGUE_VERSION.as_bytes());
    hasher.update(b"\n");
    for node in all() {
        for part in [
            node.id,
            node.label,
            node.agent_description,
            node.family.as_str(),
            node.tier.as_str(),
            node.executor.as_str(),
            node.audience.as_str(),
            node.input_schema,
            node.output_schema,
        ] {
            hasher.update(part.as_bytes());
            hasher.update(b"\x1f");
        }
        hasher.update(if node.idempotent { b"y" } else { b"n" });
        for event in node.emits {
            hasher.update(event.as_bytes());
            hasher.update(b"\x1e");
        }
        hasher.update(b"\x1d");
    }
    hex::encode(hasher.finalize())
}

/// One node by id.
pub fn get(id: &str) -> Option<&'static NodeType> {
    all().iter().find(|n| n.id == id)
}

/// `get` as a Result for call sites that refuse an unknown id.
pub fn require(id: &str) -> Result<&'static NodeType, NodeRegistryError> {
    get(id).ok_or_else(|| NodeRegistryError::UnknownNode { id: id.to_string() })
}

/// The canvas palette: Tier A first, then B, then C, human-visible nodes
/// only, in declaration order inside a tier.
pub fn palette() -> Vec<&'static NodeType> {
    let mut nodes: Vec<&'static NodeType> = all()
        .iter()
        .filter(|n| n.audience.includes_human())
        .collect();
    nodes.sort_by_key(|n| n.tier);
    nodes
}

/// The LLM tool catalogue: every node the authoring model may see, with
/// its agent-oriented description and idempotent flag (ADR 0016 decision
/// 8 — Activepieces' `aiMetadata` pattern). Schemas are exposed ON DEMAND
/// through the API, never inlined into a prompt.
pub fn agent_catalogue() -> Vec<&'static NodeType> {
    all()
        .iter()
        .filter(|n| n.audience.includes_ai())
        .collect()
}

/// The Tier A nodes — the only tier the simple rule builder exposes.
pub fn tier_a() -> Vec<&'static NodeType> {
    all().iter().filter(|n| n.tier == NodeTier::A).collect()
}

/// The event types a node can produce, resolved WITH its params: the
/// static declarations for every fixed node, and for `domain.update` the
/// selected operation's event (Tier B rides the same emit seam as Tier A
/// — that is what makes the loop refusal complete rather than approximate).
pub fn emitted_event_types(node_id: &str, params: &Value) -> Vec<&'static str> {
    let Some(node) = get(node_id) else {
        return Vec::new();
    };
    match node.executor {
        ExecutorPath::DomainUpdate => params
            .get("operation")
            .and_then(Value::as_str)
            .and_then(DomainOp::parse)
            .map(|op| match op.emitted_event() {
                "" => Vec::new(),
                event => vec![event],
            })
            .unwrap_or_default(),
        _ => node.emits.to_vec(),
    }
}

/// The retry refusal lane 14's operator retry and `flow.retry` both call:
/// a node declared `idempotent: false` must never be wrapped in an
/// in-graph retry, and the ledger's operator retry must know the same
/// fact. For `domain.update` the flag resolves per operation, because a
/// generic update is as safe to repeat as the operation it selected.
pub fn retry_refusal(node_id: &str, params: &Value) -> Result<(), NodeRegistryError> {
    let node = require(node_id)?;
    let idempotent = match node.executor {
        ExecutorPath::DomainUpdate => params
            .get("operation")
            .and_then(Value::as_str)
            .and_then(DomainOp::parse)
            .map(|op| op.idempotent())
            .unwrap_or(false),
        _ => node.idempotent,
    };
    if idempotent {
        Ok(())
    } else {
        Err(NodeRegistryError::RetryRefused {
            id: node_id.to_string(),
        })
    }
}

/// One node of the workflow being validated. The registry resolves the
/// emits of every first-party node itself; `plugin_emits` carries the
/// declared `emits[]` of a plugin-contributed node, which only the caller
/// can know because it reads them off the install at author time. That
/// field is what makes the loop refusal complete rather than approximate:
/// without it, a plugin that emits `ticket.status_changed` could deadlock a
/// workflow that then waits for exactly that event.
#[derive(Debug, Clone, Copy)]
pub struct LoopCheckNode<'a> {
    pub id: &'a str,
    pub params: &'a Value,
    pub plugin_emits: &'a [String],
}

impl<'a> LoopCheckNode<'a> {
    /// A first-party node: nothing to supply, the registry resolves it.
    pub fn first_party(id: &'a str, params: &'a Value) -> Self {
        Self {
            id,
            params,
            plugin_emits: &[],
        }
    }
}

/// The static loop refusal lane 15's author-time validation calls.
///
/// A `flow.waitFor*` whose matched event type is producible by a node in
/// the SAME workflow can resume itself forever (`waitFor` → write → the
/// write matches the wait → resume). Each node declares the event types it
/// can produce, so the check is static and free — a validation error at
/// author time rather than a runtime surprise. `waiting_id` names the
/// waiting node for the error; `workflow_nodes` is the workflow's nodes.
pub fn check_loop_safety(
    waiting_id: &str,
    awaited_events: &[String],
    workflow_nodes: &[LoopCheckNode<'_>],
) -> Result<(), NodeRegistryError> {
    for awaited in awaited_events {
        for node in workflow_nodes {
            if node.id == waiting_id {
                continue;
            }
            let mut producible = emitted_event_types(node.id, node.params)
                .into_iter()
                .map(str::to_string)
                .chain(node.plugin_emits.iter().cloned());
            if producible.any(|produced| &produced == awaited) {
                return Err(NodeRegistryError::LoopRefusal {
                    waiting: waiting_id.to_string(),
                    event: awaited.clone(),
                    producing: node.id.to_string(),
                });
            }
        }
    }
    Ok(())
}

/// Validates a node's params against its declared input schema. This is
/// the boundary ADR 0016 rule (a) draws: params are validated JSON, never
/// expressions or scripts.
pub fn validate_params(node_id: &str, params: &Value) -> Result<(), NodeRegistryError> {
    let node = require(node_id)?;
    let schema = parse_schema(node.input_schema).map_err(|detail| {
        NodeRegistryError::SchemaViolation {
            id: node_id.to_string(),
            detail: format!("declared input schema is not valid JSON Schema: {detail}"),
        }
    })?;
    validate_against_schema(&schema, params).map_err(|violation| {
        NodeRegistryError::SchemaViolation {
            id: node_id.to_string(),
            detail: violation.to_string(),
        }
    })
}

/// The node's parsed output schema, for the properties panel and the
/// on-demand schema tool. `None` when the declaration is not parseable
/// (a registry-internal invariant — the catalogue tests refuse it).
pub fn output_schema(node_id: &str) -> Option<Value> {
    let node = get(node_id)?;
    parse_schema(node.output_schema).ok()
}

/// The node's parsed INPUT schema — what a params object must satisfy.
/// The properties panel renders from it and the authoring model reads it
/// on demand; the planner validates against it before a document saves.
pub fn input_schema(node_id: &str) -> Option<Value> {
    let node = get(node_id)?;
    parse_schema(node.input_schema).ok()
}

/// True when the node declares an output schema a downstream node can
/// reference. `value.reference` and every condition gate call this first.
pub fn has_referenceable_output(node_id: &str) -> Result<bool, NodeRegistryError> {
    require(node_id)?;
    Ok(output_schema(node_id).is_some_and(|schema| schema_has_fields(&schema)))
}

fn schema_has_fields(schema: &Value) -> bool {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|props| !props.is_empty())
}

fn parse_schema(raw: &str) -> Result<Value, String> {
    serde_json::from_str(raw).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// The declared JSON-Schema subset
// ---------------------------------------------------------------------------

/// One violation of a declared schema: the JSON-pointer-ish path to the
/// offending instance and why it offends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaViolation {
    pub path: String,
    pub message: String,
}

impl fmt::Display for SchemaViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.path, self.message)
        }
    }
}

/// Validates an instance against a DECLARED-subset JSON Schema. The
/// subset is closed and deterministic — the whole point is that a model
/// hallucinating inside a params object gets a typed refusal, never a
/// best-effort parse:
///
/// * `type`: `object`, `array`, `string`, `integer`, `number`, `boolean`
/// * object: `properties`, `required`, `additionalProperties: false`
/// * array: `items` (single schema), `minItems`, `maxItems`
/// * string: `minLength`, `maxLength`, `enum`, `const`, `format: uuid`
/// * number/integer: `minimum`, `maximum`, `enum`, `const`
/// * any node: `enum`, `const`, `description` (ignored)
///
/// Unsupported keywords are refused RATHER THAN IGNORED at declaration
/// time by [`validate_declared_schema`] — a schema the validator cannot
/// fully honour must not ship as if it could.
pub fn validate_against_schema(
    schema: &Value,
    instance: &Value,
) -> Result<(), SchemaViolation> {
    validate_at(schema, instance, "")
}

/// Every keyword the declared subset can honour. A schema declaring
/// anything else is refused, because a keyword the validator silently
/// ignores is a contract that lies.
pub const SUPPORTED_SCHEMA_KEYWORDS: &[&str] = &[
    "type",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "minItems",
    "maxItems",
    "minLength",
    "maxLength",
    "enum",
    "const",
    "format",
    "minimum",
    "maximum",
    "description",
];

/// Checks a declaration's schema is inside the subset. Called on every
/// catalogue node (its tests) and on every plugin entry a manifest
/// declares — one implementation, so a keyword available to a first-party
/// node can never be quietly unavailable to a plugin one.
pub fn validate_declared_schema(schema: &Value) -> Result<(), SchemaViolation> {
    validate_declared_at(schema, "")
}

fn validate_declared_at(schema: &Value, path: &str) -> Result<(), SchemaViolation> {
    if schema.as_bool().is_some() {
        return Ok(());
    }
    let Some(spec) = schema.as_object() else {
        return Err(violation(path, "schema must be an object or boolean"));
    };
    for (key, value) in spec {
        if !SUPPORTED_SCHEMA_KEYWORDS.contains(&key.as_str()) {
            return Err(violation(
                path,
                &format!("keyword {key:?} is outside the declared subset"),
            ));
        }
        if let Some(declared) = value.as_str() {
            if !matches!(declared, "object" | "array" | "string" | "integer" | "number" | "boolean" | "null" | "uuid")
                && !matches!(key.as_str(), "format")
            {
                return Err(violation(
                    path,
                    &format!("keyword {key:?} declares {declared:?}, which the subset does not define"),
                ));
            }
        }
    }
    if let Some(props) = spec.get("properties").and_then(Value::as_object) {
        for (name, child) in props {
            let child_path = child_path(path, name);
            validate_declared_at(child, &child_path)?;
        }
    }
    if let Some(items) = spec.get("items") {
        validate_declared_at(items, &format!("{path}[]"))?;
    }
    Ok(())
}

fn validate_at(schema: &Value, instance: &Value, path: &str) -> Result<(), SchemaViolation> {
    // Boolean schemas: `true` admits anything, `false` admits nothing.
    if let Some(admit) = schema.as_bool() {
        return if admit {
            Ok(())
        } else {
            Err(violation(path, "no value is allowed here"))
        };
    }
    let Some(spec) = schema.as_object() else {
        return Err(violation(path, "schema must be an object or boolean"));
    };

    if let Some(expected) = spec.get("const") {
        if instance != expected {
            return Err(violation(path, &format!("must equal {expected}")));
        }
    }
    if let Some(allowed) = spec.get("enum").and_then(Value::as_array) {
        if !allowed.contains(instance) {
            return Err(violation(
                path,
                &format!("must be one of {}", render_enum(allowed)),
            ));
        }
    }

    let Some(declared_type) = spec.get("type").and_then(Value::as_str) else {
        // No type declared: only the const/enum gates above apply.
        return Ok(());
    };
    match declared_type {
        "object" => validate_object(spec, instance, path),
        "array" => validate_array(spec, instance, path),
        "string" => validate_string(spec, instance, path),
        "integer" => validate_number(spec, instance, path, true),
        "number" => validate_number(spec, instance, path, false),
        "boolean" => match instance.as_bool() {
            Some(_) => Ok(()),
            None => Err(violation(path, "must be a boolean")),
        },
        "null" => match instance.is_null() {
            true => Ok(()),
            false => Err(violation(path, "must be null")),
        },
        other => Err(violation(
            path,
            &format!("schema declares unsupported type {other:?}"),
        )),
    }
}

fn validate_object(
    spec: &serde_json::Map<String, Value>,
    instance: &Value,
    path: &str,
) -> Result<(), SchemaViolation> {
    let Some(map) = instance.as_object() else {
        return Err(violation(path, "must be an object"));
    };
    let props = spec
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required: Vec<&str> = spec
        .get("required")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    for key in &required {
        if !map.contains_key(*key) {
            return Err(violation(path, &format!("missing required key {key:?}")));
        }
    }
    if spec.get("additionalProperties").and_then(Value::as_bool) == Some(false) {
        for key in map.keys() {
            if !props.contains_key(key) {
                return Err(violation(
                    path,
                    &format!("unknown key {key:?} — the declared schema is the contract"),
                ));
            }
        }
    }
    for (key, value) in map {
        if let Some(prop_schema) = props.get(key) {
            let child = child_path(path, key);
            validate_at(prop_schema, value, &child)?;
        }
    }
    Ok(())
}

fn validate_array(
    spec: &serde_json::Map<String, Value>,
    instance: &Value,
    path: &str,
) -> Result<(), SchemaViolation> {
    let Some(items) = instance.as_array() else {
        return Err(violation(path, "must be an array"));
    };
    if let Some(min) = spec.get("minItems").and_then(Value::as_u64) {
        if (items.len() as u64) < min {
            return Err(violation(path, &format!("needs at least {min} items")));
        }
    }
    if let Some(max) = spec.get("maxItems").and_then(Value::as_u64) {
        if (items.len() as u64) > max {
            return Err(violation(path, &format!("allows at most {max} items")));
        }
    }
    if let Some(item_schema) = spec.get("items") {
        for (index, item) in items.iter().enumerate() {
            validate_at(item_schema, item, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

fn validate_string(
    spec: &serde_json::Map<String, Value>,
    instance: &Value,
    path: &str,
) -> Result<(), SchemaViolation> {
    let Some(text) = instance.as_str() else {
        return Err(violation(path, "must be a string"));
    };
    if let Some(min) = spec.get("minLength").and_then(Value::as_u64) {
        if (text.chars().count() as u64) < min {
            return Err(violation(path, &format!("must be at least {min} characters")));
        }
    }
    if let Some(max) = spec.get("maxLength").and_then(Value::as_u64) {
        if (text.chars().count() as u64) > max {
            return Err(violation(path, &format!("must be at most {max} characters")));
        }
    }
    if spec.get("format").and_then(Value::as_str) == Some("uuid")
        && uuid::Uuid::parse_str(text).is_err()
    {
        return Err(violation(path, "must be a uuid"));
    }
    Ok(())
}

fn validate_number(
    spec: &serde_json::Map<String, Value>,
    instance: &Value,
    path: &str,
    integer_only: bool,
) -> Result<(), SchemaViolation> {
    let Some(number) = instance.as_f64() else {
        return Err(violation(
            path,
            if integer_only {
                "must be an integer"
            } else {
                "must be a number"
            },
        ));
    };
    if integer_only && instance.as_i64().is_none() && instance.as_u64().is_none() {
        return Err(violation(path, "must be an integer"));
    }
    if let Some(min) = spec.get("minimum").and_then(Value::as_f64) {
        if number < min {
            return Err(violation(path, &format!("must be >= {min}")));
        }
    }
    if let Some(max) = spec.get("maximum").and_then(Value::as_f64) {
        if number > max {
            return Err(violation(path, &format!("must be <= {max}")));
        }
    }
    Ok(())
}

fn child_path(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

fn violation(path: &str, message: &str) -> SchemaViolation {
    SchemaViolation {
        path: path.to_string(),
        message: message.to_string(),
    }
}

fn render_enum(allowed: &[Value]) -> String {
    allowed
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

// ---------------------------------------------------------------------------
// The plugin node contract (A2): six fields, not one
// ---------------------------------------------------------------------------

/// The `plugin.<slug>.<id>` registry key rule engine documents already
/// carry for plugin actions.
pub fn plugin_node_id(slug: &str, local_id: &str) -> String {
    format!("plugin.{slug}.{local_id}")
}

/// The six-field plugin node contract, normalized into the registry's
/// own vocabulary. Parsed from a `plugin_surface_entries` row (migration
/// 0128 stores the five new fields as columns; `params_schema` keeps
/// riding inside `definition` where it has always lived).
///
/// Defaults, and why they point the conservative way:
///   * `idempotent` defaults to FALSE — an undeclared plugin action's
///     retry-safety is unknown, so `flow.retry` refuses it and the
///     operator retry treats it as double-apply-unsafe until the plugin
///     says otherwise. A plugin that wants retries declares them.
///   * `audience` defaults to HUMAN — an undeclared entry stays off the
///     LLM tool catalogue, so a silent plugin cannot flood the model's
///     view; consent already shows it to the human.
///   * `emits` defaults to empty — an undeclared entry can never be
///     statically linked to a wait, which is the safe side of the loop
///     refusal.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginNodeContract {
    /// `plugin.<slug>.<id>` — the entry key IS the node id.
    pub node_id: String,
    /// Derived from the entry's surface; the UI kinds are refused.
    pub kind: PluginNodeKind,
    pub label: String,
    pub input_schema: Option<Value>,
    pub output_schema: Option<Value>,
    pub idempotent: bool,
    pub emits: Vec<String>,
    pub audience: NodeAudience,
    pub agent_description: Option<String>,
}

impl PluginNodeContract {
    /// Parses one surface entry. `surface` is the row's surface kind;
    /// `definition` is its JSONB (carrying `params_schema` and the read
    /// guardrails); the five new fields arrive as the row's columns
    /// (migration 0128).
    #[allow(clippy::too_many_arguments)]
    pub fn parse(
        surface: &str,
        entry_key: &str,
        label: &str,
        definition: &Value,
        output_schema: Option<Value>,
        idempotent: bool,
        emits: &[String],
        audience: &str,
        agent_description: Option<&str>,
    ) -> Result<Self, NodeRegistryError> {
        let kind = PluginNodeKind::from_surface(surface).ok_or_else(|| {
            NodeRegistryError::SchemaViolation {
                id: entry_key.to_string(),
                detail: format!("surface {surface:?} is not a workflow node kind"),
            }
        })?;
        let input_schema = definition
            .get("params_schema")
            .filter(|s| !s.is_null())
            .cloned();
        let audience = NodeAudience::parse(audience).ok_or_else(|| {
            NodeRegistryError::SchemaViolation {
                id: entry_key.to_string(),
                detail: format!("audience {audience:?} is not human/ai/both"),
            }
        })?;
        Ok(Self {
            node_id: entry_key.to_string(),
            kind,
            label: label.to_string(),
            input_schema,
            output_schema,
            idempotent,
            emits: emits.to_vec(),
            audience,
            agent_description: agent_description.map(str::to_string),
        })
    }

    /// Validates a plugin node's params against the entry's declared
    /// `params_schema` (the same boundary the rule engine's
    /// `PluginAction` already enforces at execution time).
    pub fn validate_params(&self, params: &Value) -> Result<(), NodeRegistryError> {
        let Some(schema) = &self.input_schema else {
            return Ok(());
        };
        validate_against_schema(schema, params).map_err(|violation| {
            NodeRegistryError::SchemaViolation {
                id: self.node_id.clone(),
                detail: violation.to_string(),
            }
        })
    }

    /// The registry-shaped view of this entry: a `NodeType` with static
    /// lifetime. Tier C entries are few (a manifest declares at most 8
    /// per section) and parsed once per catalogue read, so the leak is
    /// bounded by the number of DISTINCT plugin entries the process has
    /// served — and it buys the one interface the palette, the picker
    /// and the LLM tool catalogue all read.
    pub fn as_node_type(&self) -> NodeType {
        let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
        let emits: &'static [&'static str] = Box::leak(
            self.emits
                .iter()
                .map(|e| leak(e.clone()))
                .collect::<Vec<&'static str>>()
                .into_boxed_slice(),
        );
        NodeType {
            id: leak(self.node_id.clone()),
            label: leak(self.label.clone()),
            agent_description: leak(
                self.agent_description
                    .clone()
                    .unwrap_or_else(|| self.label.clone()),
            ),
            family: NodeFamily::Action,
            tier: NodeTier::C,
            idempotent: self.idempotent,
            audience: self.audience,
            executor: ExecutorPath::Plugin(self.kind),
            input_schema: leak(
                self.input_schema
                    .as_ref()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| r#"{"type":"object"}"#.to_string()),
            ),
            output_schema: leak(
                self.output_schema
                    .as_ref()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| r#"{"type":"object"}"#.to_string()),
            ),
            emits,
        }
    }
}

impl NodeDeclaration for PluginNodeContract {
    fn declaration(&self) -> Cow<'_, NodeType> {
        Cow::Owned(self.as_node_type())
    }
}

// ---------------------------------------------------------------------------
// The AutomationAction drift guard (B)
// ---------------------------------------------------------------------------

/// Every `AutomationAction` variant, mapped onto the registry node that
/// executes it. The rule builder's five effects are Tier A nodes in the
/// catalogue; the plugin effect is the Tier C `plugin.action` node. A new
/// variant added to `AutomationAction` without an entry here fails
/// `automation_action_variants_cannot_drift_from_the_registry` — that is
/// the mechanism that keeps the two from drifting.
pub const AUTOMATION_ACTION_NODES: [(&str, &str); 5] = [
    ("assign_user", "ticket.assign"),
    ("set_priority", "ticket.setPriority"),
    ("add_internal_comment", "ticket.internalNote"),
    ("notify_users", "notification.send"),
    ("plugin_action", "plugin.action"),
];

/// The node id the given `AutomationAction::as_str()` tag executes
/// through, when it has one.
pub fn automation_action_node(tag: &str) -> Option<&'static str> {
    AUTOMATION_ACTION_NODES
        .iter()
        .find(|(t, _)| *t == tag)
        .map(|(_, node)| *node)
}

#[cfg(test)]
mod tests;
