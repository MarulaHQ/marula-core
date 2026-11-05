// Central IF3 envelope reason-code → user copy mapping (Wave 13 lane 2).
//
// Purpose: ONE place maps every machine-readable reason the API emits
// (crates/core-api/src/error.rs registry) onto friendly, specific user
// copy. No surface renders raw JSON, stack traces, constraint names, or
// serde text — an unmapped reason degrades to the generic rejection line,
// never to a leak. Network failures and timeouts get their own copy so
// offline/retry states read the instant-first way (queued, will replay).
//
// Non-purpose: no React, no toasts — callers render `text` wherever their
// surface already shows errors (inline, banner, dock).

/** The typed envelope body, wherever openapi-fetch parked it. */
export interface EnvelopeError {
  reason?: string;
  message?: string;
  /** Whole envelope object — conflict extras (suggestion, missing, …). */
  details?: Record<string, unknown>;
}

/** Copy ready for rendering. */
export interface FriendlyError {
  text: string;
  /** Machine reason, when one was on the wire (for branching callers). */
  reason?: string;
  /** True when the failure was offline/timeout — safe to queue/retry. */
  transient?: boolean;
}

/** One entry per reason code in the API's registry (keep in sync with
 * crates/core-api/src/error.rs — the conformance test there is the
 * server-side half of this contract). */
const REASON_COPY: Readonly<Record<string, string>> = {
  not_found: "That record no longer exists, or it isn't visible to you.",
  rls_sparse:
    "That item is no longer in your visible workspace — refresh to sync, then try again.",
  unauthorized: "Your session expired. Sign in again to continue.",
  // Signing in is not the same as an expired session: say what happened.
  signin_failed:
    "We couldn't sign you in with those details. Check the email and password and try again.",
  forbidden: "You don't have permission for this action — ask an owner or admin.",
  validation_failed:
    "Some details are missing or invalid. Check the form and try again.",
  conflict:
    "This conflicts with an existing change. Refresh, then try again with an updated value.",
  // Two scanner refusals, one vocabulary (Wave 62 lane `p62-attachment-verdicts`).
  // Both used to fall through to a generic rejection, so a user could not tell
  // "we are holding this file for an operator" from "nothing was stored".
  upload_quarantined:
    "The malware scanner rejected this file and it is held for an operator — retrying the same file will not change the answer.",
  upload_refused:
    "The malware scanner rejected this file. Nothing was stored.",
  upload_too_large_to_scan:
    "This file is larger than this deployment will scan, so nothing was stored.",
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

const GENERIC_REJECTION =
  "That change couldn't be saved. Refresh and try again — if it keeps failing, the value may already be in use.";

/**
 * Why an AI-backed proposal is not available, and what would make it available.
 *
 * Wave 46 lane 6: the two "describe it" entries (reports and dashboards) and
 * the authoring page all sit on the same admission path, and it refuses with a
 * typed reason before any model runs. One row per reason the API can emit
 * (`crates/core-api/src/routes/ai_policy.rs`, `authorize_surface_call`), so the
 * two surfaces cannot describe the same refusal two different ways. A reason
 * that is not listed here still gets the generic row below rather than a bare
 * date or a dead control.
 */
export interface AiGateCopy {
  /** What is not available, as a statement of fact. */
  title: string;
  /** The one thing a person can do about it. */
  remedy: string;
}

const AI_GATE_COPY: Readonly<Record<string, AiGateCopy>> = {
  ai_features_disabled: {
    title: "AI is switched off for this organization",
    remedy: "An owner or admin can switch it on under Settings, AI.",
  },
  ai_surface_disabled: {
    title: "This AI surface is switched off",
    remedy: "An owner or admin can enable the surface under Settings, AI.",
  },
  ai_surface_not_granted: {
    title: "No grant covers you on this surface",
    remedy: "Ask an owner or admin to grant it under Settings, AI, which is deny-by-default.",
  },
  ai_surface_internal: {
    title: "That surface is internal",
    remedy: "Drafting here uses the general chat surface; an internal surface cannot be called directly.",
  },
  ai_hard_cap_reached: {
    title: "AI usage has reached its cap for this period",
    remedy: "Wait for the period to reset, or ask an owner or admin to raise the cap under Settings, AI.",
  },
  ai_no_credential: {
    title: "No model credential is available",
    remedy: "An owner or admin can add a provider key under Settings, AI, credentials.",
  },
  ai_no_models_configured: {
    title: "No model is configured",
    remedy: "An owner or admin can add one to the catalog under Settings, AI.",
  },
  ai_model_not_allowed: {
    title: "That model is not on this organization's allowlist",
    remedy: "Pick an allowlisted model, or ask an owner or admin to add it under Settings, AI.",
  },
  ai_model_not_capable: {
    title: "The selected model cannot draft reports",
    remedy: "Drafting needs tool calling; ask an owner or admin to allow a model that carries it.",
  },
  ai_agent_capability_not_granted: {
    title: "The agent capability this run needs is not granted",
    remedy: "Ask an owner or admin to grant the capability under Settings, AI.",
  },
};

const AI_GATE_FALLBACK: AiGateCopy = {
  title: "AI is not available for this organization right now",
  remedy: "An owner or admin can check the switch, the grant, the model and the credential under Settings, AI.",
};

/**
 * Reads one refusal reason into a stated reason plus its remedy.
 *
 * Returns null for anything that is not an AI admission refusal, so a caller
 * keeps the ordinary error copy for a validation or permission failure.
 */
export function aiGateCopy(reason: string): AiGateCopy | null {
  if (!reason.startsWith("ai_")) return null;
  return AI_GATE_COPY[reason] ?? AI_GATE_FALLBACK;
}

/** Offline / timeout copy. Two registers per the instant-first doctrine:
 * surfaces WITH the durable queue (instant runtime) tell the user the
 * change is saved and will replay; direct surfaces (envelope-write) tell
 * the user to check the connection and retry. */
export const OFFLINE_QUEUED_COPY =
  "You're offline — your change is saved on this device and will sync automatically when the connection returns.";
export const OFFLINE_RETRY_COPY =
  "You're offline — check your connection and try again.";
export const TIMEOUT_QUEUED_COPY =
  "The server took too long to respond — your change is queued and will retry automatically.";
export const TIMEOUT_RETRY_COPY =
  "The server took too long to respond — try again in a moment.";

/** Extracts the IF3 envelope from an openapi-fetch `error` value, a
 * thrown Error carrying one (SDK `MarulaApiError.body`), or a bare
 * body object. */
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
  // A thrown Error's own `message` may be library/framework text — never
  // trusted as copy. Only envelope objects (bare or parked under
  // `error`) and an SDK error's carried body speak.
  const isError = outer instanceof Error;
  const direct = (outer.error ?? (isError ? undefined : outer)) as Record<string, unknown> | undefined;
  if (direct !== undefined && (typeof direct.reason === "string" || typeof direct.message === "string")) {
    return {
      reason: typeof direct.reason === "string" ? direct.reason : undefined,
      message: typeof direct.message === "string" ? direct.message : undefined,
      details: direct,
    };
  }
  // Thrown Error carrying the whole envelope response body (`body`).
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

/** Reason-code → copy, with details folded in where the registry's
 * conflict extras sharpen the message (checklist blockers, rate window). */
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
  if (reason === "dispatch_move_conflict" && typeof det.blocker_title === "string" && det.blocker_title !== "") {
    return `${base} Blocking booking: “${det.blocker_title}”.`;
  }
  if (reason === "upload_quarantined" || reason === "upload_refused") {
    const name = typeof det.filename === "string" ? det.filename : "";
    const scanner = typeof det.scanner === "string" ? det.scanner : "";
    const detail = typeof det.detail === "string" ? det.detail.trim() : "";
    const parts = [base];
    if (name !== "") parts.push(`File: ${name}.`);
    if (scanner !== "" && scanner !== "none") parts.push(`Scanner: ${scanner}.`);
    if (detail !== "") parts.push(`Detail: ${detail}.`);
    return parts.join(" ");
  }
  return base;
}

/** Classifies thrown transport failures (fetch aborts, offline). DOM
 * exceptions don't reliably extend Error in every runtime — classify by
 * duck-typed `name`, never by prototype. */
function transportCopy(error: unknown, queued: boolean): FriendlyError | null {
  const offline = typeof navigator !== "undefined" && navigator.onLine === false;
  const offlineText = queued ? OFFLINE_QUEUED_COPY : OFFLINE_RETRY_COPY;
  const timeoutText = queued ? TIMEOUT_QUEUED_COPY : TIMEOUT_RETRY_COPY;
  if (offline) return { text: offlineText, transient: true };
  const name = (error as { name?: unknown } | null | undefined)?.name;
  if (name === "TimeoutError" || name === "AbortError") {
    return { text: timeoutText, transient: true };
  }
  // fetch() network failures surface as TypeError.
  if (name === "TypeError") {
    return { text: offlineText, transient: true };
  }
  return null;
}

/** Maps any mutation failure onto friendly copy: an offline/timeout first,
 * then the typed reason, then the server's fixed human message, then the
 * caller's `fallback` (or the generic rejection). The one helper every
 * mutation surface should call.
 *
 * `queued: true` selects the instant-runtime register (offline/timeout
 * copy that promises automatic replay) — only true for writes that went
 * through the durable offline queue. `fallback` lets read-failure
 * call sites keep their own wording instead of the mutation register. */
export function friendlyMutationError(
  error: unknown,
  options?: { queued?: boolean; fallback?: string },
): FriendlyError {
  const transport = transportCopy(error, options?.queued ?? false);
  if (transport !== null) return transport;
  const env = envelopeOf(error);
  if (env.reason !== undefined) {
    return { text: reasonCopy(env.reason, env.details), reason: env.reason };
  }
  // Lane 1 guarantees `message` is fixed human copy — acceptable as a
  // fallback, but only when it is a plain short line (never JSON).
  if (
    env.message !== undefined &&
    !env.message.trim().startsWith("{") &&
    !env.message.includes("\n")
  ) {
    return { text: env.message };
  }
  return { text: options?.fallback ?? GENERIC_REJECTION };
}

/** Convenience for callers that only need the text. */
export function friendlyMutationText(
  error: unknown,
  fallback?: string,
): string {
  return friendlyMutationError(error, { fallback }).text;
}

/** Copy for an openapi-fetch result whose pin declares no error responses
 * (so `error` is typed away but still carries the IF3 envelope body at
 * runtime on any non-2xx). Reads `error` when present, else the whole
 * result; thrown transport failures keep their own copy. */
export function resultErrorText(result: unknown, fallback?: string): string {
  const r = result as { error?: unknown } | null | undefined;
  const err = r?.error;
  if (err !== undefined && err !== null) return friendlyMutationText(err, fallback);
  return friendlyMutationText(result, fallback);
}

/** Copy for a rejected instant WriteResult (or any rejection-shaped
 * object): typed reason copy when a reason rode the envelope, else the
 * server's fixed human message, else the caller's fallback. Read-model
 * branches may still test `result.message` for keywords — this helper is
 * only for what gets RENDERED. */
export function rejectionText(
  result: { reason?: string; message?: string; details?: unknown },
  fallback?: string,
): string {
  if (result.reason !== undefined) {
    return reasonCopy(result.reason, result.details);
  }
  if (
    result.message !== undefined &&
    !result.message.trim().startsWith("{") &&
    !result.message.includes("\n")
  ) {
    return result.message;
  }
  return fallback ?? GENERIC_REJECTION;
}
