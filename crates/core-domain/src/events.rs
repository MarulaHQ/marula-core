//! Domain event payloads. Every mutation emits one of these through the
//! transactional outbox (§4.2); the envelope in `core-events` wraps them.
//! Payloads are additive-evolving: never remove or retype a field.

use crate::entities::{ChecklistItem, TicketComment, TimeEntry};
use crate::ids::*;
use crate::status::{NoteKind, Priority, TicketStatusCategory};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Event type strings on the wire (`ticket.created`, `timer.started`, …).
pub mod types {
    pub const ORG_CREATED: &str = "org.created";
    pub const USER_CREATED: &str = "user.created";
    /// A staff user was invited into an org (inactive until first
    /// password set via the one-time setup token).
    pub const USER_INVITED: &str = "user.invited";
    /// A staff user was re-activated by an org manager.
    pub const USER_ACTIVATED: &str = "user.activated";
    /// A staff user was deactivated: live sessions revoked, login blocked.
    pub const USER_DEACTIVATED: &str = "user.deactivated";
    /// A staff user's builtin org role changed.
    pub const USER_ROLE_CHANGED: &str = "user.role_changed";
    /// A shipped preset role was forked into a custom copy (RB1, ADR
    /// 0004): the preset row is untouched, the org now owns a mutable
    /// duplicate. `from_builtin` names the preset the copy came from.
    pub const ROLE_FORKED: &str = "role.forked";
    /// A shipped preset role row was restored to its canonical permission
    /// set (RB1, ADR 0004) — the presets stay canonical, orgs fork.
    pub const ROLE_RESTORED: &str = "role.restored";
    /// A template was created (org-created, or a fork of a shipped
    /// default — KB3/KB4, ADR 0004 template registry).
    pub const TEMPLATE_CREATED: &str = "template.created";
    /// A template's content changed (edit of an org-owned row, or restore
    /// of a forked preset row to its canonical copy).
    pub const TEMPLATE_UPDATED: &str = "template.updated";
    /// An org-owned template row was removed (shipped defaults are
    /// immutable, never removable).
    pub const TEMPLATE_REMOVED: &str = "template.removed";
    pub const CLIENT_CREATED: &str = "client.created";
    pub const CLIENT_UPDATED: &str = "client.updated";
    /// A client workflow override was set or explicitly disabled (Wave 12
    /// lane 5). `state: "set"|"disabled"` distinguishes an override value
    /// from an explicit deny; absent rows inherit the org default and emit
    /// nothing.
    pub const CLIENT_WORKFLOW_OVERRIDE_SET: &str = "client.workflow_override_set";
    /// A client workflow override row was cleared: the client inherits the
    /// org default again (this is NOT the same as an explicit disable).
    pub const CLIENT_WORKFLOW_OVERRIDE_CLEARED: &str = "client.workflow_override_cleared";
    pub const SITE_CREATED: &str = "site.created";
    /// A site row was updated in place (import framework, TASKLIST D2).
    pub const SITE_UPDATED: &str = "site.updated";
    pub const CONTACT_CREATED: &str = "contact.created";
    /// A contact row was updated in place (import framework, TASKLIST D2).
    pub const CONTACT_UPDATED: &str = "contact.updated";
    /// Import batch lifecycle (TASKLIST D2): a run opened / closed so
    /// webhook consumers can bracket bulk-change windows. Row-level
    /// mutations inside the batch keep emitting their ordinary events.
    pub const IMPORT_BATCH_STARTED: &str = "import.batch_started";
    pub const IMPORT_BATCH_FINISHED: &str = "import.batch_finished";
    /// Migration connector lifecycle (TASKLIST D3): a third-party bulk-push
    /// session opened / closed. Parallel to the import batch events — the
    /// connector session pairs 1:1 with a D2 batch, but webhook consumers
    /// see the connector-level bracket without parsing engine internals.
    pub const MIGRATION_PUSH_SESSION_STARTED: &str = "migration.push_session_started";
    pub const MIGRATION_PUSH_SESSION_FINISHED: &str = "migration.push_session_finished";
    pub const BOARD_CREATED: &str = "board.created";
    /// Board content update (import framework, TASKLIST D2 remainder).
    pub const BOARD_UPDATED: &str = "board.updated";
    pub const BOARD_STATUS_ADDED: &str = "board.status_added";
    pub const BOARD_STATUS_UPDATED: &str = "board.status_updated";
    pub const BOARD_STATUS_REMOVED: &str = "board.status_removed";
    pub const BOARD_TRANSITIONS_UPDATED: &str = "board.transitions_updated";
    /// Emitted when a board's pre-close checklist template or enforcement
    /// toggle changes (TASKLIST 2.8).
    pub const BOARD_CHECKLIST_TEMPLATE_UPDATED: &str = "board.checklist_template_updated";
    pub const TICKET_CREATED: &str = "ticket.created";
    pub const TICKET_UPDATED: &str = "ticket.updated";
    pub const TICKET_STATUS_CHANGED: &str = "ticket.status_changed";
    /// A ticket was moved to a different board (Wave 11 lane 3): the
    /// status was remapped through the SEMANTIC categories, and the SLA
    /// state was either preserved verbatim (`sla_reset: false`) or
    /// explicitly re-initialized (`sla_reset: true`).
    pub const TICKET_BOARD_MOVED: &str = "ticket.board_moved";
    pub const TICKET_ASSIGNED: &str = "ticket.assigned";
    pub const TICKET_COMMENT_ADDED: &str = "ticket.comment_added";
    pub const TICKET_CHECKLIST_UPDATED: &str = "ticket.checklist_updated";
    pub const TICKET_CLOSED: &str = "ticket.closed";
    /// Undo (IF7): a revert executed as a normal authorized idempotent
    /// write appended a NEW trail row restoring prior values. The
    /// append-only trail is never rewritten — this event IS the audited
    /// revert.
    pub const TICKET_REVERTED: &str = "ticket.reverted";
    pub const TIMER_STARTED: &str = "timer.started";
    pub const TIMER_PAUSED: &str = "timer.paused";
    pub const TIMER_RESUMED: &str = "timer.resumed";
    pub const TIMER_SWITCHED: &str = "timer.switched";
    pub const TIMER_STOPPED: &str = "timer.stopped";
    pub const TIME_ENTRY_CREATED: &str = "time_entry.created";
    pub const TIME_ENTRY_UPDATED: &str = "time_entry.updated";
    /// Wave 14 lane 1 approval workflow: one event per lifecycle step,
    /// emitted from the same transaction as the state change.
    pub const TIME_ENTRY_APPROVAL_SUBMITTED: &str = "time_entry.approval_submitted";
    pub const TIME_ENTRY_APPROVAL_APPROVED: &str = "time_entry.approval_approved";
    pub const TIME_ENTRY_APPROVAL_REJECTED: &str = "time_entry.approval_rejected";
    pub const NOTIFICATION_CREATED: &str = "notification.created";
    /// Emitted when a user marks notifications read (single, bulk, or all).
    pub const NOTIFICATION_READ: &str = "notification.read";
    /// Emitted when a user overrides one preference row.
    pub const NOTIFICATION_PREFS_UPDATED: &str = "notification_prefs.updated";
    pub const EMAIL_MESSAGE_RECORDED: &str = "email.message_recorded";
    /// A ladder step fired for a ticket clock (risk/breach/escalate).
    pub const SLA_ESCALATION_FIRED: &str = "sla.escalation_fired";
    /// An automation rule matched and its effects were applied.
    pub const AUTOMATION_TRIGGERED: &str = "automation.triggered";
    /// SLA configuration surfaces mutated (calendars/policies/ladders).
    pub const SLA_CALENDAR_UPDATED: &str = "sla.calendar_updated";
    pub const SLA_POLICY_UPDATED: &str = "sla.policy_updated";
    /// An automation rule definition was created/changed/disabled.
    pub const AUTOMATION_RULE_UPDATED: &str = "automation.rule_updated";
    /// Report definitions as data (TASKLIST R3): a stored report spec was
    /// created / changed / removed through the admin-gated CRUD surface.
    pub const REPORT_DEFINITION_CREATED: &str = "report_definition.created";
    pub const REPORT_DEFINITION_UPDATED: &str = "report_definition.updated";
    pub const REPORT_DEFINITION_DELETED: &str = "report_definition.deleted";
    /// Scheduled report delivery (TASKLIST R4): a delivery schedule was
    /// created / changed / removed. The worker's run fan-out emits NO
    /// events — the `report_deliveries` ledger is the run record.
    pub const REPORT_SCHEDULE_CREATED: &str = "report_schedule.created";
    pub const REPORT_SCHEDULE_UPDATED: &str = "report_schedule.updated";
    pub const REPORT_SCHEDULE_DELETED: &str = "report_schedule.deleted";
    /// Agent platform v1 (TASKLIST Phase 3, IF8; docs/10-agent-platform
    /// §4/§5): the proposal pipeline. `agent_proposal.created` fires when
    /// an agent-authored change is filed for approval; approved (applied
    /// through the IF1 write path in the same transaction), declined (no
    /// write at all), and withdrawn (agent cancelled) close it out.
    pub const AGENT_PROPOSAL_CREATED: &str = "agent_proposal.created";
    pub const AGENT_PROPOSAL_APPROVED: &str = "agent_proposal.approved";
    pub const AGENT_PROPOSAL_DECLINED: &str = "agent_proposal.declined";
    pub const AGENT_PROPOSAL_WITHDRAWN: &str = "agent_proposal.withdrawn";
    /// Versioned agent-skill manifests (§4): first publication
    /// (`agent_skill.created`) and every appended version thereafter,
    /// rollbacks included (`agent_skill.updated` — history is append-only).
    pub const AGENT_SKILL_CREATED: &str = "agent_skill.created";
    pub const AGENT_SKILL_UPDATED: &str = "agent_skill.updated";
    /// Capability-registry changes (deny-by-default MCP/agent surface):
    /// a capability was granted/re-dialed or revoked for the org.
    pub const MCP_CAPABILITY_UPDATED: &str = "mcp_capability.updated";
    /// `calendar.sync_completed` — see the payload struct below.
    /// Plugin runtime v2 + marketplace registry (Wave 14 lane 5): the
    /// registry lifecycle. `plugin.catalog_entry_created` fires when a
    /// catalog entry (declarative manifest snapshot — data, never code)
    /// is published; `plugin.installed` when an org adopts an entry at a
    /// pinned version (created DISABLED — deny-by-default);
    /// `plugin.install_updated` on enable/disable/upgrade/grant change;
    /// `plugin.install_revoked` when registry revocation or an explicit
    /// revoke kills the install.
    pub const PLUGIN_CATALOG_ENTRY_CREATED: &str = "plugin.catalog_entry_created";
    pub const PLUGIN_INSTALLED: &str = "plugin.installed";
    pub const PLUGIN_INSTALL_UPDATED: &str = "plugin.install_updated";
    pub const PLUGIN_INSTALL_REVOKED: &str = "plugin.install_revoked";
    /// ADR 0009 distribution lifecycle (Wave 16 lane 7):
    /// `plugin.staged_import_created` — a member upload landed staged
    /// (pending, validated, un-executed, uploader+admin visible);
    /// `plugin.staged_import_approved` — owner/admin consent diff passed
    /// and the adoption/skill was minted; `plugin.staged_import_refused`
    /// — the reviewer declined (terminal, with note);
    /// `plugin.update_pending` — an update proposal pinned the OLD
    /// version while awaiting the re-consent diff;
    /// `plugin.settings_updated` — per-install config (rendered FROM the
    /// manifest's settings JSON-Schema) changed. Every chain touch also
    /// appends to the provenance read surface via audit.
    pub const PLUGIN_STAGED_IMPORT_CREATED: &str = "plugin.staged_import_created";
    pub const PLUGIN_STAGED_IMPORT_APPROVED: &str = "plugin.staged_import_approved";
    pub const PLUGIN_STAGED_IMPORT_REFUSED: &str = "plugin.staged_import_refused";
    pub const PLUGIN_UPDATE_PENDING: &str = "plugin.update_pending";
    pub const PLUGIN_SETTINGS_UPDATED: &str = "plugin.settings_updated";
    /// Emitted when an org- or user-scoped setting changes through the
    /// typed settings registry.
    pub const SETTING_UPDATED: &str = "setting.updated";
    /// BYOK sealed-key lifecycle (Wave 9 adapters lane): an API/OAuth key
    /// was sealed for a provider (`ai.byok_updated`) or removed
    /// (`ai.byok_deleted`). Payloads carry the provider id and presence
    /// flags only — secret bytes never leave the sealed store.
    pub const AI_BYOK_UPDATED: &str = "ai.byok_updated";
    pub const AI_BYOK_DELETED: &str = "ai.byok_deleted";
    /// ADR 0008 availability matrix (Wave 18 lane 1): admin mutations of
    /// the AI policy surface. `ai.surface_grant_updated` /
    /// `ai.surface_grant_revoked` record per-surface per-role/user grants
    /// (deny-by-default posture, every grant audited);
    /// `ai.model_catalog_updated` records a curated (provider, model)
    /// entry upsert/disable; `ai.model_credential_updated` /
    /// `ai.model_credential_revoked` record the sealed ModelCredential
    /// slot lifecycle (presence-only payloads — secret bytes never leave
    /// the sealed store); `ai.cap_updated` records org-wide / per-user
    /// soft+hard token caps; `ai.model_pick_updated` records a sticky
    /// per-surface model pick (user or org default scope).
    pub const AI_SURFACE_GRANT_UPDATED: &str = "ai.surface_grant_updated";
    pub const AI_SURFACE_GRANT_REVOKED: &str = "ai.surface_grant_revoked";
    pub const AI_MODEL_CATALOG_UPDATED: &str = "ai.model_catalog_updated";
    pub const AI_MODEL_CREDENTIAL_UPDATED: &str = "ai.model_credential_updated";
    pub const AI_MODEL_CREDENTIAL_REVOKED: &str = "ai.model_credential_revoked";
    pub const AI_CAP_UPDATED: &str = "ai.cap_updated";
    pub const AI_MODEL_PICK_UPDATED: &str = "ai.model_pick_updated";
    /// Emitted when the org profile (name/ticket prefix) is edited from
    /// General settings or the first-run wizard.
    pub const ORG_UPDATED: &str = "org.updated";
    /// Org-scoped ticket-workspace pod-visibility matrix changed (Wave 12
    /// lane 6): the admin per-RBAC matrix moved from device-local panel
    /// state to the org store (`org_pod_visibility`, migration 0064).
    pub const POD_VISIBILITY_UPDATED: &str = "org.pod_visibility_updated";
    /// Board organization + board-level RBAC (TASKLIST 2.11).
    pub const BOARD_GROUP_CREATED: &str = "board.group_created";
    pub const BOARD_GROUP_UPDATED: &str = "board.group_updated";
    pub const BOARD_GROUP_REMOVED: &str = "board.group_removed";
    pub const BOARD_MEMBERSHIP_UPDATED: &str = "board.membership_updated";
    pub const BOARD_MEMBERSHIP_REMOVED: &str = "board.membership_removed";
    pub const BOARD_FAVORITE_TOGGLED: &str = "board.favorite_toggled";
    /// Portal v1 lifecycle: admin provisioned / changed / removed a client
    /// portal identity, or toggled a board×client exposure. Portal ticket
    /// replies intentionally reuse `ticket.comment_added` so every staff
    /// notification path keeps working unchanged.
    pub const PORTAL_USER_CREATED: &str = "portal_user.created";
    pub const PORTAL_USER_UPDATED: &str = "portal_user.updated";
    pub const PORTAL_USER_DELETED: &str = "portal_user.deleted";
    pub const BOARD_PORTAL_EXPOSURE_UPDATED: &str = "board_portal_exposure.updated";
    /// A subscription test-fire from the webhook manager; flows through
    /// the same stream + delivery pipeline as real domain events.
    pub const WEBHOOK_TEST: &str = "webhook.test";
    /// Scheduling/calendars/availability foundation (TASKLIST 2.12).
    pub const APPOINTMENT_CREATED: &str = "appointment.created";
    pub const APPOINTMENT_UPDATED: &str = "appointment.updated";
    pub const APPOINTMENT_CANCELLED: &str = "appointment.cancelled";
    /// Portal v2 appointment requests (TASKLIST Phase 3): a client asked
    /// for a slot, staff decided, or the client withdrew the request.
    /// Approval additionally emits the standard `appointment.created` —
    /// the booking enters the world through the normal creation path.
    pub const APPOINTMENT_REQUEST_CREATED: &str = "appointment_request.created";
    pub const APPOINTMENT_REQUEST_APPROVED: &str = "appointment_request.approved";
    pub const APPOINTMENT_REQUEST_DECLINED: &str = "appointment_request.declined";
    pub const APPOINTMENT_REQUEST_CANCELLED: &str = "appointment_request.cancelled";
    /// Alert ingestor (TASKLIST Phase 3): one normalized alert was
    /// accepted through the HMAC ingest surface and correlated. Emitted
    /// for `incident_created` and `collapsed` outcomes only — a replayed
    /// `duplicate` dedup_key writes nothing and emits nothing.
    pub const ALERT_INGESTED: &str = "alert.ingested";
    /// Alert triage (Wave 17 lane 13, migration 0089): a staff member
    /// acknowledged or resolved a stored alert through the /alerts
    /// surface. Forward-only through the API — the row carries the
    /// actor + timestamp it reported.
    pub const ALERT_ACKNOWLEDGED: &str = "alert.acknowledged";
    pub const ALERT_RESOLVED: &str = "alert.resolved";
    /// Lightweight change markers — consumers refetch the user's windows.
    pub const AVAILABILITY_UPDATED: &str = "availability.updated";
    pub const ON_CALL_UPDATED: &str = "on_call.updated";
    pub const CALENDAR_EXCLUSION_UPDATED: &str = "calendar_exclusion.updated";
    /// Calendly-style scheduling links (Wave 11 lane 4). A link was
    /// created (raw token NEVER appears — only its hash is stored),
    /// first viewed by the client, booked through (co-emits the
    /// standard `appointment.created`), or revoked by staff.
    pub const SCHEDULING_LINK_CREATED: &str = "scheduling_link.created";
    pub const SCHEDULING_LINK_VIEWED: &str = "scheduling_link.viewed";
    pub const SCHEDULING_LINK_SCHEDULED: &str = "scheduling_link.scheduled";
    pub const SCHEDULING_LINK_REVOKED: &str = "scheduling_link.revoked";
    /// Scheduling templates (Wave 17 lane 13): a meeting type was hard
    /// deleted by an admin. DELETE refuses when a scheduling link still
    /// references the row (typed conflict — the FK RESTRICT surfaces).
    pub const MEETING_TYPE_DELETED: &str = "meeting_type.deleted";
    /// Two-way calendar sync (Wave 14 lane 2). Connection lifecycle (a
    /// connection was created/updated/disabled or a poll failed →
    /// `status: "error"`), and the outcome of one inbound sync run
    /// (import counters + the overlaps the conflict rule reports). The
    /// raw outbound feed token NEVER appears in any event — only its
    /// SHA-256 hash is stored.
    pub const CALENDAR_SYNC_CONNECTION_UPDATED: &str = "calendar.sync_connection_updated";
    pub const CALENDAR_SYNC_COMPLETED: &str = "calendar.sync_completed";
    /// Coaching scorecards (Wave 14 lane 3): a staff member changed their
    /// scorecard settings (currently the opt-in gamification toggle).
    /// Scorecard METRICS are read-only aggregation — this is the only
    /// scorecard mutation that emits.
    pub const SCORECARD_SETTINGS_UPDATED: &str = "scorecard.settings_updated";
    /// Attachments (TASKLIST 2.14): added on upload/extraction, linked
    /// when bound to a comment, deleted when a row (and possibly its
    /// content-addressed object) goes away.
    pub const ATTACHMENT_ADDED: &str = "attachment.added";
    pub const ATTACHMENT_LINKED: &str = "attachment.linked";
    pub const ATTACHMENT_DELETED: &str = "attachment.deleted";
    /// Documents attachments (Wave 17 lane 13, migration 0089): the
    /// documents surface over the same content-addressed seam — a distinct
    /// family from `attachment.*` (that one is ticket/email-shaped).
    pub const DOCUMENT_ATTACHMENT_ADDED: &str = "document_attachment.added";
    pub const DOCUMENT_ATTACHMENT_DELETED: &str = "document_attachment.deleted";
    /// Custom fields / dynamic forms v1 (TASKLIST 2.17): definition CRUD
    /// through the board settings surface and typed ticket-value writes.
    pub const CUSTOM_FIELD_DEF_CREATED: &str = "custom_field.def_created";
    pub const CUSTOM_FIELD_DEF_UPDATED: &str = "custom_field.def_updated";
    pub const CUSTOM_FIELD_DEF_REMOVED: &str = "custom_field.def_removed";
    pub const TICKET_FIELD_SET: &str = "ticket.field_set";
    pub const TICKET_FIELD_CLEARED: &str = "ticket.field_cleared";
    /// Native KB v1 (TASKLIST Phase 3): article lifecycle. The indexer
    /// re-reads the CURRENT row on created/updated and drops the index
    /// entry on deleted (`kb_article.deleted`).
    pub const KB_ARTICLE_CREATED: &str = "kb_article.created";
    pub const KB_ARTICLE_UPDATED: &str = "kb_article.updated";
    pub const KB_ARTICLE_DELETED: &str = "kb_article.deleted";
    /// KB rich content (Wave 13 lane 4, migration 0069; ADR 0004): the
    /// oEmbed allowlist registry, per-article embed instances, and linked
    /// external documents. Payloads ride the KB pattern (untyped additive
    /// JSON; the row is the read model). `link_checked` records the
    /// link-check outcome — the note/columns are data, never a side
    /// effect of reads.
    pub const KB_EMBED_PROVIDER_CREATED: &str = "kb_embed_provider.created";
    pub const KB_EMBED_PROVIDER_UPDATED: &str = "kb_embed_provider.updated";
    pub const KB_EMBED_PROVIDER_DELETED: &str = "kb_embed_provider.deleted";
    pub const KB_EMBED_CREATED: &str = "kb_embed.created";
    pub const KB_EMBED_DELETED: &str = "kb_embed.deleted";
    pub const KB_EXTERNAL_DOCUMENT_CREATED: &str = "kb_external_document.created";
    pub const KB_EXTERNAL_DOCUMENT_UPDATED: &str = "kb_external_document.updated";
    pub const KB_EXTERNAL_DOCUMENT_DELETED: &str = "kb_external_document.deleted";
    pub const KB_EXTERNAL_DOCUMENT_LINK_CHECKED: &str = "kb_external_document.link_checked";
    /// DocumentVersion entities (TASKLIST D4): content changes arrive as
    /// NEW versions (update/restore), so every one of these re-indexes the
    /// document's current version. Documents are never deleted in v1, so
    /// the history stays intact.
    pub const DOCUMENT_CREATED: &str = "document.created";
    pub const DOCUMENT_UPDATED: &str = "document.updated";
    pub const DOCUMENT_RESTORED: &str = "document.restored";
    /// Docs-platform v1 (Wave 11 lane 7, migration 0053): typed document
    /// layouts, cross-entity links, and the password vault. Payloads ride
    /// the D4 pattern (untyped additive JSON; the row itself is the read
    /// model). `password.revealed` is the provenance rail for "who saw
    /// this credential" — never carries the secret itself.
    pub const DOCUMENT_LAYOUT_CREATED: &str = "document_layout.created";
    pub const DOCUMENT_LAYOUT_UPDATED: &str = "document_layout.updated";
    pub const DOCUMENT_LAYOUT_REMOVED: &str = "document_layout.removed";
    pub const PASSWORD_CREATED: &str = "password.created";
    pub const PASSWORD_UPDATED: &str = "password.updated";
    pub const PASSWORD_DELETED: &str = "password.deleted";
    pub const PASSWORD_REVEALED: &str = "password.revealed";
    pub const LINK_ADDED: &str = "link.added";
    pub const LINK_REMOVED: &str = "link.removed";
    /// Native secrets vault (ADR 0006, Wave 16 lane 3, migration 0078):
    /// secret category trees, SecretRecord generalization, category
    /// grants. Payloads ride the additive pattern; `SECRET_REVEALED` is
    /// the provenance rail for "who saw these secret fields" — NEVER
    /// carries the secret itself, and payloads never carry the plaintext
    /// beyond the dedicated reveal RESPONSE body (audited).
    pub const SECRET_FOLDER_CREATED: &str = "secret.folder_created";
    pub const SECRET_FOLDER_UPDATED: &str = "secret.folder_updated";
    pub const SECRET_FOLDER_DELETED: &str = "secret.folder_deleted";
    pub const SECRET_RECORD_CREATED: &str = "secret.record_created";
    pub const SECRET_RECORD_UPDATED: &str = "secret.record_updated";
    pub const SECRET_RECORD_DELETED: &str = "secret.record_deleted";
    pub const SECRET_REVEALED: &str = "secret.revealed";
    pub const SECRET_GRANT_UPDATED: &str = "secret.grant_updated";
    pub const SECRET_GRANT_DELETED: &str = "secret.grant_deleted";
    /// Asset taxonomy on the docs-platform (Wave 11 lane 5, migration
    /// 0054): typed asset instances ride `device_assets` +
    /// `document_layouts`. Payloads ride the untyped additive pattern (the
    /// row itself is the read model); `asset.updated`/`asset.deleted`
    /// re-index or drop the instance for the MCP/connector seam that
    /// mirrors an external DMS.
    pub const ASSET_CREATED: &str = "asset.created";
    pub const ASSET_UPDATED: &str = "asset.updated";
    pub const ASSET_DELETED: &str = "asset.deleted";
    /// Contracts + recurring billing rules (TASKLIST Phase 3): contract
    /// CRUD lifecycle. Billing previews are read-only and emit nothing;
    /// invoice runs are Wave 9.
    pub const CONTRACT_CREATED: &str = "contract.created";
    pub const CONTRACT_UPDATED: &str = "contract.updated";
    pub const CONTRACT_REMOVED: &str = "contract.removed";
    /// Native invoicing v1 (TASKLIST Phase 3, Wave 9): invoice runs,
    /// invoice lifecycle, payments through the `PaymentProvider` seam,
    /// and dunning-state transitions (states as data, forward-only).
    pub const INVOICE_RUN_COMPLETED: &str = "invoice_run.completed";
    pub const INVOICE_CREATED: &str = "invoice.created";
    pub const INVOICE_VOIDED: &str = "invoice.voided";
    pub const INVOICE_PAYMENT_RECORDED: &str = "invoice.payment_recorded";
    pub const INVOICE_DUNNING_CHANGED: &str = "invoice.dunning_changed";
    /// Credit notes (TASKLIST Money+docs backend gap): the credit
    /// document lifecycle — issued, allocated onto an invoice (the
    /// allocation ALSO emits invoice.payment_recorded with provider
    /// `credit_note`), refunded back out.
    pub const CREDIT_NOTE_ISSUED: &str = "credit_note.issued";
    pub const CREDIT_NOTE_APPLIED: &str = "credit_note.applied";
    pub const CREDIT_NOTE_REFUNDED: &str = "credit_note.refunded";
    /// Currency & FX (ADR 0002, Wave 12 lane 10). `fx.rate_recorded` — a
    /// rate row entered the append-only book (manual entry or a managed
    /// feed); consumers re-price pending conversions and reconcile caches.
    /// `fx.base_currency_changed` — the org's ONE reporting currency moved
    /// through the governed re-snapshot path; every money view must
    /// re-hydrate (the core never revaluates in place).
    pub const FX_RATE_RECORDED: &str = "fx.rate_recorded";
    pub const FX_BASE_CURRENCY_CHANGED: &str = "fx.base_currency_changed";
    /// Quote builder + catalog + sales pipeline (TASKLIST Phase 3, Wave 12
    /// lane 2). Catalog lifecycle is create/update/archive (archival is a
    /// flag, never a delete). The quote lifecycle is data
    /// (`draft` → `sent` → `accepted` | `declined` | `expired`); only a
    /// draft is editable. `quote.converted` fires when the accepted quote
    /// was materialized through the convert-to-invoice seam (contract +
    /// draft invoice via the existing invoicing persistence). Pipeline
    /// stage moves append an audited transition ledger row —
    /// `pipeline.deal_stage_changed` is the consumer signal, the ledger is
    /// ground truth.
    pub const CATALOG_ITEM_CREATED: &str = "catalog.item_created";
    pub const CATALOG_ITEM_UPDATED: &str = "catalog.item_updated";
    pub const CATALOG_ITEM_ARCHIVED: &str = "catalog.item_archived";
    pub const QUOTE_CREATED: &str = "quote.created";
    pub const QUOTE_UPDATED: &str = "quote.updated";
    pub const QUOTE_SENT: &str = "quote.sent";
    pub const QUOTE_ACCEPTED: &str = "quote.accepted";
    pub const QUOTE_DECLINED: &str = "quote.declined";
    pub const QUOTE_EXPIRED: &str = "quote.expired";
    pub const QUOTE_CONVERTED: &str = "quote.converted";
    pub const PIPELINE_STAGE_CREATED: &str = "pipeline.stage_created";
    pub const PIPELINE_STAGE_UPDATED: &str = "pipeline.stage_updated";
    pub const PIPELINE_DEAL_CREATED: &str = "pipeline.deal_created";
    pub const PIPELINE_DEAL_UPDATED: &str = "pipeline.deal_updated";
    pub const PIPELINE_DEAL_STAGE_CHANGED: &str = "pipeline.deal_stage_changed";
    /// Dispatch v2 (TASKLIST Phase 3, Wave 9 lane 9): a field-service
    /// status push landed against an appointment (connector or staff);
    /// the `dispatch_status_updates` ledger is the ground truth.
    pub const DISPATCH_STATUS_UPDATED: &str = "dispatch.status_updated";
    /// A dispatcher set or cleared an unassigned-queue rank override;
    /// consumers refetch the queue ordering.
    pub const DISPATCH_QUEUE_UPDATED: &str = "dispatch.queue_updated";
    /// A travel/mileage draft was submitted through the existing
    /// time-entry path; `time_entry.created` co-emits for the entry
    /// itself — this payload carries the draft→entry provenance pair.
    pub const TRAVEL_DRAFT_SUBMITTED: &str = "travel_draft.submitted";
    /// Projects line (TASKLIST Phase 3, migration 0030): project/phase/task
    /// lifecycle, saved views, templates, and the milestone billing seam.
    pub const PROJECT_CREATED: &str = "project.created";
    pub const PROJECT_UPDATED: &str = "project.updated";
    pub const PROJECT_PHASE_CREATED: &str = "project.phase_created";
    pub const PROJECT_PHASE_UPDATED: &str = "project.phase_updated";
    pub const PROJECT_PHASE_COMPLETED: &str = "project.phase_completed";
    pub const PROJECT_TASK_CREATED: &str = "project.task_created";
    pub const PROJECT_TASK_UPDATED: &str = "project.task_updated";
    pub const PROJECT_TASK_COMPLETED: &str = "project.task_completed";
    /// THE core-billing seam: emitted when a milestone phase or task
    /// completes. Payloads are additive-only; core-billing (Wave 9
    /// invoicing) consumes the envelope — no crate-level import in either
    /// direction.
    pub const PROJECT_MILESTONE_COMPLETED: &str = "project.milestone_completed";
    pub const PROJECT_VIEW_UPDATED: &str = "project.view_updated";
    pub const PROJECT_VIEW_REMOVED: &str = "project.view_removed";
    pub const PROJECT_TEMPLATE_UPDATED: &str = "project.template_updated";
    pub const PROJECT_TEMPLATE_REMOVED: &str = "project.template_removed";
    /// Workflow constructs (Wave 11 lane 9, migration 0056): org tag
    /// taxonomy lifecycle + per-ticket application (`ticket.tags_changed`
    /// covers apply AND remove; consumers refetch the ticket's tags).
    pub const TAG_CREATED: &str = "tag.created";
    pub const TAG_UPDATED: &str = "tag.updated";
    pub const TAG_DELETED: &str = "tag.deleted";
    pub const TICKET_TAGS_CHANGED: &str = "ticket.tags_changed";
    /// Ticket merge/parent-child/major-incident linking. `ticket.merged`
    /// is emitted ONCE per merge with the fold summary; link changes ride
    /// the paired linked/unlinked markers.
    pub const TICKET_MERGED: &str = "ticket.merged";
    pub const TICKET_LINKED: &str = "ticket.linked";
    pub const TICKET_UNLINKED: &str = "ticket.unlinked";
    /// Per-ticket money lines: cost lines (parts/flat/other) and expense
    /// entries. Added/updated/removed; the invoicing-candidate feed reads
    /// the billable-unbilled rows, the `invoice_line_id` consumption
    /// marker keeps "billed once, ever".
    pub const TICKET_COST_LINE_ADDED: &str = "ticket.cost_line_added";
    pub const TICKET_COST_LINE_UPDATED: &str = "ticket.cost_line_updated";
    pub const TICKET_COST_LINE_REMOVED: &str = "ticket.cost_line_removed";
    pub const TICKET_EXPENSE_RECORDED: &str = "ticket.expense_recorded";
    pub const TICKET_EXPENSE_UPDATED: &str = "ticket.expense_updated";
    pub const TICKET_EXPENSE_REMOVED: &str = "ticket.expense_removed";
    /// Recurring/scheduled tickets (PM schedules): schedule CRUD lifecycle.
    /// The worker tick emits NOTHING — the `pm_runs` ledger is the run
    /// record and each generated instance emits the ordinary
    /// `ticket.created` through its normal write path.
    pub const PM_SCHEDULE_CREATED: &str = "pm_schedule.created";
    pub const PM_SCHEDULE_UPDATED: &str = "pm_schedule.updated";
    pub const PM_SCHEDULE_DELETED: &str = "pm_schedule.deleted";
    /// Configuration identity (ADR 0003, Wave 12 lane 3). The canonical
    /// item IS a `device_assets` row; `config_item.*` covers its CI
    /// lifecycle — feed syncs, human curation, and the audited/undoable
    /// merge queue.
    pub const CONFIG_ITEM_CREATED: &str = "config_item.created";
    pub const CONFIG_ITEM_UPDATED: &str = "config_item.updated";
    pub const CONFIG_ITEM_MERGED: &str = "config_item.merged";
    pub const CONFIG_ITEM_MERGE_UNDONE: &str = "config_item.merge_undone";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketCreated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    pub number: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    pub title: String,
    pub priority: Priority,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub client_id: Option<ClientId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub assignee_id: Option<UserId>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketStatusChanged {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    /// Board the ticket lives on — drives WS fan-out to `board:<id>`.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    /// Current assignee — drives WS fan-out to `user:<id>`.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub assignee_id: Option<UserId>,
    pub from_status: String,
    pub to_status: String,
    pub from_category: TicketStatusCategory,
    pub to_category: TicketStatusCategory,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub changed_by: Option<UserId>,
    pub occurred_at: DateTime<Utc>,
}

/// A ticket moved to a different board (Wave 11 lane 3). The new status is
/// remapped through the SEMANTIC categories — never through the taxonomy
/// trees, which are classification-only (§5). `sla_reset` records which
/// SLA rule applied: `false` = the existing `ticket_sla_state` row was
/// preserved untouched (the default), `true` = the caller asked for an
/// explicit re-init and the clocks were restarted from the move moment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketBoardMoved {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    pub number: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub from_board_id: BoardId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub to_board_id: BoardId,
    pub from_status: String,
    pub to_status: String,
    pub from_category: TicketStatusCategory,
    pub to_category: TicketStatusCategory,
    pub sla_reset: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub moved_by: Option<UserId>,
    pub occurred_at: DateTime<Utc>,
}

/// A status was added to a board (custom workflow definition, §5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardStatusAdded {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    pub status: crate::entities::BoardStatus,
}

/// A board status's label/category/ordering was re-bound.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardStatusUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    pub status: crate::entities::BoardStatus,
}

/// An unused board status was removed. Carries the name because the row is
/// gone — consumers render history from this payload alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardStatusRemoved {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub status_id: crate::ids::TicketStatusId,
    pub name: String,
}

/// The board's explicit transition matrix was replaced. Empty `edges`
/// means the semantic-category floor governs again.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardTransitionsUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    pub edges: Vec<crate::workflow::TransitionEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketAssigned {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    /// Board the ticket lives on — drives WS fan-out to `board:<id>`.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub assignee_id: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub assigned_by: Option<UserId>,
}

/// `ticket.reverted` — an IF7 revert restored prior field values by
/// appending a NEW write-trail row (op `revert`); nothing in the trail was
/// rewritten. `reverted_seq` names the trail write the revert stepped back
/// over; `fields` lists the record fields whose values were restored.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketReverted {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    /// Board the ticket lives on — drives WS fan-out to `board:<id>`.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    /// Current assignee — drives WS fan-out to `user:<id>`.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub assignee_id: Option<UserId>,
    /// The write-trail seq the revert stepped back over.
    pub reverted_seq: i64,
    /// Record fields whose values were restored.
    pub fields: Vec<String>,
    /// Whether the revert was a whole-record step (no explicit field list).
    pub whole_record: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub reverted_by: Option<UserId>,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketCommentAdded {
    pub comment: TicketComment,
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<uuid::Uuid>))]
    pub mentioned_user_ids: Vec<UserId>,
}

/// A client company was created (payload for `client.created`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientCreated {
    pub client_id: ClientId,
    pub name: String,
}

/// A staff user was invited into an org (payload for `user.invited`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInvited {
    pub user_id: UserId,
    pub email: String,
    pub display_name: String,
    pub role: String,
}

/// A staff user was re-activated (payload for `user.activated`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserActivated {
    pub user_id: UserId,
}

/// A staff user was deactivated (payload for `user.deactivated`); live
/// sessions were revoked in the same transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDeactivated {
    pub user_id: UserId,
}

/// A preset (or any) role was forked into a custom copy (payload for
/// `role.forked`). `from_builtin` is the shipped preset the copy was
/// seeded from, `None` for forks of org-created roles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleForked {
    pub role_id: UserId,
    pub name: String,
    pub from_builtin: Option<String>,
}

/// A shipped preset role row was restored to its canonical permission set
/// (payload for `role.restored`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleRestored {
    pub role_id: UserId,
    pub builtin: String,
}

/// A template row was created (payload for `template.created`) — an
/// org-created template or a fork of a shipped default (KB3/KB4, ADR
/// 0004). `from_builtin` names the shipped default the row was forked
/// from, `None` for org-created rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateCreated {
    pub template_id: Uuid,
    pub kind: String,
    pub slug: String,
    pub from_builtin: Option<String>,
}

/// A template's content changed (payload for `template.updated`) — an
/// org-owned edit or a restore of a forked preset row to canonical.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateUpdated {
    pub template_id: Uuid,
    pub kind: String,
    pub slug: String,
}

/// An org-owned template row was removed (payload for
/// `template.removed`). Shipped defaults are immutable and never removed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateRemoved {
    pub template_id: Uuid,
    pub kind: String,
    pub slug: String,
}

/// A staff user's builtin org role changed (payload for
/// `user.role_changed`). `from_role` is `None` when no builtin role was
/// held before.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRoleChanged {
    pub user_id: UserId,
    pub from_role: Option<String>,
    pub to_role: String,
}

/// A site was created under a client (payload for `site.created`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteCreated {
    pub site_id: SiteId,
    pub client_id: ClientId,
    pub name: String,
}

/// A contact was created under a client (payload for `contact.created`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactCreated {
    pub contact_id: ContactId,
    pub client_id: ClientId,
    pub name: String,
}

/// A client was updated in place (payload for `client.updated`; import
/// framework update policy and future staff edit surfaces).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientUpdated {
    pub client_id: ClientId,
    pub name: String,
}

/// A site was updated in place (payload for `site.updated`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteUpdated {
    pub site_id: SiteId,
    pub client_id: ClientId,
    pub name: String,
}

/// A contact was updated in place (payload for `contact.updated`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactUpdated {
    pub contact_id: ContactId,
    pub client_id: ClientId,
    pub name: String,
}

/// An import run opened (payload for `import.batch_started`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportBatchStarted {
    pub batch_id: crate::ids::ImportBatchId,
    pub entity_kind: String,
    pub conflict_policy: String,
    pub label: String,
}

/// An import run finished (payload for `import.batch_finished`). Counters
/// are cumulative over every committed chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportBatchFinished {
    pub batch_id: crate::ids::ImportBatchId,
    pub entity_kind: String,
    pub rows_total: i32,
    pub rows_created: i32,
    pub rows_updated: i32,
    pub rows_skipped: i32,
    pub rows_failed: i32,
}

/// A migration connector push session opened (payload for
/// `migration.push_session_started`, TASKLIST D3). The session pairs 1:1
/// with the D2 import batch that does the writing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationPushSessionStarted {
    pub session_id: MigrationSessionId,
    pub batch_id: ImportBatchId,
    pub entity_kind: String,
    pub conflict_policy: String,
    pub label: String,
}

/// A migration connector push session closed (payload for
/// `migration.push_session_finished`). Counters mirror the underlying D2
/// batch's cumulative totals; `chunks_committed` is the checkpoint cursor
/// length the tool actually pushed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationPushSessionFinished {
    pub session_id: MigrationSessionId,
    pub batch_id: ImportBatchId,
    pub entity_kind: String,
    pub chunks_committed: i32,
    pub rows_total: i32,
    pub rows_created: i32,
    pub rows_updated: i32,
    pub rows_skipped: i32,
    pub rows_failed: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketChecklistUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    /// Board the ticket lives on — drives WS fan-out to `board:<id>`.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    pub items: Vec<ChecklistItem>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub updated_by: Option<UserId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimerStarted {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub session_id: TimerSessionId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    pub started_at: DateTime<Utc>,
    /// Session auto-paused by this start (single-active-timer workflow).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub switched_from: Option<TimerSessionId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimerPaused {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub session_id: TimerSessionId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    pub reason: Option<String>,
    pub elapsed_secs: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimerStopped {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub session_id: TimerSessionId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    pub total_secs: i64,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub time_entry_id: Option<TimeEntryId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimeEntryCreated {
    pub entry: TimeEntry,
}

/// Wave 14 lane 1 — the entry owner (or an admin) pushed the entry into
/// the approver queue. `submitted_by` may differ from `user_id` when an
/// admin submits on someone's behalf.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimeEntryApprovalSubmitted {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub entry_id: TimeEntryId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub entry_user_id: UserId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub submitted_by: UserId,
    pub submitted_at: DateTime<Utc>,
}

/// An approver holding `special:approve_time` accepted the entry. Approved
/// entries are the only ones billing reads may consume.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimeEntryApprovalApproved {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub entry_id: TimeEntryId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub approved_by: UserId,
    pub decided_at: DateTime<Utc>,
}

/// An approver rejected the entry with a typed reason code from the
/// [`crate::entities::TimeRejectionReason`] vocabulary; `note` carries the
/// optional free-text explanation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TimeEntryApprovalRejected {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub entry_id: TimeEntryId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub rejected_by: UserId,
    pub reason_code: String,
    pub note: Option<String>,
    pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NotificationCreated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    pub kind: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<TicketId>,
    pub title: String,
}

/// Emitted whenever an email message is recorded in the ledger (either
/// direction), with the routing outcome so consumers can correlate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EmailMessageRecorded {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub message_id: uuid::Uuid,
    pub rfc_message_id: String,
    /// "inbound" | "outbound" — kept as a string to avoid coupling
    /// consumers to the enum across versions.
    pub direction: String,
    pub outcome: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<TicketId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub comment_id: Option<uuid::Uuid>,
}

/// A board group was created (TASKLIST 2.11).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardGroupCreated {
    pub group: crate::entities::BoardGroup,
}

/// The org's per-RBAC pod-visibility matrix was rewritten (Wave 12 lane
/// 6). The matrix is pod id → RBAC level → visible; absent pods/levels
/// fall back to the client registry defaults, so the payload carries the
/// stored document verbatim rather than a closed struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PodVisibilityUpdated {
    /// The stored matrix document (validated shape at the API boundary).
    pub config: serde_json::Value,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub updated_by: Option<uuid::Uuid>,
}

/// A board group's name/position changed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardGroupUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub group_id: crate::ids::OrgId,
    pub name: String,
    pub position: i32,
}

/// A board group was deleted; member boards became ungrouped.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardGroupRemoved {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub group_id: crate::ids::OrgId,
}

/// A user's board membership was created or re-roled.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardMembershipUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: crate::ids::BoardId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: crate::ids::UserId,
    pub role: crate::policy::BoardRole,
}

/// A user's board membership was removed (back to org-governed access).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardMembershipRemoved {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: crate::ids::BoardId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: crate::ids::UserId,
}

/// A user toggled a board favorite for the nav tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BoardFavoriteToggled {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: crate::ids::UserId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: crate::ids::BoardId,
    pub favorite: bool,
}

/// A booking was created on a user's calendar (TASKLIST 2.12).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AppointmentCreated {
    pub appointment: crate::entities::Appointment,
}

/// An appointment's editable fields or times changed (`rescheduled` marks
/// moves so dispatch consumers can re-plan travel).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AppointmentUpdated {
    pub appointment: crate::entities::Appointment,
    pub rescheduled: bool,
}

/// An appointment was cancelled; its slot is freed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AppointmentCancelled {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub appointment_id: crate::ids::AppointmentId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: crate::ids::UserId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub cancelled_by: Option<crate::ids::UserId>,
}

/// Availability templates changed for one user (create/delete).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AvailabilityChanged {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: crate::ids::UserId,
}

/// On-call rotations changed for one user.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OnCallChanged {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: crate::ids::UserId,
}

/// Time-off exclusions changed for one user.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CalendarExclusionChanged {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: crate::ids::UserId,
}

/// `calendar.sync_connection_updated` — a calendar-sync connection was
/// created, updated, disabled, or its last poll failed (`status: "error"`
/// with `last_error` set). Carries the connection's identity and health;
/// the outbound feed token never appears here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CalendarSyncConnectionUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub connection_id: crate::ids::CalendarSyncConnectionId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: crate::ids::UserId,
    pub provider: String,
    /// `outbound` | `inbound` | `two_way`.
    pub direction: String,
    /// `active` | `error` | `disabled`.
    pub status: String,
    pub last_error: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub occurred_at: DateTime<Utc>,
}

/// `calendar.sync_completed` — one inbound sync run finished: how much
/// busy time was imported/updated/released and how many overlaps the
/// conflict rule reported (overlaps are surfaced, never blocked).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CalendarSyncCompleted {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub connection_id: crate::ids::CalendarSyncConnectionId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: crate::ids::UserId,
    pub imported: i64,
    pub updated: i64,
    pub released: i64,
    pub overlaps: i64,
    pub duplicates: i64,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub synced_at: DateTime<Utc>,
}

/// `scorecard.settings_updated` — a staff member changed their scorecard
/// settings. `gamification_opt_in` is the R5 opt-in toggle: false keeps
/// the scorecard a private coaching surface (the default).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ScorecardSettingsUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: crate::ids::UserId,
    pub gamification_opt_in: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub occurred_at: DateTime<Utc>,
}

// --- plugin runtime v2 + marketplace registry (Wave 14 lane 5) ---------------
// (Grant lists are capability KEYS only — the resolved permissions stay
// server-side; a plugin's effective authority is always re-derived from
// the existing permission matrix at check time.)

/// `plugin.catalog_entry_created` — a catalog entry was published with
/// its declarative manifest snapshot. Capabilities are the manifest's
/// DECLARED keys (what the plugin may ask for), never a grant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginCatalogEntryCreated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub catalog_id: crate::ids::PluginCatalogEntryId,
    pub slug: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub interface_version: i64,
    pub capabilities: Vec<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub created_at: DateTime<Utc>,
}

/// `plugin.installed` — an org adopted a catalog entry at a pinned
/// version. Installs are created DISABLED; the event carries the granted
/// capability keys (subset of the manifest's declared set).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginInstalled {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub install_id: crate::ids::PluginInstallId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub catalog_id: crate::ids::PluginCatalogEntryId,
    pub slug: String,
    pub version: String,
    pub granted_capabilities: Vec<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub installed_at: DateTime<Utc>,
}

/// `plugin.install_updated` — an install was enabled, disabled,
/// upgraded to a new pinned version, or its granted capability set
/// changed. Carries the post-change state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginInstallUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub install_id: crate::ids::PluginInstallId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub catalog_id: crate::ids::PluginCatalogEntryId,
    pub slug: String,
    pub version: String,
    /// `disabled` | `enabled` | `revoked`.
    pub status: String,
    pub granted_capabilities: Vec<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub occurred_at: DateTime<Utc>,
}

/// `plugin.install_revoked` — the install was killed: registry-level
/// revocation of the catalog entry, or an explicit org revoke. A revoked
/// install never runs again until re-installed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginInstallRevoked {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub install_id: crate::ids::PluginInstallId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub catalog_id: crate::ids::PluginCatalogEntryId,
    pub slug: String,
    pub reason: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub revoked_at: DateTime<Utc>,
}

// --- plugin & skill distribution lifecycle (Wave 16 lane 7, ADR 0009) --------

/// `plugin.staged_import_created` — a member upload (skill or plugin)
/// landed staged: validated, never executed, visible only to its
/// uploader and admins. Capabilities are the staged manifest's DECLARED
/// keys; consent happens at approval, never here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginStagedImportCreated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub staged_item_id: Uuid,
    /// `plugin` | `skill`.
    pub artifact_kind: String,
    /// `marketplace_registry` | `github_repo` | `local_zip` |
    /// `skill_community` | `manual_upload`.
    pub source_kind: String,
    pub source_url: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub fetched_ref: Option<String>,
    pub artifact_digest: String,
    pub declared_capabilities: Vec<String>,
    pub uploaded_by: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub uploaded_at: DateTime<Utc>,
}

/// `plugin.staged_import_approved` — an owner/admin reviewed the consent
/// diff and the wait ended in adoption: the catalog entry/version +
/// install were minted (plugins) or the agent skill was imported
/// (skills), provenance recorded, still deny-by-default on enable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginStagedImportApproved {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub staged_item_id: Uuid,
    pub reviewed_by: Uuid,
    pub review_note: Option<String>,
    pub artifact_kind: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub adopted_install_id: Option<crate::ids::PluginInstallId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Uuid>))]
    pub adopted_skill_id: Option<crate::ids::PluginCatalogEntryId>,
    pub granted_capabilities: Vec<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub approved_at: DateTime<Utc>,
}

/// `plugin.staged_import_refused` — the reviewer declined the staged
/// import (terminal for that row; the uploader may re-stage fresh).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginStagedImportRefused {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub staged_item_id: Uuid,
    pub reviewed_by: Uuid,
    pub review_note: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub refused_at: DateTime<Utc>,
}

/// `plugin.update_pending` — an update proposal is waiting on the
/// re-consent diff: the OLD version stays pinned and running until the
/// owner/admin consents; consent commits the swap.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginUpdatePending {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub install_id: crate::ids::PluginInstallId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub catalog_id: crate::ids::PluginCatalogEntryId,
    pub slug: String,
    /// The version still running (pinned until consented).
    pub current_version: String,
    /// The candidate version awaiting consent.
    pub pending_version: String,
    pub pending_digest: String,
    pub added_capabilities: Vec<String>,
    pub removed_capabilities: Vec<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub occurred_at: DateTime<Utc>,
}

/// `plugin.settings_updated` — a per-install config rendered FROM the
/// pinned manifest's settings JSON-Schema changed (validated at the
/// write boundary; no hand-authored per-plugin settings pages exist).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PluginSettingsUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub install_id: crate::ids::PluginInstallId,
    pub slug: String,
    pub version: String,
    /// The validated config keys (values stay server-side; consumers
    /// refetch through the settings surface).
    pub config_keys: Vec<String>,
    pub updated_by: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub occurred_at: DateTime<Utc>,
}

// --- projects line (TASKLIST Phase 3, migration 0030) ------------------------

/// `project.created` — the full applied project snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectCreated {
    pub project: crate::entities::Project,
}

/// `project.updated` — change marker; consumers refetch the project.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub project_id: ProjectId,
}

/// `project.phase_created` — the full applied phase snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectPhaseCreated {
    pub phase: crate::entities::ProjectPhase,
}

/// `project.phase_updated` — change marker; consumers refetch the phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectPhaseUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub phase_id: PhaseId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub project_id: ProjectId,
}

/// `project.phase_completed` — the completed phase snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectPhaseCompleted {
    pub phase: crate::entities::ProjectPhase,
}

/// `project.task_created` — the full applied task snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectTaskCreated {
    pub task: crate::entities::ProjectTask,
}

/// `project.task_updated` — change marker; consumers refetch the task.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectTaskUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub task_id: TaskId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub project_id: ProjectId,
}

/// `project.task_completed` — the completed task snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectTaskCompleted {
    pub task: crate::entities::ProjectTask,
}

/// `project.milestone_completed` — THE core-billing seam payload. Emitted
/// exactly when a milestone phase or task transitions to completed, in the
/// same transaction as the completion write. Consumers MUST tolerate
/// unknown fields; new fields are added, never removed or retyped (policy
/// §2). `task_id`/`phase_id` name which kind completed (`None` on the
/// other); `amount` is `None` for informational milestones with no billing
/// amount attached.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectMilestoneCompleted {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub project_id: ProjectId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub phase_id: Option<PhaseId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub task_id: Option<TaskId>,
    /// The milestone's display name at completion time (the row may later
    /// be renamed; the event carries what was billed).
    pub name: String,
    /// Amount to release to invoicing (minor units + ISO 4217 currency);
    /// `None` = informational milestone.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<crate::money::Money>))]
    pub amount: Option<crate::money::Money>,
    /// The project's billing method at completion time (routing context
    /// for the consumer; additive context, not the trigger condition).
    pub billing_method: String,
    pub completed_at: DateTime<Utc>,
}

/// `project.view_updated` — a saved view was created or changed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectViewUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub view_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub project_id: Option<ProjectId>,
}

/// `project.view_removed` — a saved view was deleted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectViewRemoved {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub view_id: uuid::Uuid,
}

/// `project.template_updated` — a template was created or changed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectTemplateUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub template_id: uuid::Uuid,
}

/// `project.template_removed` — a template was deleted. Existing projects
/// instantiated from it are unaffected (templates are blueprints, not
/// live references).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProjectTemplateRemoved {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub template_id: uuid::Uuid,
}

/// `alert.ingested` — one normalized alert passed HMAC verification and
/// the dedup gate. `outcome` tells consumers what the correlation stage
/// did: `incident_created` (this alert anchored a NEW incident ticket),
/// or `collapsed` (folded into the still-open storm incident — no new
/// ticket). `alert_count` is the collapsed group's size AFTER this alert,
/// so consumers can track storm growth without re-reading the alert row.
/// Severity and outcome ride as strings to avoid coupling consumers to
/// enums across versions (additive policy §2).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AlertIngested {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub alert_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub source_id: uuid::Uuid,
    /// Public ingest routing id of the source (stable across rotations).
    pub source_ref: String,
    pub dedup_key: String,
    pub correlation_key: String,
    pub title: String,
    /// "info" | "warning" | "critical"
    pub severity: String,
    /// "incident_created" | "collapsed"
    pub outcome: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<TicketId>,
    pub alert_count: i64,
    pub occurred_at: DateTime<Utc>,
}

/// `alert.acknowledged` — a staff member acknowledged the alert
/// (Wave 17 lane 13, migration 0089). The row's `status` is
/// `acknowledged` after this; forward-only through the API (the route
/// refuses re-ack with a typed validation error).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AlertAcknowledged {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub alert_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub acknowledged_by: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub acknowledged_at: DateTime<Utc>,
}

/// `alert.resolved` — a staff member resolved the alert. From any
/// open/acknowledged state; the row's `status` is `resolved` after.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AlertResolved {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub alert_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub resolved_by: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub resolved_at: DateTime<Utc>,
}

/// Extracts @mentions from a note body (`@alice`, `@alice@org` → user ids
/// resolved by the caller). Returns the distinct handle strings.
pub fn extract_mentions(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in body.split_whitespace() {
        let t = token.trim_matches(|c: char| c == ',' || c == '.' || c == ')' || c == '(');
        if let Some(handle) = t.strip_prefix('@') {
            let handle = handle.split('@').next().unwrap_or(handle);
            if !handle.is_empty() && !out.contains(&handle.to_string()) {
                out.push(handle.to_string());
            }
        }
    }
    out
}

/// attachment.added — a file was stored and its metadata row committed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AttachmentAdded {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub attachment_id: uuid::Uuid,
    pub kind: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_comment_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub email_message_id: Option<uuid::Uuid>,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    /// sha256 hex — consumers can verify integrity after download.
    pub sha256: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub uploaded_by_user_id: Option<uuid::Uuid>,
}

/// attachment.linked — an unattached upload was bound to a comment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AttachmentLinked {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub attachment_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_comment_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: uuid::Uuid,
}

/// attachment.deleted — the metadata row went away; `object_removed`
/// reports whether the backing bytes were reclaimed too (refcount hit 0).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AttachmentDeleted {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub attachment_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<uuid::Uuid>,
    pub object_removed: bool,
}

// --- workflow constructs (Wave 11 lane 9, migration 0056) --------------------

/// `meeting_type.deleted` — a meeting type was hard deleted by an admin
/// (Wave 17 lane 13). The endpoint refuses (typed conflict) while a
/// scheduling link still references the row, so this event means the row
/// is GONE — paused templates are PATCHes, not deletes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MeetingTypeDeleted {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub meeting_type_id: uuid::Uuid,
    pub name: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub deleted_by: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub deleted_at: DateTime<Utc>,
}

/// `document_attachment.added` — a file was stored and bound to a
/// document over the documents-attachments surface (Wave 17 lane 13,
/// migration 0089). Distinct from `attachment.added` (that family is
/// ticket/email-shaped); same content-addressed dedup doctrine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DocumentAttachmentAdded {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub attachment_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub document_id: uuid::Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    /// sha256 hex — consumers can verify integrity after download.
    pub sha256: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub uploaded_by: Option<uuid::Uuid>,
}

/// `document_attachment.deleted` — the metadata row went away;
/// `object_removed` reports whether the backing bytes were reclaimed too
/// (refcount hit 0).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DocumentAttachmentDeleted {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub attachment_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub document_id: uuid::Uuid,
    pub object_removed: bool,
}

/// `tag.created` — a tag was added to the org taxonomy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TagCreated {
    pub tag: crate::entities::Tag,
}

/// `tag.updated` — the tag's name/color changed. Refetch the tag.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TagUpdated {
    pub tag: crate::entities::Tag,
}

/// `tag.deleted` — the tag was removed; `applied_count` is how many
/// ticket applications it had at deletion (the join rows went with it).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TagDeleted {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub tag_id: uuid::Uuid,
    pub name: String,
    pub applied_count: i64,
}

/// `ticket.tags_changed` — a tag was applied to or removed from a ticket
/// (`applied: bool` distinguishes; consumers refetch the ticket's tags).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketTagsChanged {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub board_id: BoardId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub tag_id: uuid::Uuid,
    pub applied: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub changed_by: Option<UserId>,
}

/// `ticket.linked` / `ticket.unlinked` — a typed edge was created or
/// removed between two tickets. `link_type` rides as a string to avoid
/// coupling consumers to the enum across versions (additive policy §2).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketLinkChanged {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub from_ticket_id: TicketId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub to_ticket_id: TicketId,
    /// "parent_child" | "major_incident"
    pub link_type: String,
    pub linked: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub changed_by: Option<UserId>,
}

/// `ticket.merged` — the fold summary. `source_*` names the ticket that
/// was absorbed (comments, time, costs, expenses, and child links folded
/// into `survivor_id`); `conflicts` lists the typed conflict keys the
/// caller resolved (empty when none). The source's row stays queryable
/// (doctrine #9) but is closed + marked `merged_into_ticket_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketMerged {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub source_id: TicketId,
    pub source_number: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub survivor_id: TicketId,
    pub survivor_number: String,
    pub comments_folded: i64,
    pub time_entries_folded: i64,
    pub time_minutes_folded: i64,
    pub cost_lines_folded: i64,
    pub expenses_folded: i64,
    pub children_relinked: i64,
    pub conflicts: Vec<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub merged_by: Option<UserId>,
    pub occurred_at: DateTime<Utc>,
}

/// `ticket.cost_line_added` / `ticket.cost_line_updated` — the applied
/// line snapshot; consumers refetch the ticket's cost lines.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketCostLineAdded {
    pub line: crate::entities::TicketCostLine,
}

/// `ticket.cost_line_removed` — the line went away; the payload carries
/// what it last was so consumers can render history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketCostLineRemoved {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub line_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
}

/// `ticket.expense_recorded` / `ticket.expense_updated` — the applied
/// expense snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketExpenseRecorded {
    pub expense: crate::entities::TicketExpense,
}

/// `ticket.expense_removed` — the expense entry went away.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TicketExpenseRemoved {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub expense_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: TicketId,
}

/// `pm_schedule.created` / `pm_schedule.updated` — the applied schedule
/// snapshot (the worker tick emits nothing; the `pm_runs` ledger is the
/// run record and each instance emits the ordinary `ticket.created`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PmScheduleChanged {
    pub schedule: crate::entities::PmSchedule,
}

/// `pm_schedule.deleted` — the schedule was removed; existing generated
/// tickets are unaffected (templates are blueprints, not live references).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PmScheduleDeleted {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub schedule_id: uuid::Uuid,
    pub name: String,
}

/// Convenience for building the note-kind of a comment event.
pub fn comment_kind_label(kind: NoteKind) -> &'static str {
    match kind {
        NoteKind::Internal => "internal",
        NoteKind::ClientVisible => "client-visible",
        NoteKind::InHouseUpdate => "in-house update",
    }
}

/// contract.created — a commercial agreement was created. Billing-rule
/// previews emit nothing; invoice runs are Wave 9.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContractCreated {
    pub contract: crate::entities::Contract,
}

/// contract.updated — the agreement's fields, rate card, or fee schedule
/// changed. `after`-style consumers should refetch rates/fees with it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContractUpdated {
    pub contract: crate::entities::Contract,
}

/// contract.removed — the agreement (with its rate card and fee schedule)
/// was deleted. Invoiced contracts cannot be deleted: the Wave 9 invoicing
/// lane guards deletes behind existing invoice references.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContractRemoved {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub contract_id: uuid::Uuid,
    pub number: String,
}

/// invoice_run.completed — the roll-up of one evaluation sweep: window,
/// invoice count, and summed total. Emitted once per run, after every
/// invoice it created committed in the same transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct InvoiceRunCompleted {
    pub run: crate::entities::InvoiceRun,
}

/// invoice.created — an issued invoice snapshot (lines excluded;
/// consumers refetch the detail). One per contract that produced at
/// least one candidate line in the run's window.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct InvoiceCreated {
    pub invoice: crate::entities::Invoice,
}

/// invoice.voided — the invoice was withdrawn. Consumption markers
/// (billed periods, time-entry links) stay: voiding releases nothing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct InvoiceVoided {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub invoice_id: uuid::Uuid,
    pub number: String,
}

/// invoice.payment_recorded — a receipt landed through the
/// `PaymentProvider` seam. `invoice_status` is the invoice's status AFTER
/// the payment applied (`issued` while a balance remains, `paid` once
/// settled).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct InvoicePaymentRecorded {
    pub payment: crate::entities::InvoicePayment,
    pub invoice_status: String,
}

/// credit_note.issued — a client credit document was created. `invoice_id`
/// names the issuing context when the credit was raised against one
/// specific invoice (otherwise a free-standing client credit).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreditNoteIssued {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub credit_note_id: uuid::Uuid,
    pub number: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub invoice_id: Option<uuid::Uuid>,
    pub currency: String,
    pub total_minor: i64,
    pub reason: Option<String>,
}

/// credit_note.applied — part of a credit note's value moved onto an
/// invoice. The invoice-side receipt co-emits `invoice.payment_recorded`
/// (provider `credit_note`); this payload carries the credit-side
/// counters AFTER the allocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreditNoteApplied {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub credit_note_id: uuid::Uuid,
    pub credit_note_number: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub invoice_id: uuid::Uuid,
    pub amount_minor: i64,
    pub currency: String,
    pub applied_minor: i64,
    pub remaining_minor: i64,
}

/// credit_note.refunded — part of a credit note's value was paid back to
/// the client (none-mode provider approves locally; no external PSP
/// refund in v1). Counters are AFTER the refund.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreditNoteRefunded {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub credit_note_id: uuid::Uuid,
    pub credit_note_number: String,
    pub amount_minor: i64,
    pub currency: String,
    pub refunded_minor: i64,
    pub remaining_minor: i64,
}

/// invoice.dunning_changed — a forward-only collections-posture move
/// (`none` → `reminder_sent` → `overdue` → `final_notice` →
/// `collections`; skipping rungs is allowed, backwards is refused).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct InvoiceDunningChanged {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub invoice_id: uuid::Uuid,
    pub number: String,
    pub from_state: String,
    pub to_state: String,
    pub note: Option<String>,
    pub changed_at: DateTime<Utc>,
}

/// fx.rate_recorded — a rate row entered the org's append-only book
/// (ADR 0002). Corrections are NEW rows; consumers re-price pending
/// conversions from the effective rate (newest captured_at ≤ as_of).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FxRateRecorded {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub rate_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: uuid::Uuid,
    pub base_currency: String,
    pub quote_currency: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub as_of_date: chrono::NaiveDate,
    /// QUOTE major units per 1 BASE major unit ("17.850000000000").
    pub rate: String,
    /// 'manual' for the built-in provider; managed feeds stamp their own.
    pub source: String,
    pub captured_at: DateTime<Utc>,
}

/// fx.base_currency_changed — the org's ONE base (reporting) currency
/// moved through the governed re-snapshot path (ADR 0002, FX1). Every
/// money view must re-hydrate; the core never revaluates in place.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FxBaseCurrencyChanged {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: uuid::Uuid,
    pub from: String,
    pub to: String,
    pub changed_at: DateTime<Utc>,
}

/// catalog.item_created — a product/service catalog item was created.
/// The payload carries the full item snapshot (money is minor units).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CatalogItemCreated {
    pub item: crate::entities::CatalogItem,
}

/// catalog.item_updated — a catalog item changed (any field including
/// `active`). The snapshot is AFTER the write.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CatalogItemUpdated {
    pub item: crate::entities::CatalogItem,
}

/// catalog.item_archived — a catalog item was archived (`active = false`).
/// Archival is a flag: quote history is never rewritten, and the item
/// stays readable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CatalogItemArchived {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub item_id: uuid::Uuid,
    pub name: String,
}

/// quote.created — a numbered quote document was created in draft with
/// its initial lines (lines excluded from the payload; consumers refetch).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct QuoteCreated {
    pub quote: crate::entities::Quote,
}

/// quote.updated — a draft quote's header or lines were edited; the
/// roll-ups are recomputed. After the write.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct QuoteUpdated {
    pub quote: crate::entities::Quote,
}

/// quote.sent — the draft was locked and sent to the client. Sending is
/// the immutability boundary: only lifecycle legs write past it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct QuoteSent {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub quote_id: uuid::Uuid,
    pub number: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    pub currency: String,
    pub total_minor: i64,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub valid_until: chrono::NaiveDate,
    pub sent_at: DateTime<Utc>,
}

/// quote.accepted — the client accepted the quote. Terminal; the quote is
/// now convertible through the convert-to-invoice seam while valid.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct QuoteAccepted {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub quote_id: uuid::Uuid,
    pub number: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    pub currency: String,
    pub total_minor: i64,
    pub decided_at: DateTime<Utc>,
}

/// quote.declined — the client declined the quote. Terminal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct QuoteDeclined {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub quote_id: uuid::Uuid,
    pub number: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    pub decided_at: DateTime<Utc>,
}

/// quote.expired — the quote was explicitly expired past its validity
/// window (expiry is also derivable from `valid_until` for consumers).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct QuoteExpired {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub quote_id: uuid::Uuid,
    pub number: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    pub decided_at: DateTime<Utc>,
}

/// quote.converted — the convert-to-invoice seam materialized an accepted
/// quote into what native invoicing consumes: a fixed-fee contract (one
/// scheduled fee per quote line) plus a DRAFT invoice built through the
/// existing invoice persistence. Both ids are always set.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct QuoteConverted {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub quote_id: uuid::Uuid,
    pub quote_number: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub contract_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub invoice_id: uuid::Uuid,
    pub currency: String,
    pub total_minor: i64,
    pub converted_at: DateTime<Utc>,
}

/// pipeline.stage_created — a pipeline stage was created.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PipelineStageCreated {
    pub stage: crate::entities::PipelineStage,
}

/// pipeline.stage_updated — a stage's name/position/kind/active changed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PipelineStageUpdated {
    pub stage: crate::entities::PipelineStage,
}

/// pipeline.deal_created — an opportunity was created and placed on its
/// initial stage (the placement transition row co-commits).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PipelineDealCreated {
    pub deal: crate::entities::PipelineDeal,
}

/// pipeline.deal_updated — a deal's editable fields changed (not its
/// stage; that fires `pipeline.deal_stage_changed`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PipelineDealUpdated {
    pub deal: crate::entities::PipelineDeal,
}

/// pipeline.deal_stage_changed — a deal moved between stages. The
/// immutable transition ledger row is ground truth; this payload carries
/// the AFTER posture (`status` mirrors the target stage's kind — landing
/// on a won/lost stage closes the deal).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PipelineDealStageChanged {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub deal_id: uuid::Uuid,
    pub name: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub from_stage_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub to_stage_id: uuid::Uuid,
    pub to_stage_type: crate::entities::StageType,
    pub status: crate::entities::DealStatus,
    pub amount_minor: i64,
    pub currency: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub actor_id: Option<uuid::Uuid>,
    pub occurred_at: DateTime<Utc>,
}

/// appointment_request.created — a client portal user asked for a slot on
/// a staff member's calendar (Portal v2). The request is an INTENT:
/// nothing occupies the calendar until the approval event below fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AppointmentRequestCreated {
    pub request: crate::entities::AppointmentRequest,
}

/// appointment_request.approved — staff turned a pending request into a
/// real booking after re-validating availability and conflicts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AppointmentRequestApproved {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub request_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub appointment_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    pub decided_by: uuid::Uuid,
    pub decided_at: DateTime<Utc>,
}

/// appointment_request.declined — staff refused a pending request; no
/// calendar effect. `note` carries the optional reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AppointmentRequestDeclined {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub request_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    pub decided_by: uuid::Uuid,
    pub note: Option<String>,
    pub decided_at: DateTime<Utc>,
}

/// appointment_request.cancelled — the requesting portal user withdrew
/// their own still-pending request; no calendar effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AppointmentRequestCancelled {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub request_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub cancelled_by_portal_user_id: uuid::Uuid,
    pub cancelled_at: DateTime<Utc>,
}

/// scheduling_link.created — staff minted a shareable booking link.
/// The raw token never appears here; the tech's HTTP create response
/// carries it exactly once.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SchedulingLinkCreated {
    pub link: crate::entities::SchedulingLink,
    pub created_by: uuid::Uuid,
}

/// scheduling_link.viewed — the unauthenticated public page resolved the
/// token (lifecycle signal only; lightweight by design).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SchedulingLinkViewed {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub link_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub viewed_at: DateTime<Utc>,
    pub view_count: i64,
}

/// scheduling_link.scheduled — a booking landed through the link.
/// Co-emits the standard `appointment.created` (the booking enters the
/// world through the normal creation path).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SchedulingLinkScheduled {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub link_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub appointment_id: uuid::Uuid,
    pub booker_name: Option<String>,
    pub booker_email: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub scheduled_at: DateTime<Utc>,
}

/// scheduling_link.revoked — staff pulled the link; it can never book
/// again. Existing bookings are untouched.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SchedulingLinkRevoked {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub link_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub revoked_by: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub revoked_at: DateTime<Utc>,
}

/// dispatch.status_updated — a field-service status push landed against
/// an appointment. The appointment's lifecycle status is untouched; this
/// is the connector-visible progress vocabulary (en_route, arrived,
/// in_progress, completed, blocked, delayed).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DispatchStatusUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub appointment_id: uuid::Uuid,
    pub status: String,
    pub note: Option<String>,
    /// `connector` (machine key) or `staff`.
    pub source: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub actor_id: uuid::Uuid,
    pub occurred_at: DateTime<Utc>,
}

/// dispatch.queue_updated — an unassigned-queue rank override was set or
/// cleared for one ticket. `rank` is the new explicit rank; cleared
/// overrides carry `rank: 0` with `cleared: true`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DispatchQueueUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: uuid::Uuid,
    pub rank: i32,
    pub cleared: bool,
}

/// travel_draft.submitted — a travel/mileage draft converted into a real
/// time entry through the existing time-entry path. `time_entry_id` is
/// the backlink the draft row also stores; `time_entry.created` co-emits
/// for the entry itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TravelDraftSubmitted {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub draft_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub ticket_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub time_entry_id: uuid::Uuid,
    pub distance: f64,
    pub distance_unit: String,
    pub vehicle: String,
}

// ---- configuration identity (ADR 0003, Wave 12 lane 3) ---------------------

/// `config_item.created` — a fresh canonical item (feed-created or manual);
/// `source_link` carries the feeder binding when the create was a sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ConfigItemCreated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub config_item_id: DeviceAssetId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: ClientId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub source_link_id: ConfigSourceLinkId,
    /// "feed" | "manual"
    pub origin: String,
    pub source_system: String,
    pub occurred_at: DateTime<Utc>,
}

/// `config_item.updated` — feed sync updates, curation, and adoption of an
/// existing item through its source link.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ConfigItemUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub config_item_id: DeviceAssetId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub source_link_id: ConfigSourceLinkId,
    /// "feed_sync" | "adopted_existing" | "curated" | "accept_observed"
    pub change: String,
    /// Field keys written by THIS change (empty for pure link touches).
    #[serde(default)]
    pub fields_changed: Vec<String>,
    /// Observed values that now contradict a curated value (CI5).
    #[serde(default)]
    pub contradictions: Vec<String>,
    pub occurred_at: DateTime<Utc>,
}

/// `config_item.merged` — one audited merge; undo rides
/// `config_item.merge_undone`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ConfigItemMerged {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub primary_config_item_id: DeviceAssetId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub merged_config_item_id: DeviceAssetId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub candidate_id: Option<CiMergeCandidateId>,
    pub confidence: Option<f64>,
    pub reason: String,
    pub auto: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub merged_by: Option<UserId>,
    pub occurred_at: DateTime<Utc>,
}

/// `config_item.merge_undone` — a split restored the absorbed item from
/// the merge audit row's moved packet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ConfigItemMergeUndone {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub primary_config_item_id: DeviceAssetId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub restored_config_item_id: DeviceAssetId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub merge_id: CiMergeId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub undone_by: Option<UserId>,
    pub occurred_at: DateTime<Utc>,
}

// --- client workflow overrides (Wave 12 lane 5, migration 0063) --------------

/// `client.workflow_override_set` — a client override was written as `set`
/// (value replaces the org default) or `disabled` (explicit deny wins over an
/// org-enabled default). `value` is the typed payload (uuid string for
/// reference constructs, boolean for toggles) and is `null` when `disabled`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ClientWorkflowOverrideSet {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    /// Registry key from `core_domain::workflow_overrides` (closed set).
    pub key: String,
    /// "set" | "disabled".
    pub state: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>, nullable = true))]
    pub value: Option<serde_json::Value>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub changed_by: Option<uuid::Uuid>,
    pub occurred_at: DateTime<Utc>,
}

/// `client.workflow_override_cleared` — the override row was removed, so the
/// client inherits the org default again. Distinct from an explicit disable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ClientWorkflowOverrideCleared {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    pub key: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub cleared_by: Option<uuid::Uuid>,
    pub occurred_at: DateTime<Utc>,
}

// --- native secrets vault (Wave 16 lane 3, migration 0078; contract
// registration Wave 16 lane 9) ----------------------------------------------

/// `secret.folder_created` — a category folder appeared (admin create OR
/// the fork path in the `"fork"` variant). Scope carries `client_id` only
/// for client-scope folders; org-scope rows are `null`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretFolderCreated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub folder_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub client_id: Option<uuid::Uuid>,
    pub slug: String,
    pub restricted: bool,
}

/// `secret.folder_created` variant `"fork"` — the deep-copy fork of a
/// subtree into client scope; provenance stays on the row itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretFolderForked {
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub client_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub forked_from: uuid::Uuid,
    pub slug: String,
}

/// `secret.folder_updated` — label/slug/kind/restricted edits (seed rows
/// already refuse; the store enforces the fork mandate).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretFolderUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub folder_id: uuid::Uuid,
    pub slug: String,
    pub restricted: bool,
}

/// `secret.folder_deleted` — the folder grew no records (delete is
/// refused otherwise).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretFolderDeleted {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub folder_id: uuid::Uuid,
    pub slug: String,
}

/// `secret.record_created` — a sealed record landed in a folder. Field
/// material (ciphertext, plaintext) NEVER rides here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretRecordCreated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub record_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub client_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub folder_id: uuid::Uuid,
    pub title: String,
}

/// `secret.record_updated` — record metadata (title/notes/url edits or
/// field rotation/clear) changed; `fields_rotated` flags whether any
/// SEALED field bytes churned.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretRecordUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub record_id: uuid::Uuid,
    pub fields_rotated: bool,
}

/// `secret.record_deleted` — the record and its sealed fields are gone.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretRecordDeleted {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub record_id: uuid::Uuid,
    pub title: String,
}

/// `secret.revealed` — the audit provenance rail for a decrypt: who saw
/// the record's sealed fields is queryable; the payloads carry ids/titles
/// only and NEVER the plaintext or ciphertext.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretRevealed {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub record_id: uuid::Uuid,
    pub title: String,
}

/// `secret.grant_updated` — a (role × folder) deny-by-default grant was
/// upserted in `secret_category_grants`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretGrantUpdated {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub grant_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub role_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub folder_id: uuid::Uuid,
    pub can_read: bool,
    pub can_edit: bool,
}

/// `secret.grant_deleted` — one grant row was revoked; the folder chain
/// falls back to deny-by-default matching.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SecretGrantDeleted {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub role_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub folder_id: uuid::Uuid,
}

/// `agent_skill.created` / `agent_skill.updated` default — the original
/// publish path (0040 lane): a skill slug versioned at max+1. Wave 16
/// lane 9 contract typing; the wire shape is unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentSkillEventPayload {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub skill_id: uuid::Uuid,
    pub slug: String,
    pub version: i32,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub created_by: uuid::Uuid,
}

/// `agent_skill.updated` variant `"enabled"` — the ADR 0009 enable
/// toggle from the distribution lifecycle (deny-by-default gate flips on
/// the ACTIVE version row).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentSkillEnabledPayload {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub skill_id: uuid::Uuid,
    pub slug: String,
    pub version: i32,
    pub enabled: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub updated_by: uuid::Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mention_extraction() {
        let m = extract_mentions("ping @alice and @bob, cc @alice. also @carol@external");
        assert_eq!(m, vec!["alice", "bob", "carol"]);
    }

    #[test]
    fn no_mentions() {
        assert!(extract_mentions("no handles here, email@example.com").is_empty());
    }
}
