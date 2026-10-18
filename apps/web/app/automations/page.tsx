// /automations — Event-triggered automation rule editor (Wave 11 lane 8;
// Wave 17 lane 6 UX pass; Wave 43 lane 17b rebuild).
//
// Purpose: list + author automation rules as WHEN (trigger event) → IF
// (conditions, ALL must match) → THEN (actions). The condition editor is
// built on `rule-grammar.ts`: a field declares its own operators, a bad row
// is reported by NAMING the offending token, and the one-sentence read-back
// is derived from the same clauses the controls show, so the two cannot
// disagree. "Test run" samples recent tickets through the rule WITHOUT
// executing anything and renders the result as a Terraform-style per-ticket
// diff of what WOULD change.
//
// AI legs (Wave 43 lane 17b), both on the native `general_chat` surface and
// both behind the org master switch — NOT the seeded `automation_writing`
// key, which is a premium plugin surface core only registers:
//   C1 — a live streamed summary of the draft, rendered as frames arrive.
//   C2 — one sentence of prose becomes a whole-workflow draft via lane 17's
//        authoring endpoint; it is projected into this editor as an editable
//        draft and is never saved by the AI itself.
// Both degrade quietly: when AI is refused, the deterministic read-back (the
// live sentence this page derives from the clauses on every keystroke) is
// what the person keeps.
//
// Non-purpose: no canvas builder (no drag-drop), no second prose path beside
// lane 17's endpoint, no new action types beyond the SDK's closed
// `AutomationAction` vocabulary.
"use client";

import {
  Badge,
  Button,
  Checkbox,
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
  SelectGroup,
  SelectItem,
  SelectLabel,
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
import { LocalTime } from "../../components/local-time";
import { AssistantCard } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useRouter } from "next/navigation";
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
import {
  OPERATOR_VERB,
  RULE_FIELDS,
  STARTERS,
  blankRow,
  conditionFromRow,
  describeClause,
  describeConditions,
  describeRule,
  fieldByKey,
  operatorNeedsValue,
  operatorTakesMany,
  rowFromCondition,
  rowProblems,
  withOperator,
  type AutomationCondition,
  type ClauseRow,
  type ConditionRow,
  type GrammarOperator,
  type Priority,
  type Starter,
} from "./rule-grammar";
import { projectDraft, type PluginActionDraft, type ProjectedAction } from "./draft-projection";
import { authorWorkflowDraft, streamDraftSummary, type AuthorDraft } from "./ai-client";
import {
  eventLabel,
  groupOf,
  groupsForSubject,
  hasTriggerValue,
  registryTriggerGroups,
  subjectOf,
  triggerPhrase,
  type NodeCatalogueView,
  type TriggerGroup,
} from "../../lib/workflow/trigger-picker";

type AutomationAction = components["schemas"]["AutomationAction"];
type BoardNavEntry = components["schemas"]["BoardNavEntry"];
type OrgUser = components["schemas"]["OrgUser"];
type NotificationKind = components["schemas"]["NotificationKind"];
type AutomationTestRunResponse = components["schemas"]["AutomationTestRunResponse"];

// §4.6: plugin-contributed action kinds ride the same action list. The
// SDK's closed vocabulary gains `plugin_action` when the pins regen; the
// local draft type keeps the editor honest in the meantime.
type AutomationActionDraft = AutomationAction | PluginActionDraft;

type TicketStatusCategory = components["schemas"]["TicketStatusCategory"];

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

const NOTIFY_KINDS: { value: NotificationKind; label: string }[] = [
  { value: "mention", label: "Mention" },
  { value: "assignment", label: "Assignment" },
  { value: "sla_risk", label: "SLA risk" },
  { value: "sla_breach", label: "SLA breach" },
  { value: "escalation", label: "Escalation" },
  { value: "ticket_update", label: "Ticket update" },
];

const PRIORITIES: Priority[] = ["p1", "p2", "p3", "p4"];

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

function actionText(a: AutomationActionDraft, userName: (id: string) => string): string {
  if (a.type === "plugin_action") {
    const body = (a.params as { body?: unknown }).body;
    return typeof body === "string" && body.trim() !== "" ? `${a.entry_key} — note “${body.trim()}”` : a.entry_key;
  }
  if (a.type === "set_priority") return `Set priority to ${PRIORITY_LABEL[a.priority]}`;
  if (a.type === "add_internal_comment") return `Add internal note “${a.body}”`;
  if (a.type === "assign_user") return `Assign to ${userName(a.user_id)}`;
  const who =
    a.user_ids.length === 0
      ? "nobody yet"
      : a.user_ids.length === 1
        ? userName(a.user_ids[0]!)
        : `${a.user_ids.length} people`;
  return `Notify ${who} · ${a.kind.replace(/_/g, " ")} — “${a.title}”`;
}

/** The action as a clause of the read-back sentence, not as a list label. */
function actionPhrase(a: AutomationActionDraft, userName: (id: string) => string): string {
  const text = actionText(a, userName);
  return text.charAt(0).toUpperCase() === text.charAt(0)
    ? text.charAt(0).toLowerCase() + text.slice(1)
    : text;
}

// Client-side typed validation mirroring the server's rule — shape errors
// surface here, never as raw wire prose.
function emptyAction(a: AutomationActionDraft, users: OrgUser[]): string | null {
  if (a.type === "plugin_action") {
    if (a.entry_key === "") return "Plugin action needs a registered kind.";
    const body = (a.params as { body?: unknown }).body;
    if (typeof body !== "string" || body.trim() === "") return "Plugin action needs a note body.";
    return null;
  }
  if (a.type === "add_internal_comment" && a.body.trim() === "") return "Internal note needs a body.";
  if (a.type === "assign_user" && !UUID_RE.test(a.user_id)) return "Assign needs a user.";
  if (a.type === "notify_users") {
    if (a.user_ids.length === 0) return "Notify needs at least one user.";
    if (a.title.trim() === "") return "Notify needs a title.";
  }
  void users;
  return null;
}

/**
 * A condition the editor holds, with its own stable identity.
 *
 * The grammar derives a row's id from its content, so two identical rows
 * would collide on it; React keys and control ids need one that does not
 * move when the content does.
 */
interface DraftCondition {
  uid: number;
  row: ConditionRow;
}

export default function AutomationsPage() {
  const auth = useRequireAuth();
  const router = useRouter();

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

  // AI availability: a grant on the native surface plus an enabled model in
  // the catalog. This is the affordance probe only — the admission edge is
  // still the authority, so a refusal here collapses the affordance rather
  // than leaving a button that fails.
  useEffect(() => {
    if (auth.status !== "authed") return;
    let cancelled = false;
    void (async () => {
      try {
        const res = await api.api.GET("/api/v1/ai/me/surfaces");
        if (cancelled) return;
        const surface = (res.data?.surfaces ?? []).find((s) => s.surface_key === "general_chat");
        setAiAvailable(surface?.granted === true && surface?.models_available === true);
      } catch {
        if (!cancelled) setAiAvailable(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [auth.status]);

  // The trigger catalogue. A failure leaves the picker holding only the event
  // the rule already carries, never an empty control that cannot be saved.
  const loadTriggerCatalogue = useCallback(async () => {
    setCatalogueError(null);
    try {
      const { data, error } = await api.api.GET("/api/v1/workflows/nodes", {
        params: { query: { family: "trigger" } as never },
      });
      if (error !== undefined || data === undefined) {
        setTriggerGroups(null);
        setCatalogueError(rejectionText(error as never, "The trigger catalogue did not load."));
        return;
      }
      setTriggerGroups(registryTriggerGroups(data as unknown as NodeCatalogueView));
    } catch {
      setTriggerGroups(null);
      setCatalogueError("The trigger catalogue did not load.");
    }
  }, []);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void loadTriggerCatalogue();
  }, [auth.status, loadTriggerCatalogue]);

  const [rules, setRules] = useState<AutomationRuleView[] | null>(null);
  const [view, setView] = useState("all");
  // The trigger picker's options come from the node registry, not a constant
  // (Wave 43 lane 18). Read once; the catalogue only moves with a release.
  const [triggerGroups, setTriggerGroups] = useState<TriggerGroup[] | null>(null);
  const [catalogueError, setCatalogueError] = useState<string | null>(null);
  const [clauses, setClauses] = useState<{ id: string; field: string; op: never; value?: string }[]>([]);
  const [columns, setColumns] = useState<RuleColumn[]>(RULE_COLUMNS);
  const [suggestedNudge, setSuggestedNudge] = useState(false);
  const density = useDensity();
  const [boards, setBoards] = useState<BoardNavEntry[]>([]);
  const [users, setUsers] = useState<OrgUser[]>([]);
  const [loading, setLoading] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [rowError, setRowError] = useState<string | null>(null);
  const [aiAvailable, setAiAvailable] = useState(false);

  // Modal create/edit rule state
  const [modalOpen, setModalOpen] = useState(false);
  const [editingRule, setEditingRule] = useState<AutomationRuleView | null>(null);
  const [name, setName] = useState("");
  const [boardId, setBoardId] = useState<string>("");
  const [triggerEvent, setTriggerEvent] = useState<string>("ticket.created");
  // Rule subject (Wave 43 lane 3): a `board` rule fires on envelopes that
  // carry no ticket — the inbound webhook events above.
  const [subjectKind, setSubjectKind] = useState<"ticket" | "board">("ticket");

  // Condition rows: the grammar's rows plus their own identity.
  const [conditionRows, setConditionRows] = useState<DraftCondition[]>([]);
  const uidRef = useRef(1);
  const nextUid = () => uidRef.current++;

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

  // C1 — the live streamed draft summary. Keyed off the WHOLE draft, so a
  // name-only edit re-summarises too; the last-summarised key is a ref, not
  // state, so a delta landing mid-stream cannot re-schedule a second stream.
  const [summary, setSummary] = useState("");
  const [summaryBusy, setSummaryBusy] = useState(false);
  const [summaryMeta, setSummaryMeta] = useState<{ model: string; ms: number; tokens: number } | null>(null);
  const [summaryUnavailable, setSummaryUnavailable] = useState<string | null>(null);
  const [summaryNonce, setSummaryNonce] = useState(0);
  const summarySeqRef = useRef(0);
  const lastSummaryKeyRef = useRef("");

  // C2 — the prose leg.
  const [prose, setProse] = useState("");
  const [proseBusy, setProseBusy] = useState(false);
  const [proseError, setProseError] = useState<string | null>(null);
  const [proseDraft, setProseDraft] = useState<(AuthorDraft & { latencyMs: number; unmappedCount: number }) | null>(
    null,
  );

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

  const boardName = (id: string | null): string | null =>
    id === null ? null : (boards.find((b) => b.id === id)?.name ?? null);

  const openNewRule = () => {
    setEditingRule(null);
    setName("");
    setBoardId("");
    setTriggerEvent("ticket.created");
    setSubjectKind("ticket");
    setConditionRows([]);
    setActions([]);
    setFormError(null);
    setProse("");
    setProseDraft(null);
    setProseError(null);
    setSummary("");
    setSummaryMeta(null);
    setSummaryUnavailable(null);
    lastSummaryKeyRef.current = "";
    setSuggestedNudge(false);
    setModalOpen(true);
  };

  const openEditRule = (r: AutomationRuleView) => {    setEditingRule(r);
    setName(r.name);
    setBoardId(r.board_id ?? "");
    setTriggerEvent(r.trigger_event);
    setSubjectKind(r.subject_kind === "board" ? "board" : "ticket");
    setConditionRows((r.conditions ?? []).map(rowFromCondition).map((row) => ({ uid: nextUid(), row })));
    setActions(r.actions ?? []);
    setFormError(null);
    setProse("");
    setProseDraft(null);
    setProseError(null);
    setSummary("");
    setSummaryMeta(null);
    setSummaryUnavailable(null);
    lastSummaryKeyRef.current = "";
    setSuggestedNudge(false);
    setModalOpen(true);
  };

  // --- condition rows -----------------------------------------------------

  const addCondition = () => {
    setConditionRows((rows) => [...rows, { uid: nextUid(), row: blankRow("priority") }]);
    setFormError(null);
  };

  const updateClause = (uid: number, next: ClauseRow) => {
    setConditionRows((rows) => rows.map((d) => (d.uid === uid ? { ...d, row: next } : d)));
  };

  const setClauseField = (uid: number, field: string) => {
    setConditionRows((rows) => rows.map((d) => (d.uid === uid ? { ...d, row: blankRow(field) } : d)));
  };

  const setClauseOperator = (uid: number, op: GrammarOperator) => {
    setConditionRows((rows) =>
      rows.map((d) => (d.uid === uid ? { ...d, row: d.row.kind === "clause" ? withOperator(d.row, op) : d.row } : d)),
    );
  };

  const removeCondition = (uid: number) => {
    setConditionRows((rows) => rows.filter((d) => d.uid !== uid));
  };

  const applyStarter = (starter: Starter) => {
    setName((cur) => (cur.trim() === "" ? starter.label : cur));
    setTriggerEvent(starter.trigger);
    setSubjectKind(subjectOf(starter.trigger));
    setConditionRows(starter.rows.map((row) => ({ uid: nextUid(), row })));
    setActions(starter.actions);
    setFormError(null);
  };

  // --- actions ------------------------------------------------------------

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

  // --- the draft, validated live -----------------------------------------

  const clauseProblems = useMemo(
    () =>
      conditionRows.map((d) => ({
        uid: d.uid,
        problem: d.row.kind === "clause" ? rowProblems(d.row) : null,
      })),
    [conditionRows],
  );

  const readBack = useMemo(() => {
    const rows = conditionRows.map((d) => d.row);
    const board = boardName(boardId === "" || boardId === "all" ? null : boardId);
    return describeRule({
      trigger: triggerPhrase(triggerEvent),
      rows,
      actions: actions.map((a) => actionPhrase(a, userName)),
      boardName: subjectKind === "board" ? null : board,
    });
    // `boards` is read through boardName; keep it honest in the deps.
  }, [conditionRows, actions, triggerEvent, boardId, boards, userName, subjectKind]);

  const draftValid =
    name.trim() !== "" &&
    clauseProblems.every((d) => d.problem === null) &&
    actions.every((a) => emptyAction(a, users) === null);

  const handleSaveRule = async () => {
    if (!draftValid || actions.length === 0) return;
    const conditions: AutomationCondition[] = [];
    for (const d of conditionRows) {
      if (d.row.kind === "opaque") {
        conditions.push(d.row.condition);
        continue;
      }
      const built = conditionFromRow(d.row);
      if (!built.ok) {
        setFormError(built.problem.message);
        return;
      }
      conditions.push(built.condition);
    }
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
    setConditionRows([]);
    setActions([]);
    void refreshRules();
  };

  const handleToggleRule = async (rule: AutomationRuleView) => {
    setRowError(null);
    setRules((cur) => (cur ?? []).map((r) => (r.id === rule.id ? { ...r, enabled: !r.enabled } : r)));
    const { error } = await api.api.PATCH("/api/v1/automation-rules/{id}", {
      params: { path: { id: rule.id } },
      body: { enabled: !rule.enabled, idempotency_key: idempotencyKey() } as never,
    });
    if (error !== undefined) {
      setRules((cur) => (cur ?? []).map((r) => (r.id === rule.id ? { ...r, enabled: rule.enabled } : r)));
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

  // --- C1: the summary follows the draft, debounced -----------------------

  // Every fact the summary is written from, folded into one key. `JSON.
  // stringify` of a small stable-shaped object is cheap and makes the
  // dependency list honest: the effect re-runs exactly when the draft did.
  const draftKey = JSON.stringify({
    name: name.trim(),
    trigger: triggerEvent,
    subject: subjectKind,
    board: boardId,
    conditions: conditionRows.map((d) => d.row),
    actions: actions.map((a) => actionText(a, userName)),
    problems: clauseProblems.filter((p) => p.problem !== null).length,
  });

  useEffect(() => {
    if (!modalOpen || !aiAvailable) return;
    // Nothing to summarise before the draft says what it does.
    if (actions.length === 0) {
      setSummary("");
      setSummaryMeta(null);
      return;
    }
    if (draftKey === lastSummaryKeyRef.current) return;
    const timer = setTimeout(() => {
      lastSummaryKeyRef.current = draftKey;
      const seq = ++summarySeqRef.current;
      setSummaryBusy(true);
      setSummaryUnavailable(null);
      const started = performance.now();
      void (async () => {
        const result = await streamDraftSummary(
          {
            surface_key: "general_chat",
            name: name.trim() === "" ? null : name.trim(),
            trigger_event: triggerEvent,
            subject_kind: subjectKind,
            scope: subjectKind === "board" ? null : boardName(boardId === "" || boardId === "all" ? null : boardId),
            conditions_text: describeConditions(conditionRows.map((d) => d.row)),
            actions_text: actions.map((a) => actionPhrase(a, userName)),
            incomplete_conditions: clauseProblems.filter((d) => d.problem !== null).length,
          },
          (fragment) => {
            if (seq === summarySeqRef.current) setSummary((cur) => cur + fragment);
          },
        );
        const ms = Math.round(performance.now() - started);
        if (seq !== summarySeqRef.current) return;
        setSummaryBusy(false);
        if (result.refusal !== undefined) {
          setSummary("");
          setSummaryMeta(null);
          setSummaryUnavailable(result.refusalMessage ?? "AI is not available for this org.");
          return;
        }
        setSummary(result.text);
        setSummaryMeta({
          model: result.modelInfo?.model ?? "",
          ms,
          tokens: (result.usage?.prompt_tokens ?? 0) + (result.usage?.completion_tokens ?? 0),
        });
      })();
    }, 900);
    // An edit inside the debounce window replaces the request rather than
    // queueing a second one — a summary of a draft that no longer exists is
    // worse than a short wait.
    return () => clearTimeout(timer);
  }, [modalOpen, aiAvailable, draftKey, summaryNonce]);

  // --- C2: one sentence becomes a whole-workflow draft --------------------

  const handleProseDraft = async () => {
    if (prose.trim() === "" || proseBusy) return;
    setProseBusy(true);
    setProseError(null);
    setProseDraft(null);
    const outcome = await authorWorkflowDraft(prose.trim());
    setProseBusy(false);
    if (!outcome.ok) {
      setProseError(outcome.message);
      return;
    }
    if (outcome.result.error !== undefined) {
      setProseError(outcome.result.error);
      return;
    }
    // Count what this editor would drop BEFORE the person decides, so the
    // apply button can say it rather than discovering it afterwards.
    const projected = projectDraft(outcome.result.name, outcome.result.trigger_event, outcome.result.draft);
    setProseDraft({ ...outcome.result, latencyMs: outcome.latencyMs, unmappedCount: projected.unmapped.length });
  };

  const applyProseDraft = () => {
    if (proseDraft?.draft === undefined || proseDraft.draft === null) return;
    const projected = projectDraft(proseDraft.name, proseDraft.trigger_event, proseDraft.draft);
    if (projected.triggerEvent !== null) {
      setTriggerEvent(projected.triggerEvent);
      setSubjectKind(subjectOf(projected.triggerEvent));
    }
    if (projected.name !== null && name.trim() === "") setName(projected.name);
    setConditionRows(projected.conditions.map(rowFromCondition).map((row) => ({ uid: nextUid(), row })));
    setActions(projected.actions);
    setProseDraft(null);
    setFormError(null);
  };

  // The picker's groups: the registry's own cut, narrowed to the rule's
  // subject. A group left with no matching event is dropped rather than shown
  // empty. When the catalogue has not answered (or a saved rule names an event
  // it no longer lists) the rule's own trigger stays selectable, so an edit can
  // never silently change or lose it.
  const registryGroups = useMemo(() => triggerGroups ?? [], [triggerGroups]);
  const subjectGroups = useMemo(
    () => groupsForSubject(registryGroups, subjectKind),
    [registryGroups, subjectKind],
  );
  const selectedGroup = groupOf(subjectGroups, triggerEvent);
  const triggerInList = hasTriggerValue(subjectGroups, triggerEvent);

  const rulesSorted = useMemo(
    () => (rules ?? []).slice().sort((a, b) => ((a.updated_at ?? "") < (b.updated_at ?? "") ? -1 : 1)),
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
            {/* The rule builder is the default view; the canvas is the same
                workflows, authored as a graph. Two routes, one toggle. */}
            <ViewSwitch
              label="Authoring view"
              views={[
                { id: "rules", label: "Rules" },
                { id: "canvas", label: "Canvas" },
              ]}
              activeId="rules"
              onChange={(id) => {
                if (id === "canvas") router.push("/automations/canvas");
              }}
            />
            <Button variant="secondary" asChild>
              <Link href="/automations/runs">Runs</Link>
            </Button>
            <Button variant="secondary" asChild>
              <Link href="/settings/integrations">Webhook activity</Link>
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
                        return r.subject_kind === "board" ? `${r.trigger_event} (no ticket)` : r.trigger_event;
                      if (c.key === "conditions")
                        return (r.conditions ?? []).map(describeStoredCondition).join(" and ");
                      if (c.key === "actions") return (r.actions ?? []).map((x) => x.type).join(" ");
                      if (c.key === "board")
                        return r.board_id == null ? "All boards" : (boards.find((b) => b.id === r.board_id)?.name ?? "—");
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
                action={<Button onClick={openNewRule}>New rule</Button>}
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
                            {r.subject_kind === "board" ? <Badge tone="neutral">no ticket</Badge> : null}
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
                              {r.subject_kind === "board" ? "matches every occurrence" : "matches every ticket"}
                            </InlineVerdict>
                          ) : (
                            <span
                              className="text-[12px]"
                              style={{ color: "var(--m-text-secondary)" }}
                              title={(r.conditions ?? []).map(describeStoredCondition).join(" and ")}
                            >
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
                            {r.board_id == null ? "All boards" : (boards.find((b) => b.id === r.board_id)?.name ?? "—")}
                          </span>
                        ) : null}
                        {c.key === "updated" ? (
                          <span
                            className="tabular-nums text-[12px]"
                            style={{ color: "var(--m-text-secondary)" }}
                          >
                            {r.updated_at == null ? "—" : <LocalTime value={r.updated_at} kind="date" />}
                          </span>
                        ) : null}
                      </Td>
                    ))}
                    <Td className="text-right">
                      <div className="flex justify-end gap-1">
                        {/* This rule's own history: the ledger, narrowed to it. */}
                        <Button variant="ghost" size="sm" asChild>
                          <Link
                            href={`/automations/runs?rule=${r.id}`}
                            aria-label={`See the runs of ${r.name}`}
                            data-testid={`automation-runs-${r.id}`}
                          >
                            Runs
                          </Link>
                        </Button>
                        <Button variant="ghost" size="sm" asChild>
                          <Link href={`/automations/canvas?rule=${r.id}`} aria-label={`Open ${r.name} in the canvas`}>
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
                            <Button
                              variant="destructive-solid"
                              size="sm"
                              onClick={() => void handleDeleteRule(r.id)}
                            >
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
          {/* The dry run lives inside the card that scrolls, not after it:
              a long proposal in a fixed-height frame would overflow the
              footer and the toolbar instead of scrolling with the ledger. */}
          {testError !== null ? (
            <div className="px-4 pb-3" role="alert">
              <BlushNote title="The test run did not come back.">{testError}</BlushNote>
            </div>
          ) : null}
          {testRun !== null ? (
            <div className="px-4 pb-4">
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

            {editingRule === null ? <StarterPicker onPick={applyStarter} /> : null}

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
                      // impossible pair selected. The subject is derived from
                      // the event itself, so the two cannot disagree.
                      if (subjectOf(triggerEvent) !== next) {
                        setTriggerEvent(next === "board" ? "webhook.ingested" : "ticket.created");
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
                    <Select value={boardId} onValueChange={(v) => setBoardId(v ?? "")}>
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
                        {subjectGroups.map((group) => (
                          <SelectGroup key={group.id}>
                            <SelectLabel>{group.label}</SelectLabel>
                            {group.options.map((option) => (
                              <SelectItem key={option.value} value={option.value}>
                                {option.label}
                              </SelectItem>
                            ))}
                          </SelectGroup>
                        ))}
                        {triggerInList ? null : (
                          <SelectGroup>
                            <SelectLabel>This rule&rsquo;s trigger</SelectLabel>
                            <SelectItem value={triggerEvent}>{eventLabel(triggerEvent)}</SelectItem>
                          </SelectGroup>
                        )}
                      </SelectContent>
                    </Select>
                    <FieldHint>
                      {selectedGroup !== null ? (
                        selectedGroup.description
                      ) : subjectKind === "board" ? (
                        "Ticket-less events — an inbound webhook, an alert, or another platform envelope. Plugin-emitted types need an org grant and are not listable until the registry gains a read endpoint."
                      ) : (
                        "Events about a ticket, from the node registry."
                      )}{" "}
                      {subjectKind === "board" ? (
                        <Link href="/settings/integrations">See recent webhook deliveries</Link>
                      ) : null}
                    </FieldHint>
                    {catalogueError !== null ? (
                      <FieldHint>
                        {catalogueError}{" "}
                        <button
                          type="button"
                          className="underline"
                          onClick={() => void loadTriggerCatalogue()}
                        >
                          Try again
                        </button>
                      </FieldHint>
                    ) : null}
                  </div>
                </div>
              </div>
            </RuleStep>

            {/* IF — the clause rows, validated live and read back in one
                sentence. An incomplete row is never a silent half of a rule:
                the offending control is marked and the sentence carries the
                incomplete clause verbatim until it is fixed. */}
            <RuleStep
              word="IF"
              summary={
                conditionRows.length === 0
                  ? suggestedNudge
                    ? "Every occurrence of the trigger matches. That is usually not what you meant."
                    : "Every occurrence of the trigger matches."
                  : describeConditions(conditionRows.map((d) => d.row))
              }
            >
              {conditionRows.length === 0 ? (
                <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                  No conditions — the rule matches every occurrence of the trigger.
                </p>
              ) : (
                <div className="mt-2 grid gap-2">
                  {conditionRows.map((d) => (
                    <ConditionRowEditor
                      key={d.uid}
                      uid={d.uid}
                      row={d.row}
                      problem={clauseProblems.find((p) => p.uid === d.uid)?.problem ?? null}
                      onFieldChange={setClauseField}
                      onOperatorChange={setClauseOperator}
                      onChange={updateClause}
                      onRemove={removeCondition}
                    />
                  ))}
                </div>
              )}
              <div className="mt-3 border-t pt-3" style={{ borderColor: "var(--m-border)" }}>
                <Button size="sm" variant="secondary" onClick={addCondition} data-testid="automation-add-condition">
                  <PlusIcon size={14} /> Add condition
                </Button>
              </div>
              {subjectKind === "board" ? (
                <p className="mt-2 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                  Board rules fire on envelopes with no ticket, so ticket-shaped
                  conditions (priority, status, assignee) never match them — the
                  engine skips them with a warning.
                </p>
              ) : null}
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
                    <ActionChip
                      key={i}
                      index={i}
                      text={actionText(a, userName)}
                      onRemove={() => setActions(actions.filter((_, j) => j !== i))}
                    />
                  ))}
                </div>
              )}
              <div
                className="mt-3 flex flex-wrap items-end gap-2 border-t pt-3"
                style={{ borderColor: "var(--m-border)" }}
              >
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
                      <SelectTrigger id="ar-act-priority" className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {PRIORITIES.map((p) => (
                          <SelectItem key={p} value={p}>
                            {PRIORITY_LABEL[p]}
                          </SelectItem>
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
                        <SelectTrigger id="ar-act-kind" className="w-full">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {NOTIFY_KINDS.map((k) => (
                            <SelectItem key={k.value} value={k.value}>
                              {k.label}
                            </SelectItem>
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

            {/* The read-back, derived from the same rows the controls render —
                it cannot disagree with them because it is not stored beside
                them. This is also the leg that survives when AI does not. */}
            <div
              className="rounded-md border p-3 text-[13px] leading-relaxed"
              style={{ borderColor: "var(--m-border)" }}
              data-testid="automation-read-back"
            >
              <p className="m-0" style={{ color: "var(--m-text)" }}>
                {readBack}
              </p>
            </div>

            <SummaryStrip
              summary={summary}
              busy={summaryBusy}
              meta={summaryMeta}
              unavailable={summaryUnavailable}
              available={aiAvailable}
              onRegenerate={() => {
                lastSummaryKeyRef.current = "";
                setSummary("");
                setSummaryUnavailable(null);
                setSummaryNonce((n) => n + 1);
              }}
            />

            {aiAvailable ? (
              <ProseDraftPanel
                prose={prose}
                busy={proseBusy}
                error={proseError}
                draft={proseDraft}
                onProseChange={setProse}
                onDraft={() => void handleProseDraft()}
                onApply={applyProseDraft}
                onDismiss={() => setProseDraft(null)}
              />
            ) : null}

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
                disabled={saving || name.trim() === "" || actions.length === 0 || !draftValid}
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
  return typeof crypto !== "undefined" && crypto.randomUUID !== undefined ? crypto.randomUUID() : `auto-${Date.now()}`;
}

/**
 * The honest dry-run rendering, as a diff.
 *
 * A test run is a PROPOSAL, so the useful thing to show is not "matched" but
 * what WOULD change on each matched ticket: `~ priority  P2 — high → P1 —
 * critical`, the same shape a plan reads. `+` is an effect that adds
 * something (a note, a notification) and carries no before; `=` is an effect
 * whose target already holds the value — the line that stops a rule looking
 * busy while doing nothing. When nothing changes the summary says so rather
 * than leaving three `=` lines to be scrolled.
 */
interface DiffLine {
  mark: "~" | "+" | "=";
  label: string;
  detail: string;
}

interface MatchDiff {
  ticketId: string;
  number: string;
  title: string;
  rows: DiffLine[];
  changed: number;
}

function matchDiffs(result: AutomationTestRunResponse, userName: (id: string) => string): MatchDiff[] {
  const previewActions = Array.isArray(result.actions_preview) ? (result.actions_preview as AutomationAction[]) : [];
  return result.matches.map((m) => {
    const rows: DiffLine[] = [];
    for (const a of previewActions) {
      if (a.type === "set_priority") {
        const before = PRIORITY_LABEL[m.priority];
        const after = PRIORITY_LABEL[a.priority];
        rows.push(
          before === after
            ? { mark: "=", label: "priority", detail: `already ${after}` }
            : { mark: "~", label: "priority", detail: `${before} → ${after}` },
        );
        continue;
      }
      if (a.type === "assign_user") {
        const before = m.assignee_id === null || m.assignee_id === undefined ? "unassigned" : userName(m.assignee_id);
        const after = userName(a.user_id);
        rows.push(
          before === after
            ? { mark: "=", label: "assignee", detail: `already ${after}` }
            : { mark: "~", label: "assignee", detail: `${before} → ${after}` },
        );
        continue;
      }
      if (a.type === "add_internal_comment") {
        rows.push({ mark: "+", label: "note", detail: `“${a.body}”` });
        continue;
      }
      if (a.type === "notify_users") {
        rows.push({
          mark: "+",
          label: "notify",
          detail: `${a.kind.replace(/_/g, " ")} — “${a.title}” → ${
            a.user_ids.length === 1 ? userName(a.user_ids[0]!) : `${a.user_ids.length} people`
          }`,
        });
        continue;
      }
      if (a.type === "plugin_action") {
        const body = (a.params as { body?: unknown }).body;
        rows.push({
          mark: "+",
          label: a.entry_key,
          detail: typeof body === "string" && body.trim() !== "" ? `“${body.trim()}”` : "",
        });
      }
    }
    return {
      ticketId: m.ticket_id,
      number: m.number,
      title: m.title,
      rows,
      changed: rows.filter((r) => r.mark !== "=").length,
    };
  });
}

function TestRunResult({
  result,
  userName,
  onClose,
}: {
  result: AutomationTestRunResponse;
  userName(id: string): string;
  onClose(): void;
}) {
  const diffs = matchDiffs(result, userName);
  const changes = diffs.reduce((sum, d) => sum + d.changed, 0);
  return (
    <div
      className="mt-3 grid gap-2 rounded-md border p-3"
      style={{ borderColor: "var(--m-border)" }}
      data-testid="automation-test-run-result"
    >
      <div className="flex items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2 text-[12px]">
          <StepMark tone="plain">DRY RUN</StepMark>
          <span className="tabular-nums">
            sampled {result.sampled_count} · matched {result.matched_count} ·{" "}
            {result.matched_count === 0
              ? "no tickets would fire this rule"
              : changes === 0
                ? "every match already looks like this — nothing would change"
                : `${changes} change${changes === 1 ? "" : "s"} across ${result.matched_count} ticket${
                    result.matched_count === 1 ? "" : "s"
                  }`}
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
      {diffs.map((d) => (
        <div key={d.ticketId} className="grid gap-1">
          <div className="flex items-center gap-2 text-[12px]">
            <Link
              href={`/tickets/${d.ticketId}`}
              className="font-medium underline decoration-[var(--m-border)] underline-offset-2 hover:text-[var(--m-accent)]"
            >
              #{d.number}
            </Link>
            <span className="min-w-0 truncate">{d.title}</span>
          </div>
          <ul className="m-0 grid gap-0.5 pl-0 font-mono text-[11px]" style={{ listStyle: "none" }}>
            {d.rows.map((row, i) => (
              <li key={i} className="flex items-baseline gap-2">
                <span
                  className="w-3 shrink-0 text-right"
                  style={{
                    color:
                      row.mark === "+"
                        ? "var(--m-status-success-fg)"
                        : row.mark === "~"
                          ? "var(--m-text)"
                          : "var(--m-text-muted)",
                  }}
                  aria-label={
                    row.mark === "+" ? "adds" : row.mark === "~" ? "changes" : "no change"
                  }
                >
                  {row.mark}
                </span>
                <span style={{ color: "var(--m-text-secondary)" }}>{row.label}</span>
                <span
                  className="min-w-0 truncate"
                  style={{ color: row.mark === "=" ? "var(--m-text-muted)" : "var(--m-text)" }}
                >
                  {row.detail}
                </span>
              </li>
            ))}
          </ul>
        </div>
      ))}
      <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
        Proposal only — the values are what the tickets look like now, sampled,
        and nothing was executed by a test run.
      </p>
    </div>
  );
}

/**
 * C1 — the summary, rendered as its frames arrive.
 *
 * Its absence is a state too: a person who never granted AI must see the
 * rule's own read-back and nothing else, not a locked control. When the
 * leg is refused mid-session the strip says why and gets out of the way.
 */
function SummaryStrip({
  summary,
  busy,
  meta,
  unavailable,
  available,
  onRegenerate,
}: {
  summary: string;
  busy: boolean;
  meta: { model: string; ms: number; tokens: number } | null;
  unavailable: string | null;
  available: boolean;
  onRegenerate(): void;
}) {
  if (!available) return null;
  return (
    <div
      className="grid gap-1 rounded-md border p-3"
      style={{ borderColor: "var(--m-border)" }}
      data-testid="automation-summary-strip"
    >
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <StepMark tone="plain">AI SUMMARY</StepMark>
          {busy ? <Spinner /> : null}
        </div>
        <button
          className="text-[11px] underline-offset-2 hover:underline"
          style={{ color: "var(--m-text-muted)" }}
          onClick={onRegenerate}
        >
          Regenerate
        </button>
      </div>
      {unavailable !== null ? (
        <p className="m-0 text-[12px]" style={{ color: "var(--m-text-muted)" }} role="status">
          {unavailable}
        </p>
      ) : summary !== "" ? (
        <p className="m-0 text-[13px] leading-relaxed" style={{ color: "var(--m-text)" }} role="status">
          {summary}
          {busy ? <span style={{ color: "var(--m-text-muted)" }}> …</span> : null}
        </p>
      ) : (
        <p className="m-0 text-[12px]" style={{ color: "var(--m-text-muted)" }} role="status">
          {busy ? "Reading the draft…" : "Writing a summary of the draft…"}
        </p>
      )}
      {meta !== null && !busy ? (
        <p className="m-0 text-[11px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
          {meta.model} · {meta.ms} ms · {meta.tokens} tokens — counts, not cost; this seam has no price table.
        </p>
      ) : null}
    </div>
  );
}

/**
 * C2 — the prose panel, and the ONE inverted surface this screen carries.
 * The card holds the assistant's proposal, the issues it already knows
 * about, and the action a person approves; `saved` is false on the endpoint
 * by contract, so "Apply" fills this editor and the save button stays the
 * only thing that writes.
 */
function ProseDraftPanel({
  prose,
  busy,
  error,
  draft,
  onProseChange,
  onDraft,
  onApply,
  onDismiss,
}: {
  prose: string;
  busy: boolean;
  error: string | null;
  draft: (AuthorDraft & { latencyMs: number; unmappedCount: number }) | null;
  onProseChange(next: string): void;
  onDraft(): void;
  onApply(): void;
  onDismiss(): void;
}) {
  return (
    <div className="grid gap-2">
      <div>
        <Label htmlFor="ar-prose">Describe the rule in a sentence</Label>
        <textarea
          id="ar-prose"
          className="m-input"
          rows={2}
          placeholder="e.g. When an RMM alert arrives on the Support board and nobody has picked it up within five minutes, set it to P2 and note it."
          value={prose}
          onChange={(e) => onProseChange(e.target.value)}
        />
        <div className="mt-2 flex items-center gap-2">
          <Button size="sm" variant="secondary" disabled={busy || prose.trim() === ""} onClick={onDraft}>
            {busy ? <Spinner /> : null} Draft it for me
          </Button>
          <FieldHint>
            Produces a whole-workflow draft. Anything the simple editor cannot
            show is named before you apply it — nothing is saved here.
          </FieldHint>
        </div>
      </div>
      {error !== null ? (
        <p className="m-0 text-[12px]" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
          {error}
        </p>
      ) : null}
      {draft !== null ? (
        <AssistantCard
          eyebrow="Workflow draft"
          heading={draft.name ?? "Untitled draft"}
          action={
            draft.valid ? (
              <Button variant="assistant" size="sm" onClick={onApply}>
                {draft.unmappedCount === 0
                  ? "Use this draft"
                  : `Use this draft (drops ${draft.unmappedCount} step${draft.unmappedCount === 1 ? "" : "s"})`}
              </Button>
            ) : null
          }
          secondaryActions={
            <Button variant="ghost" size="sm" onClick={onDismiss}>
              Not this
            </Button>
          }
          footer={
            <span className="tabular-nums">
              {draft.model_info.provider}/{draft.model_info.model} · {draft.latencyMs} ms ·{" "}
              {draft.usage.prompt_tokens + draft.usage.completion_tokens} tokens · {draft.usage.steps} step
              {draft.usage.steps === 1 ? "" : "s"} · catalogue {draft.catalogue.version}
            </span>
          }
          data-testid="automation-prose-draft"
        >
          {draft.issues.length > 0 ? (
            <ul className="m-0 grid gap-0.5 pl-4">
              {draft.issues.map((issue, i) => (
                <li key={i} className="text-[12px]">
                  {issue.message}
                </li>
              ))}
            </ul>
          ) : (
            <p className="m-0 text-[12px]">
              {draft.trigger_event} · {draft.diff.length} change
              {draft.diff.length === 1 ? "" : "s"} against an empty document.
            </p>
          )}
        </AssistantCard>
      ) : null}
    </div>
  );
}

/**
 * The four rules an MSP writes on its first afternoon. Offered only to a NEW
 * draft: offering them over an existing rule would be the editor proposing
 * to throw work away.
 */
function StarterPicker({ onPick }: { onPick(starter: Starter): void }) {
  return (
    <div className="grid gap-1" data-testid="automation-starters">
      <Label>Start from a common case</Label>
      <div className="grid gap-2 sm:grid-cols-2">
        {STARTERS.map((starter) => (
          <button
            key={starter.id}
            type="button"
            className="rounded-md border p-2 text-left"
            style={{ borderColor: "var(--m-border)" }}
            onClick={() => onPick(starter)}
          >
            <span className="block text-[12px] font-semibold" style={{ color: "var(--m-text)" }}>
              {starter.label}
            </span>
            <span className="block text-[11px]" style={{ color: "var(--m-text-muted)" }}>
              {starter.note}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

/**
 * One condition row: field, operator, value, and the problem naming whichever
 * of the three is wrong. The offending control carries `aria-invalid`, so the
 * message and the mark point at the same place rather than at the row.
 */
function ConditionRowEditor({
  uid,
  row,
  problem,
  onFieldChange,
  onOperatorChange,
  onChange,
  onRemove,
}: {
  uid: number;
  row: ConditionRow;
  problem: ReturnType<typeof rowProblems>;
  onFieldChange(uid: number, field: string): void;
  onOperatorChange(uid: number, op: GrammarOperator): void;
  onChange(uid: number, row: ClauseRow): void;
  onRemove(uid: number): void;
}) {
  if (row.kind === "opaque") {
    return (
      <div
        className="flex items-center justify-between gap-2 rounded-md px-2 py-1.5 text-[12px]"
        style={{ background: "color-mix(in srgb, var(--m-accent) 8%, transparent)" }}
      >
        <span className="min-w-0 truncate" title={row.label}>
          {row.label}
        </span>
        <div className="flex shrink-0 items-center gap-2">
          <Badge tone="neutral">from the canvas</Badge>
          <button
            className="text-[11px] underline-offset-2 hover:underline"
            style={{ color: "var(--m-status-danger-fg)" }}
            aria-label="Remove condition"
            onClick={() => onRemove(uid)}
          >
            Remove
          </button>
        </div>
      </div>
    );
  }

  const spec = fieldByKey(row.field);
  const fieldInvalid = problem?.token === "field";
  const operatorInvalid = problem?.token === "operator";
  const valueInvalid = problem?.token === "value";
  const takesMany = operatorTakesMany(row.op);
  const needsValue = operatorNeedsValue(row.op);

  return (
    <div className="grid gap-1">
      <div className="flex flex-wrap items-end gap-2">
        <div className="w-36">
          <Label htmlFor={`${uid}-field`} className="sr-only">
            Condition field
          </Label>
          <Select value={row.field} onValueChange={(v) => onFieldChange(uid, v)}>
            <SelectTrigger id={`${uid}-field`} className="w-full" aria-invalid={fieldInvalid === true}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {RULE_FIELDS.map((f) => (
                <SelectItem key={f.key} value={f.key}>
                  {f.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <div className="w-40">
          <Label htmlFor={`${uid}-op`} className="sr-only">
            Condition operator
          </Label>
          <Select value={row.op} onValueChange={(v) => onOperatorChange(uid, v as GrammarOperator)}>
            <SelectTrigger id={`${uid}-op`} className="w-full" aria-invalid={operatorInvalid === true}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {(spec?.operators ?? []).map((op) => (
                <SelectItem key={op} value={op}>
                  {OPERATOR_VERB[op]}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        {needsValue && takesMany && spec !== undefined ? (
          <div className="flex flex-1 flex-wrap items-center gap-2 pt-1">
            {(spec.options ?? []).map((option) => {
              const picked = (row.values ?? []).includes(option.value);
              return (
                <span key={option.value} className="flex items-center gap-1.5">
                  <Checkbox
                    id={`${uid}-value-${option.value}`}
                    checked={picked}
                    onCheckedChange={(checked) => {
                      const next = checked === true
                        ? [...(row.values ?? []), option.value]
                        : (row.values ?? []).filter((v) => v !== option.value);
                      onChange(uid, { ...row, values: next });
                    }}
                    aria-invalid={valueInvalid === true}
                  />
                  <Label htmlFor={`${uid}-value-${option.value}`} className="text-[12px]">
                    {option.label}
                  </Label>
                </span>
              );
            })}
          </div>
        ) : null}

        {needsValue && !takesMany && spec?.valueKind === "enum" ? (
          <div className="flex-1">
            <Label htmlFor={`${uid}-value`} className="sr-only">
              Condition value
            </Label>
            <Select value={row.value ?? ""} onValueChange={(v) => onChange(uid, { ...row, value: v ?? "" })}>
              <SelectTrigger id={`${uid}-value`} className="w-full" aria-invalid={valueInvalid === true}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {(spec.options ?? []).map((option) => (
                  <SelectItem key={option.value} value={option.value}>
                    {option.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        ) : null}

        {needsValue && !takesMany && spec?.valueKind !== undefined && spec.valueKind !== "enum" ? (
          <div className="flex-1">
            <Label htmlFor={`${uid}-value`} className="sr-only">
              Condition value
            </Label>
            <Input
              id={`${uid}-value`}
              value={row.value ?? ""}
              aria-invalid={valueInvalid === true}
              placeholder={spec.valueKind === "number" ? "3" : "Outage / urgent"}
              onChange={(e) => onChange(uid, { ...row, value: e.target.value })}
            />
          </div>
        ) : null}

        <Button
          variant="ghost"
          size="sm"
          aria-label="Remove condition"
          onClick={() => onRemove(uid)}
        >
          Remove
        </Button>
      </div>

      {/* The sentence fragment the row carries, under the controls it came
          from. A person should be able to read the rule out of the rows. */}
      <p className="m-0 pl-1 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
        {describeClause(row)}
      </p>

      {problem !== null ? (
        <p className="m-0 pl-1 text-[11px]" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
          {problem.message}
        </p>
      ) : null}
    </div>
  );
}

function StepMark({
  tone,
  children,
}: {
  tone: "when" | "if" | "then" | "plain";
  children: React.ReactNode;
}) {
  return (
    <span
      className="rounded px-1.5 py-0.5 text-[10px] font-semibold tracking-wide"
      style={{
        background:
          tone === "when"
            ? "var(--m-accent)"
            : tone === "plain"
              ? "transparent"
              : "color-mix(in srgb, var(--m-accent) 12%, transparent)",
        color:
          tone === "when"
            ? "var(--m-background)"
            : tone === "plain"
              ? "var(--m-text-muted)"
              : "var(--m-text)",
        border: tone === "plain" ? "1px solid var(--m-border)" : undefined,
      }}
    >
      {children}
    </span>
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
              onClick={() => onChange(picked ? selected.filter((x) => x !== u.id) : [...selected, u.id])}
            >
              {u.display_name || u.email}
            </button>
          );
        })}
        {selected.some((id) => users.every((u) => u.id !== id))
          ? selected
              .filter((id) => users.every((u) => u.id !== id))
              .map((id) => (
                <span
                  key={id}
                  className="rounded-full border px-2 py-0.5 text-[11px]"
                  style={{ borderColor: "var(--m-border)" }}
                >
                  {id.slice(0, 8)}
                  <button
                    className="ml-1 underline-offset-2 hover:underline"
                    onClick={() => onChange(selected.filter((x) => x !== id))}
                  >
                    ×
                  </button>
                </span>
              ))
          : null}
      </div>
      <FieldHint>Users beyond the first eight render by id chip.</FieldHint>
    </div>
  );
}

const PRIORITY_LABEL: Record<Priority, string> = {
  p1: "P1 — critical",
  p2: "P2 — high",
  p3: "P3 — normal",
  p4: "P4 — low",
};

/** A stored condition, as the ledger's CSV and title attributes read it. */
function describeStoredCondition(c: AutomationCondition): string {
  const row = rowFromCondition(c);
  return row.kind === "opaque" ? row.label : describeClause(row);
}
