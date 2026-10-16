// Unit suite: the automation condition grammar (Wave 43 lane 17b).
//
// Covers the four promises the rebuild makes, each of which used to be a
// select row with no opinion about itself:
//   1. a field declares its own operators, and an operator the wire enum
//      cannot store is refused rather than offered;
//   2. a bad row is reported by NAMING the offending token, not by saying
//      "invalid condition";
//   3. the one-sentence read-back is built from the same clauses the chips
//      render, so the two cannot disagree;
//   4. every stored condition survives a round trip through the editor —
//      all twelve of them, including the one variant the grammar cannot
//      express, which is preserved rather than dropped.
import { describe, expect, it } from "vitest";
import {
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
  starterShapeProblems,
  withOperator,
  type AutomationCondition,
  type ClauseRow,
} from "../../app/automations/rule-grammar";

describe("field registry", () => {
  it("offers only operators a stored condition can carry", () => {
    // Each list is exactly the variants `automations.rs` has for that field.
    // Offering `is not` for a field with no `_is_not` variant would build a
    // row that cannot be saved.
    expect(fieldByKey("priority")?.operators).toEqual(["is", "is_not", "is_any_of"]);
    expect(fieldByKey("category")?.operators).toEqual(["is", "is_not", "is_any_of"]);
    expect(fieldByKey("title")?.operators).toEqual(["contains", "does_not_contain"]);
    expect(fieldByKey("assignee")?.operators).toEqual(["is_not_set", "is_set"]);
    expect(fieldByKey("csat_score")?.operators).toEqual(["below"]);
  });

  it("says whether an operator takes a value, and whether it takes a set", () => {
    expect(operatorNeedsValue("is")).toBe(true);
    expect(operatorNeedsValue("does_not_contain")).toBe(true);
    expect(operatorNeedsValue("below")).toBe(true);
    expect(operatorNeedsValue("is_not_set")).toBe(false);
    expect(operatorNeedsValue("is_set")).toBe(false);

    expect(operatorTakesMany("is_any_of")).toBe(true);
    expect(operatorTakesMany("is")).toBe(false);
    expect(operatorTakesMany("is_not_set")).toBe(false);
  });

  it("gives every field a sentence subject", () => {
    for (const field of RULE_FIELDS) {
      expect(field.subject.trim()).not.toBe("");
    }
  });
});

describe("live validation names the offending token", () => {
  const base: ClauseRow = { kind: "clause", id: "priority:is:p1", field: "priority", op: "is", value: "p1" };

  it("flags a field the rule cannot read", () => {
    const problem = rowProblems({ ...base, field: "wat" });
    expect(problem?.token).toBe("field");
    expect(problem?.message).toContain("wat");
  });

  it("flags an operator the field does not accept, and names both sides", () => {
    const problem = rowProblems({ ...base, op: "contains" });
    expect(problem?.token).toBe("operator");
    expect(problem?.message).toContain("Priority");
    expect(problem?.message).toContain("is");
  });

  it("flags a missing value", () => {
    const problem = rowProblems({ ...base, field: "title", op: "contains", value: "   " });
    expect(problem?.token).toBe("value");
    expect(problem?.message).toContain("Title");
  });

  it("flags a non-numeric threshold", () => {
    const problem = rowProblems({ kind: "clause", id: "x", field: "csat_score", op: "below", value: "soon" });
    expect(problem?.token).toBe("value");
    expect(problem?.message).toContain("soon");
  });

  it("flags an option that does not exist", () => {
    const problem = rowProblems({ ...base, value: "p9" });
    expect(problem?.token).toBe("value");
    expect(problem?.message).toContain("p9");
  });

  it("flags an empty set member", () => {
    const problem = rowProblems({ kind: "clause", id: "s", field: "priority", op: "is_any_of", values: [] });
    expect(problem?.token).toBe("value");
    expect(problem?.message).toContain("Priority");
  });

  it("flags an option inside a set that does not exist", () => {
    const problem = rowProblems({
      kind: "clause",
      id: "s",
      field: "priority",
      op: "is_any_of",
      values: ["p1", "p9"],
    });
    expect(problem?.token).toBe("value");
    expect(problem?.message).toContain("p9");
  });

  it("passes a complete row, a set row, and a presence row with no value", () => {
    expect(rowProblems(base)).toBeNull();
    expect(rowProblems({ kind: "clause", id: "s", field: "priority", op: "is_any_of", values: ["p1", "p2"] })).toBeNull();
    expect(rowProblems({ kind: "clause", id: "a", field: "assignee", op: "is_not_set" })).toBeNull();
    expect(rowProblems({ kind: "clause", id: "a", field: "assignee", op: "is_set" })).toBeNull();
  });
});

describe("stored condition round trip", () => {
  it("rebuilds the same condition it was given, for every variant the grammar covers", () => {
    const cases: AutomationCondition[] = [
      { type: "priority_is", priority: "p2" },
      { type: "priority_is_not", priority: "p2" },
      { type: "priority_in", priorities: ["p1", "p2"] },
      { type: "category_is", category: "waiting" },
      { type: "category_is_not", category: "closed" },
      { type: "category_in", categories: ["open", "waiting"] },
      { type: "title_contains", needle: "outage" },
      { type: "title_not_contains", needle: "voicemail" },
      { type: "unassigned" },
      { type: "assignee_is_set" },
      { type: "csat_score_below", threshold: 3 },
    ];
    for (const condition of cases) {
      const row = rowFromCondition(condition);
      expect(row.kind, `${condition.type} projects to a clause row`).toBe("clause");
      const built = conditionFromRow(row as ClauseRow);
      expect(built.ok, `${condition.type} rebuilds`).toBe(true);
      if (built.ok) expect(built.condition).toEqual(condition);
    }
  });

  it("refuses to build a condition from an unfillable row", () => {
    const built = conditionFromRow({ kind: "clause", id: "t", field: "title", op: "contains", value: "" });
    expect(built.ok).toBe(false);
    if (!built.ok) expect(built.problem.token).toBe("value");
  });

  it("drops a duplicate set member rather than storing it twice", () => {
    const built = conditionFromRow({
      kind: "clause",
      id: "s",
      field: "priority",
      op: "is_any_of",
      values: ["p1", "p1", "p2"],
    });
    expect(built.ok).toBe(true);
    if (built.ok) expect(built.condition).toEqual({ type: "priority_in", priorities: ["p1", "p2"] });
  });

  it("keeps the decision condition it cannot express, rather than dropping it", () => {
    const decision: AutomationCondition = {
      type: "ai_decision",
      surface: "triage",
      question: "is_this_a_breach",
      operator: "confidence_above",
      threshold: 0.8,
      confidence_path: null,
    };
    const row = rowFromCondition(decision);
    expect(row.kind).toBe("opaque");
    if (row.kind === "opaque") expect(row.condition).toEqual(decision);
  });

  it("survives a stored condition written before the enum existed", () => {
    // The demo seed writes rules in the old data-surface filter shape. The
    // engine skips those at evaluation; the editor must not turn one into a
    // blank page.
    const legacy = { field: "priority", op: "eq", value: "p1" } as unknown as AutomationCondition;
    const row = rowFromCondition(legacy);
    expect(row.kind).toBe("opaque");
    if (row.kind === "opaque") {
      expect(row.label).toContain("priority");
      expect(row.condition).toEqual(legacy);
    }
    const total = rowFromCondition({ nonsense: true } as unknown as AutomationCondition);
    expect(total.kind).toBe("opaque");
  });
});

describe("the read-back is one sentence a manager can check", () => {
  const rows: ClauseRow[] = [
    { kind: "clause", id: "assignee:is_not_set:", field: "assignee", op: "is_not_set" },
    { kind: "clause", id: "priority:is:p1", field: "priority", op: "is", value: "p1" },
  ];

  it("reads a presence test as a phrase, both ways", () => {
    expect(describeClause(rows[0]!)).toBe("it has no assignee");
    expect(describeClause({ kind: "clause", id: "a", field: "assignee", op: "is_set" })).toBe("it has an assignee");
  });

  it("reads a comparison as a comparison", () => {
    expect(describeClause(rows[1]!)).toBe("its priority is P1 — critical");
  });

  it("reads a negation and a set membership", () => {
    expect(describeClause({ kind: "clause", id: "n", field: "priority", op: "is_not", value: "p4" })).toBe(
      "its priority is not P4 — low",
    );
    expect(
      describeClause({ kind: "clause", id: "s", field: "category", op: "is_any_of", values: ["open", "waiting"] }),
    ).toBe("its status is open or waiting");
  });

  it("reads both title predicates with the needle quoted", () => {
    expect(describeClause({ kind: "clause", id: "c", field: "title", op: "contains", value: "vpn" })).toBe(
      "its title contains “vpn”",
    );
    expect(describeClause({ kind: "clause", id: "d", field: "title", op: "does_not_contain", value: "vpn" })).toBe(
      "its title does not contain “vpn”",
    );
  });

  it("joins clauses with 'and'", () => {
    expect(describeConditions(rows)).toBe("it has no assignee and its priority is P1 — critical");
  });

  it("says when a rule matches everything", () => {
    expect(describeConditions([])).toBe("every ticket on this trigger");
  });

  it("builds the whole sentence, with the trigger and the scope", () => {
    expect(
      describeRule({
        trigger: "a ticket is created",
        rows,
        actions: ["assign it to Alice"],
        boardName: "Support",
      }),
    ).toBe(
      "Every time a ticket is created on Support, if it has no assignee and its priority is P1 — critical, assign it to Alice.",
    );
  });

  it("is honest when the rule has no action yet", () => {
    expect(describeRule({ trigger: "a ticket is created", rows, actions: [] })).toBe(
      "Every time a ticket is created, if it has no assignee and its priority is P1 — critical, nothing happens yet.",
    );
  });

  it("lists three actions without a serial comma pile-up", () => {
    expect(describeRule({ trigger: "a ticket is created", rows: [], actions: ["a", "b", "c"] })).toBe(
      "Every time a ticket is created, a, b and c.",
    );
  });
});

describe("starters", () => {
  it("are all offerable — none is structurally broken", () => {
    for (const starter of STARTERS) {
      expect(starterShapeProblems(starter), starter.id).toEqual([]);
    }
  });

  it("leave the person, the word or the note for the operator to fill in, on purpose", () => {
    const assign = STARTERS.find((s) => s.id === "assign")!;
    const notify = STARTERS.find((s) => s.id === "notify")!;
    expect(assign.actions[0]).toMatchObject({ type: "assign_user", user_id: "" });
    expect(notify.rows[0]).toMatchObject({ field: "title", value: "" });
  });

  it("covers the four common cases the brief names", () => {
    expect(STARTERS.map((s) => s.id)).toEqual(["escalate", "assign", "notify", "bulk-update"]);
  });

  it("uses the widened operators where the common case needs them", () => {
    const bulk = STARTERS.find((s) => s.id === "bulk-update")!;
    expect(bulk.rows[0]).toMatchObject({ op: "is_any_of", values: ["open", "in_progress"] });
    expect(bulk.rows[1]).toMatchObject({ op: "is_not_set" });
  });
});

describe("blank rows", () => {
  it("start with the field's first operator and option, so a new row is valid", () => {
    const row = blankRow("category");
    expect(row.op).toBe("is");
    expect(row.value).toBe("open");
    expect(rowProblems(row)).toBeNull();
  });

  it("leave the value off a presence field, and default it to the common case", () => {
    const row = blankRow("assignee");
    expect(row.op).toBe("is_not_set");
    expect(row.value).toBeUndefined();
    expect(rowProblems(row)).toBeNull();
  });
});

describe("switching a row's operator carries the value across", () => {
  it("moves a single value into a set", () => {
    const row = withOperator({ kind: "clause", id: "p", field: "priority", op: "is", value: "p2" }, "is_any_of");
    expect(row.op).toBe("is_any_of");
    expect(row.values).toEqual(["p2"]);
    expect(rowProblems(row)).toBeNull();
  });

  it("moves the first set member back out", () => {
    const row = withOperator(
      { kind: "clause", id: "p", field: "priority", op: "is_any_of", values: ["p2", "p3"] },
      "is",
    );
    expect(row.value).toBe("p2");
    expect(rowProblems(row)).toBeNull();
  });

  it("drops the value entirely for a presence test", () => {
    const row = withOperator({ kind: "clause", id: "a", field: "assignee", op: "is_set" }, "is_not_set");
    expect(row.value).toBeUndefined();
    expect(rowProblems(row)).toBeNull();
  });

  it("reports an operator the field cannot carry, instead of building an unsaveable row", () => {
    const row = withOperator({ kind: "clause", id: "p", field: "priority", op: "is", value: "p2" }, "is_not_set");
    expect(rowProblems(row)?.token).toBe("operator");
    expect(conditionFromRow(row).ok).toBe(false);
  });

  it("seeds an empty set from the field's first option, so the row never lands invalid", () => {
    const row = withOperator({ kind: "clause", id: "t", field: "priority", op: "is", value: "" }, "is_any_of");
    expect(row.values).toEqual(["p1"]);
    expect(rowProblems(row)).toBeNull();
  });
});
