// Typed IF3 envelope handling — the mobile twin of apps/web/lib/error-copy.ts.
//
// Purpose: map every machine-readable reason the API emits onto the SAME
// user copy the web app renders, and classify transport failures into the
// instant-first registers (queued / retry). A drift test asserts the
// REASON_COPY table stays byte-identical to the web map
// (tests/error-copy-parity.test.ts), so "typed 404/409 states identical to
// web" is mechanically checked, not a promise.
//
// Non-purpose: no UI, no toasts, no storage. Callers render `text`.

export interface EnvelopeError {
  reason?: string;
  message?: string;
  details?: Record<string, unknown>;
}

export interface FriendlyError {
  text: string;
  reason?: string;
  transient?: boolean;
}

/** Keep byte-identical to apps/web/lib/error-copy.ts REASON_COPY. */
export const REASON_COPY: Readonly<Record<string, string>> = {
  not_found: "That record no longer exists, or it isn't visible to you.",
  rls_sparse:
    "That item is no longer in your visible workspace — refresh to sync, then try again.",
  unauthorized: "Your session expired. Sign in again to continue.",
  forbidden: "You don't have permission for this action — ask an owner or admin.",
  validation_failed:
    "Some details are missing or invalid. Check the form and try again.",
  conflict:
    "This conflicts with an existing change. Refresh, then try again with an updated value.",
  org_slug_taken: "That workspace address is already taken.",
  time_entry_locked:
    "This time entry is locked — it's already on an invoice, so it can't be changed.",
  user_email_taken:
    "An account with this email already exists — try signing in instead.",
  role_name_taken: "A role with this name already exists in this org.",
  sku_taken: "An item with this SKU already exists in the catalog.",
  quote_number_taken: "A quote with this number already exists.",
  stage_name_taken: "A stage with this name already exists on this pipeline.",
  category_name_taken: "A category with this name already exists.",
  settings_key_taken: "A setting with this key already exists.",
  alert_source_ref_taken: "An alert source with this ref already exists.",
  already_seeded: "The demo org is already seeded — use the force-reseed control to wipe and start over.",
  version_conflict:
    "Someone else updated this while you were editing. Refresh to load their changes, then re-apply yours.",
  idempotency_key_reused:
    "This change was already submitted. Refresh — if it's missing, try once more.",
  pre_close_checklist_blocked:
    "This ticket can't close yet — required checklist items are still open.",
  tasks_unassigned:
    "Every task needs a completed / N-A status before this ticket can close.",
  dispatch_move_conflict: "That slot overlaps an existing booking — pick another slot.",
  board_move_status_conflict:
    "The target board has no matching status — choose one to continue the move.",
  import_chunk_rejected:
    "This import batch was rejected and nothing from it was saved. Fix the listed conflicts, then retry.",
  migration_cursor_mismatch:
    "The migration arrived out of order — resume from the session view so the server's expected chunk comes first.",
  migration_chunk_reused:
    "That migration chunk was already committed with different content — check the source before resending.",
  rate_limited: "Too many requests in a short window — wait a few seconds, then try again.",
  task_dependencies_unsatisfied:
    "This task still has unfinished dependencies — complete those first.",
  undo_reverts_only:
    "This record is closed — only individual fields can be reverted, not the whole record.",
  distribution_source_not_pinned:
    "This source isn't pinned — a git TAG and/or exact commit is required (never HEAD, main, or latest).",
  distribution_digest_invalid:
    "The digest doesn't check out — the artifact's sha256 doesn't match the pinned digest.",
  distribution_signature_invalid:
    "The signature block is missing or empty — code-signed artifacts are required.",
  distribution_manifest_invalid:
    "The manifest is malformed or uses unsupported constructs — nothing was imported.",
  distribution_archive_cap:
    "The archive exceeds a sandbox cap (8 MiB archive, 256 entries, 4 MB per member) — nothing was imported.",
  distribution_archive_invalid:
    "The archive couldn't be read as a valid zip (traversal-unsafe entries or a missing manifest.json).",
  service_unavailable:
    "This capability is temporarily unavailable. Try again in a moment.",
  internal_error:
    "Something went wrong on our side — your change wasn't saved. Try again.",
};

export const GENERIC_REJECTION =
  "That change couldn't be saved. Refresh and try again — if it keeps failing, the value may already be in use.";

export const OFFLINE_QUEUED_COPY =
  "You're offline — your change is saved on this device and will sync automatically when the connection returns.";
export const OFFLINE_RETRY_COPY =
  "You're offline — check your connection and try again.";
export const TIMEOUT_QUEUED_COPY =
  "The server took too long to respond — your change is queued and will retry automatically.";
export const TIMEOUT_RETRY_COPY =
  "The server took too long to respond — try again in a moment.";

/** Extracts the IF3 envelope from an SDK MeridianApiError body, a bare
 * envelope object, or an openapi-fetch `error` value. Mirror of web
 * envelopeOf(): never trusts a thrown Error's own `message` as copy. */
export function envelopeOf(error: unknown): EnvelopeError {
  const outer = error as
    | {
        error?: { reason?: unknown; message?: unknown } & Record<string, unknown>;
        reason?: unknown;
        message?: unknown;
        body?: unknown;
      }
    | null
    | undefined;
  if (outer === null || outer === undefined || typeof outer !== "object") return {};
  const isError = outer instanceof Error;
  const direct = (outer.error ?? (isError ? undefined : outer)) as
    | Record<string, unknown>
    | undefined;
  if (
    direct !== undefined &&
    (typeof direct.reason === "string" || typeof direct.message === "string")
  ) {
    return {
      reason: typeof direct.reason === "string" ? direct.reason : undefined,
      message: typeof direct.message === "string" ? direct.message : undefined,
      details: direct,
    };
  }
  const carried = (outer.body ?? null) as { error?: Record<string, unknown> } | null;
  if (carried !== null && typeof carried === "object" && carried.error !== undefined) {
    const inner = carried.error;
    return {
      reason: typeof inner.reason === "string" ? inner.reason : undefined,
      message: typeof inner.message === "string" ? inner.message : undefined,
      details: inner,
    };
  }
  return {};
}

/** Reason-code → copy with the same details folding as web. */
export function reasonCopy(reason: string, details?: unknown): string {
  const base = REASON_COPY[reason];
  if (base === undefined) return GENERIC_REJECTION;
  const det = (details ?? {}) as Record<string, unknown>;
  if (reason === "pre_close_checklist_blocked" && Array.isArray(det.missing)) {
    const items = (det.missing as unknown[]).filter(
      (m): m is string => typeof m === "string",
    );
    if (items.length > 0) {
      return `${base} Outstanding: ${items.slice(0, 3).join("; ")}${items.length > 3 ? `; +${items.length - 3} more` : ""}.`;
    }
  }
  if (
    reason === "dispatch_move_conflict" &&
    typeof det.blocker_title === "string" &&
    det.blocker_title !== ""
  ) {
    return `${base} Blocking booking: “${det.blocker_title}”.`;
  }
  return base;
}

/** Transport classification for the queue and read paths. */
export interface FailureKind {
  kind: "http" | "offline" | "timeout";
  status?: number;
  reason?: string;
  message: string;
  transient: boolean;
  conflict: boolean;
  /** IF3 conflict extras (e.g. version_conflict's expected/provided). */
  details?: Record<string, unknown>;
}

const CONFLICT_REASONS = new Set(["version_conflict", "conflict", "idempotency_key_reused"]);

export function classifyFailure(
  error: unknown,
  options?: { status?: number; offline?: boolean },
): FailureKind {
  const name = (error as { name?: unknown } | null | undefined)?.name;
  if (options?.offline === true || name === "TypeError") {
    return {
      kind: "offline",
      message: OFFLINE_RETRY_COPY,
      transient: true,
      conflict: false,
    };
  }
  if (name === "AbortError" || name === "TimeoutError") {
    return {
      kind: "timeout",
      message: TIMEOUT_RETRY_COPY,
      transient: true,
      conflict: false,
    };
  }
  const env = envelopeOf(error);
  const status = options?.status ?? statusOf(error);
  const reason = env.reason;
  return {
    kind: "http",
    status,
    reason,
    message: reason !== undefined ? reasonCopy(reason, env.details) : shortMessage(env.message),
    transient: status === 429 || (status !== undefined && status >= 500),
    conflict: reason !== undefined && CONFLICT_REASONS.has(reason),
    ...(env.details !== undefined ? { details: env.details } : {}),
  };
}

function shortMessage(message: string | undefined): string {
  if (message === undefined || message.trim().startsWith("{") || message.includes("\n")) {
    return GENERIC_REJECTION;
  }
  return message;
}

function statusOf(error: unknown): number | undefined {
  const status = (error as { status?: unknown } | null | undefined)?.status;
  return typeof status === "number" ? status : undefined;
}

/** RN fetch failures surface as TypeError; AbortError covers our timeouts. */
function failureName(error: unknown): unknown {
  return (error as { name?: unknown } | null | undefined)?.name;
}

/** The one helper every mutation surface calls. `queued: true` selects the
 * instant-runtime register (offline/timeout copy that promises replay). */
export function friendlyMutationError(
  error: unknown,
  options?: { queued?: boolean; fallback?: string; offline?: boolean },
): FriendlyError {
  const queued = options?.queued ?? false;
  const name = failureName(error);
  if (options?.offline === true || name === "TypeError") {
    return { text: queued ? OFFLINE_QUEUED_COPY : OFFLINE_RETRY_COPY, transient: true };
  }
  if (name === "AbortError" || name === "TimeoutError") {
    return { text: queued ? TIMEOUT_QUEUED_COPY : TIMEOUT_RETRY_COPY, transient: true };
  }
  const env = envelopeOf(error);
  if (env.reason !== undefined) {
    return { text: reasonCopy(env.reason, env.details), reason: env.reason };
  }
  if (
    env.message !== undefined &&
    !env.message.trim().startsWith("{") &&
    !env.message.includes("\n")
  ) {
    return { text: env.message };
  }
  return { text: options?.fallback ?? GENERIC_REJECTION };
}
