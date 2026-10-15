//! The node catalogue — every first-party node, declared once.
//!
//! The catalogue is specced in `docs/reference/workflow-node-catalogue.md`
//! (user-reviewed 2026-10-01) and is deliberately narrow: a node earns its
//! place when two or more realistic workflows would otherwise hand-roll the
//! same thing. Everything the long tail needs is `domain.update` /
//! `domain.query` (Tier B), not 150 more declarations, and the rejected
//! candidates are recorded in the landing record so the next lane does not
//! re-add them.
//!
//! Three rules from the catalogue shape every declaration:
//!
//! * **Every node declares `idempotent`** — creations, sends, messages and
//!   ledger movements are not safe to repeat; state setters and reads are.
//!   `decision.ask` is `false` because a second call costs money and may
//!   answer differently.
//! * **Every action declares what it emits.** The static loop refusal in
//!   the parent module is only as complete as this list: a `flow.waitFor`
//!   whose matched event is producible here can be refused at author time.
//! * **A Tier A action is a thin wrapper.** Its executor path names the
//!   closed domain operation it rides, so a wrapper and `domain.update`
//!   can never drift.
//!
//! What is NOT here, on purpose: the four internal decision surfaces
//! (`decision_search_rerank`, `decision_llm_verify`, `decision_tool_routing`,
//! `decision_loop_termination`), the tool-calling edge, raw SQL, a generic
//! HTTP node, a secret read, sub-workflow invocation, an unbounded loop and
//! a schedule node. `decision.ask`'s `surface` enum cannot name an internal
//! surface, which is what makes that refusal structural rather than a check
//! someone has to remember to run.

use super::{AiEdge, DomainOp, ExecutorPath, NodeFamily, NodeTier, NodeType};

// ---------------------------------------------------------------------------
// Triggers (NodeFamily::Trigger, Tier A, ExecutorPath::Trigger)
//
// The offered cut is a decision, not the whole registry: the domain
// publishes 286 event types and offering configuration churn
// (`board.status_added`, `setting.updated`, every `*.created`/`*.updated`
// pair on boards, roles and tags) invites a workflow that breaks the first
// time a status is renamed. The cut below is the moments a user actually
// thinks in; anything else is reachable through the event-type registry
// when an org holds the grant, and the landing record says so explicitly.
// ---------------------------------------------------------------------------

declare_nodes! {
    /// Fires on any ticket-lifecycle moment: created, status changed,
    /// assigned, closed, reverted, board moved, priority changed, tags
    /// changed, commented, a field set. One node with the moment picked as
    /// a parameter, because a workflow author thinks in the moment, not in
    /// the event string.
    TRIGGER_TICKET: NodeType::new(
        "trigger.ticket",
        "Ticket lifecycle",
        "Starts a run when something happens to a ticket: it is created, its status or priority changes, it is assigned, closed, reverted, moved to another board, tagged, commented on, or a field is set. Choose the moment; an optional board filter narrows it to one board.",
        NodeFamily::Trigger,
        NodeTier::A,
        ExecutorPath::Trigger,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"eventType":{"type":"string","enum":["ticket.created","ticket.status_changed","ticket.assigned","ticket.closed","ticket.reverted","ticket.board_moved","ticket.updated","ticket.tags_changed","ticket.comment_added","ticket.field_set"]},"boardId":{"type":"string","format":"uuid"}},"required":["eventType"]}"#)
    .output(r#"{"type":"object","properties":{"eventType":{"type":"string"},"eventId":{"type":"string","format":"uuid"},"ticketId":{"type":"string","format":"uuid"},"boardId":{"type":"string","format":"uuid"},"clientId":{"type":"string","format":"uuid"},"subject":{"type":"object"},"payload":{"type":"object"}}}"#),

    /// Client and site records being created or changed.
    TRIGGER_CLIENT: NodeType::new(
        "trigger.client",
        "Client or site",
        "Starts a run when a client is created or updated, a site is created under a client, or a contact is added.",
        NodeFamily::Trigger,
        NodeTier::A,
        ExecutorPath::Trigger,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"eventType":{"type":"string","enum":["client.created","client.updated","site.created","contact.created"]}},"required":["eventType"]}"#)
    .output(r#"{"type":"object","properties":{"eventType":{"type":"string"},"eventId":{"type":"string","format":"uuid"},"clientId":{"type":"string","format":"uuid"},"siteId":{"type":"string","format":"uuid"},"subject":{"type":"object"},"payload":{"type":"object"}}}"#),

    /// Time and attendance: timers and the approval lifecycle on entries.
    TRIGGER_TIME: NodeType::new(
        "trigger.time",
        "Time and attendance",
        "Starts a run on a timer start or stop, a time entry being created, or a time entry moving through approval (submitted, approved, rejected).",
        NodeFamily::Trigger,
        NodeTier::A,
        ExecutorPath::Trigger,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"eventType":{"type":"string","enum":["timer.started","timer.stopped","time_entry.created","time_entry.updated","time_entry.approval_submitted","time_entry.approval_approved","time_entry.approval_rejected"]}},"required":["eventType"]}"#)
    .output(r#"{"type":"object","properties":{"eventType":{"type":"string"},"eventId":{"type":"string","format":"uuid"},"timeEntryId":{"type":"string","format":"uuid"},"userId":{"type":"string","format":"uuid"},"subject":{"type":"object"},"payload":{"type":"object"}}}"#),

    /// Money: quotes, invoices, the run that posts them, credit notes and
    /// contracts.
    TRIGGER_MONEY: NodeType::new(
        "trigger.money",
        "Quotes and invoices",
        "Starts a run when a quote is accepted, converted or declined, an invoice is created, paid or voided, dunning posture changes, an invoice run completes, a credit note is issued, or a contract is created.",
        NodeFamily::Trigger,
        NodeTier::A,
        ExecutorPath::Trigger,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"eventType":{"type":"string","enum":["quote.accepted","quote.converted","quote.declined","invoice.created","invoice.payment_recorded","invoice.voided","invoice.dunning_changed","invoice_run.completed","credit_note.issued","contract.created"]}},"required":["eventType"]}"#)
    .output(r#"{"type":"object","properties":{"eventType":{"type":"string"},"eventId":{"type":"string","format":"uuid"},"invoiceId":{"type":"string","format":"uuid"},"quoteId":{"type":"string","format":"uuid"},"clientId":{"type":"string","format":"uuid"},"subject":{"type":"object"},"payload":{"type":"object"}}}"#),

    /// Scheduling: appointments, requests, dispatch and on-call.
    TRIGGER_SCHEDULING: NodeType::new(
        "trigger.scheduling",
        "Scheduling and dispatch",
        "Starts a run when an appointment is created or cancelled, an appointment request is created, approved or declined, dispatch status or the queue changes, or the on-call roster is updated.",
        NodeFamily::Trigger,
        NodeTier::A,
        ExecutorPath::Trigger,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"eventType":{"type":"string","enum":["appointment.created","appointment.updated","appointment.cancelled","appointment_request.created","appointment_request.approved","appointment_request.declined","dispatch.status_updated","dispatch.queue_updated","on_call.updated"]}},"required":["eventType"]}"#)
    .output(r#"{"type":"object","properties":{"eventType":{"type":"string"},"eventId":{"type":"string","format":"uuid"},"appointmentId":{"type":"string","format":"uuid"},"subject":{"type":"object"},"payload":{"type":"object"}}}"#),

    /// Field service: inbound alerts, their acknowledgement and resolution,
    /// and a submitted travel draft.
    TRIGGER_FIELD_SERVICE: NodeType::new(
        "trigger.fieldService",
        "Alerts and travel",
        "Starts a run when an alert is ingested by a signed inbound source, acknowledged or resolved, or a travel/mileage draft is submitted.",
        NodeFamily::Trigger,
        NodeTier::A,
        ExecutorPath::Trigger,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"eventType":{"type":"string","enum":["alert.ingested","alert.acknowledged","alert.resolved","travel_draft.submitted"]}},"required":["eventType"]}"#)
    .output(r#"{"type":"object","properties":{"eventType":{"type":"string"},"eventId":{"type":"string","format":"uuid"},"alertId":{"type":"string","format":"uuid"},"boardId":{"type":"string","format":"uuid"},"subject":{"type":"object"},"payload":{"type":"object"}}}"#),

    /// Quality: a recorded CSAT response and a fired SLA escalation.
    TRIGGER_QUALITY: NodeType::new(
        "trigger.quality",
        "CSAT and SLA",
        "Starts a run when a CSAT response is recorded or an SLA escalation fires.",
        NodeFamily::Trigger,
        NodeTier::A,
        ExecutorPath::Trigger,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"eventType":{"type":"string","enum":["csat.response_recorded","sla.escalation_fired"]}},"required":["eventType"]}"#)
    .output(r#"{"type":"object","properties":{"eventType":{"type":"string"},"eventId":{"type":"string","format":"uuid"},"ticketId":{"type":"string","format":"uuid"},"subject":{"type":"object"},"payload":{"type":"object"}}}"#),

    /// An event from an external system, arriving through the generic
    /// signed ingest (Wave 43 lane 1) with its type registered in the
    /// event-type registry (lane 2). This is the inbound leg the RMM
    /// example rides: the connector HMACs a POST and needs no surface
    /// declaration at all.
    TRIGGER_WEBHOOK: NodeType::new(
        "trigger.webhook",
        "External event",
        "Starts a run when an external system posts a signed envelope to the generic ingest — a monitoring device going offline, a payment provider settling, anything a connector declares. The type must already be registered, and an org without the grant gets a typed refusal rather than a silent run.",
        NodeFamily::Trigger,
        NodeTier::A,
        ExecutorPath::Trigger,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"eventType":{"type":"string","maxLength":160},"boardId":{"type":"string","format":"uuid"}},"required":["eventType"]}"#)
    .output(r#"{"type":"object","properties":{"eventType":{"type":"string"},"eventId":{"type":"string","format":"uuid"},"sourceRef":{"type":"string"},"boardId":{"type":"string","format":"uuid"},"subject":{"type":"object"},"payload":{"type":"object"}}}"#),

    /// The typed schedule trigger: a cron-shaped wake with its own
    /// delivery record, already shipped as a first-class surface. There is
    /// deliberately no schedule CONTROL node — scheduling is a trigger.
    TRIGGER_SCHEDULE: NodeType::new(
        "trigger.schedule",
        "Schedule",
        "Starts a run on a schedule the org owns, with the typed delivery record the schedule surface already keeps. Use this for the 'the RMM has no push API' case: schedule a read, then branch on what came back.",
        NodeFamily::Trigger,
        NodeTier::A,
        ExecutorPath::Trigger,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"scheduleId":{"type":"string","format":"uuid"}},"required":["scheduleId"]}"#)
    .output(r#"{"type":"object","properties":{"eventType":{"type":"string"},"eventId":{"type":"string","format":"uuid"},"scheduleId":{"type":"string","format":"uuid"},"subject":{"type":"object"},"payload":{"type":"object"}}}"#),

    /// An operator pressing the button: the test-run path, and the honest
    /// way to exercise a workflow before wiring a trigger to it.
    TRIGGER_MANUAL: NodeType::new(
        "trigger.manual",
        "Manual run",
        "Starts a run when an operator presses the button — the way to test a workflow, and the way to run one that has no natural event.",
        NodeFamily::Trigger,
        NodeTier::A,
        ExecutorPath::Trigger,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"subject":{"type":"object"}},"required":["subject"]}"#)
    .output(r#"{"type":"object","properties":{"eventType":{"type":"string"},"eventId":{"type":"string","format":"uuid"},"userId":{"type":"string","format":"uuid"},"subject":{"type":"object"},"payload":{"type":"object"}}}"#),

    /// An inbound email. The scan and inbound routes already record one;
    /// this is the workflow's view of the same moment.
    TRIGGER_EMAIL: NodeType::new(
        "trigger.email",
        "Email received",
        "Starts a run when an email arrives — a support address, a reply to a quote. The message is already scanned and recorded by the inbound mail pipeline before a run sees it.",
        NodeFamily::Trigger,
        NodeTier::A,
        ExecutorPath::Trigger,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"mailbox":{"type":"string","maxLength":320}}}"#)
    .output(r#"{"type":"object","properties":{"eventType":{"type":"string"},"eventId":{"type":"string","format":"uuid"},"messageId":{"type":"string"},"subject":{"type":"object"},"payload":{"type":"object"}}}"#),

// ---------------------------------------------------------------------------
// The Decision Engine and the AI edges
//
// The six surfaces below are the user-facing half of the ten that exist.
// The four internal ones (`decision_search_rerank`, `decision_llm_verify`,
// `decision_tool_routing`, `decision_loop_termination`) are not nameable
// here — the enum is the refusal, so it cannot be forgotten at runtime.
// The two engine surfaces (`decision_automation_conditions`,
// `decision_workbench`) are excluded with the same mechanism: the first is
// the existing `ai_decision` CONDITION, which gates on already-recorded
// answers and must not become a second live-call path, and the second is
// the workbench assistant, which is not a workflow.
// ---------------------------------------------------------------------------

    /// One call, a batch of atomic questions, every answer separately
    /// addressable. The engine refuses composite questions by design, so
    /// asking four things at once is four answers, not one vague one.
    DECISION_ASK: NodeType::new(
        "decision.ask",
        "Ask the decision engine",
        "Sends a composed context plus a set of atomic questions to one configured decision surface in a single call and returns every answer separately addressable — picked option label, the full distribution, confidence, and a verdict computed per question from that question's own act/review thresholds. Ask several related things together: the answers correlate, and four separate calls are four blind spots. Not safe to repeat (a second call costs money and may answer differently), so flow.retry refuses it.",
        NodeFamily::Decision,
        NodeTier::A,
        ExecutorPath::Ai(AiEdge::Decision),
    )
    .idempotent(false)
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"surface":{"type":"string","enum":["decision_ticket_triage","decision_alert_triage","decision_dispatch","decision_billing_audit","decision_contract_scope","decision_record_dedupe"]},"questionSet":{"type":"object","additionalProperties":false,"properties":{"source":{"type":"string","enum":["surface_config","workflow"]},"id":{"type":"string","format":"uuid"}},"required":["source"]},"context":{"type":"object"},"subject":{"type":"object","additionalProperties":false,"properties":{"entityType":{"type":"string"},"entityId":{"type":"string","format":"uuid"}},"required":["entityType"]}},"required":["surface","questionSet"]}"#)
    .output(r#"{"type":"object","properties":{"answers":{"type":"array","items":{"type":"object"}},"questionIds":{"type":"array","items":{"type":"string"}},"verdicts":{"type":"object"},"cost":{"type":"integer"}}}"#),

    /// Transcribe an attachment — an email with a voice note becomes a
    /// ticket the workflow can write.
    AI_TRANSCRIPTION: NodeType::new(
        "ai.transcription",
        "Transcribe attachment",
        "Transcribes an audio or video attachment into text, so a voicemail or a recorded site clip can be routed, parsed or turned into a ticket. Cheap completion call behind a narrow surface.",
        NodeFamily::Decision,
        NodeTier::A,
        ExecutorPath::Ai(AiEdge::Transcription),
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"attachmentId":{"type":"string","format":"uuid"},"language":{"type":"string","maxLength":16}},"required":["attachmentId"]}"#)
    .output(r#"{"type":"object","properties":{"text":{"type":"string"},"durationSeconds":{"type":"integer"},"language":{"type":"string"},"cost":{"type":"integer"}}}"#),

    /// The free-text completion edge, behind an explicit org grant. The
    /// most open-ended thing in the system, which is exactly why its
    /// output is never auto-actioned without a decision node downstream.
    AI_COMPLETION: NodeType::new(
        "ai.completion",
        "Free-text completion",
        "Writes text from a prompt and context — a summary, a drafted reply, a category name. The most open-ended capability in the product, so it only runs where the org has granted the surface, and its output should never drive an action without a decision node downstream judging it.",
        NodeFamily::Decision,
        NodeTier::A,
        ExecutorPath::Ai(AiEdge::Completion),
    )
    .idempotent(false)
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"prompt":{"type":"string","minLength":1,"maxLength":16000},"context":{"type":"object"},"maxTokens":{"type":"integer","minimum":1,"maximum":8000}},"required":["prompt"]}"#)
    .output(r#"{"type":"object","properties":{"text":{"type":"string"},"finishReason":{"type":"string"},"cost":{"type":"integer"}}}"#),

// ---------------------------------------------------------------------------
// Conditions (NodeFamily::Condition, Tier A, ExecutorPath::Condition)
//
// Conditions answer a question; flow.branch and flow.switch decide where
// the answer sends the run. `field.equals` is the load-bearing one: every
// entity's fields come from the registry, so a client field added next
// year works today without a new condition node. Nothing here writes.
// ---------------------------------------------------------------------------

    CONDITION_FIELD_EQUALS: NodeType::new(
        "field.equals",
        "Field equals",
        "True when a named field of the subject or an upstream node's output equals a given value. The generic one: any registry-declared field on any entity, so a new field needs no new node.",
        NodeFamily::Condition,
        NodeTier::A,
        ExecutorPath::Condition,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"field":{"type":"string","minLength":1,"maxLength":200},"value":{}},"required":["field","value"]}"#)
    .output(r#"{"type":"object","properties":{"matched":{"type":"boolean"},"field":{"type":"string"},"value":{}}}"#),

    CONDITION_FIELD_NOT_EQUALS: NodeType::new(
        "field.notEquals",
        "Field does not equal",
        "True when a named field is anything other than a given value. Use for 'status is not Closed' guards.",
        NodeFamily::Condition,
        NodeTier::A,
        ExecutorPath::Condition,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"field":{"type":"string","minLength":1,"maxLength":200},"value":{}},"required":["field","value"]}"#)
    .output(r#"{"type":"object","properties":{"matched":{"type":"boolean"},"field":{"type":"string"},"value":{}}}"#),

    CONDITION_FIELD_CONTAINS: NodeType::new(
        "field.contains",
        "Field contains",
        "True when a text field contains a substring, case-insensitively. Use for title and body matching, not for exact values.",
        NodeFamily::Condition,
        NodeTier::A,
        ExecutorPath::Condition,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"field":{"type":"string","minLength":1,"maxLength":200},"needle":{"type":"string","maxLength":2000}},"required":["field","needle"]}"#)
    .output(r#"{"type":"object","properties":{"matched":{"type":"boolean"},"field":{"type":"string"},"needle":{"type":"string"}}}"#),

    CONDITION_FIELD_IS_EMPTY: NodeType::new(
        "field.isEmpty",
        "Field is empty",
        "True when a field is absent, null, an empty string or an empty collection. Use for 'nobody has picked this up yet'.",
        NodeFamily::Condition,
        NodeTier::A,
        ExecutorPath::Condition,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"field":{"type":"string","minLength":1,"maxLength":200}},"required":["field"]}"#)
    .output(r#"{"type":"object","properties":{"matched":{"type":"boolean"},"field":{"type":"string"}}}"#),

    CONDITION_NUMBER_GT: NodeType::new(
        "number.gt",
        "Number greater than",
        "True when a numeric field is strictly greater than a value.
        Covers SLA minutes remaining, stock level, hours logged, quote totals and any probability from a decision answer.",
        NodeFamily::Condition,
        NodeTier::A,
        ExecutorPath::Condition,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"field":{"type":"string","minLength":1,"maxLength":200},"value":{"type":"number"}},"required":["field","value"]}"#)
    .output(r#"{"type":"object","properties":{"matched":{"type":"boolean"},"field":{"type":"string"},"value":{"type":"number"}}}"#),

    CONDITION_NUMBER_GTE: NodeType::new(
        "number.gte",
        "Number at least",
        "True when a numeric field is greater than or equal to a value. Use for 'at or past the reorder point'.",
        NodeFamily::Condition,
        NodeTier::A,
        ExecutorPath::Condition,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"field":{"type":"string","minLength":1,"maxLength":200},"value":{"type":"number"}},"required":["field","value"]}"#)
    .output(r#"{"type":"object","properties":{"matched":{"type":"boolean"},"field":{"type":"string"},"value":{"type":"number"}}}"#),

    CONDITION_NUMBER_LT: NodeType::new(
        "number.lt",
        "Number less than",
        "True when a numeric field is strictly less than a value. Use for 'under an hour of buffer left'.",
        NodeFamily::Condition,
        NodeTier::A,
        ExecutorPath::Condition,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"field":{"type":"string","minLength":1,"maxLength":200},"value":{"type":"number"}},"required":["field","value"]}"#)
    .output(r#"{"type":"object","properties":{"matched":{"type":"boolean"},"field":{"type":"string"},"value":{"type":"number"}}}"#),

    CONDITION_NUMBER_LTE: NodeType::new(
        "number.lte",
        "Number at most",
        "True when a numeric field is less than or equal to a value. Use for 'two or fewer spares on the shelf'.",
        NodeFamily::Condition,
        NodeTier::A,
        ExecutorPath::Condition,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"field":{"type":"string","minLength":1,"maxLength":200},"value":{"type":"number"}},"required":["field","value"]}"#)
    .output(r#"{"type":"object","properties":{"matched":{"type":"boolean"},"field":{"type":"string"},"value":{"type":"number"}}}"#),

    CONDITION_NUMBER_BETWEEN: NodeType::new(
        "number.between",
        "Number between",
        "True when a numeric field sits inside an inclusive range.
        One node instead of two comparisons chained by flow.branch.",
        NodeFamily::Condition,
        NodeTier::A,
        ExecutorPath::Condition,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"field":{"type":"string","minLength":1,"maxLength":200},"min":{"type":"number"},"max":{"type":"number"}},"required":["field","min","max"]}"#)
    .output(r#"{"type":"object","properties":{"matched":{"type":"boolean"},"field":{"type":"string"},"min":{"type":"number"},"max":{"type":"number"}}}"#),

    CONDITION_DATE_BEFORE: NodeType::new(
        "date.isBefore",
        "Date is before",
        "True when a timestamp field is before an instant. Use for 'due date is already past'.",
        NodeFamily::Condition,
        NodeTier::A,
        ExecutorPath::Condition,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"field":{"type":"string","minLength":1,"maxLength":200},"at":{"type":"string"}},"required":["field","at"]}"#)
    .output(r#"{"type":"object","properties":{"matched":{"type":"boolean"},"field":{"type":"string"},"at":{"type":"string"}}}"#),

    CONDITION_DATE_AFTER: NodeType::new(
        "date.isAfter",
        "Date is after",
        "True when a timestamp field is after an instant. Use for 'approved since the last sweep'.",
        NodeFamily::Condition,
        NodeTier::A,
        ExecutorPath::Condition,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"field":{"type":"string","minLength":1,"maxLength":200},"at":{"type":"string"}},"required":["field","at"]}"#)
    .output(r#"{"type":"object","properties":{"matched":{"type":"boolean"},"field":{"type":"string"},"at":{"type":"string"}}}"#),

    CONDITION_DATE_WITHIN_LAST: NodeType::new(
        "date.withinLast",
        "Date within last",
        "True when a timestamp field falls inside a relative window such as 2h or 3d.
        The relative form of 'recently' that survives a workflow being re-run.",
        NodeFamily::Condition,
        NodeTier::A,
        ExecutorPath::Condition,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"field":{"type":"string","minLength":1,"maxLength":200},"window":{"type":"string","maxLength":32}},"required":["field","window"]}"#)
    .output(r#"{"type":"object","properties":{"matched":{"type":"boolean"},"field":{"type":"string"},"window":{"type":"string"}}}"#),

    CONDITION_DATE_OVERDUE: NodeType::new(
        "date.isOverdue",
        "Date is overdue",
        "True when a due timestamp has passed.
        Reads the entity's own clock; there is no 'overdue by how much' argument because that is number.gt on the remaining minutes.",
        NodeFamily::Condition,
        NodeTier::A,
        ExecutorPath::Condition,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"field":{"type":"string","minLength":1,"maxLength":200}},"required":["field"]}"#)
    .output(r#"{"type":"object","properties":{"matched":{"type":"boolean"},"field":{"type":"string"}}}"#),

    CONDITION_LIST_CONTAINS: NodeType::new(
        "list.contains",
        "List contains",
        "True when a collection field contains a given value. Use for 'the on-call roster includes this tech'.",
        NodeFamily::Condition,
        NodeTier::A,
        ExecutorPath::Condition,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"field":{"type":"string","minLength":1,"maxLength":200},"value":{}},"required":["field","value"]}"#)
    .output(r#"{"type":"object","properties":{"matched":{"type":"boolean"},"field":{"type":"string"}}}"#),

    CONDITION_LIST_LENGTH_GTE: NodeType::new(
        "list.length.gte",
        "List length at least",
        "True when a collection holds at least N items. Use for 'more than one candidate site matched'.",
        NodeFamily::Condition,
        NodeTier::A,
        ExecutorPath::Condition,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"field":{"type":"string","minLength":1,"maxLength":200},"length":{"type":"integer","minimum":0}},"required":["field","length"]}"#)
    .output(r#"{"type":"object","properties":{"matched":{"type":"boolean"},"field":{"type":"string"},"length":{"type":"integer"}}}"#),

    CONDITION_LIST_IS_EMPTY: NodeType::new(
        "list.isEmpty",
        "List is empty",
        "True when a collection holds nothing. Use for 'no open tickets at this site yet' before creating one.",
        NodeFamily::Condition,
        NodeTier::A,
        ExecutorPath::Condition,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"field":{"type":"string","minLength":1,"maxLength":200}},"required":["field"]}"#)
    .output(r#"{"type":"object","properties":{"matched":{"type":"boolean"},"field":{"type":"string"}}}"#),

    /// Gates on an answer the engine has ALREADY recorded. Deliberately
    /// distinct from `decision.ask`: this one spends nothing and reads
    /// what a previous run decided, which is the right posture for a
    /// cheap deterministic gate.
    CONDITION_DECISION_MATCHES: NodeType::new(
        "decision.matches",
        "Recorded decision matches",
        "True when the newest recorded answer for a surface and question satisfies a comparison against a threshold or a confidence path. Reads answers a run already bought — no model call. Missing or stale answers match nothing, so a gate never fires on a guess. For a live call, branch on decision.ask's outputs instead.",
        NodeFamily::Condition,
        NodeTier::A,
        ExecutorPath::Condition,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"surface":{"type":"string","minLength":1,"maxLength":160},"question":{"type":"string","minLength":1,"maxLength":200},"operator":{"type":"string","enum":["gte","lte","eq","neq","confidence_gte"]},"threshold":{"type":"number"},"confidencePath":{"type":"string","maxLength":200}},"required":["surface","question","operator"]}"#)
    .output(r#"{"type":"object","properties":{"matched":{"type":"boolean"},"surface":{"type":"string"},"question":{"type":"string"},"answer":{}}}"#),

    /// The explicit fallthrough, so a branch never dangles.
    CONDITION_ELSE: NodeType::new(
        "condition.else",
        "Otherwise",
        "Always true. Attach it as the last arm of a branch so the graph never ends in a path that goes nowhere.",
        NodeFamily::Condition,
        NodeTier::A,
        ExecutorPath::Condition,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{}}"#)
    .output(r#"{"type":"object","properties":{"matched":{"type":"boolean"}}}"#),

// ---------------------------------------------------------------------------
// Tier A actions — tickets (NodeFamily::Action, ExecutorPath::DomainWrapper)
//
// Every wrapper below is a thin validation layer over the same executor
// path `domain.update` rides; the operation it names is in the constructor,
// so a wrapper can never drift from the generic path. The `idempotent`
// flag and the emitted event both come from that operation.
// ---------------------------------------------------------------------------

    ACTION_TICKET_CREATE: NodeType::new(
        "ticket.create",
        "Create ticket",
        "Creates a ticket on a board, optionally for a client and site, with a title, description, priority and assignee. Emits ticket.created, so a workflow that waits for a ticket it just opened must not wait for this one.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TicketCreate),
    )
    .idempotent(DomainOp::TicketCreate.idempotent())
    .emits(&["ticket.created"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"boardId":{"type":"string","format":"uuid"},"title":{"type":"string","minLength":1,"maxLength":300},"description":{"type":"string","maxLength":20000},"clientId":{"type":"string","format":"uuid"},"siteId":{"type":"string","format":"uuid"},"priority":{"type":"string","enum":["p1","p2","p3","p4"]},"assigneeId":{"type":"string","format":"uuid"},"category":{"type":"string","maxLength":120},"tags":{"type":"array","items":{"type":"string"},"maxItems":32},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["boardId","title"]}"#)
    .output(r#"{"type":"object","properties":{"ticketId":{"type":"string","format":"uuid"},"boardId":{"type":"string","format":"uuid"},"status":{"type":"string"}}}"#),

    ACTION_TICKET_UPDATE: NodeType::new(
        "ticket.update",
        "Update ticket",
        "Updates fields on an existing ticket. A safe-to-repeat state setter: running it twice leaves the same values set.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TicketUpdate),
    )
    .idempotent(DomainOp::TicketUpdate.idempotent())
    .emits(&["ticket.updated"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"fields":{"type":"object"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["fields"]}"#)
    .output(r#"{"type":"object","properties":{"ticketId":{"type":"string","format":"uuid"},"changed":{"type":"array","items":{"type":"string"}}}}"#),

    ACTION_TICKET_TRANSITION: NodeType::new(
        "ticket.transition",
        "Change ticket status",
        "Moves a ticket along its board's transition graph to a named status. A refused transition is a typed error naming the transitions that were actually available, not a silent no-op.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TicketTransition),
    )
    .idempotent(DomainOp::TicketTransition.idempotent())
    .emits(&["ticket.status_changed"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"status":{"type":"string","minLength":1,"maxLength":80},"comment":{"type":"string","maxLength":20000},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["status"]}"#)
    .output(r#"{"type":"object","properties":{"ticketId":{"type":"string","format":"uuid"},"status":{"type":"string"}}}"#),

    ACTION_TICKET_ASSIGN: NodeType::new(
        "ticket.assign",
        "Assign ticket",
        "Assigns a ticket to a user, or clears the assignment when no assignee is given. The same effect the rule builder's assign step rides.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TicketAssign),
    )
    .idempotent(DomainOp::TicketAssign.idempotent())
    .emits(&["ticket.assigned"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"userId":{"type":"string","format":"uuid"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":[]}"#)
    .output(r#"{"type":"object","properties":{"ticketId":{"type":"string","format":"uuid"},"assigneeId":{"type":"string","format":"uuid"}}}"#),

    ACTION_TICKET_COMMENT: NodeType::new(
        "ticket.comment",
        "Add comment",
        "Adds a public comment to a ticket, visible to the client where the board's portal visibility allows it. Not safe to repeat: a second run is a second comment.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TicketComment),
    )
    .idempotent(DomainOp::TicketComment.idempotent())
    .emits(&["ticket.comment_added"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"body":{"type":"string","minLength":1,"maxLength":20000},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["body"]}"#)
    .output(r#"{"type":"object","properties":{"ticketId":{"type":"string","format":"uuid"},"commentId":{"type":"string","format":"uuid"}}}"#),

    ACTION_TICKET_INTERNAL_NOTE: NodeType::new(
        "ticket.internalNote",
        "Add internal note",
        "Adds an internal note, never portal-visible. Not safe to repeat: a second run is a second note.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TicketInternalNote),
    )
    .idempotent(DomainOp::TicketInternalNote.idempotent())
    .emits(&["ticket.comment_added"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"body":{"type":"string","minLength":1,"maxLength":20000},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["body"]}"#)
    .output(r#"{"type":"object","properties":{"ticketId":{"type":"string","format":"uuid"},"commentId":{"type":"string","format":"uuid"}}}"#),

    ACTION_TICKET_SET_PRIORITY: NodeType::new(
        "ticket.setPriority",
        "Set priority",
        "Sets a ticket's priority to one of the four tiers. A safe-to-repeat state setter, and the effect the rule builder's set-priority step rides.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TicketSetPriority),
    )
    .idempotent(DomainOp::TicketSetPriority.idempotent())
    .emits(&["ticket.updated"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"priority":{"type":"string","enum":["p1","p2","p3","p4"]},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["priority"]}"#)
    .output(r#"{"type":"object","properties":{"ticketId":{"type":"string","format":"uuid"},"priority":{"type":"string"}}}"#),

    ACTION_TICKET_SET_TAGS: NodeType::new(
        "ticket.setTags",
        "Set tags",
        "Replaces a ticket's tags with the given list. Setters replace rather than add, so a repeated run leaves the same set.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TicketSetTags),
    )
    .idempotent(DomainOp::TicketSetTags.idempotent())
    .emits(&["ticket.tags_changed"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"tags":{"type":"array","items":{"type":"string","maxLength":60},"maxItems":32},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["tags"]}"#)
    .output(r#"{"type":"object","properties":{"ticketId":{"type":"string","format":"uuid"},"tags":{"type":"array","items":{"type":"string"}}}}"#),

    ACTION_TICKET_ADD_CHECKLIST_ITEM: NodeType::new(
        "ticket.addChecklistItem",
        "Add checklist item",
        "Appends one item to a ticket's checklist. Not safe to repeat: a second run is a second item.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TicketAddChecklistItem),
    )
    .idempotent(DomainOp::TicketAddChecklistItem.idempotent())
    .emits(&["ticket.checklist_updated"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"text":{"type":"string","minLength":1,"maxLength":500},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["text"]}"#)
    .output(r#"{"type":"object","properties":{"ticketId":{"type":"string","format":"uuid"},"itemId":{"type":"string","format":"uuid"}}}"#),

    ACTION_TICKET_COMPLETE_CHECKLIST_ITEM: NodeType::new(
        "ticket.completeChecklistItem",
        "Complete checklist item",
        "Marks one checklist item done. Setting a state twice leaves it set, so a repeated run is harmless.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TicketCompleteChecklistItem),
    )
    .idempotent(DomainOp::TicketCompleteChecklistItem.idempotent())
    .emits(&["ticket.checklist_updated"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"itemId":{"type":"string","format":"uuid"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["itemId"]}"#)
    .output(r#"{"type":"object","properties":{"ticketId":{"type":"string","format":"uuid"},"itemId":{"type":"string","format":"uuid"},"done":{"type":"boolean"}}}"#),

    ACTION_TICKET_LINK: NodeType::new(
        "ticket.link",
        "Link tickets",
        "Links two tickets as related. A relation is a row, so a repeated run is a second row — not safe to repeat.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TicketLink),
    )
    .idempotent(DomainOp::TicketLink.idempotent())
    .emits(&["ticket.linked"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"otherId":{"type":"string","format":"uuid"},"relation":{"type":"string","enum":["related","duplicate_of","blocks","blocked_by","caused_by"]},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["otherId"]}"#)
    .output(r#"{"type":"object","properties":{"ticketId":{"type":"string","format":"uuid"},"otherId":{"type":"string","format":"uuid"}}}"#),

    ACTION_TICKET_MERGE: NodeType::new(
        "ticket.merge",
        "Merge tickets",
        "Merges one ticket into another, carrying comments, time and attachments across. Not safe to repeat: a second merge double-carries whatever the first one moved.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TicketMerge),
    )
    .idempotent(DomainOp::TicketMerge.idempotent())
    .emits(&["ticket.merged"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"intoId":{"type":"string","format":"uuid"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["intoId"]}"#)
    .output(r#"{"type":"object","properties":{"ticketId":{"type":"string","format":"uuid"},"intoId":{"type":"string","format":"uuid"}}}"#),

    ACTION_TICKET_MOVE_TO_BOARD: NodeType::new(
        "ticket.moveToBoard",
        "Move to board",
        "Moves a ticket to a different board, re-resolving its status against that board's graph.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TicketMoveToBoard),
    )
    .idempotent(DomainOp::TicketMoveToBoard.idempotent())
    .emits(&["ticket.board_moved"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"boardId":{"type":"string","format":"uuid"},"status":{"type":"string","maxLength":80},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["boardId"]}"#)
    .output(r#"{"type":"object","properties":{"ticketId":{"type":"string","format":"uuid"},"boardId":{"type":"string","format":"uuid"},"status":{"type":"string"}}}"#),

    ACTION_TICKET_SET_FIELD: NodeType::new(
        "ticket.setField",
        "Set custom field",
        "Sets one custom field on a ticket by key. The generic field write: a new custom field needs no new node.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TicketSetField),
    )
    .idempotent(DomainOp::TicketSetField.idempotent())
    .emits(&["ticket.field_set"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"key":{"type":"string","minLength":1,"maxLength":120},"value":{},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["key"]}"#)
    .output(r#"{"type":"object","properties":{"ticketId":{"type":"string","format":"uuid"},"key":{"type":"string"}}}"#),

// ---------------------------------------------------------------------------
// Tier A actions — time and clients
// ---------------------------------------------------------------------------

    ACTION_TIME_ENTRY_LOG: NodeType::new(
        "timeEntry.log",
        "Log time",
        "Logs worked time against a ticket for a user over a period, at a billable or non-billable rate. Not safe to repeat: a second run is a second entry and a second invoice line.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TimeEntryLog),
    )
    .idempotent(DomainOp::TimeEntryLog.idempotent())
    .emits(&["time_entry.created"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"userId":{"type":"string","format":"uuid"},"minutes":{"type":"integer","minimum":1},"startedAt":{"type":"string"},"billable":{"type":"boolean"},"note":{"type":"string","maxLength":2000},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["minutes"]}"#)
    .output(r#"{"type":"object","properties":{"timeEntryId":{"type":"string","format":"uuid"},"minutes":{"type":"integer"}}}"#),

    ACTION_TIME_ENTRY_UPDATE: NodeType::new(
        "timeEntry.update",
        "Update time entry",
        "Updates fields on an existing time entry. A safe-to-repeat state setter.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TimeEntryUpdate),
    )
    .idempotent(DomainOp::TimeEntryUpdate.idempotent())
    .emits(&["time_entry.updated"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"timeEntryId":{"type":"string","format":"uuid"},"fields":{"type":"object"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["timeEntryId","fields"]}"#)
    .output(r#"{"type":"object","properties":{"timeEntryId":{"type":"string","format":"uuid"},"changed":{"type":"array","items":{"type":"string"}}}}"#),

    ACTION_TIME_ENTRY_SUBMIT_FOR_APPROVAL: NodeType::new(
        "timeEntry.submitForApproval",
        "Submit time for approval",
        "Submits time entries for approval. A state move, safe to repeat, and the trigger the approval lifecycle waits on.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TimeEntrySubmitForApproval),
    )
    .idempotent(DomainOp::TimeEntrySubmitForApproval.idempotent())
    .emits(&["time_entry.approval_submitted"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"timeEntryIds":{"type":"array","items":{"type":"string","format":"uuid"},"minItems":1},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["timeEntryIds"]}"#)
    .output(r#"{"type":"object","properties":{"submitted":{"type":"integer"}}}"#),

    ACTION_TIMER_START: NodeType::new(
        "timer.start",
        "Start timer",
        "Starts a running timer for a user on a ticket. Not safe to repeat: a second start is a second timer.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TimerStart),
    )
    .idempotent(DomainOp::TimerStart.idempotent())
    .emits(&["timer.started"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"userId":{"type":"string","format":"uuid"},"note":{"type":"string","maxLength":2000},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":[]}"#)
    .output(r#"{"type":"object","properties":{"timerId":{"type":"string","format":"uuid"},"startedAt":{"type":"string"}}}"#),

    ACTION_TIMER_STOP: NodeType::new(
        "timer.stop",
        "Stop timer",
        "Stops a running timer and books the elapsed time. Safe to repeat once nothing is running.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TimerStop),
    )
    .idempotent(DomainOp::TimerStop.idempotent())
    .emits(&["timer.stopped"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"timerId":{"type":"string","format":"uuid"},"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":[]}"#)
    .output(r#"{"type":"object","properties":{"minutes":{"type":"integer"},"timeEntryId":{"type":"string","format":"uuid"}}}"#),

    ACTION_CLIENT_CREATE: NodeType::new(
        "client.create",
        "Create client",
        "Creates a client account. Not safe to repeat: a second run is a second client, and the duplicate is expensive to find later.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::ClientCreate),
    )
    .idempotent(DomainOp::ClientCreate.idempotent())
    .emits(&["client.created"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"name":{"type":"string","minLength":1,"maxLength":200},"externalRef":{"type":"string","maxLength":120},"notes":{"type":"string","maxLength":20000},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["name"]}"#)
    .output(r#"{"type":"object","properties":{"clientId":{"type":"string","format":"uuid"},"name":{"type":"string"}}}"#),

    ACTION_CLIENT_UPDATE: NodeType::new(
        "client.update",
        "Update client",
        "Updates fields on an existing client. A safe-to-repeat state setter.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::ClientUpdate),
    )
    .idempotent(DomainOp::ClientUpdate.idempotent())
    .emits(&["client.updated"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"clientId":{"type":"string","format":"uuid"},"fields":{"type":"object"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["clientId","fields"]}"#)
    .output(r#"{"type":"object","properties":{"clientId":{"type":"string","format":"uuid"},"changed":{"type":"array","items":{"type":"string"}}}}"#),

    ACTION_CLIENT_ADD_NOTE: NodeType::new(
        "client.addNote",
        "Add client note",
        "Appends a note to a client's record. Not safe to repeat: a second run is a second note.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::ClientAddNote),
    )
    .idempotent(DomainOp::ClientAddNote.idempotent())
    .emits(&["client.updated"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"clientId":{"type":"string","format":"uuid"},"body":{"type":"string","minLength":1,"maxLength":20000},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["clientId","body"]}"#)
    .output(r#"{"type":"object","properties":{"clientId":{"type":"string","format":"uuid"},"noteId":{"type":"string","format":"uuid"}}}"#),

    ACTION_CONTACT_CREATE: NodeType::new(
        "contact.create",
        "Create contact",
        "Creates a contact person under a client. Not safe to repeat: a second run is a second contact.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::ContactCreate),
    )
    .idempotent(DomainOp::ContactCreate.idempotent())
    .emits(&["contact.created"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"clientId":{"type":"string","format":"uuid"},"name":{"type":"string","minLength":1,"maxLength":200},"email":{"type":"string","maxLength":320},"phone":{"type":"string","maxLength":40},"role":{"type":"string","maxLength":120},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["clientId","name"]}"#)
    .output(r#"{"type":"object","properties":{"contactId":{"type":"string","format":"uuid"},"clientId":{"type":"string","format":"uuid"}}}"#),

    ACTION_CONTACT_UPDATE: NodeType::new(
        "contact.update",
        "Update contact",
        "Updates fields on an existing contact. A safe-to-repeat state setter.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::ContactUpdate),
    )
    .idempotent(DomainOp::ContactUpdate.idempotent())
    .emits(&["contact.updated"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"contactId":{"type":"string","format":"uuid"},"fields":{"type":"object"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["contactId","fields"]}"#)
    .output(r#"{"type":"object","properties":{"contactId":{"type":"string","format":"uuid"},"changed":{"type":"array","items":{"type":"string"}}}}"#),

    ACTION_SITE_CREATE: NodeType::new(
        "site.create",
        "Create site",
        "Creates a site under a client, with its address. Not safe to repeat: a second run is a second site.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::SiteCreate),
    )
    .idempotent(DomainOp::SiteCreate.idempotent())
    .emits(&["site.created"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"clientId":{"type":"string","format":"uuid"},"name":{"type":"string","minLength":1,"maxLength":200},"address":{"type":"object"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["clientId","name"]}"#)
    .output(r#"{"type":"object","properties":{"siteId":{"type":"string","format":"uuid"},"clientId":{"type":"string","format":"uuid"}}}"#),

// ---------------------------------------------------------------------------
// Tier A actions — money
// ---------------------------------------------------------------------------

    ACTION_QUOTE_CREATE: NodeType::new(
        "quote.create",
        "Create quote",
        "Creates a draft quote for a client with line items. Not safe to repeat: a second run is a second quote.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::QuoteCreate),
    )
    .idempotent(DomainOp::QuoteCreate.idempotent())
    .emits(&["quote.created"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"clientId":{"type":"string","format":"uuid"},"lines":{"type":"array","items":{"type":"object"},"minItems":1},"expiresAt":{"type":"string"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["clientId","lines"]}"#)
    .output(r#"{"type":"object","properties":{"quoteId":{"type":"string","format":"uuid"},"total":{"type":"number"}}}"#),

    ACTION_QUOTE_SEND: NodeType::new(
        "quote.send",
        "Send quote",
        "Sends a quote to the client. Not safe to repeat: a second run sends the quote twice.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::QuoteSend),
    )
    .idempotent(DomainOp::QuoteSend.idempotent())
    .emits(&["quote.sent"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"quoteId":{"type":"string","format":"uuid"},"contactId":{"type":"string","format":"uuid"},"message":{"type":"string","maxLength":20000},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["quoteId"]}"#)
    .output(r#"{"type":"object","properties":{"quoteId":{"type":"string","format":"uuid"},"sentAt":{"type":"string"}}}"#),

    ACTION_QUOTE_CONVERT: NodeType::new(
        "quote.convert",
        "Convert quote to invoice",
        "Converts an accepted quote into an invoice, carrying lines and totals. Not safe to repeat: a converted quote must not convert twice.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::QuoteConvert),
    )
    .idempotent(DomainOp::QuoteConvert.idempotent())
    .emits(&["quote.converted"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"quoteId":{"type":"string","format":"uuid"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["quoteId"]}"#)
    .output(r#"{"type":"object","properties":{"invoiceId":{"type":"string","format":"uuid"},"total":{"type":"number"}}}"#),

    ACTION_INVOICE_CREATE: NodeType::new(
        "invoice.create",
        "Create invoice",
        "Creates a draft invoice with lines. Not safe to repeat: a second run is a second invoice.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::InvoiceCreate),
    )
    .idempotent(DomainOp::InvoiceCreate.idempotent())
    .emits(&["invoice.created"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"clientId":{"type":"string","format":"uuid"},"lines":{"type":"array","items":{"type":"object"},"minItems":1},"dueAt":{"type":"string"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["clientId","lines"]}"#)
    .output(r#"{"type":"object","properties":{"invoiceId":{"type":"string","format":"uuid"},"total":{"type":"number"}}}"#),

    ACTION_INVOICE_RECORD_PAYMENT: NodeType::new(
        "invoice.recordPayment",
        "Record payment",
        "Records a payment against an invoice. Not safe to repeat: a second recorded payment is money taken twice.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::InvoiceRecordPayment),
    )
    .idempotent(DomainOp::InvoiceRecordPayment.idempotent())
    .emits(&["invoice.payment_recorded"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"invoiceId":{"type":"string","format":"uuid"},"amount":{"type":"number"},"method":{"type":"string","maxLength":40},"reference":{"type":"string","maxLength":120},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["invoiceId","amount"]}"#)
    .output(r#"{"type":"object","properties":{"invoiceId":{"type":"string","format":"uuid"},"balance":{"type":"number"}}}"#),

    ACTION_INVOICE_VOID: NodeType::new(
        "invoice.void",
        "Void invoice",
        "Voids an invoice. A state move, safe to repeat.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::InvoiceVoid),
    )
    .idempotent(DomainOp::InvoiceVoid.idempotent())
    .emits(&["invoice.voided"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"invoiceId":{"type":"string","format":"uuid"},"reason":{"type":"string","maxLength":2000},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["invoiceId"]}"#)
    .output(r#"{"type":"object","properties":{"invoiceId":{"type":"string","format":"uuid"},"status":{"type":"string"}}}"#),

    ACTION_CREDIT_NOTE_ISSUE: NodeType::new(
        "creditNote.issue",
        "Issue credit note",
        "Issues a credit note against an invoice. Not safe to repeat: a second credit is money refunded twice.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::CreditNoteIssue),
    )
    .idempotent(DomainOp::CreditNoteIssue.idempotent())
    .emits(&["credit_note.issued"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"invoiceId":{"type":"string","format":"uuid"},"amount":{"type":"number"},"reason":{"type":"string","maxLength":2000},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["invoiceId","amount"]}"#)
    .output(r#"{"type":"object","properties":{"creditNoteId":{"type":"string","format":"uuid"},"amount":{"type":"number"}}}"#),

    ACTION_CONTRACT_CREATE: NodeType::new(
        "contract.create",
        "Create contract",
        "Creates a service contract for a client. Not safe to repeat: a second run is a second contract.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::ContractCreate),
    )
    .idempotent(DomainOp::ContractCreate.idempotent())
    .emits(&["contract.created"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"clientId":{"type":"string","format":"uuid"},"name":{"type":"string","minLength":1,"maxLength":200},"terms":{"type":"object"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["clientId","name"]}"#)
    .output(r#"{"type":"object","properties":{"contractId":{"type":"string","format":"uuid"}}}"#),

// ---------------------------------------------------------------------------
// Tier A actions — scheduling
// ---------------------------------------------------------------------------

    ACTION_APPOINTMENT_CREATE: NodeType::new(
        "appointment.create",
        "Create appointment",
        "Books an appointment on the schedule. Not safe to repeat: a second run double-books a slot.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::AppointmentCreate),
    )
    .idempotent(DomainOp::AppointmentCreate.idempotent())
    .emits(&["appointment.created"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid"},"userId":{"type":"string","format":"uuid"},"startsAt":{"type":"string"},"endsAt":{"type":"string"},"siteId":{"type":"string","format":"uuid"},"note":{"type":"string","maxLength":2000},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["startsAt","endsAt"]}"#)
    .output(r#"{"type":"object","properties":{"appointmentId":{"type":"string","format":"uuid"},"startsAt":{"type":"string"}}}"#),

    ACTION_APPOINTMENT_UPDATE: NodeType::new(
        "appointment.update",
        "Update appointment",
        "Updates fields on an existing appointment. A safe-to-repeat state setter.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::AppointmentUpdate),
    )
    .idempotent(DomainOp::AppointmentUpdate.idempotent())
    .emits(&["appointment.updated"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"appointmentId":{"type":"string","format":"uuid"},"fields":{"type":"object"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["appointmentId","fields"]}"#)
    .output(r#"{"type":"object","properties":{"appointmentId":{"type":"string","format":"uuid"},"changed":{"type":"array","items":{"type":"string"}}}}"#),

    ACTION_APPOINTMENT_CANCEL: NodeType::new(
        "appointment.cancel",
        "Cancel appointment",
        "Cancels an appointment and frees the slot. A state move, safe to repeat.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::AppointmentCancel),
    )
    .idempotent(DomainOp::AppointmentCancel.idempotent())
    .emits(&["appointment.cancelled"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"appointmentId":{"type":"string","format":"uuid"},"reason":{"type":"string","maxLength":2000},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["appointmentId"]}"#)
    .output(r#"{"type":"object","properties":{"appointmentId":{"type":"string","format":"uuid"},"status":{"type":"string"}}}"#),

    ACTION_DISPATCH_ASSIGN: NodeType::new(
        "dispatch.assign",
        "Assign work",
        "Assigns a ticket or work item to a technician on the dispatch board. A state setter, safe to repeat.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::DispatchAssign),
    )
    .idempotent(DomainOp::DispatchAssign.idempotent())
    .emits(&["dispatch.status_updated"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"userId":{"type":"string","format":"uuid"},"scheduledFor":{"type":"string"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["userId"]}"#)
    .output(r#"{"type":"object","properties":{"ticketId":{"type":"string","format":"uuid"},"userId":{"type":"string","format":"uuid"}}}"#),

    ACTION_DISPATCH_UPDATE_STATUS: NodeType::new(
        "dispatch.updateStatus",
        "Update dispatch status",
        "Updates a technician's dispatch status — en route, on site, wrapping up. A state setter, safe to repeat.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::DispatchUpdateStatus),
    )
    .idempotent(DomainOp::DispatchUpdateStatus.idempotent())
    .emits(&["dispatch.status_updated"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"userId":{"type":"string","format":"uuid"},"status":{"type":"string","minLength":1,"maxLength":80},"location":{"type":"object"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["userId","status"]}"#)
    .output(r#"{"type":"object","properties":{"userId":{"type":"string","format":"uuid"},"status":{"type":"string"}}}"#),

    ACTION_APPOINTMENT_REQUEST_APPROVE: NodeType::new(
        "appointmentRequest.approve",
        "Approve appointment request",
        "Approves a pending appointment request. Not safe to repeat: the second call refuses or double-books.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::AppointmentRequestApprove),
    )
    .idempotent(DomainOp::AppointmentRequestApprove.idempotent())
    .emits(&["appointment_request.approved"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"requestId":{"type":"string","format":"uuid"},"startsAt":{"type":"string"},"endsAt":{"type":"string"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["requestId"]}"#)
    .output(r#"{"type":"object","properties":{"appointmentId":{"type":"string","format":"uuid"},"requestId":{"type":"string","format":"uuid"}}}"#),

    ACTION_APPOINTMENT_REQUEST_DECLINE: NodeType::new(
        "appointmentRequest.decline",
        "Decline appointment request",
        "Declines a pending appointment request with a reason. Not safe to repeat: the second call has nothing to decline.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::AppointmentRequestDecline),
    )
    .idempotent(DomainOp::AppointmentRequestDecline.idempotent())
    .emits(&["appointment_request.declined"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"requestId":{"type":"string","format":"uuid"},"reason":{"type":"string","maxLength":2000},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["requestId"]}"#)
    .output(r#"{"type":"object","properties":{"requestId":{"type":"string","format":"uuid"},"status":{"type":"string"}}}"#),

// ---------------------------------------------------------------------------
// Tier A actions — communication
// ---------------------------------------------------------------------------

    ACTION_EMAIL_SEND: NodeType::new(
        "email.send",
        "Send email",
        "Sends an email through the org's configured transport. Not safe to repeat: a second run is a second email in someone's inbox.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::EmailSend),
    )
    .idempotent(DomainOp::EmailSend.idempotent())
    .emits(&["email.message_recorded"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"to":{"type":"string","maxLength":320},"subject":{"type":"string","maxLength":400},"body":{"type":"string","minLength":1,"maxLength":100000},"ticketId":{"type":"string","format":"uuid"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["to","subject","body"]}"#)
    .output(r#"{"type":"object","properties":{"messageId":{"type":"string"},"to":{"type":"string"}}}"#),

    ACTION_NOTIFICATION_SEND: NodeType::new(
        "notification.send",
        "Send notification",
        "Sends an in-product notification to one or more users, honouring each recipient's preferences and quiet hours. Not safe to repeat: a second run is a second notification.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::NotificationSend),
    )
    .idempotent(DomainOp::NotificationSend.idempotent())
    .emits(&["notification.created"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"userIds":{"type":"array","items":{"type":"string","format":"uuid"},"minItems":1},"kind":{"type":"string","enum":["mention","assignment","sla_risk","sla_breach","escalation","missing_time","ticket_update","report_delivery","ai_cap_warn","secret_rotation","dedupe_report","scan_detection"]},"title":{"type":"string","minLength":1,"maxLength":300},"body":{"type":"string","maxLength":20000},"ticketId":{"type":"string","format":"uuid"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["userIds","kind","title"]}"#)
    .output(r#"{"type":"object","properties":{"sent":{"type":"integer"},"suppressed":{"type":"integer"}}}"#),

    ACTION_SMS_SEND: NodeType::new(
        "sms.send",
        "Send SMS",
        "Sends an SMS through the org's configured provider. Downgrades to a typed skip when no provider is configured, so a workflow is never silently broken by a missing integration. Not safe to repeat.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::SmsSend),
    )
    .idempotent(DomainOp::SmsSend.idempotent())
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"to":{"type":"string","maxLength":40},"body":{"type":"string","minLength":1,"maxLength":1600},"ticketId":{"type":"string","format":"uuid"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["to","body"]}"#)
    .output(r#"{"type":"object","properties":{"sent":{"type":"boolean"},"reason":{"type":"string"}}}"#),

    ACTION_CSAT_REQUEST: NodeType::new(
        "csat.request",
        "Request CSAT response",
        "Sends a satisfaction survey to the ticket's requester. Not safe to repeat: a second run is a second survey.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::CsatRequest),
    )
    .idempotent(DomainOp::CsatRequest.idempotent())
    .emits(&["csat.survey_sent"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"delay":{"type":"string","maxLength":32},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":[]}"#)
    .output(r#"{"type":"object","properties":{"ticketId":{"type":"string","format":"uuid"},"tokenId":{"type":"string","format":"uuid"}}}"#),

    ACTION_PORTAL_NOTIFY: NodeType::new(
        "portal.notify",
        "Notify portal user",
        "Notifies a client's portal user through the portal channel rather than email. Not safe to repeat.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::PortalNotify),
    )
    .idempotent(DomainOp::PortalNotify.idempotent())
    .emits(&["notification.created"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"portalUserId":{"type":"string","format":"uuid"},"title":{"type":"string","minLength":1,"maxLength":300},"body":{"type":"string","maxLength":20000},"ticketId":{"type":"string","format":"uuid"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["portalUserId","title"]}"#)
    .output(r#"{"type":"object","properties":{"notificationId":{"type":"string","format":"uuid"}}}"#),

// ---------------------------------------------------------------------------
// Tier A actions — field service
// ---------------------------------------------------------------------------

    ACTION_ALERT_ACKNOWLEDGE: NodeType::new(
        "alert.acknowledge",
        "Acknowledge alert",
        "Acknowledges an ingested alert. A state move, safe to repeat.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::AlertAcknowledge),
    )
    .idempotent(DomainOp::AlertAcknowledge.idempotent())
    .emits(&["alert.acknowledged"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"alertId":{"type":"string","format":"uuid"},"userId":{"type":"string","format":"uuid"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["alertId"]}"#)
    .output(r#"{"type":"object","properties":{"alertId":{"type":"string","format":"uuid"},"status":{"type":"string"}}}"#),

    ACTION_ALERT_RESOLVE: NodeType::new(
        "alert.resolve",
        "Resolve alert",
        "Resolves an ingested alert with a resolution note. A state move, safe to repeat.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::AlertResolve),
    )
    .idempotent(DomainOp::AlertResolve.idempotent())
    .emits(&["alert.resolved"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"alertId":{"type":"string","format":"uuid"},"resolution":{"type":"string","maxLength":2000},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["alertId"]}"#)
    .output(r#"{"type":"object","properties":{"alertId":{"type":"string","format":"uuid"},"status":{"type":"string"}}}"#),

    ACTION_TRAVEL_DRAFT_CREATE: NodeType::new(
        "travelDraft.create",
        "Create travel draft",
        "Creates a travel and mileage draft from a dispatch leg, for approval like any other time. Not safe to repeat: a second run is a second draft.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::TravelDraftCreate),
    )
    .idempotent(DomainOp::TravelDraftCreate.idempotent())
    .emits(&["travel_draft.submitted"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"userId":{"type":"string","format":"uuid"},"distanceKm":{"type":"number"},"startedAt":{"type":"string"},"endedAt":{"type":"string"},"note":{"type":"string","maxLength":2000},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["distanceKm"]}"#)
    .output(r#"{"type":"object","properties":{"travelDraftId":{"type":"string","format":"uuid"}}}"#),

    ACTION_SIGN_OFF_CAPTURE: NodeType::new(
        "signOff.capture",
        "Capture sign-off",
        "Files a captured customer signature against a ticket, carrying the signer, witness and the consent text version they agreed to (Wave 43 lane 12). Not safe to repeat: a second capture is a second piece of evidence.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::SignOffCapture),
    )
    .idempotent(DomainOp::SignOffCapture.idempotent())
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"signerName":{"type":"string","minLength":1,"maxLength":200},"attachmentId":{"type":"string","format":"uuid"},"consentVersion":{"type":"string","maxLength":40},"witnessId":{"type":"string","format":"uuid"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["signerName","attachmentId","consentVersion"]}"#)
    .output(r#"{"type":"object","properties":{"signOffId":{"type":"string","format":"uuid"},"capturedAt":{"type":"string"}}}"#),

    ACTION_SIGN_OFF_FILE: NodeType::new(
        "signOff.file",
        "File sign-off",
        "Attaches a captured sign-off to the record it belongs on. Not safe to repeat.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::SignOffFile),
    )
    .idempotent(DomainOp::SignOffFile.idempotent())
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"signOffId":{"type":"string","format":"uuid"},"timeEntryId":{"type":"string","format":"uuid"},"ticketId":{"type":"string","format":"uuid"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["signOffId"]}"#)
    .output(r#"{"type":"object","properties":{"signOffId":{"type":"string","format":"uuid"},"filed":{"type":"boolean"}}}"#),

// ---------------------------------------------------------------------------
// Tier A actions — assets and stock
// ---------------------------------------------------------------------------

    ACTION_STOCK_ITEM_CREATE: NodeType::new(
        "stockItem.create",
        "Create stock item",
        "Creates a stock item with its reorder point and serialization rules. Not safe to repeat: a second run is a second item.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::StockItemCreate),
    )
    .idempotent(DomainOp::StockItemCreate.idempotent())
    .emits(&["stock.item_created"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"name":{"type":"string","minLength":1,"maxLength":200},"partNumber":{"type":"string","maxLength":120},"reorderPoint":{"type":"integer","minimum":0},"serialized":{"type":"boolean"},"unit":{"type":"string","maxLength":40},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["name"]}"#)
    .output(r#"{"type":"object","properties":{"itemId":{"type":"string","format":"uuid"}}}"#),

    ACTION_STOCK_ITEM_MOVE: NodeType::new(
        "stockItem.move",
        "Move stock",
        "Records a real stock movement between locations — a ledger leg, never an in-place UPDATE. Not safe to repeat: a second run is a second movement and the balances lie.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::StockItemMove),
    )
    .idempotent(DomainOp::StockItemMove.idempotent())
    .emits(&["stock.movement_recorded"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"itemId":{"type":"string","format":"uuid"},"fromLocationId":{"type":"string","format":"uuid"},"toLocationId":{"type":"string","format":"uuid"},"quantity":{"type":"integer","minimum":1},"serialIds":{"type":"array","items":{"type":"string","format":"uuid"}},"reason":{"type":"string","maxLength":200},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["itemId","quantity"]}"#)
    .output(r#"{"type":"object","properties":{"movementId":{"type":"string","format":"uuid"},"itemId":{"type":"string","format":"uuid"}}}"#),

    ACTION_TOOL_CHECKOUT: NodeType::new(
        "tool.checkout",
        "Check out tool",
        "Checks a tool out to a technician. Not safe to repeat: a second run double-checks one serial.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::ToolCheckout),
    )
    .idempotent(DomainOp::ToolCheckout.idempotent())
    .emits(&["tool.checkout_completed"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"serialId":{"type":"string","format":"uuid"},"userId":{"type":"string","format":"uuid"},"dueAt":{"type":"string"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["serialId","userId"]}"#)
    .output(r#"{"type":"object","properties":{"checkoutId":{"type":"string","format":"uuid"}}}"#),

    ACTION_TOOL_CHECKIN: NodeType::new(
        "tool.checkin",
        "Check in tool",
        "Returns a checked-out tool. Safe to repeat once nothing is out.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::ToolCheckin),
    )
    .idempotent(DomainOp::ToolCheckin.idempotent())
    .emits(&["tool.checkin_completed"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"checkoutId":{"type":"string","format":"uuid"},"condition":{"type":"string","maxLength":200},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["checkoutId"]}"#)
    .output(r#"{"type":"object","properties":{"checkoutId":{"type":"string","format":"uuid"},"returnedAt":{"type":"string"}}}"#),

    ACTION_LOAN_ISSUE: NodeType::new(
        "loan.issue",
        "Issue loan",
        "Issues a loanable asset to a client fleet. Not safe to repeat: a second issue double-counts one asset.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::LoanIssue),
    )
    .idempotent(DomainOp::LoanIssue.idempotent())
    .emits(&["loan.issued"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"serialId":{"type":"string","format":"uuid"},"clientId":{"type":"string","format":"uuid"},"dueAt":{"type":"string"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["serialId","clientId"]}"#)
    .output(r#"{"type":"object","properties":{"loanId":{"type":"string","format":"uuid"}}}"#),

    ACTION_LOAN_RETURN: NodeType::new(
        "loan.return",
        "Accept loan return",
        "Accepts a loaned asset back from a client. Safe to repeat once nothing is out.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::LoanReturn),
    )
    .idempotent(DomainOp::LoanReturn.idempotent())
    .emits(&["loan.returned"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"loanId":{"type":"string","format":"uuid"},"condition":{"type":"string","maxLength":200},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["loanId"]}"#)
    .output(r#"{"type":"object","properties":{"loanId":{"type":"string","format":"uuid"},"returnedAt":{"type":"string"}}}"#),

    ACTION_PURCHASE_ORDER_CREATE: NodeType::new(
        "purchaseOrder.create",
        "Create purchase order",
        "Creates a purchase order for a vendor. Not safe to repeat: a second run is a second order.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::PurchaseOrderCreate),
    )
    .idempotent(DomainOp::PurchaseOrderCreate.idempotent())
    .emits(&["purchase.order_created"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"vendorId":{"type":"string","format":"uuid"},"lines":{"type":"array","items":{"type":"object"},"minItems":1},"expectedAt":{"type":"string"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["vendorId","lines"]}"#)
    .output(r#"{"type":"object","properties":{"purchaseOrderId":{"type":"string","format":"uuid"},"total":{"type":"number"}}}"#),

    ACTION_PURCHASE_ORDER_RECEIVE: NodeType::new(
        "purchaseOrder.receive",
        "Receive purchase order",
        "Records a receipt against a purchase order, writing the stock movements it implies. Not safe to repeat: a second receipt double-counts the stock.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::PurchaseOrderReceive),
    )
    .idempotent(DomainOp::PurchaseOrderReceive.idempotent())
    .emits(&["purchase.receipt_recorded"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"purchaseOrderId":{"type":"string","format":"uuid"},"received":{"type":"array","items":{"type":"object"},"minItems":1},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["purchaseOrderId","received"]}"#)
    .output(r#"{"type":"object","properties":{"receiptId":{"type":"string","format":"uuid"},"linesReceived":{"type":"integer"}}}"#),

    ACTION_PURCHASE_ORDER_POST_BILL: NodeType::new(
        "purchaseOrder.postBill",
        "Post purchase order bill",
        "Posts a vendor bill against a received purchase order. Not safe to repeat: a second posting is a second bill.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::PurchaseOrderPostBill),
    )
    .idempotent(DomainOp::PurchaseOrderPostBill.idempotent())
    .emits(&["purchase.bill_posted"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"purchaseOrderId":{"type":"string","format":"uuid"},"amount":{"type":"number"},"reference":{"type":"string","maxLength":120},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["purchaseOrderId","amount"]}"#)
    .output(r#"{"type":"object","properties":{"billId":{"type":"string","format":"uuid"}}}"#),

// ---------------------------------------------------------------------------
// Tier A actions — records and projects
// ---------------------------------------------------------------------------

    ACTION_DOCUMENT_CREATE: NodeType::new(
        "document.create",
        "Create document",
        "Creates a document record. Not safe to repeat: a second run is a second document.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::DocumentCreate),
    )
    .idempotent(DomainOp::DocumentCreate.idempotent())
    .emits(&["document.created"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"title":{"type":"string","minLength":1,"maxLength":300},"body":{"type":"string","maxLength":200000},"clientId":{"type":"string","format":"uuid"},"ticketId":{"type":"string","format":"uuid"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["title"]}"#)
    .output(r#"{"type":"object","properties":{"documentId":{"type":"string","format":"uuid"}}}"#),

    ACTION_DOCUMENT_ATTACH: NodeType::new(
        "document.attach",
        "Attach document",
        "Attaches an existing document to a record. Not safe to repeat: the attachment is a row.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::DocumentAttach),
    )
    .idempotent(DomainOp::DocumentAttach.idempotent())
    .emits(&["document_attachment.added"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"documentId":{"type":"string","format":"uuid"},"ticketId":{"type":"string","format":"uuid","description":"The ticket to act on; omit to use the ticket that triggered the run"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["documentId"]}"#)
    .output(r#"{"type":"object","properties":{"documentId":{"type":"string","format":"uuid"},"ticketId":{"type":"string","format":"uuid"}}}"#),

    ACTION_ATTACHMENT_UPLOAD: NodeType::new(
        "attachment.upload",
        "Upload attachment",
        "Stores bytes in the content-addressed attachment store and records the metadata row. Not safe to repeat: a second run stores a second copy.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::AttachmentUpload),
    )
    .idempotent(DomainOp::AttachmentUpload.idempotent())
    .emits(&["attachment.added"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"ticketId":{"type":"string","format":"uuid"},"contentBase64":{"type":"string","minLength":1},"mimeType":{"type":"string","maxLength":120},"filename":{"type":"string","maxLength":300},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["contentBase64","filename"]}"#)
    .output(r#"{"type":"object","properties":{"attachmentId":{"type":"string","format":"uuid"},"sha256":{"type":"string"}}}"#),

    ACTION_KB_ARTICLE_CREATE: NodeType::new(
        "kbArticle.create",
        "Create KB article",
        "Creates a knowledge-base article. Not safe to repeat: a second run is a second article.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::KbArticleCreate),
    )
    .idempotent(DomainOp::KbArticleCreate.idempotent())
    .emits(&["kb_article.created"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"title":{"type":"string","minLength":1,"maxLength":300},"body":{"type":"string","minLength":1,"maxLength":200000},"category":{"type":"string","maxLength":120},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["title","body"]}"#)
    .output(r#"{"type":"object","properties":{"articleId":{"type":"string","format":"uuid"}}}"#),

    ACTION_KB_ARTICLE_UPDATE: NodeType::new(
        "kbArticle.update",
        "Update KB article",
        "Updates a knowledge-base article. A safe-to-repeat state setter.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::KbArticleUpdate),
    )
    .idempotent(DomainOp::KbArticleUpdate.idempotent())
    .emits(&["kb_article.updated"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"articleId":{"type":"string","format":"uuid"},"fields":{"type":"object"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["articleId","fields"]}"#)
    .output(r#"{"type":"object","properties":{"articleId":{"type":"string","format":"uuid"},"changed":{"type":"array","items":{"type":"string"}}}}"#),

    /// Write-only. There is deliberately no read schema here, and no
    /// plugin `read` output may carry one either, because the run ledger
    /// is a place operators read.
    ACTION_SECRET_RECORD: NodeType::new(
        "secret.record",
        "Record secret",
        "Stores a credential in the vault, sealed. Writes only: no workflow node can read a secret value back, and no plugin read output may carry one, because the run ledger is somewhere operators read.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::SecretRecord),
    )
    .idempotent(DomainOp::SecretRecord.idempotent())
    .emits(&["secret.record_created"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"name":{"type":"string","minLength":1,"maxLength":200},"value":{"type":"string","minLength":1,"maxLength":20000},"rotateAt":{"type":"string"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["name","value"]}"#)
    .output(r#"{"type":"object","properties":{"secretId":{"type":"string","format":"uuid"},"recorded":{"type":"boolean"}}}"#),

    ACTION_PROJECT_CREATE: NodeType::new(
        "project.create",
        "Create project",
        "Creates a project for a client. Not safe to repeat: a second run is a second project.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::ProjectCreate),
    )
    .idempotent(DomainOp::ProjectCreate.idempotent())
    .emits(&["project.created"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"clientId":{"type":"string","format":"uuid"},"name":{"type":"string","minLength":1,"maxLength":200},"startsAt":{"type":"string"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["clientId","name"]}"#)
    .output(r#"{"type":"object","properties":{"projectId":{"type":"string","format":"uuid"}}}"#),

    ACTION_PROJECT_PHASE_COMPLETE: NodeType::new(
        "projectPhase.complete",
        "Complete project phase",
        "Marks a project phase complete. A state move, safe to repeat.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::ProjectPhaseComplete),
    )
    .idempotent(DomainOp::ProjectPhaseComplete.idempotent())
    .emits(&["project.phase_completed"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"phaseId":{"type":"string","format":"uuid"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["phaseId"]}"#)
    .output(r#"{"type":"object","properties":{"phaseId":{"type":"string","format":"uuid"},"completedAt":{"type":"string"}}}"#),

    ACTION_PROJECT_TASK_CREATE: NodeType::new(
        "projectTask.create",
        "Create project task",
        "Creates a task under a project phase. Not safe to repeat: a second run is a second task.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::ProjectTaskCreate),
    )
    .idempotent(DomainOp::ProjectTaskCreate.idempotent())
    .emits(&["project.task_created"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"phaseId":{"type":"string","format":"uuid"},"name":{"type":"string","minLength":1,"maxLength":300},"assigneeId":{"type":"string","format":"uuid"},"dueAt":{"type":"string"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["phaseId","name"]}"#)
    .output(r#"{"type":"object","properties":{"taskId":{"type":"string","format":"uuid"}}}"#),

    ACTION_PROJECT_TASK_COMPLETE: NodeType::new(
        "projectTask.complete",
        "Complete project task",
        "Marks a project task complete. A state move, safe to repeat.",
        NodeFamily::Action,
        NodeTier::A,
        ExecutorPath::DomainWrapper(DomainOp::ProjectTaskComplete),
    )
    .idempotent(DomainOp::ProjectTaskComplete.idempotent())
    .emits(&["project.task_completed"])
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"taskId":{"type":"string","format":"uuid"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["taskId"]}"#)
    .output(r#"{"type":"object","properties":{"taskId":{"type":"string","format":"uuid"},"completedAt":{"type":"string"}}}"#),

// ---------------------------------------------------------------------------
// Tier B — the generic operation nodes
//
// `domain.update` is the single most important decision in this catalogue:
// it is what keeps the registry a single source of truth instead of ~150
// hand-written action nodes whose schemas come along too. Its operation
// selector is the same vocabulary every Tier A wrapper names, and its
// `idempotent` flag and emitted event resolve per selected operation.
// ---------------------------------------------------------------------------

    ACTION_DOMAIN_UPDATE: NodeType::new(
        "domain.update",
        "Update any record",
        "Updates fields on any registry-declared entity through the closed operation selector. The one node that replaces roughly 150 hand-written action nodes: pick the entity through the operation, then the fields it takes. Whether a repeat is safe depends on the operation — a state setter is safe, a create is not.",
        NodeFamily::Action,
        NodeTier::B,
        ExecutorPath::DomainUpdate,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"operation":{"type":"string"},"entityId":{"type":"string","format":"uuid"},"fields":{"type":"object"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["operation"]}"#)
    .output(r#"{"type":"object","properties":{"entityId":{"type":"string","format":"uuid"},"operation":{"type":"string"},"changed":{"type":"array","items":{"type":"string"}}}}"#),

    ACTION_DOMAIN_QUERY: NodeType::new(
        "domain.query",
        "Query records",
        "Reads entities matching a schema-derived filter and returns rows that feed conditions and downstream nodes. Read-only and rate-capped, because a filter that matches everything is a query that costs everything.",
        NodeFamily::Action,
        NodeTier::B,
        ExecutorPath::DomainQuery,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"entity":{"type":"string","minLength":1,"maxLength":80},"filter":{"type":"object"},"limit":{"type":"integer","minimum":1,"maximum":200},"orderBy":{"type":"string","maxLength":80}},"required":["entity"]}"#)
    .output(r#"{"type":"object","properties":{"rows":{"type":"array","items":{"type":"object"}},"total":{"type":"integer"},"truncated":{"type":"boolean"}}}"#),

// ---------------------------------------------------------------------------
// Tier C — the plugin slots
//
// There is no `plugin.trigger` node, and that is deliberate: a
// plugin-contributed trigger is not a node at all, it is an event type the
// connector declares through the signed ingest, which the event-type
// registry makes subscribable. Declaring a node for it would invent a
// second trigger path.
// ---------------------------------------------------------------------------

    ACTION_PLUGIN_ACTION: NodeType::new(
        "plugin.action",
        "Plugin action",
        "Calls a consented plugin action — the same effect the rule builder's plugin step rides. The entry is re-read at execution time, so a revoked install degrades to a typed skip naming the entry rather than a silent half-finished run, and credentials are resolved from the install by the host and never appear in the document. Slot key: the entry key of a real surface entry.",
        NodeFamily::Action,
        NodeTier::C,
        ExecutorPath::Plugin(super::PluginNodeKind::Action),
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"entryKey":{"type":"string","minLength":1,"maxLength":200},"params":{"type":"object"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["entryKey"]}"#)
    .output(r#"{"type":"object","properties":{"entryKey":{"type":"string"},"result":{"type":"object"},"skipped":{"type":"boolean"},"reason":{"type":"string"}}}"#),

    ACTION_PLUGIN_READ: NodeType::new(
        "plugin.read",
        "Plugin read",
        "Calls a consented plugin read — the only way a workflow fetches from an external system, and the reason the RMM example works. Guardrails are stricter than a write's: rate-capped per run and per org, bounded output so a device list cannot grow the run ledger by megabytes, and never returns a vault secret. Slot key: the entry key of a real surface entry.",
        NodeFamily::Action,
        NodeTier::C,
        ExecutorPath::Plugin(super::PluginNodeKind::Read),
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"entryKey":{"type":"string","minLength":1,"maxLength":200},"params":{"type":"object"},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["entryKey"]}"#)
    .output(r#"{"type":"object","properties":{"entryKey":{"type":"string"},"result":{"type":"object"},"truncated":{"type":"boolean"},"skipped":{"type":"boolean"},"reason":{"type":"string"}}}"#),

// ---------------------------------------------------------------------------
// Control — the time-based waits
// ---------------------------------------------------------------------------

    CONTROL_DELAY: NodeType::new(
        "flow.delay",
        "Wait for a duration",
        "Continues the run after a duration such as 4h or 2d. Never holds a worker: the run parks with a resume time and the queue does the waking.",
        NodeFamily::Control,
        NodeTier::A,
        ExecutorPath::Control,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"duration":{"type":"string","maxLength":32},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["duration"]}"#)
    .output(r#"{"type":"object","properties":{"resumesAt":{"type":"string"}}}"#),

    CONTROL_WAIT_UNTIL: NodeType::new(
        "flow.waitUntil",
        "Wait until a moment",
        "Continues at a wall-clock instant. Bound a calendar and the instant resolves business-hours-aware, reusing the org's SLA calendars rather than inventing a second one.",
        NodeFamily::Control,
        NodeTier::A,
        ExecutorPath::Control,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"at":{"type":"string"},"calendar":{"type":"string","maxLength":120},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["at"]}"#)
    .output(r#"{"type":"object","properties":{"resumesAt":{"type":"string"},"calendarResolved":{"type":"boolean"}}}"#),

    /// The escalation node. An escalation is not a wait plus a check; it
    /// is one node, because the alternative is two nodes whose failure mode
    /// is "nobody ever noticed".
    CONTROL_DEADLINE: NodeType::new(
        "flow.deadline",
        "Escalate before a deadline",
        "Continues when a matching event arrives, but never past a time you set. This is the escalation node: a ticket that will breach SLA in four hours needs a wake-up whether or not the awaited event arrives, and that wake-up must not depend on someone remembering to check. Pick what happens on expiry — fail, continue, or route to an error branch.",
        NodeFamily::Control,
        NodeTier::A,
        ExecutorPath::Control,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"until":{"type":"string"},"eventTypes":{"type":"array","items":{"type":"string"},"minItems":1},"where":{"type":"object"},"onExpiry":{"type":"string","enum":["fail","continue","route"]},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["until","onExpiry"]}"#)
    .output(r#"{"type":"object","properties":{"outcome":{"type":"string"},"matchedEventType":{"type":"string"},"expired":{"type":"boolean"}}}"#),

// ---------------------------------------------------------------------------
// Control — the event-based waits (parking nodes)
// ---------------------------------------------------------------------------

    CONTROL_WAIT_FOR: NodeType::new(
        "flow.waitFor",
        "Wait for an event",
        "Parks the run and resumes on the first matching event, or on timeout. The registry refuses a wait whose matched event a node in the same workflow can produce, because that workflow resumes itself forever.",
        NodeFamily::Control,
        NodeTier::A,
        ExecutorPath::Control,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"eventTypes":{"type":"array","items":{"type":"string"},"minItems":1},"where":{"type":"object"},"timeout":{"type":"string","maxLength":32},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["eventTypes"]}"#)
    .output(r#"{"type":"object","properties":{"matchedEventType":{"type":"string"},"expired":{"type":"boolean"},"event":{"type":"object"}}}"#),

    CONTROL_WAIT_FOR_ALL: NodeType::new(
        "flow.waitForAll",
        "Wait for all events",
        "Parks the run until every declared event type has arrived, or the timeout passes. A run waiting on three approvals must not proceed on the first one.",
        NodeFamily::Control,
        NodeTier::A,
        ExecutorPath::Control,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"eventTypes":{"type":"array","items":{"type":"string"},"minItems":2},"where":{"type":"object"},"timeout":{"type":"string","maxLength":32},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["eventTypes"]}"#)
    .output(r#"{"type":"object","properties":{"complete":{"type":"boolean"},"received":{"type":"array","items":{"type":"string"}},"missing":{"type":"array","items":{"type":"string"}}}}"#),

    /// A human gate, kept separate from `waitFor` because it carries
    /// identity. Requires a timeout and an onExpiry: a gate with no expiry
    /// is a way to lose a run permanently.
    CONTROL_AWAIT_APPROVAL: NodeType::new(
        "flow.awaitApproval",
        "Wait for approval",
        "Parks the run until named approvers approve or decline, riding the existing agent-proposal lifecycle so the approval carries identity and an audit trail. A timeout and an onExpiry are required — a gate with no expiry is a run nobody can find again.",
        NodeFamily::Control,
        NodeTier::A,
        ExecutorPath::Control,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"approvers":{"type":"array","items":{"type":"string"},"minItems":1},"timeout":{"type":"string","maxLength":32},"onExpiry":{"type":"string","enum":["fail","continue","route"]},"prompt":{"type":"string","maxLength":20000},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["approvers","timeout","onExpiry"]}"#)
    .output(r#"{"type":"object","properties":{"outcome":{"type":"string"},"approverId":{"type":"string","format":"uuid"},"proposalId":{"type":"string","format":"uuid"}}}"#),

// ---------------------------------------------------------------------------
// Control — joins, fan-out and iteration
// ---------------------------------------------------------------------------

    CONTROL_PARALLEL: NodeType::new(
        "flow.parallel",
        "Run branches in parallel",
        "Fans out to several branches at once. Bounded: the cap belongs to the org and the author cannot raise it, because an unbounded fan-out against five thousand records is an outage with a canvas.",
        NodeFamily::Control,
        NodeTier::A,
        ExecutorPath::Control,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"branches":{"type":"array","items":{"type":"string"},"minItems":2,"maxItems":16},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["branches"]}"#)
    .output(r#"{"type":"object","properties":{"completed":{"type":"integer"},"failed":{"type":"integer"}}}"#),

    CONTROL_JOIN: NodeType::new(
        "flow.join",
        "Join branches",
        "Converges branches: wait for all of them, for any of them, or for N of M. Carries its own concurrency cap for the same reason fan-out does.",
        NodeFamily::Control,
        NodeTier::A,
        ExecutorPath::Control,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"mode":{"type":"string","enum":["all","any","n_of_m"]},"count":{"type":"integer","minimum":1},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["mode"]}"#)
    .output(r#"{"type":"object","properties":{"joined":{"type":"integer"},"mode":{"type":"string"}}}"#),

    /// The dangerous one, with all three defences the catalogue requires:
    /// a hard per-run cap the org can raise, batched iterations in one run
    /// rather than one run per item, and `loop.maxDepth` on the graph.
    CONTROL_FOREACH: NodeType::new(
        "flow.foreach",
        "For each item",
        "Iterates a collection, binding each item for the branch body. Hard-capped per run and refused at author time above the cap, and iterations are batched into one run so a thousand items do not become a thousand runs. Refused outright above the cap rather than silently truncated — the palette shows the item count beside the collection for a reason.",
        NodeFamily::Control,
        NodeTier::A,
        ExecutorPath::Control,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"collection":{"type":"string","minLength":1,"maxLength":200},"maxItems":{"type":"integer","minimum":1,"maximum":10000},"batchSize":{"type":"integer","minimum":1,"maximum":500},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["collection"]}"#)
    .output(r#"{"type":"object","properties":{"processed":{"type":"integer"},"batches":{"type":"integer"},"item":{"type":"object"}}}"#),

// ---------------------------------------------------------------------------
// Control — branching and error handling
// ---------------------------------------------------------------------------

    CONTROL_BRANCH: NodeType::new(
        "flow.branch",
        "Branch",
        "If/else with two labelled outgoing edges. The condition node supplies the answer; this decides where the answer sends the run.",
        NodeFamily::Control,
        NodeTier::A,
        ExecutorPath::Control,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"condition":{"type":"string","minLength":1,"maxLength":200},"trueLabel":{"type":"string","maxLength":200},"falseLabel":{"type":"string","maxLength":200}},"required":["condition"]}"#)
    .output(r#"{"type":"object","properties":{"branch":{"type":"string"}}}"#),

    CONTROL_SWITCH: NodeType::new(
        "flow.switch",
        "Switch",
        "Multi-way on one value: one outgoing edge per case, plus a default. Use it where a chain of four branches would be four arms nobody can read.",
        NodeFamily::Control,
        NodeTier::A,
        ExecutorPath::Control,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"value":{"type":"string","minLength":1,"maxLength":200},"cases":{"type":"array","items":{"type":"string"},"minItems":2},"defaultLabel":{"type":"string","maxLength":200}},"required":["value","cases"]}"#)
    .output(r#"{"type":"object","properties":{"case":{"type":"string"},"matched":{"type":"boolean"}}}"#),

    CONTROL_ERROR_HANDLER: NodeType::new(
        "flow.errorHandler",
        "Error branch",
        "Where error branches converge. Its outputs carry the failing node's id, the error class and the step's inputs, so the handler can decide from facts rather than from a message somebody parses.",
        NodeFamily::Control,
        NodeTier::A,
        ExecutorPath::Control,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{}}"#)
    .output(r#"{"type":"object","properties":{"failedNode":{"type":"string"},"errorClass":{"type":"string"},"stepInput":{"type":"object"}}}"#),

    /// Must refuse to wrap any node declared `idempotent: false`, which is
    /// the registry's job: the parent module's `retry_refusal` reads the
    /// wrapped node's flag before this runs.
    CONTROL_RETRY: NodeType::new(
        "flow.retry",
        "Retry with backoff",
        "Retries a wrapped step with backoff. Refuses to wrap any node declared idempotent false — decision.ask among them, because it spends money and may answer differently — and stays clear of the queue's own attempt budget so a transient blip does not become twelve calls.",
        NodeFamily::Control,
        NodeTier::A,
        ExecutorPath::Control,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"maxAttempts":{"type":"integer","minimum":1,"maximum":10},"backoff":{"type":"string","maxLength":32},"onError":{"type":"string","enum":["halt","continue","route"]}},"required":["maxAttempts"]}"#)
    .output(r#"{"type":"object","properties":{"attempts":{"type":"integer"},"succeeded":{"type":"boolean"}}}"#),

// ---------------------------------------------------------------------------
// Control — termination. `stop`, `fail` and `abandon` are three nodes
// because they read differently in the ledger, and a queue of identical
// failures is unreadable while a queue that mixes the two is triage.
// ---------------------------------------------------------------------------

    CONTROL_STOP: NodeType::new(
        "flow.stop",
        "End run",
        "Ends the run successfully.",
        NodeFamily::Control,
        NodeTier::A,
        ExecutorPath::Control,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{}}"#)
    .output(r#"{"type":"object","properties":{"status":{"type":"string"}}}"#),

    CONTROL_FAIL: NodeType::new(
        "flow.fail",
        "End run with an error",
        "Ends the run with an error an operator sees in the ledger. Use it when something broke.",
        NodeFamily::Control,
        NodeTier::A,
        ExecutorPath::Control,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"reason":{"type":"string","minLength":1,"maxLength":2000}},"required":["reason"]}"#)
    .output(r#"{"type":"object","properties":{"status":{"type":"string"},"reason":{"type":"string"}}}"#),

    CONTROL_ABANDON: NodeType::new(
        "flow.abandon",
        "End run deliberately",
        "Ends the run having decided not to act. Distinct from flow.fail on purpose: 'we chose not to dispatch this' must read differently from 'the dispatch broke'.",
        NodeFamily::Control,
        NodeTier::A,
        ExecutorPath::Control,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"reason":{"type":"string","minLength":1,"maxLength":2000}},"required":["reason"]}"#)
    .output(r#"{"type":"object","properties":{"status":{"type":"string"},"reason":{"type":"string"}}}"#),

// ---------------------------------------------------------------------------
// Value nodes — not control flow, but a workflow cannot be written
// without them. `value.reference` is a TYPED reference, not an expression
// string, and `value.template` is the one place string building is
// allowed: a closed substitution set, never an expression language.
// ---------------------------------------------------------------------------

    VALUE_LITERAL: NodeType::new(
        "value.literal",
        "Constant value",
        "A constant: a fixed string, number or boolean. Every other node takes inputs, and a workflow that cannot name a fixed value is not writable.",
        NodeFamily::Value,
        NodeTier::A,
        ExecutorPath::Value,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"value":{}},"required":["value"]}"#)
    .output(r#"{"type":"object","properties":{"value":{}}}"#),

    VALUE_REFERENCE: NodeType::new(
        "value.reference",
        "Reference a previous output",
        "Points at a field of a previous step's output. A typed reference, not an expression string — the registry checks the target declares an output schema carrying that field, so a reference cannot rot silently when an upstream node changes.",
        NodeFamily::Value,
        NodeTier::A,
        ExecutorPath::Value,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"node":{"type":"string","minLength":1,"maxLength":200},"path":{"type":"string","minLength":1,"maxLength":400}},"required":["node","path"]}"#)
    .output(r#"{"type":"object","properties":{"value":{},"path":{"type":"string"}}}"#),

    VALUE_MATH: NodeType::new(
        "value.math",
        "Calculate",
        "Arithmetic on numeric outputs for 'remaining = due minus used'. A closed operator set — there is no expression language in a workflow document.",
        NodeFamily::Value,
        NodeTier::A,
        ExecutorPath::Value,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"left":{"type":"number"},"operator":{"type":"string","enum":["+","-","*","/"]},"right":{"type":"number"}},"required":["left","operator","right"]}"#)
    .output(r#"{"type":"object","properties":{"value":{"type":"number"}}}"#),

    VALUE_TEMPLATE: NodeType::new(
        "value.template",
        "Compose text",
        "Interpolates named fields into text — an email body, a notification title. The one place a workflow builds a string, and it stays a closed substitution set rather than an expression language.",
        NodeFamily::Value,
        NodeTier::A,
        ExecutorPath::Value,
    )
    .input(r#"{"type":"object","additionalProperties":false,"properties":{"template":{"type":"string","minLength":1,"maxLength":20000},"fields":{"type":"object"}},"required":["template"]}"#)
    .output(r#"{"type":"object","properties":{"text":{"type":"string"}}}"#),
}
