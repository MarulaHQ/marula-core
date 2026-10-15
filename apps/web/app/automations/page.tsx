// /automations — Event-triggered automation rule editor (Wave 11 lane 8;
// Wave 17 lane 6 UX pass).
//
// Purpose: list + author automation rules as WHEN (trigger event) → IF
// (conditions, ALL must match) → THEN (actions). Condition/action steps
// render as human-readable sentences with typed editors and client-side
// validation (shape errors surface before the wire); the user picker
// reads the real org directory; enable/disable is optimistic; delete is
// two-step. "Test run" samples the org's recent tickets through the rule
// WITHOUT executing anything — the result renders as a dry-run proposal.
//
// Non-purpose: no canvas builder (no drag-drop), no new action types
// beyond the SDK's closed `AutomationAction` vocabulary.
"use client";

import {
  Badge,
  Button,
  Card,
  ColumnChooser,
  DataToolbar,
  DensitySwitch,
  Dialog,
  DialogActions,
  DialogContent,
  EmptyState,
  ExportControl,
  FieldHint,
  FilterBar,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  SurfaceHeader,
  Table,
  Td,
  Th,
  Tr,
  ViewSwitch,
  applyDensity,
  useDensity,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  BlushNote,
  HEAD_CELL_CLASS,
  InlineVerdict,
  KeyHint,
  LedgerFooter,
  NoMatches,
  PageFrame,
  PlusIcon,
  RuleStep,
  SkeletonRows,
  StatPill,
  StatusPill,
  TableCard,
  downloadCsv,
} from "../../components/knowledge/ledger";
import { api, useRequireAuth } from "../../lib/api";
import { rejectionText } from "../../lib/error-copy";

type Priority = components["schemas"]["Priority"];
type TicketStatusCategory = components["schemas"]["TicketStatusCategory"];
type AutomationCondition = components["schemas"]["AutomationCondition"];
type AutomationAction = components["schemas"]["AutomationAction"];
// §4.6: plugin-contributed action kinds ride the same action list. The
// SDK's closed vocabulary gains `plugin_action` when the pins regen; the
// local draft type keeps the editor honest in the meantime.
type PluginActionDraft = {
  type: "plugin_action";
  entry_key: string;
  params: Record<string, unknown>;
};
type AutomationActionDraft = AutomationAction | PluginActionDraft;
type BoardNavEntry = components["schemas"]["BoardNavEntry"];
type OrgUser = components["schemas"]["OrgUser"];
type NotificationKind = components["schemas"]["NotificationKind"];
type AutomationTestRunResponse = components["schemas"]["AutomationTestRunResponse"];

interface AutomationRuleView {
  id: string;
  name: string;
  enabled: boolean;
  board_id: string | null;
  trigger_event: string;
  conditions: AutomationCondition[];
  actions: AutomationAction[];
  /**
   * What the rule's subject is (Wave 43 lane 3): `ticket` (default,
   * historical) or `board` — a rule that fires on ticket-less envelopes
   * such as inbound webhook events. Absent reads as `ticket`.
   */
  subject_kind?: "ticket" | "board" | null;
  created_at?: string;
  updated_at?: string;
}

type RuleColumnKey =
  | "rule"
  | "status"
  | "trigger"
  | "conditions"
  | "actions"
  | "board"
  | "updated";

interface RuleColumn {
  key: RuleColumnKey;
  label: string;
  visible: boolean;
  required?: boolean;
}

/** The pack's ledger columns; `rule` is the identity and cannot be hidden. */
const RULE_COLUMNS: RuleColumn[] = [
  { key: "rule", label: "Rule", visible: true, required: true },
  { key: "status", label: "Status", visible: true },
  { key: "trigger", label: "Trigger", visible: true },
  { key: "conditions", label: "Conditions", visible: true },
  { key: "actions", label: "Actions", visible: true },
  { key: "board", label: "Board", visible: false },
  { key: "updated", label: "Updated", visible: false },
];

const TRIGGER_EVENTS: { value: string; label: string; ticketless?: boolean }[] = [
  { value: "ticket.created", label: "Ticket created" },
  { value: "ticket.updated", label: "Ticket updated" },
  { value: "ticket.status_changed", label: "Ticket status changed" },
  { value: "ticket.assigned", label: "Ticket assigned" },
  { value: "ticket.comment_added", label: "Comment added" },
  { value: "ticket.closed", label: "Ticket closed" },
  { value: "csat.response_recorded", label: "CSAT response recorded" },
  // Ticket-less platform events (Wave 43 lanes 1 + 3): the envelopes a
  // board-subject rule fires on — the generic receiver's emitted event,
  // the alert ingestor's, and the other non-ticket platform types.
  { value: "webhook.ingested", label: "Inbound webhook received", ticketless: true },
  { value: "alert.ingested", label: "Alert ingested", ticketless: true },
  { value: "client.created", label: "Client created", ticketless: true },
  { value: "client.updated", label: "Client updated", ticketless: true },
  { value: "notification.created", label: "Notification created", ticketless: true },
  { value: "sla.escalation_fired", label: "SLA escalation fired", ticketless: true },
];

const PRIORITIES: Priority[] = ["p1", "p2", "p3", "p4"];
const CATEGORIES: TicketStatusCategory[] = ["open", "in_progress", "waiting", "closed"];

// The built-in notification kinds automations may send surfaced as
// operator-useful labels; the closed vocabulary stays the SDK's.
const NOTIFY_KINDS: { value: NotificationKind; label: string }[] = [
  { value: "mention", label: "Mention" },
  { value: "assignment", label: "Assignment" },
  { value: "sla_risk", label: "SLA risk" },
  { value: "sla_breach", label: "SLA breach" },
  { value: "escalation", label: "Escalation" },
  { value: "ticket_update", label: "Ticket update" },
];

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

const PRIORITY_LABEL: Record<Priority, string> = {
  p1: "P1 — critical",
  p2: "P2 — high",
  p3: "P3 — normal",
  p4: "P4 — low",
};

const CATEGORY_LABEL: Record<TicketStatusCategory, string> = {
  open: "open",
  in_progress: "in progress",
  waiting: "waiting",
  closed: "closed",
};

function conditionText(c: AutomationCondition): string {
  if (c.type === "priority_is") return `Priority is ${PRIORITY_LABEL[c.priority]}`;
  if (c.type === "category_is") return `Status category is ${CATEGORY_LABEL[c.category]}`;
  if (c.type === "title_contains") return `Title contains “${c.needle}”`;
  if (c.type === "csat_score_below") return `CSAT score below ${c.threshold}`;
  return "Ticket is unassigned";
}

function actionText(a: AutomationActionDraft, userName: (id: string) => string): string {
  if (a.type === "plugin_action") {
    const params = a.params as { body?: unknown };
    return `${a.entry_key} “${String(params.body ?? "")}”`;
  }
  if (a.type === "set_priority") return `Set priority to ${PRIORITY_LABEL[a.priority]}`;
  if (a.type === "add_internal_comment") return `Add internal note “${a.body}”`;
  if (a.type === "assign_user") return `Assign to ${userName(a.user_id)}`;
  return `Notify ${a.user_ids.length === 1 ? userName(a.user_ids[0]!) : `${a.user_ids.length} users`} · ${a.kind.replace(/_/g, " “")}“${a.title}”`;
}

// Client-side typed validation mirroring the server's要求 — shape errors
// surface here, never as raw wire prose.
function emptyCondition(c: AutomationCondition): string | null {
  if (c.type === "title_contains" && c.needle.trim() === "") return "Title-contains needs text.";
  return null;
}

function emptyAction(a: AutomationActionDraft, users: OrgUser[]): string | null {
  if (a.type === "plugin_action") {
    if (a.entry_key === "") return "Plugin action needs a registered kind.";
    const body = (a.params as { body?: unknown }).body;
    if (typeof body !== "string" || body.trim() === "")
      return "Plugin action needs a note body.";
    return null;
  }
  if (a.type === "add_internal_comment" && a.body.trim() === "")
    return "Internal note needs a body.";
  if (a.type === "assign_user" && !UUID_RE.test(a.user_id)) return "Assign needs a user.";
  if (a.type === "notify_users") {
    if (a.user_ids.length === 0) return "Notify needs at least one user.";
    if (a.title.trim() === "") return "Notify needs a title.";
  }
  void users;
  return null;
}

export default function AutomationsPage() {
  const auth = useRequireAuth();

  // §4.6 surface registry: the org's plugin action kinds for the picker.
  useEffect(() => {
    if (auth.status !== "authed") return;
    let cancelled = false;
    void (async () => {
      try {
        const res = await api.api.GET("/api/v1/plugins/surface/automation-actions", {});
        if (cancelled) return;
        setPluginActions(
          (res.data?.entries ?? []).map((e) => ({
            entry_key: e.entry_key,
            label: e.label,
          })),
        );
      } catch {
        if (!cancelled) setPluginActions([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [auth.status]);

  const [rules, setRules] = useState<AutomationRuleView[] | null>(null);
  const [view, setView] = useState("all");
  const [clauses, setClauses] = useState<{ id: string; field: string; op: never; value?: string }[]>(
    [],
  );
  const [columns, setColumns] = useState<RuleColumn[]>(RULE_COLUMNS);
  const [suggestedNudge, setSuggestedNudge] = useState(false);
  const density = useDensity();
  const [boards, setBoards] = useState<BoardNavEntry[]>([]);
  const [users, setUsers] = useState<OrgUser[]>([]);
  const [loading, setLoading] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [rowError, setRowError] = useState<string | null>(null);

  // Modal create/edit rule state
  const [modalOpen, setModalOpen] = useState(false);
  const [editingRule, setEditingRule] = useState<AutomationRuleView | null>(null);
  const [name, setName] = useState("");
  const [boardId, setBoardId] = useState<string>("");
  const [triggerEvent, setTriggerEvent] = useState<string>("ticket.created");
  // Rule subject (Wave 43 lane 3): a `board` rule fires on envelopes that
  // carry no ticket — the inbound webhook events above.
  const [subjectKind, setSubjectKind] = useState<"ticket" | "board">("ticket");

  // Condition builder state
  const [condType, setCondType] = useState<"priority_is" | "category_is" | "title_contains" | "unassigned">("priority_is");
  const [condPriority, setCondPriority] = useState<Priority>("p1");
  const [condCategory, setCondCategory] = useState<TicketStatusCategory>("open");
  const [condNeedle, setCondNeedle] = useState("");
  const [condError, setCondError] = useState<string | null>(null);
  const [conditions, setConditions] = useState<AutomationCondition[]>([]);

  // Action builder state
  const [actType, setActType] = useState<string>("set_priority");
  // §4.6: registered plugin action kinds (entry_key + label), listed
  // after the built-ins in the action picker.
  const [pluginActions, setPluginActions] = useState<{ entry_key: string; label: string }[]>([]);
  const [actPriority, setActPriority] = useState<Priority>("p1");
  const [actBody, setActBody] = useState("");
  const [actUserId, setActUserId] = useState("");
  const [actNotifyUsers, setActNotifyUsers] = useState<string[]>([]);
  const [actNotifyKind, setActNotifyKind] = useState<NotificationKind>("ticket_update");
  const [actNotifyTitle, setActNotifyTitle] = useState("");
  const [actError, setActError] = useState<string | null>(null);
  const [actions, setActions] = useState<AutomationActionDraft[]>([]);

  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [deleteArm, setDeleteArm] = useState<string | null>(null);

  // Dry-run test-run state, one rule at a time (requestId guards races).
  const [testRun, setTestRun] = useState<AutomationTestRunResponse | null>(null);
  const [testingId, setTestingId] = useState<string | null>(null);
  const [testError, setTestError] = useState<string | null>(null);
  const [testedId, setTestedId] = useState<string | null>(null);
  const testRunSeqRef = useRef(0);

  const handleTestRun = async (rule: AutomationRuleView) => {
    const seq = ++testRunSeqRef.current;
    setTestingId(rule.id);
    setTestError(null);
    const { data, error } = await api.api.POST("/api/v1/automation-rules/{id}/test-run", {
      params: { path: { id: rule.id } },
    });
    if (seq !== testRunSeqRef.current) return;
    setTestingId(null);
    setTestedId(rule.id);
    if (error !== undefined || data === undefined) {
      setTestError(rejectionText(error as never, "The test run failed. Verify the rule still exists."));
      setTestRun(null);
      return;
    }
    setTestRun(data);
  };

  const refreshUsers = useCallback(async () => {
    const { data } = await api.api.GET("/api/v1/org/users", {
      params: { query: { limit: 200 } },
    });
    if (data !== undefined) setUsers(data.users);
  }, []);

  const refreshRules = useCallback(async () => {
    setLoading(true);
    setErrorMsg(null);
    const { data, error } = await api.api.GET("/api/v1/automation-rules");
    setLoading(false);
    if (error !== undefined || data === null) {
      setErrorMsg("Failed to load automation rules.");
      setRules([]);
      return;
    }
    setRules((data as unknown as AutomationRuleView[]) ?? []);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") {
      void api.api.GET("/api/v1/boards/nav").then(({ data }) => {
        if (data !== undefined) setBoards(data);
      });
      void refreshUsers();
      void refreshRules();
    }
  }, [auth.status, refreshUsers, refreshRules]);

  const userName = useCallback(
    (id: string) => {
      const u = users.find((x) => x.id === id);
      // Honest label when the membership roster hasn't resolved (or the user
      // was deactivated) — never a raw UUID fragment.
      return u !== undefined ? u.display_name || u.email : "unknown user";
    },
    [users],
  );

  const openNewRule = () => {
    setEditingRule(null);
    setName("");
    setBoardId("");
    setTriggerEvent("ticket.created");
    setSubjectKind("ticket");
    setConditions([]);
    setActions([]);
    setFormError(null);
    setModalOpen(true);
  };

  const openEditRule = (r: AutomationRuleView) => {
    setEditingRule(r);
    setName(r.name);
    setBoardId(r.board_id ?? "");
    setTriggerEvent(r.trigger_event);
    setSubjectKind(r.subject_kind === "board" ? "board" : "ticket");
    setConditions(r.conditions ?? []);
    setActions(r.actions ?? []);
    setFormError(null);
    setModalOpen(true);
  };

  const handleAddCondition = () => {
    const cond: AutomationCondition =
      condType === "priority_is"
        ? { type: "priority_is", priority: condPriority }
        : condType === "category_is"
          ? { type: "category_is", category: condCategory }
          : condType === "title_contains"
            ? { type: "title_contains", needle: condNeedle.trim() }
            : { type: "unassigned" };
    const problem = emptyCondition(cond);
    setCondError(problem);
    if (problem !== null) return;
    setConditions([...conditions, cond]);
    setCondError(null);
    setCondNeedle("");
  };

  const handleAddAction = () => {
    const act: AutomationActionDraft =
      actType.startsWith("plugin:")
        ? {
            type: "plugin_action",
            entry_key: actType.slice("plugin:".length),
            params: { body: actBody.trim() },
          }
        : actType === "set_priority"
        ? { type: "set_priority", priority: actPriority }
        : actType === "add_internal_comment"
        ? { type: "add_internal_comment", body: actBody.trim() }
        : actType === "assign_user"
        ? { type: "assign_user", user_id: actUserId.trim() }
        : {
            type: "notify_users",
            user_ids: actNotifyUsers,
            kind: actNotifyKind,
            title: actNotifyTitle.trim(),
          };
    const problem = emptyAction(act, users);
    setActError(problem);
    if (problem !== null) return;
    setActions([...actions, act]);
    setActError(null);
    setActBody("");
    setActNotifyTitle("");
  };

  const draftValid =
    name.trim() !== "" &&
    conditions.every((c) => emptyCondition(c) === null) &&
    actions.every((a) => emptyAction(a, users) === null);

  const handleSaveRule = async () => {
    if (!draftValid || actions.length === 0) return;
    setSaving(true);
    setFormError(null);
    const body = {
      name: name.trim(),
      board_id: boardId === "" || boardId === "all" ? null : boardId,
      subject_kind: subjectKind,
      trigger_event: triggerEvent,
      conditions,
      actions,
      idempotency_key: idempotencyKey(),
    } as never;
    const { error } =
      editingRule !== null
        ? await api.api.PATCH("/api/v1/automation-rules/{id}", {
            params: { path: { id: editingRule.id } },
            body,
          })
        : await api.api.POST("/api/v1/automation-rules", { body });
    setSaving(false);
    if (error !== undefined) {
      setFormError(
        editingRule !== null
          ? rejectionText(error as never, "The rule could not be updated. Verify the conditions and actions.")
          : rejectionText(error as never, "The rule could not be created. Verify the conditions and actions."),
      );
      return;
    }
    setModalOpen(false);
    setEditingRule(null);
    setName("");
    setConditions([]);
    setActions([]);
    void refreshRules();
  };

  const handleToggleRule = async (rule: AutomationRuleView) => {
    setRowError(null);
    setRules((cur) =>
      (cur ?? []).map((r) => (r.id === rule.id ? { ...r, enabled: !r.enabled } : r)),
    );
    const { error } = await api.api.PATCH("/api/v1/automation-rules/{id}", {
      params: { path: { id: rule.id } },
      body: { enabled: !rule.enabled, idempotency_key: idempotencyKey() } as never,
    });
    if (error !== undefined) {
      setRules((cur) =>
        (cur ?? []).map((r) => (r.id === rule.id ? { ...r, enabled: rule.enabled } : r)),
      );
      setRowError(rejectionText(error as never, "The enable toggle was rejected."));
      return;
    }
    void refreshRules();
  };

  const handleDeleteRule = async (id: string) => {
    setRowError(null);
    const { error } = await api.api.DELETE("/api/v1/automation-rules/{id}", {
      params: { path: { id } },
    });
    setDeleteArm(null);
    if (error !== undefined) {
      setRowError(rejectionText(error as never, "The rule could not be deleted."));
      return;
    }
    void refreshRules();
  };

  const rulesSorted = useMemo(
    () =>
      (rules ?? [])
        .slice()
        .sort((a, b) => ((a.updated_at ?? "") < (b.updated_at ?? "") ? -1 : 1)),
    [rules],
  );

  const visibleRules = useMemo(() => {
    let rows = rulesSorted;
    if (view === "enabled") rows = rows.filter((r) => r.enabled);
    if (view === "paused") rows = rows.filter((r) => !r.enabled);
    for (const clause of clauses) {
      const value = (clause.value ?? "").toLowerCase();
      if (clause.field === "name") rows = rows.filter((r) => r.name.toLowerCase().includes(value));
      if (clause.field === "trigger")
        rows = rows.filter((r) => r.trigger_event.toLowerCase().includes(value));
      if (clause.field === "board") {
        rows = rows.filter((r) => {
          const name = r.board_id == null ? "all boards" : (boards.find((b) => b.id === r.board_id)?.name ?? "");
          return name.toLowerCase().includes(value);
        });
      }
    }
    return rows;
  }, [rulesSorted, view, clauses, boards]);

  const shownColumns = columns.filter((c) => c.visible);

  if (auth.status !== "authed") return null;

  return (
    <PageFrame>
      <SurfaceHeader
        title="Automation rules"
        description="WHEN · IF · THEN, in that order"
        meta={
          <>
            <StatPill
              tone="gold"
              label="Enabled"
              count={rulesSorted.filter((r) => r.enabled).length}
              onClick={() => setView(view === "enabled" ? "all" : "enabled")}
              pressed={view === "enabled"}
            />
            <StatPill
              tone="neutral"
              label="Paused"
              count={rulesSorted.filter((r) => !r.enabled).length}
              onClick={() => setView(view === "paused" ? "all" : "paused")}
              pressed={view === "paused"}
            />
            <StatPill
              tone="blush"
              label="Match every ticket"
              count={rulesSorted.filter((r) => (r.conditions ?? []).length === 0).length}
            />
          </>
        }
        actions={
          <>
            <Button variant="secondary" asChild>
              <Link href="/automations/canvas">Open the canvas</Link>
            </Button>
            <Button onClick={openNewRule} data-testid="automation-new-rule">
              <PlusIcon size={16} /> New rule
            </Button>
          </>
        }
      />

      {errorMsg !== null ? (
        <div className="mb-2">
          <BlushNote title="The rules did not load.">{errorMsg}</BlushNote>
        </div>
      ) : null}
      {rowError !== null ? (
        <div className="mb-2" role="alert">
          <BlushNote title="The last action on a rule was rejected.">{rowError}</BlushNote>
        </div>
      ) : null}

      <div className="flex min-h-0 flex-1 flex-col">
        <DataToolbar
          label="Automation controls"
          views={
            <ViewSwitch
              label="Rule views"
              views={[
                { id: "all", label: "All rules", hint: String(rulesSorted.length) },
                { id: "enabled", label: "Enabled" },
                { id: "paused", label: "Paused" },
              ]}
              activeId={view}
              onChange={setView}
            />
          }
          filters={
            <FilterBar
              label="Rule filters"
              emptyHint="No filters"
              fields={[
                { key: "name", label: "Rule", operators: ["contains"] },
                { key: "trigger", label: "Trigger", operators: ["contains"] },
                { key: "board", label: "Board", operators: ["contains"] },
              ]}
              clauses={clauses}
              onChange={(next) => setClauses(next.map((c) => ({ ...c, op: c.op as never })))}
            />
          }
          columns={
            <ColumnChooser
              label="Rule columns"
              columns={columns.map((c) => ({
                key: c.key,
                label: c.label,
                visible: c.visible,
                required: c.required,
              }))}
              onChange={(next) =>
                setColumns(
                  next.map((c) => ({
                    key: c.key as RuleColumnKey,
                    label: c.label,
                    visible: c.visible,
                    required: c.required,
                  })),
                )
              }
            />
          }
          export={
            <ExportControl
              label="Export"
              formats={["csv"]}
              columns={shownColumns.map((c) => c.label)}
              rowCount={visibleRules.length}
              filters={clauses}
              onExport={() =>
                downloadCsv(
                  "automation-rules.csv",
                  shownColumns.map((c) => c.label),
                  visibleRules.map((r) =>
                    shownColumns.map((c) => {
                      if (c.key === "rule") return r.name;
                      if (c.key === "status") return r.enabled ? "enabled" : "paused";
                      if (c.key === "trigger")
                        return r.subject_kind === "board"
                          ? `${r.trigger_event} (no ticket)`
                          : r.trigger_event;
                      if (c.key === "conditions")
                        return (r.conditions ?? []).map((x) => x.type).join(" ");
                      if (c.key === "actions") return (r.actions ?? []).map((x) => x.type).join(" ");
                      if (c.key === "board")
                        return r.board_id == null
                          ? "All boards"
                          : (boards.find((b) => b.id === r.board_id)?.name ?? "—");
                      return r.updated_at ?? "";
                    }),
                  ),
                )
              }
            />
          }
        />

        <TableCard label="Automation rules">
          {loading && rules === null ? (
            <SkeletonRows sentence="Loading the rules." columns={5} />
          ) : visibleRules.length === 0 ? (
            rulesSorted.length === 0 ? (
              <EmptyState
                message="No automation rules yet. A rule answers: WHEN an event fires, IF these conditions hold, THEN apply these effects."
                action={
                  <Button onClick={openNewRule}>New rule</Button>
                }
              />
            ) : (
              <NoMatches
                title="No rules match these filters."
                hint="Try removing one. Every rule you have is still here."
                noun="rules"
                onClear={() => {
                  setClauses([]);
                  setView("all");
                }}
              />
            )
          ) : (
            <Table data-testid="automation-rules-table">
              <thead>
                <tr>
                  {shownColumns.map((c) => (
                    <Th key={c.key} className={HEAD_CELL_CLASS}>
                      {c.label}
                    </Th>
                  ))}
                  {/* The buttons are the row's affordance, not another column
                      of the record, so this header stays empty rather than
                      saying "Actions" a second time. */}
                  <Th className={`${HEAD_CELL_CLASS} text-right`}>
                    <span className="sr-only">Row actions</span>
                  </Th>
                </tr>
              </thead>
              <tbody>
                {visibleRules.map((r) => (
                  <Tr key={r.id} style={{ height: "var(--m-density-row)" }}>
                    {shownColumns.map((c) => (
                      <Td key={c.key}>
                        {c.key === "rule" ? (
                          <div className="flex min-w-0 items-center gap-2">
                            <button
                              type="button"
                              className="truncate font-semibold underline decoration-transparent underline-offset-2 transition-colors duration-100 hover:decoration-[var(--m-border-strong)]"
                              style={{ color: "var(--m-text)" }}
                              onClick={() => openEditRule(r)}
                            >
                              {r.name}
                            </button>
                          </div>
                        ) : null}
                        {c.key === "status" ? (
                          <StatusPill tone={r.enabled ? "success" : "neutral"}>
                            {r.enabled ? "enabled" : "paused"}
                          </StatusPill>
                        ) : null}
                        {c.key === "trigger" ? (
                          <span className="flex flex-wrap items-center gap-1.5">
                            <code className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                              {r.trigger_event}
                            </code>
                            {r.subject_kind === "board" ? (
                              <Badge tone="neutral">no ticket</Badge>
                            ) : null}
                          </span>
                        ) : null}
                        {c.key === "conditions" ? (
                          (r.conditions ?? []).length === 0 ? (
                            // P3: the gap and its one-click fix, in the row.
                            <InlineVerdict
                              tone="blush"
                              onFix={() => {
                                openEditRule(r);
                                setSuggestedNudge(true);
                              }}
                              fixLabel="Add one"
                            >
                              {r.subject_kind === "board"
                                ? "matches every occurrence"
                                : "matches every ticket"}
                            </InlineVerdict>
                          ) : (
                            <span className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                              {(r.conditions ?? []).length} condition
                              {(r.conditions ?? []).length === 1 ? "" : "s"}
                            </span>
                          )
                        ) : null}
                        {c.key === "actions" ? (
                          <span className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                            {(r.actions ?? []).length} action
                            {(r.actions ?? []).length === 1 ? "" : "s"}
                          </span>
                        ) : null}
                        {c.key === "board" ? (
                          <span style={{ color: "var(--m-text-secondary)" }}>
                            {r.board_id == null
                              ? "All boards"
                              : (boards.find((b) => b.id === r.board_id)?.name ?? "—")}
                          </span>
                        ) : null}
                        {c.key === "updated" ? (
                          <span className="tabular-nums text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                            {r.updated_at == null ? "—" : new Date(r.updated_at).toLocaleDateString()}
                          </span>
                        ) : null}
                      </Td>
                    ))}
                    <Td className="text-right">
                      <div className="flex justify-end gap-1">
                        <Button variant="ghost" size="sm" asChild>
                          <Link
                            href={`/automations/canvas?rule=${r.id}`}
                            aria-label={`Open ${r.name} in the canvas`}
                          >
                            Open in canvas
                          </Link>
                        </Button>
                        <Button
                          data-testid={`automation-test-run-${r.id}`}
                          variant="ghost"
                          size="sm"
                          disabled={testingId !== null && testingId !== r.id}
                          onClick={() => void handleTestRun(r)}
                        >
                          {testingId === r.id ? <Spinner /> : null} Test run
                        </Button>
                        <Button variant="ghost" size="sm" onClick={() => void handleToggleRule(r)}>
                          {r.enabled ? "Pause" : "Enable"}
                        </Button>
                        <Button variant="ghost" size="sm" onClick={() => openEditRule(r)}>
                          Edit
                        </Button>
                        {deleteArm === r.id ? (
                          <>
                            <Button variant="destructive-solid" size="sm" onClick={() => void handleDeleteRule(r.id)}>
                              Confirm delete
                            </Button>
                            <Button variant="ghost" size="sm" onClick={() => setDeleteArm(null)}>
                              Keep
                            </Button>
                          </>
                        ) : (
                          <Button variant="ghost" size="sm" onClick={() => setDeleteArm(r.id)}>
                            Delete
                          </Button>
                        )}
                      </div>
                    </Td>
                  </Tr>
                ))}
              </tbody>
            </Table>
          )}
        </TableCard>

        <LedgerFooter
          shown={visibleRules.length}
          total={rulesSorted.length}
          noun="rules"
          density={<DensitySwitch value={density} onChange={applyDensity} />}
          hints={
            <>
              <KeyHint keys="J K" label="move" />
              <KeyHint keys="↵" label="edit" />
              <KeyHint keys="F" label="filter" />
            </>
          }
        />
      </div>

      {testError !== null && testedId !== null ? (
        <div className="mt-2" role="alert">
          <BlushNote title="The test run did not come back.">{testError}</BlushNote>
        </div>
      ) : null}
      {testRun !== null ? (
        <div className="mt-3">
          <TestRunResult
            result={testRun}
            userName={userName}
            onClose={() => {
              setTestRun(null);
              setTestError(null);
            }}
          />
        </div>
      ) : null}

      <Dialog open={modalOpen} onOpenChange={setModalOpen}>
        <DialogContent
          title={editingRule !== null ? "Edit automation rule" : "New automation rule"}
          description="WHEN the trigger event fires, IF every condition holds, THEN apply each action in order."
        >
          <div className="grid max-h-[75vh] gap-4 overflow-y-auto p-4">
            <div>
              <Label htmlFor="ar-name">Rule name</Label>
              <Input
                id="ar-name"
                placeholder="e.g. Escalate unassigned VIP tickets"
                value={name}
                onChange={(e) => setName(e.target.value)}
                required
              />
            </div>

            {/* WHEN — the subject, the trigger and the scope it fires in */}
            <RuleStep
              word="WHEN"
              summary={
                <>
                  <code className="">{triggerEvent}</code> fires{" "}
                  {subjectKind === "board" ? "with no ticket" : "on a ticket"} on{" "}
                  {boardId === "" || boardId === "all"
                    ? "every board"
                    : (boards.find((b) => b.id === boardId)?.name ?? "one board")}
                  .
                </>
              }
            >
              <div className="grid gap-3">
                <div>
                  <Label htmlFor="ar-subject">Rule subject</Label>
                  <Select
                    value={subjectKind}
                    onValueChange={(v) => {
                      const next = v === "board" ? "board" : "ticket";
                      setSubjectKind(next);
                      // Keep the draft coherent: a subject swap moves the
                      // trigger to a matching event rather than leaving an
                      // impossible pair selected.
                      const current = TRIGGER_EVENTS.find((ev) => ev.value === triggerEvent);
                      if (next === "board" && current?.ticketless !== true) {
                        setTriggerEvent("webhook.ingested");
                      }
                      if (next === "ticket" && current?.ticketless === true) {
                        setTriggerEvent("ticket.created");
                      }
                    }}
                  >
                    <SelectTrigger id="ar-subject" className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="ticket">Ticket — fires on events about a ticket</SelectItem>
                      <SelectItem value="board">Board — fires on events with no ticket</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                <div className="grid gap-3 md:grid-cols-2">
                  <div>
                    <Label htmlFor="ar-board">Board scope</Label>
                    <Select
                      value={boardId}
                      onValueChange={(v) => setBoardId(v ?? "")}
                    >
                      <SelectTrigger id="ar-board" className="w-full">
                        <SelectValue placeholder="All boards (org-wide)" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="all">All boards</SelectItem>
                        {boards.map((b) => (
                          <SelectItem key={b.id} value={b.id}>
                            {b.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>

                  <div>
                    <Label htmlFor="ar-event">Trigger event</Label>
                    <Select value={triggerEvent} onValueChange={(v) => setTriggerEvent(v ?? "ticket.created")}>
                      <SelectTrigger id="ar-event" className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {TRIGGER_EVENTS.filter((ev) =>
                          subjectKind === "board" ? ev.ticketless === true : ev.ticketless !== true,
                        ).map((ev) => (
                          <SelectItem key={ev.value} value={ev.value}>
                            {ev.label}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <FieldHint>
                      {subjectKind === "board"
                        ? "Ticket-less events — an inbound webhook, an alert, or another platform envelope. Plugin-emitted types need an org grant and are not listable until the registry gains a read endpoint."
                        : "Events about a ticket. Inbound webhook events live under the board subject."}
                    </FieldHint>
                  </div>
                </div>
              </div>
            </RuleStep>

            {/* IF — conditions, with the suggested one offered in place */}
            <RuleStep
              word="IF"
              summary={
                conditions.length === 0 ? (
                  "Every occurrence of the trigger matches." + (suggestedNudge ? " That is usually not what you meant." : "")
                ) : (
                  `All ${conditions.length} condition${conditions.length === 1 ? "" : "s"} must hold.`
                )
              }
            >
              {conditions.length === 0 ? (
                <div className="flex flex-wrap items-center gap-2">
                  <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                    No conditions — the rule matches every occurrence of the trigger.
                  </p>
                  {/* P3, not a panel: the suggestion sits in the rule it is
                      about, states what it would do, and adds itself in one
                      click. An assistant column for a one-line fix would cost
                      more screen than the fix is worth. */}
                  <InlineVerdict
                    tone="blush"
                    onFix={() => {
                      setConditions([
                        ...conditions,
                        { type: "unassigned" } as AutomationCondition,
                      ]);
                      setSuggestedNudge(false);
                    }}
                    fixLabel="Add “unassigned”"
                  >
                    Most new rules want one condition
                  </InlineVerdict>
                </div>
              ) : (
                <div className="mt-2 grid gap-1">
                  {conditions.map((c, i) => (
                    <ConditionChip key={i} text={conditionText(c)} onRemove={() =>
                      setConditions(conditions.filter((_, j) => j !== i))
                    } />
                  ))}
                </div>
              )}
              {subjectKind === "board" ? (
                <p className="mt-2 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                  Board rules fire on envelopes with no ticket, so ticket-shaped
                  conditions (priority, status, assignee) never match them — the
                  engine skips them with a warning.
                </p>
              ) : null}
              <div className="mt-3 flex flex-wrap items-end gap-2 border-t pt-3" style={{ borderColor: "var(--m-border)" }}>
                <div className="w-40">
                  <Label htmlFor="ar-cond-type">Add condition</Label>
                  <Select value={condType} onValueChange={(v) => setCondType(v as typeof condType)}>
                    <SelectTrigger id="ar-cond-type" className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="priority_is">Priority is</SelectItem>
                      <SelectItem value="category_is">Status category is</SelectItem>
                      <SelectItem value="title_contains">Title contains</SelectItem>
                      <SelectItem value="unassigned">Ticket is unassigned</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                {condType === "priority_is" ? (
                  <div className="w-36">
                    <Label htmlFor="ar-cond-priority">Priority</Label>
                    <Select value={condPriority} onValueChange={(v) => setCondPriority(v as Priority)}>
                      <SelectTrigger id="ar-cond-priority" className="w-full"><SelectValue /></SelectTrigger>
                      <SelectContent>
                        {PRIORITIES.map((p) => (
                          <SelectItem key={p} value={p}>{PRIORITY_LABEL[p]}</SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                ) : null}

                {condType === "category_is" ? (
                  <div className="w-36">
                    <Label htmlFor="ar-cond-cat">Category</Label>
                    <Select value={condCategory} onValueChange={(v) => setCondCategory(v as TicketStatusCategory)}>
                      <SelectTrigger id="ar-cond-cat" className="w-full"><SelectValue /></SelectTrigger>
                      <SelectContent>
                        {CATEGORIES.map((c) => (
                          <SelectItem key={c} value={c}>{CATEGORY_LABEL[c]}</SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                ) : null}

                {condType === "title_contains" ? (
                  <div className="w-48">
                    <Label htmlFor="ar-cond-needle">Text substring</Label>
                    <Input
                      id="ar-cond-needle"
                      value={condNeedle}
                      onChange={(e) => setCondNeedle(e.target.value)}
                      placeholder="Outage / urgent"
                    />
                  </div>
                ) : null}

                <Button size="sm" variant="secondary" onClick={handleAddCondition}>
                  Add condition
                </Button>
                {condError !== null ? (
                  <p className="w-full text-[11px]" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
                    {condError}
                  </p>
                ) : null}
              </div>
            </RuleStep>

            {/* THEN — actions, applied in order */}
            <RuleStep
              word="THEN"
              summary={
                actions.length === 0
                  ? "Nothing happens yet — a rule needs at least one action."
                  : `${actions.length} action${actions.length === 1 ? "" : "s"}, applied in order.`
              }
            >
              {actions.length === 0 ? (
                <p className="mt-1 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                  No actions yet — add at least one.
                </p>
              ) : (
                <div className="mt-2 grid gap-1">
                  {actions.map((a, i) => (
                    <ActionChip key={i} index={i} text={actionText(a, userName)} onRemove={() =>
                      setActions(actions.filter((_, j) => j !== i))
                    } />
                  ))}
                </div>
              )}
              <div className="mt-3 flex flex-wrap items-end gap-2 border-t pt-3" style={{ borderColor: "var(--m-border)" }}>
                <div className="w-44">
                  <Label htmlFor="ar-act-type">Add action</Label>
                  <Select value={actType} onValueChange={(v) => setActType(v as typeof actType)}>
                    <SelectTrigger id="ar-act-type" className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="set_priority">Set priority</SelectItem>
                      <SelectItem value="add_internal_comment">Add internal note</SelectItem>
                      <SelectItem value="assign_user">Assign to user</SelectItem>
                      <SelectItem value="notify_users">Notify users</SelectItem>
                      {pluginActions.map((p) => (
                        <SelectItem key={p.entry_key} value={`plugin:${p.entry_key}`}>
                          {p.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>

                {actType === "set_priority" ? (
                  <div className="w-36">
                    <Label htmlFor="ar-act-priority">Priority</Label>
                    <Select value={actPriority} onValueChange={(v) => setActPriority(v as Priority)}>
                      <SelectTrigger id="ar-act-priority" className="w-full"><SelectValue /></SelectTrigger>
                      <SelectContent>
                        {PRIORITIES.map((p) => (
                          <SelectItem key={p} value={p}>{PRIORITY_LABEL[p]}</SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                ) : null}

                {actType === "add_internal_comment" || actType.startsWith("plugin:") ? (
                  <div className="w-56">
                    <Label htmlFor="ar-act-body">Note body</Label>
                    <Input
                      id="ar-act-body"
                      value={actBody}
                      onChange={(e) => setActBody(e.target.value)}
                      placeholder={actType.startsWith("plugin:") ? "Note the plugin action adds" : "Automated triage note"}
                    />
                  </div>
                ) : null}

                {actType === "assign_user" ? (
                  <div className="w-56">
                    <Label htmlFor="ar-act-user">User</Label>
                    <Select value={actUserId} onValueChange={(v) => setActUserId(v ?? "")}>
                      <SelectTrigger id="ar-act-user" className="w-full">
                        <SelectValue placeholder="Pick a user" />
                      </SelectTrigger>
                      <SelectContent>
                        {users
                          .filter((u) => u.is_active)
                          .map((u) => (
                            <SelectItem key={u.id} value={u.id}>
                              {u.display_name || u.email}
                            </SelectItem>
                          ))}
                      </SelectContent>
                    </Select>
                  </div>
                ) : null}

                {actType === "notify_users" ? (
                  <div className="grid w-full gap-2 md:grid-cols-3">
                    <div>
                      <Label htmlFor="ar-act-kind">Kind</Label>
                      <Select value={actNotifyKind} onValueChange={(v) => setActNotifyKind(v as NotificationKind)}>
                        <SelectTrigger id="ar-act-kind" className="w-full"><SelectValue /></SelectTrigger>
                        <SelectContent>
                          {NOTIFY_KINDS.map((k) => (
                            <SelectItem key={k.value} value={k.value}>{k.label}</SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                    <div className="md:col-span-2">
                      <MultiUserPicker
                        users={users.filter((u) => u.is_active)}
                        selected={actNotifyUsers}
                        onChange={setActNotifyUsers}
                      />
                    </div>
                    <div className="md:col-span-3">
                      <Label htmlFor="ar-act-title">Notification title</Label>
                      <Input
                        id="ar-act-title"
                        value={actNotifyTitle}
                        onChange={(e) => setActNotifyTitle(e.target.value)}
                        placeholder="e.g. Unassigned P1 ticket"
                      />
                    </div>
                  </div>
                ) : null}

                <Button size="sm" variant="secondary" onClick={handleAddAction}>
                  Add action
                </Button>
                {actError !== null ? (
                  <p className="w-full text-[11px]" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
                    {actError}
                  </p>
                ) : null}
              </div>
            </RuleStep>

            {formError !== null ? (
              <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {formError}
              </p>
            ) : null}

            <DialogActions>
              <Button variant="ghost" onClick={() => setModalOpen(false)}>
                Cancel
              </Button>
              <Button
                onClick={() => void handleSaveRule()}
                disabled={saving || name.trim() === "" || actions.length === 0}
                data-testid="automation-save-rule"
              >
                {saving ? <Spinner /> : null} Save rule
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>
    </PageFrame>
  );
}

function idempotencyKey(): string {
  return typeof crypto !== "undefined" && crypto.randomUUID !== undefined
    ? crypto.randomUUID()
    : `auto-${Date.now()}`;
}

/** The honest dry-run rendering: sampled vs matched counts, the matched
 * tickets as they exist right now, and the rule's CURRENT action documents
 * shown verbatim as the proposal — nothing was executed. */
function TestRunResult({
  result,
  userName,
  onClose,
}: {
  result: AutomationTestRunResponse;
  userName(id: string): string;
  onClose(): void;
}) {
  const previewActions = Array.isArray(result.actions_preview)
    ? (result.actions_preview as AutomationAction[])
    : [];
  return (
    <div
      className="mt-3 grid gap-2 rounded-md border p-3"
      style={{ borderColor: "var(--m-border)" }}
      data-testid="automation-test-run-result"
    >
      <div className="flex items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2 text-[12px]">
          <StepMark tone="when">DRY RUN</StepMark>
          <span className="tabular-nums">
            sampled {result.sampled_count} · matched {result.matched_count} ·{" "}
            {result.matched_count === 0
              ? "no tickets would fire this rule"
              : `${result.matched_count} ticket${result.matched_count === 1 ? "" : "s"} would match`}
          </span>
        </div>
        <button
          className="text-[11px] underline-offset-2 hover:underline"
          style={{ color: "var(--m-text-muted)" }}
          onClick={onClose}
        >
          Dismiss
        </button>
      </div>
      {result.matched_count > 0 ? (
        <ul className="grid gap-0.5">
          {result.matches.map((m) => (
            <li key={m.ticket_id} className="flex items-center gap-2 text-[12px]">
              <Link
                href={`/tickets/${m.ticket_id}`}
                className="font-medium underline decoration-[var(--m-border)] underline-offset-2 hover:text-[var(--m-accent)]"
              >
                #{m.number}
              </Link>
              <span className="min-w-0 truncate">{m.title}</span>
              <span className="shrink-0 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                {m.assignee_id === null || m.assignee_id === undefined ? "unassigned" : userName(m.assignee_id)}
              </span>
            </li>
          ))}
        </ul>
      ) : null}
      <div className="grid gap-1">
        {previewActions.map((a, i) => (
          <div key={i} className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            {i + 1}. {actionText(a, userName)}
          </div>
        ))}
      </div>
      <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
        Proposal only — actions are previewed, never executed, by a test run.
      </p>
    </div>
  );
}

function RuleChips({
  trigger,
  conditions,
  actions,
  userName,
}: {
  trigger: string;
  conditions: AutomationCondition[];
  actions: AutomationAction[];
  userName(id: string): string;
}) {
  const triggerLabel =
    TRIGGER_EVENTS.find((t) => t.value === trigger)?.label ?? trigger;
  return (
    <div className="grid gap-1.5 text-[12px]">
      <div className="flex flex-wrap items-center gap-1.5">
        <StepMark tone="when">WHEN</StepMark>
        <span className="text-[12px]">{triggerLabel}</span>
      </div>
      {conditions.length > 0 ? (
        <div className="flex flex-wrap items-center gap-1.5">
          <StepMark tone="if">IF</StepMark>
          <span className="text-[12px]">{conditions.map(conditionText).join(" · all of · ")}</span>
        </div>
      ) : null}
      <div className="flex flex-wrap items-center gap-1.5">
        <StepMark tone="then">THEN</StepMark>
        <span className="text-[12px]">
          {actions.map((a) => actionText(a, userName)).join(" → ")}
        </span>
      </div>
    </div>
  );
}

function StepMark({
  tone,
  children,
}: {
  tone: "when" | "if" | "then";
  children: React.ReactNode;
}) {
  return (
    <span
      className="rounded px-1.5 py-0.5 text-[10px] font-semibold tracking-wide"
      style={{
        background:
          tone === "when"
            ? "var(--m-accent)"
            : "color-mix(in srgb, var(--m-accent) 12%, transparent)",
        color: tone === "when" ? "var(--m-background)" : "var(--m-text)",
      }}
    >
      {children}
    </span>
  );
}

function ConditionChip({ text, onRemove }: { text: string; onRemove(): void }) {
  return (
    <div
      className="flex items-center justify-between gap-2 rounded-md px-2 py-1 text-[12px]"
      style={{ background: "color-mix(in srgb, var(--m-accent) 8%, transparent)" }}
    >
      <span className="truncate">{text}</span>
      <button
        className="shrink-0 text-[11px] underline-offset-2 hover:underline"
        style={{ color: "var(--m-status-danger-fg)" }}
        onClick={onRemove}
      >
        Remove
      </button>
    </div>
  );
}

function ActionChip({ index, text, onRemove }: { index: number; text: string; onRemove(): void }) {
  return (
    <div
      className="flex items-center justify-between gap-2 rounded-md px-2 py-1 text-[12px]"
      style={{ background: "color-mix(in srgb, var(--m-accent) 8%, transparent)" }}
    >
      <span className="min-w-0 truncate">
        <span className="mr-2 text-[10px] font-semibold tabular-nums" style={{ color: "var(--m-text-muted)" }}>
          {index + 1}.
        </span>
        {text}
      </span>
      <button
        className="shrink-0 text-[11px] underline-offset-2 hover:underline"
        style={{ color: "var(--m-status-danger-fg)" }}
        onClick={onRemove}
      >
        Remove
      </button>
    </div>
  );
}

function MultiUserPicker({
  users,
  selected,
  onChange,
}: {
  users: OrgUser[];
  selected: string[];
  onChange(next: string[]): void;
}) {
  return (
    <div>
      <Label>Notify who</Label>
      <div className="flex flex-wrap gap-1 pt-1">
        {users.slice(0, 8).map((u) => {
          const picked = selected.includes(u.id);
          return (
            <button
              key={u.id}
              className="rounded-full border px-2 py-0.5 text-[11px]"
              style={{
                borderColor: picked ? "var(--m-accent)" : "var(--m-border)",
                background: picked ? "var(--m-accent)" : "transparent",
                color: picked ? "var(--m-background)" : "var(--m-text-secondary)",
              }}
              aria-pressed={picked}
              onClick={() =>
                onChange(
                  picked ? selected.filter((x) => x !== u.id) : [...selected, u.id],
                )
              }
            >
              {u.display_name || u.email}
            </button>
          );
        })}
        {selected.some((id) => users.every((u) => u.id !== id)) ? (
          selected
            .filter((id) => users.every((u) => u.id !== id))
            .map((id) => (
              <span key={id} className="rounded-full border px-2 py-0.5 text-[11px]" style={{ borderColor: "var(--m-border)" }}>
                {id.slice(0, 8)}
                <button
                  className="ml-1 underline-offset-2 hover:underline"
                  onClick={() => onChange(selected.filter((x) => x !== id))}
                >
                  ×
                </button>
              </span>
            ))
        ) : null}
      </div>
      <FieldHint>Users beyond the first eight render by id chip.</FieldHint>
    </div>
  );
}
