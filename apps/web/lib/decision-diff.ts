// Decision config diff — the honest line-level summary the "v2
// available" provenance banner renders (active fork vs curated starter).
//
// Purpose: production-grade honest-diff: compare the two SurfaceConfig
// JSONs by STRUCTURE — template fields (added/removed/changed path or
// cap), questions (added/removed/changed instructions, criteria,
// options) and their thresholds (act/review deltas). Never depose the
// refusals or the tiered Choice semantics — this is a READ VIEW.
//
// Deliberately NOT a deep-char-diff: the editor diffs structure, the
// JSON text is visible in the editor itself.

type Any = Record<string, unknown>;

interface Cfg {
  state_template?: { fields?: Record<string, Any> };
  questions?: Array<Any>;
}

export function doubleDiff(starter: unknown, current: unknown): string[] {
  const lines: string[] = [];
  const a = (starter ?? {}) as Cfg;
  const b = (current ?? {}) as Cfg;
  const aFields = a.state_template?.fields ?? {};
  const bFields = b.state_template?.fields ?? {};
  const names = new Set([...Object.keys(aFields), ...Object.keys(bFields)]);
  for (const name of [...names].sort()) {
    const oldF = aFields[name];
    const newF = bFields[name];
    if (oldF === undefined) lines.push(`+ field ${name} added`);
    else if (newF === undefined) lines.push(`− field ${name} removed`);
    else if (JSON.stringify(oldF) !== JSON.stringify(newF)) {
      lines.push(`field ${name} changed: ${shortLeaf(oldF)} → ${shortLeaf(newF)}`);
    }
  }
  const aQ = mapById(a.questions ?? []);
  const bQ = mapById(b.questions ?? []);
  const ids = new Set([...aQ.keys(), ...bQ.keys()]);
  for (const id of [...ids].sort()) {
    const oldQ = aQ.get(id);
    const newQ = bQ.get(id);
    if (oldQ === undefined) {
      lines.push(`+ question ${id} added`);
      continue;
    }
    if (newQ === undefined) {
      lines.push(`− question ${id} removed`);
      continue;
    }
    const { act: aAct, review: aRev } = thresholdsOf(oldQ);
    const { act: bAct, review: bRev } = thresholdsOf(newQ);
    if (JSON.stringify(stripThresholds(oldQ)) !== JSON.stringify(stripThresholds(newQ))) {
      lines.push(`question ${id} changed (instructions/criteria/options)`);
    }
    if (aAct !== bAct || aRev !== bRev) {
      lines.push(
        `thresholds ${id}: act ${aAct}→${bAct}, review ${aRev}→${bRev}`,
      );
    }
  }
  return lines;
}

function shortLeaf(field: Any): string {
  if ("from" in field) {
    const from = (field.from ?? {}) as Any;
    return `from ${from.path ?? "?"}${from.max_chars ? ` (cap ${from.max_chars})` : ""}`;
  }
  if ("derived" in field) return `derived(${(field.derived as Any)?.name ?? "?"})`;
  if ("literal" in field) return "literal";
  return JSON.stringify(field).slice(0, 40);
}

function mapById(qs: Any[]): Map<string, Any> {
  return new Map(
    qs
      .filter((q) => q.id != null)
      .map((q) => [String(q.id), q]),
  );
}

function thresholdsOf(q: Any): { act: number; review: number } {
  const t = (q.thresholds ?? {}) as { act?: number; review?: number };
  return { act: t.act ?? 0, review: t.review ?? 0 };
}

function stripThresholds(q: Any): Any {
  const { thresholds: _drop, ...rest } = q;
  return rest;
}
