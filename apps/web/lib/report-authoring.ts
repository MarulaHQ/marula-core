// lib/report-authoring — the browser side of "describe a report".
//
// One leg, on one endpoint: `POST /api/v1/reports/author` (Wave 46 lane 3). The
// server drafts and validates; this module never validates a spec, never
// derives a shape and never invents a rule. Its whole job is:
//
//   * send the transcript and, when editing, the current spec + card;
//   * type the answer, including the refusal envelope the AI admission path
//     returns (`ai_features_disabled`, a cap, no allowlisted model, …);
//   * name the draft's status, which is client state and nothing else — the
//     Wave 46 plan gives this row no migration, so a draft is never a row;
//   * accept through the EXISTING definition routes. There is no second save
//     path: `POST /api/v1/report-definitions` for a new report,
//     `PUT /api/v1/report-definitions/{id}` for an edit, both re-validating on
//     the server exactly as they did before this lane existed. The card saves
//     with the spec (migration 0180), so the window, the refresh and the size
//     the reviewer approved are the ones the stored definition carries.
//
// Why fetch and not the generated client: the endpoint is not in the pinned
// OpenAPI snapshot yet (re-pinning boots the stack, which is a gate step, not a
// lane step). Until the pins move, the call rides `fetch` with locally typed
// shapes — the same posture `app/automations/ai-client.ts` already takes for the
// endpoints openapi-fetch cannot express. `acceptReportDraft` DOES use the typed
// client, because the save routes are pinned.

import type { components } from "@marula-core/sdk";
import { api, apiBaseUrl, loadTokens } from "./api";

export type ReportSpec = components["schemas"]["ReportSpec"];
export type CardSpec = components["schemas"]["CardSpec"];

/** One typed author-time refusal, as `report_authoring` reports it. */
export interface ReportAuthoringIssue {
  /** Stable machine code: `unknown_column`, `invalid_card`, `not_json`, … */
  code: string;
  /** The offending spec or card member, when the server knows it. */
  path?: string;
  message: string;
}

/** One structural change against the current draft; nothing is applied here. */
export interface ReportDiffEntry {
  op: "add" | "remove" | "change";
  /** `view`, `group_by/priority`, `measures/count`, `card/window_days`, … */
  path: string;
  before?: unknown;
  after?: unknown;
}

/** The draft the server validated. `shape` is derived server-side, never sent. */
export interface ReportDraft {
  name?: string;
  spec: ReportSpec;
  card: CardSpec;
  /** `chart` | `stat` | `table`. */
  shape: string;
  /** One sentence saying what the report shows, derived from the spec. */
  reads: string;
}

/**
 * One check that could not be passed, as the server reports it (Wave 64 lane
 * 5). `kind` reuses the card contract's own slugs where one exists
 * (`no_rows`, `no_headers`, `no_scalar`, `window_wider_than_refresh`, …), so a
 * reviewer reads one vocabulary for a registry refusal and a data failure.
 */
export interface DraftFinding {
  kind: string;
  /** The dashboard card this finding belongs to, zero-based. A report has none. */
  card_index?: number;
  message: string;
}

/** One verification round: what the model's answer produced. Round 0 is first. */
export interface VerificationAttempt {
  round: number;
  findings: DraftFinding[];
}

/**
 * What the data check did. `ran: false` means nothing was checked — that is not
 * the same as clean, and the row must never say it is.
 */
export interface DraftVerification {
  ran: boolean;
  /** The classes that were checked, in the server's words. */
  checked: string[];
  attempts: VerificationAttempt[];
  /** Findings a repair cleared. */
  repaired: DraftFinding[];
  /** Findings that survive; empty is the only clean answer. */
  findings: DraftFinding[];
}

export interface AuthorReportResponse {
  valid: boolean;
  /** Always false, always present: the endpoint persists nothing. */
  saved: boolean;
  draft?: ReportDraft;
  issues: ReportAuthoringIssue[];
  diff: ReportDiffEntry[];
  tool_calls: { name: string; argument: string; outcome: string }[];
  usage: { prompt_tokens: number; completion_tokens: number; steps: number };
  model_info: { provider: string; model: string; resolution: string };
  catalogue: { views: number; shapes: number; rules: number; prompt_chars: number };
  /** Absent on an answer from a server that predates the data check. */
  verification?: DraftVerification;
  error?: string;
}

/**
 * The current draft, when editing an existing report.
 *
 * `card` is optional because a definition may legitimately have no card. Without
 * one the server reports the card members as additions instead of inventing a
 * base.
 */
export interface CurrentDraft {
  id?: string;
  name?: string;
  spec: ReportSpec;
  card?: CardSpec;
}

export type AuthorOutcome =
  | { ok: true; result: AuthorReportResponse; latencyMs: number }
  | { ok: false; refusal: string; message: string };

/**
 * The draft row's status. Presentation state, derived from the answer — the
 * server has no status column to read because no draft is stored.
 */
export type DraftStatus =
  | "idle"
  | "drafting"
  | "ready"
  | "needs_fixes"
  | "refused"
  | "accepted"
  | "rejected";

/** The word the row prints next to its pill. Never a colour on its own. */
export const DRAFT_STATUS_LABEL: Record<DraftStatus, string> = {
  idle: "No draft",
  drafting: "Drafting",
  ready: "Ready to accept",
  needs_fixes: "Needs fixes",
  refused: "Not available",
  accepted: "Saved",
  rejected: "Discarded",
};

/** The pill tone for each status: success, attention, progress, neutral. */
export const DRAFT_STATUS_TONE: Record<
  DraftStatus,
  "progress" | "success" | "attention" | "neutral" | "resolved"
> = {
  idle: "neutral",
  drafting: "progress",
  ready: "success",
  needs_fixes: "attention",
  refused: "attention",
  accepted: "resolved",
  rejected: "neutral",
};

/**
 * Reads one answer into a status. `valid` alone is not enough: a refusal that
 * never reached the model (AI off, a cap) is a different state from a draft the
 * registry rejected, and the reviewer is told which. A draft the data check
 * could not draw arrives with `valid: false`, so it lands on `needs_fixes`
 * with its findings, never on `ready`.
 */
export function draftStatusOf(answer: AuthorReportResponse | null, refused: boolean): DraftStatus {
  if (refused) return "refused";
  if (answer === null) return "idle";
  if (answer.valid && answer.draft !== undefined) return "ready";
  return "needs_fixes";
}

/** `+2 −1 ~3` for the row's meta line. Empty string when nothing changed. */
export function diffTally(diff: ReportDiffEntry[]): string {
  const count = (op: ReportDiffEntry["op"]) => diff.filter((d) => d.op === op).length;
  const parts: string[] = [];
  if (count("add") > 0) parts.push(`+${count("add")}`);
  if (count("remove") > 0) parts.push(`−${count("remove")}`);
  if (count("change") > 0) parts.push(`~${count("change")}`);
  return parts.join(" ");
}

/**
 * One line for the draft row: what was checked and in how many rounds.
 *
 * `ran: false` says so plainly, because an unchecked draft and a checked one
 * are different facts and the reviewer is owed the true one. The line never
 * claims a verdict the server did not report.
 */
export function verificationSummary(verification: DraftVerification): string {
  if (!verification.ran) return "Not checked: no data check ran for this draft.";
  const rounds = verification.attempts.length;
  const classes = verification.checked.length;
  const roundWords = rounds === 1 ? "1 round" : `${rounds} rounds`;
  if (verification.findings.length > 0) {
    return `Checked ${classes} classes in ${roundWords}; ${verification.findings.length} still failing.`;
  }
  return `Checked ${classes} classes in ${roundWords}; nothing failed.`;
}

/**
 * Registry issues the verification block is not already showing.
 *
 * The server lists a surviving finding in `issues` as well, so a client that
 * reads only `issues` still tells the truth. When both are rendered the row
 * would print the same sentence twice, so the verification block owns them.
 */
export function issuesOutsideVerification(
  issues: ReportAuthoringIssue[],
  verification: DraftVerification | null | undefined,
): ReportAuthoringIssue[] {
  if (verification === null || verification === undefined) return issues;
  const shown = new Set(
    [...verification.findings, ...verification.repaired].map((finding) => finding.message),
  );
  return issues.filter((issue) => !shown.has(issue.message));
}

/** The refusal envelope the API returns for a typed AI refusal. */
interface RefusalShape {
  error?: { message?: unknown; reason?: unknown };
  message?: unknown;
  reason?: unknown;
}

function refusalOf(status: number, envelope: RefusalShape): AuthorOutcome & { ok: false } {
  const shape = envelope.error ?? envelope;
  const reason = typeof shape.reason === "string" ? shape.reason : undefined;
  const message =
    typeof shape.message === "string"
      ? shape.message
      : reason === "ai_features_disabled"
        ? "AI is switched off for this org."
        : "The draft did not come back.";
  const code = status === 403 ? (reason ?? "ai_refused") : status === 429 ? "rate_limited" : "request_failed";
  return { ok: false, refusal: code, message };
}

/**
 * Asks the server for a report draft from one transcript. Never saves.
 *
 * `current` is the report being edited; without it the answer diffs against
 * nothing, which is what a reviewer needs to read a fresh proposal.
 */
export async function authorReportDraft(args: {
  transcript: string;
  current?: CurrentDraft | null;
  signal?: AbortSignal;
}): Promise<AuthorOutcome> {
  const tokens = loadTokens();
  if (tokens === null) {
    return { ok: false, refusal: "auth", message: "Not signed in." };
  }
  const started = performance.now();
  let response: Response;
  try {
    response = await fetch(`${apiBaseUrl()}/api/v1/reports/author`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        accept: "application/json",
        authorization: `Bearer ${tokens.access_token}`,
      },
      signal: args.signal,
      body: JSON.stringify({
        surface_key: "general_chat",
        transcript: args.transcript,
        spec: args.current?.spec ?? null,
        card: args.current?.card ?? null,
        name: args.current?.name ?? null,
      }),
    });
  } catch (error) {
    return {
      ok: false,
      refusal: "transport",
      message: error instanceof Error ? error.message : "The request did not complete.",
    };
  }
  if (!response.ok) {
    const body = (await response.json().catch(() => ({}))) as RefusalShape;
    return refusalOf(response.status, body);
  }
  const body = (await response.json().catch(() => null)) as AuthorReportResponse | null;
  if (body === null) {
    return { ok: false, refusal: "transport", message: "The answer was not JSON." };
  }
  return { ok: true, result: body, latencyMs: Math.round(performance.now() - started) };
}

export type AcceptOutcome =
  | { ok: true; id: string; created: boolean }
  | { ok: false; message: string };

function idempotencyKey(prefix: string): string {
  return typeof crypto !== "undefined" && crypto.randomUUID
    ? crypto.randomUUID()
    : `${prefix}-${Date.now()}`;
}

/**
 * Saves an accepted draft through the EXISTING definition routes.
 *
 * A new report is a create; an edit is a full-replace PUT on the definition the
 * caller was editing. Both validate the spec and the card server-side before
 * storage, so a draft that arrived valid and is then saved cannot arrive invalid
 * by way of this function — it sends the spec and the card the server returned,
 * unmodified. The card is what makes the saved report keep its window, refresh
 * and size.
 */
export async function acceptReportDraft(args: {
  draft: ReportDraft;
  name: string;
  currentId?: string | null;
}): Promise<AcceptOutcome> {
  const name = args.name.trim() === "" ? (args.draft.name ?? "Untitled report") : args.name.trim();
  if (args.currentId !== undefined && args.currentId !== null && args.currentId !== "") {
    const { data, error } = await api.api.PUT("/api/v1/report-definitions/{id}", {
      params: { path: { id: args.currentId } },
      body: {
        name,
        spec: args.draft.spec,
        card: args.draft.card,
        idempotency_key: idempotencyKey("report-author"),
      },
    });
    if (error !== undefined || data === undefined) {
      return { ok: false, message: "This report could not be saved. Nothing was changed." };
    }
    return { ok: true, id: data.id, created: false };
  }
  const { data, error } = await api.api.POST("/api/v1/report-definitions", {
    body: {
      name,
      spec: args.draft.spec,
      card: args.draft.card,
      scope: "self",
      idempotency_key: idempotencyKey("report-author"),
    },
  });
  if (error !== undefined || data === undefined) {
    return { ok: false, message: "This report could not be saved. Nothing was created." };
  }
  return { ok: true, id: data.id, created: true };
}

/** One diff entry as a reviewer reads it: `view` → `group_by/priority`. */
export function diffPathParts(entry: ReportDiffEntry): { section: string; member: string } {
  const slash = entry.path.indexOf("/");
  if (slash === -1) return { section: entry.path, member: "" };
  return {
    section: entry.path.slice(0, slash),
    member: entry.path.slice(slash + 1),
  };
}

/** A value as one compact line. Never JSON.stringify's a whole object into a cell. */
export function diffValue(value: unknown): string {
  if (value === undefined || value === null) return "—";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (Array.isArray(value)) return value.map((v) => diffValue(v)).join(", ");
  if (typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>).filter(
      ([, v]) => v !== null && v !== undefined,
    );
    if (entries.length === 0) return "—";
    return entries.map(([k, v]) => `${k}: ${diffValue(v)}`).join(", ");
  }
  return String(value);
}
