/**
 * rule-grammar — the condition grammar the automation rule editor builds on.
 *
 * Why this is a module and not inline JSX: the rule editor asks the same
 * question four times, and all four answers have to agree.
 *
 *   1. Which field can this operator apply to, and which operators does this
 *      field accept? (Cloudflare's Expression Builder; GitHub's operator list.)
 *   2. Is the row currently valid, and if not, WHICH token is wrong — the
 *      field, the operator, or the value?
 *   3. What does this row mean in English a manager can read?
 *   4. What does the whole rule mean, in one sentence, before saving?
 *
 * Deriving (3) and (4) from the same clauses (2) validated is what stops the
 * chips and the sentence from disagreeing. Mirroring `FilterClause`'s shape and
 * `OPERATOR_LABEL`'s vocabulary — both already in `@marula-core/ui`, both kept in
 * step with the Rust `FilterOp` — is what stops this surface inventing a second
 * grammar next to the one the ledgers already use. It mirrors rather than
 * imports because the rule grammar is a strict SUBSET: the ledger's operator
 * list is longer, and offering an operator with no `AutomationCondition` to
 * store it in is the exact bug this module exists to prevent.
 *
 * The grammar is bounded by the wire enum, on purpose. Every operator declared
 * below has a variant in `crates/core-domain/src/automations.rs` that can carry
 * it, and `conditionFromRow` is the only place a row becomes that variant. When
 * the enum cannot express what the person asked for, `rowProblems` says so and
 * names the token rather than building a row that fails at save.
 */
import type { components } from "@marula-core/sdk";

export type AutomationCondition = components["schemas"]["AutomationCondition"];
export type AutomationAction = components["schemas"]["AutomationAction"];
export type Priority = components["schemas"]["Priority"];
export type TicketStatusCategory = components["schemas"]["TicketStatusCategory"];

/**
 * The operators the grammar offers.
 *
 * Each one is spelled the way an operator would say it, because the same
 * string appears in the operator menu, in the validation message and in the
 * sentence read-back. A person who is told "`contains` has no equivalent for
 * Priority" should be looking at a menu that says `contains`.
 */
export type GrammarOperator =
  | "is"
  | "is_not"
  | "is_any_of"
  | "contains"
  | "does_not_contain"
  | "is_set"
  | "is_not_set"
  | "below";

/** How a chip and a sentence read for each operator. */
export const OPERATOR_VERB: Record<GrammarOperator, string> = {
  is: "is",
  is_not: "is not",
  is_any_of: "is any of",
  contains: "contains",
  does_not_contain: "does not contain",
  is_set: "is set",
  is_not_set: "is not set",
  below: "is below",
};

/** Presence tests take no value; a text box beside one is a bug. */
export function operatorNeedsValue(op: GrammarOperator): boolean {
  return op !== "is_set" && op !== "is_not_set";
}

/** Only `is any of` collects a set; every other valued operator takes one value. */
export function operatorTakesMany(op: GrammarOperator): boolean {
  return op === "is_any_of";
}

export type ValueKind = "enum" | "text" | "number" | "none";

export interface GrammarField {
  key: string;
  label: string;
  /** Only the operators a stored `AutomationCondition` can carry for this field. */
  operators: GrammarOperator[];
  valueKind: ValueKind;
  options?: { value: string; label: string }[];
  /** How the field reads inside the rule sentence: "its priority is P1". */
  subject: string;
}

const PRIORITIES: Priority[] = ["p1", "p2", "p3", "p4"];
const CATEGORIES: TicketStatusCategory[] = ["open", "in_progress", "waiting", "closed"];

export const PRIORITY_LABEL: Record<Priority, string> = {
  p1: "P1 — critical",
  p2: "P2 — high",
  p3: "P3 — normal",
  p4: "P4 — low",
};

export const CATEGORY_LABEL: Record<TicketStatusCategory, string> = {
  open: "open",
  in_progress: "in progress",
  waiting: "waiting",
  closed: "closed",
};

/**
 * The fields a condition can read, and what each one can say.
 *
 * `assignee` is the only field offering a presence test, because it is the
 * only nullable fact the snapshot carries. `Unassigned` and `AssigneeIsSet`
 * are two separate variants rather than one predicate plus a flag, which is
 * why the pair is spelled out here instead of derived.
 */
export const RULE_FIELDS: GrammarField[] = [
  {
    key: "priority",
    label: "Priority",
    operators: ["is", "is_not", "is_any_of"],
    valueKind: "enum",
    options: PRIORITIES.map((p) => ({ value: p, label: PRIORITY_LABEL[p] })),
    subject: "its priority",
  },
  {
    key: "category",
    label: "Status category",
    operators: ["is", "is_not", "is_any_of"],
    valueKind: "enum",
    options: CATEGORIES.map((c) => ({ value: c, label: CATEGORY_LABEL[c] })),
    subject: "its status",
  },
  {
    key: "title",
    label: "Title",
    operators: ["contains", "does_not_contain"],
    valueKind: "text",
    subject: "its title",
  },
  {
    key: "assignee",
    label: "Assignee",
    // Unset first: "picks up work nobody owns" is the rule an MSP writes on
    // day one, and the first entry is the one a new row defaults to.
    operators: ["is_not_set", "is_set"],
    valueKind: "none",
    subject: "it",
  },
  {
    key: "csat_score",
    label: "CSAT score",
    operators: ["below"],
    valueKind: "number",
    subject: "its CSAT score",
  },
];

export function fieldByKey(key: string): GrammarField | undefined {
  return RULE_FIELDS.find((f) => f.key === key);
}

/**
 * Which token is wrong, and what to say about it.
 *
 * Naming the token rather than "invalid condition" is the whole point: a
 * person staring at a three-part row needs to be told which part to change.
 */
export type ProblemToken = "field" | "operator" | "value";

export interface RuleProblem {
  token: ProblemToken;
  message: string;
}

/** A row the grammar can express. */
export interface ClauseRow {
  kind: "clause";
  id: string;
  field: string;
  op: GrammarOperator;
  /** The single value, for every valued operator except `is any of`. */
  value?: string;
  /** The set, for `is any of` only. */
  values?: string[];
}

/**
 * A condition the grammar cannot express — `ai_decision`, whose surface,
 * question, operator and threshold come from a decision surface rather than
 * from a ticket field. It is preserved verbatim rather than dropped, because
 * silently losing a condition on save would change what a live rule does.
 */
export interface OpaqueRow {
  kind: "opaque";
  id: string;
  condition: AutomationCondition;
  label: string;
}

export type ConditionRow = ClauseRow | OpaqueRow;

function rowId(field: string, op: string, key: string): string {
  return `${field}:${op}:${key}`;
}

/** The option label for a stored value, falling back to the raw value. */
function optionLabel(spec: GrammarField, value: string): string {
  return spec.options?.find((o) => o.value === value)?.label ?? value;
}

/** An empty row for the field the person just picked. */
export function blankRow(field: string): ClauseRow {
  const spec = fieldByKey(field);
  const op = spec?.operators[0] ?? "is";
  const first = spec?.options?.[0]?.value;
  if (spec === undefined || !operatorNeedsValue(op)) {
    return { kind: "clause", id: rowId(field, op, ""), field, op };
  }
  if (operatorTakesMany(op)) {
    const values = first === undefined ? [] : [first];
    return { kind: "clause", id: rowId(field, op, values.join(",")), field, op, values };
  }
  const value = first ?? "";
  return { kind: "clause", id: rowId(field, op, value), field, op, value };
}

/**
 * Switch a row's operator, reshaping the value it carries.
 *
 * A row is a field plus an operator plus a value, and the three are dependent:
 * switching `is` to `is any of` has to move a single value into a set, and
 * switching back has to move the first set member out again. Doing that by
 * rebuilding the row would silently empty it, which reads as the editor losing
 * the person's work.
 */
export function withOperator(row: ClauseRow, op: GrammarOperator): ClauseRow {
  const spec = fieldByKey(row.field);
  const first = spec?.options?.[0]?.value;
  if (!operatorNeedsValue(op)) {
    return { kind: "clause", id: rowId(row.field, op, ""), field: row.field, op };
  }
  if (operatorTakesMany(op)) {
    const carried = row.value !== undefined && row.value.trim() !== "" ? [row.value] : [];
    const values = carried.length > 0 ? carried : first === undefined ? [] : [first];
    return { kind: "clause", id: rowId(row.field, op, values.join(",")), field: row.field, op, values };
  }
  const carried = row.values?.[0] ?? row.value;
  const value = carried !== undefined && carried.trim() !== "" ? carried : (first ?? "");
  return { kind: "clause", id: rowId(row.field, op, value), field: row.field, op, value };
}

/**
 * Live validation. Runs on every keystroke in the row editor, so it is pure
 * and allocation-light. Returns the FIRST problem: fixing one thing at a time
 * is how a person actually edits a row.
 */
export function rowProblems(row: ClauseRow): RuleProblem | null {
  const spec = fieldByKey(row.field);
  if (spec === undefined) {
    return { token: "field", message: `“${row.field}” is not a field this rule can read.` };
  }
  if (!spec.operators.includes(row.op)) {
    const allowed = spec.operators.map((o) => OPERATOR_VERB[o]).join(" or ");
    return {
      token: "operator",
      message: `${spec.label} supports ${allowed}. “${OPERATOR_VERB[row.op]}” has no equivalent in a saved rule.`,
    };
  }
  if (!operatorNeedsValue(row.op)) return null;

  if (operatorTakesMany(row.op)) {
    const values = row.values ?? [];
    if (values.length === 0) {
      return { token: "value", message: `${spec.label} needs at least one value to pick from.` };
    }
    const unknown = values.find((v) => !spec.options?.some((o) => o.value === v));
    if (unknown !== undefined) {
      return { token: "value", message: `${spec.label} has no option called “${unknown}”.` };
    }
    return null;
  }

  const raw = (row.value ?? "").trim();
  if (raw === "") {
    return { token: "value", message: `${spec.label} needs a value.` };
  }
  if (spec.valueKind === "number" && !/^-?\d+$/.test(raw)) {
    return { token: "value", message: `${spec.label} takes a whole number, and “${raw}” is not one.` };
  }
  if (spec.options !== undefined && !spec.options.some((o) => o.value === raw)) {
    return { token: "value", message: `${spec.label} has no option called “${raw}”.` };
  }
  return null;
}

/**
 * `its priority is P1 — critical`, `it has no assignee`, `its title contains
 * “outage”`. A presence test reads as a phrase rather than as a field and an
 * operator, because "Assignee is not set" is how the database thinks and "it
 * has no assignee" is how a manager does.
 */
export function describeClause(row: ClauseRow): string {
  const spec = fieldByKey(row.field);
  if (spec === undefined) return row.field;

  if (!operatorNeedsValue(row.op)) {
    if (row.field === "assignee") {
      return row.op === "is_set" ? "it has an assignee" : "it has no assignee";
    }
    return `${spec.label} ${OPERATOR_VERB[row.op]}`;
  }

  if (operatorTakesMany(row.op)) {
    const labels = (row.values ?? []).map((v) => optionLabel(spec, v));
    return labels.length === 0
      ? `${spec.subject} is any of nothing yet`
      : `${spec.subject} is ${labels.join(" or ")}`;
  }

  const value = optionLabel(spec, (row.value ?? "").trim());
  if (row.field === "title") {
    return `${spec.subject} ${OPERATOR_VERB[row.op]} “${value}”`;
  }
  return `${spec.subject} ${OPERATOR_VERB[row.op]} ${value}`;
}

/** Project a stored condition into a row the editor can show. */
export function rowFromCondition(c: AutomationCondition): ConditionRow {
  switch (c.type) {
    case "priority_is":
      return { kind: "clause", id: rowId("priority", "is", c.priority), field: "priority", op: "is", value: c.priority };
    case "priority_is_not":
      return {
        kind: "clause",
        id: rowId("priority", "is_not", c.priority),
        field: "priority",
        op: "is_not",
        value: c.priority,
      };
    case "priority_in":
      return {
        kind: "clause",
        id: rowId("priority", "is_any_of", c.priorities.join(",")),
        field: "priority",
        op: "is_any_of",
        values: [...c.priorities],
      };
    case "category_is":
      return {
        kind: "clause",
        id: rowId("category", "is", c.category),
        field: "category",
        op: "is",
        value: c.category,
      };
    case "category_is_not":
      return {
        kind: "clause",
        id: rowId("category", "is_not", c.category),
        field: "category",
        op: "is_not",
        value: c.category,
      };
    case "category_in":
      return {
        kind: "clause",
        id: rowId("category", "is_any_of", c.categories.join(",")),
        field: "category",
        op: "is_any_of",
        values: [...c.categories],
      };
    case "title_contains":
      return {
        kind: "clause",
        id: rowId("title", "contains", c.needle),
        field: "title",
        op: "contains",
        value: c.needle,
      };
    case "title_not_contains":
      return {
        kind: "clause",
        id: rowId("title", "does_not_contain", c.needle),
        field: "title",
        op: "does_not_contain",
        value: c.needle,
      };
    case "unassigned":
      return { kind: "clause", id: rowId("assignee", "is_not_set", ""), field: "assignee", op: "is_not_set" };
    case "assignee_is_set":
      return { kind: "clause", id: rowId("assignee", "is_set", ""), field: "assignee", op: "is_set" };
    case "csat_score_below":
      return {
        kind: "clause",
        id: rowId("csat_score", "below", String(c.threshold)),
        field: "csat_score",
        op: "below",
        value: String(c.threshold),
      };
    case "ai_decision":
      return {
        kind: "opaque",
        id: `opaque:ai_decision:${c.question}`,
        condition: c,
        label: `Decision “${c.question}” on ${c.surface} answers ${c.operator}`,
      };
  }
}

/**
 * Build the condition to store. Returns the problem instead of a condition
 * when the row cannot be expressed, so a caller can never save half a row.
 */
export function conditionFromRow(
  row: ClauseRow,
): { ok: true; condition: AutomationCondition } | { ok: false; problem: RuleProblem } {
  const problem = rowProblems(row);
  if (problem !== null) return { ok: false, problem };
  const raw = (row.value ?? "").trim();
  switch (row.op) {
    case "is_any_of": {
      // Deduplicate: the set is unordered to the matcher, and two identical
      // chips would read as a mistake in the sentence.
      const values = [...new Set(row.values ?? [])];
      if (row.field === "priority") {
        return { ok: true, condition: { type: "priority_in", priorities: values as Priority[] } };
      }
      return { ok: true, condition: { type: "category_in", categories: values as TicketStatusCategory[] } };
    }
    case "is":
      if (row.field === "priority") {
        return { ok: true, condition: { type: "priority_is", priority: raw as Priority } };
      }
      return { ok: true, condition: { type: "category_is", category: raw as TicketStatusCategory } };
    case "is_not":
      if (row.field === "priority") {
        return { ok: true, condition: { type: "priority_is_not", priority: raw as Priority } };
      }
      return { ok: true, condition: { type: "category_is_not", category: raw as TicketStatusCategory } };
    case "contains":
      return { ok: true, condition: { type: "title_contains", needle: raw } };
    case "does_not_contain":
      return { ok: true, condition: { type: "title_not_contains", needle: raw } };
    case "is_not_set":
      return { ok: true, condition: { type: "unassigned" } };
    case "is_set":
      return { ok: true, condition: { type: "assignee_is_set" } };
    case "below":
      return { ok: true, condition: { type: "csat_score_below", threshold: Number(raw) } };
  }
}

/** The read-back of every condition, as `it has no assignee and its priority is P1 — critical`. */
export function describeConditions(rows: ConditionRow[]): string {
  if (rows.length === 0) return "every ticket on this trigger";
  return rows
    .map((r) => (r.kind === "opaque" ? r.label : describeClause(r)))
    .join(" and ");
}

export interface RuleSentenceInput {
  /** The trigger as the header select spells it, e.g. `a ticket is created`. */
  trigger: string;
  rows: ConditionRow[];
  /** Already-phrased actions, in order. */
  actions: string[];
  /** Present when the rule is board-scoped. */
  boardName?: string | null;
}

/**
 * One sentence, for the person who has to check the rule before saving it.
 *
 * It is built from the same rows the chips render, so it cannot drift from
 * them. When a row is still incomplete the sentence falls back to the raw
 * description rather than going silent — a sentence that hides a half-built
 * clause is worse than one that reads awkwardly.
 */
export function describeRule(input: RuleSentenceInput): string {
  const scope = input.boardName != null && input.boardName !== "" ? ` on ${input.boardName}` : "";
  const ifPart = input.rows.length === 0 ? "" : `, if ${describeConditions(input.rows)}`;
  if (input.actions.length === 0) {
    return `Every time ${input.trigger}${scope}${ifPart}, nothing happens yet.`;
  }
  return `Every time ${input.trigger}${scope}${ifPart}, ${listSentence(input.actions)}.`;
}

function listSentence(parts: string[]): string {
  if (parts.length === 1) return parts[0]!;
  return `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]!}`;
}

/**
 * The four rules an MSP writes on its first afternoon, as starting shapes.
 *
 * Each one lands with the structure filled in and the person, the word or the
 * note deliberately empty, so the editor's live validation names it. A preset
 * that guessed a colleague would be a rule that notifies the wrong one.
 */
export interface Starter {
  id: string;
  label: string;
  /** What the preset is for, in the words an operator would use. */
  note: string;
  trigger: string;
  rows: ClauseRow[];
  actions: (AutomationAction | { type: "plugin_action"; entry_key: string; params: Record<string, unknown> })[];
}

export const STARTERS: Starter[] = [
  {
    id: "escalate",
    label: "Escalate a critical ticket",
    note: "Tells the on-call people the moment a critical ticket lands, and leaves a note on it.",
    trigger: "ticket.created",
    rows: [{ kind: "clause", id: "priority:is:p1", field: "priority", op: "is", value: "p1" }],
    actions: [
      { type: "notify_users", user_ids: [], kind: "escalation", title: "Critical ticket received" },
      { type: "add_internal_comment", body: "Escalated automatically: filed at critical priority." },
    ],
  },
  {
    id: "assign",
    label: "Route unassigned work",
    note: "Picks up anything that arrives with nobody on it.",
    trigger: "ticket.created",
    rows: [{ kind: "clause", id: "assignee:is_not_set:", field: "assignee", op: "is_not_set" }],
    actions: [{ type: "assign_user", user_id: "" }],
  },
  {
    id: "notify",
    label: "Watch the title for a word",
    note: "Notifies the people who care when a word shows up in the subject line.",
    trigger: "ticket.created",
    rows: [{ kind: "clause", id: "title:contains:", field: "title", op: "contains", value: "" }],
    actions: [{ type: "notify_users", user_ids: [], kind: "ticket_update", title: "" }],
  },
  {
    id: "bulk-update",
    label: "Triage what is still open",
    note: "Catches live tickets with no owner and sets them to a normal priority with a note.",
    trigger: "ticket.updated",
    rows: [
      {
        kind: "clause",
        id: "category:is_any_of:open,in_progress",
        field: "category",
        op: "is_any_of",
        values: ["open", "in_progress"],
      },
      { kind: "clause", id: "assignee:is_not_set:", field: "assignee", op: "is_not_set" },
    ],
    actions: [
      { type: "set_priority", priority: "p3" },
      { type: "add_internal_comment", body: "" },
    ],
  },
];

/**
 * Shape problems only: a preset with one is broken and must not be offered,
 * because there is no value the person could type to make it saveable.
 *
 * Value problems are deliberately NOT reported here. Three of the four
 * starters land with an empty person, word or note on purpose, so the editor's
 * live validation names the gap instead of the preset guessing a colleague.
 */
export function starterShapeProblems(starter: Starter): RuleProblem[] {
  return starter.rows
    .map(rowProblems)
    .filter((p): p is RuleProblem => p !== null && p.token !== "value");
}
