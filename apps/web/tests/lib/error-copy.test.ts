// Central reason-code → copy mapping (Wave 13 lane 2).
//
// Contract: every reason code in the API's IF3 registry maps to friendly,
// specific copy; an unmapped reason degrades to the generic rejection
// (never raw JSON or server prose); offline/timeout classify as transient
// with the queued/retry registers.
import { describe, expect, it } from "vitest";
import {
  OFFLINE_QUEUED_COPY,
  OFFLINE_RETRY_COPY,
  TIMEOUT_QUEUED_COPY,
  TIMEOUT_RETRY_COPY,
  envelopeOf,
  friendlyMutationError,
  friendlyMutationText,
  reasonCopy,
  rejectionText,
} from "../../lib/error-copy";

const envelope = (reason: string, extra: Record<string, unknown> = {}) => ({
  error: { code: 409, reason, message: `server prose for ${reason}`, ...extra },
});

describe("error-copy — registry coverage", () => {
  it("maps every reason in the API registry to specific copy", () => {
    const registry = [
      "not_found",
      "rls_sparse",
      "unauthorized",
      "forbidden",
      "validation_failed",
      "conflict",
      "org_slug_taken",
      "user_email_taken",
      "role_name_taken",
      "sku_taken",
      "quote_number_taken",
      "stage_name_taken",
      "category_name_taken",
      "settings_key_taken",
      "alert_source_ref_taken",
      "already_seeded",
      "version_conflict",
      "idempotency_key_reused",
      "pre_close_checklist_blocked",
      "dispatch_move_conflict",
      "board_move_status_conflict",
      "import_chunk_rejected",
      "migration_cursor_mismatch",
      "migration_chunk_reused",
      "rate_limited",
      "task_dependencies_unsatisfied",
      "undo_reverts_only",
      "service_unavailable",
      "internal_error",
    ];
    for (const reason of registry) {
      const text = reasonCopy(reason);
      // Friendly copy, not the raw code and not server prose.
      expect(text).not.toBe(reason);
      expect(text).not.toContain(`server prose for`);
      expect(text).not.toMatch(/^\s*$/);
    }
    // Every mapped copy is distinct — no two codes share a line.
    const texts = new Set(registry.map((r) => reasonCopy(r)));
    expect(texts.size).toBe(registry.length);
  });

  it("folds conflict extras into the copy where they sharpen it", () => {
    expect(reasonCopy("pre_close_checklist_blocked", { missing: ["Triage", "Impact"] })).toContain(
      "Triage",
    );
    expect(
      reasonCopy("dispatch_move_conflict", { blocker_title: "Router swap" }),
    ).toContain("Router swap");
    expect(reasonCopy("pre_close_checklist_blocked")).not.toContain("undefined");
  });
});

describe("error-copy — envelope extraction", () => {
  it("reads the envelope from openapi-fetch's parked error", () => {
    const env = envelopeOf({ error: { reason: "sku_taken", message: "x" } });
    expect(env.reason).toBe("sku_taken");
    expect(env.message).toBe("x");
  });

  it("reads a thrown Error carrying the envelope body", () => {
    const err = Object.assign(new Error("taken"), {
      body: envelope("org_slug_taken", { suggestion: "acme-2" }),
    });
    expect(envelopeOf(err).reason).toBe("org_slug_taken");
  });
});

describe("error-copy — friendlyMutationError", () => {
  it("uses the typed reason copy and keeps the reason for branching", () => {
    const result = friendlyMutationError(envelope("user_email_taken"));
    expect(result.reason).toBe("user_email_taken");
    expect(result.text).toContain("already exists");
    expect(result.transient).toBeUndefined();
  });

  it("classifies offline and timeout failures as transient", () => {
    const offline = new TypeError("Failed to fetch");
    expect(friendlyMutationError(offline).transient).toBe(true);
    const timeout = new DOMException("timed out", "TimeoutError");
    expect(friendlyMutationError(timeout).transient).toBe(true);
  });

  it("uses the queued register only when the write rode the durable queue", () => {
    const offline = new TypeError("Failed to fetch");
    expect(friendlyMutationError(offline).text).toBe(OFFLINE_RETRY_COPY);
    expect(friendlyMutationError(offline, { queued: true }).text).toBe(OFFLINE_QUEUED_COPY);
    const timeout = new DOMException("timed out", "TimeoutError");
    expect(friendlyMutationError(timeout).text).toBe(TIMEOUT_RETRY_COPY);
    expect(friendlyMutationError(timeout, { queued: true }).text).toBe(TIMEOUT_QUEUED_COPY);
  });

  it("never renders a thrown error's own message text", () => {
    const boom = new Error(
      'unexpected token } in JSON at position 42 {"stack":"at handler (mfa_handlers.rs:1)"}',
    );
    const text = friendlyMutationText(boom);
    expect(text).not.toContain("mfa_handlers");
    expect(text).not.toContain("unexpected token");
  });

  it("honors the caller's fallback when nothing typed is on the failure", () => {
    expect(friendlyMutationText(new Error("boom"), "Load failed")).toBe("Load failed");
  });

  it("falls back to the generic rejection line for an unmapped reason", () => {
    const result = friendlyMutationError(envelope("some_future_reason"));
    expect(result.text).toContain("couldn't be saved");
  });
});

describe("error-copy — rejectionText (WriteResult register)", () => {
  it("prefers the typed reason copy over the server message", () => {
    const text = rejectionText({
      reason: "version_conflict",
      message: "version conflict: entity moved from version 2 to 1",
    });
    expect(text).toContain("Refresh");
    expect(text).not.toContain("from version 2 to 1");
  });

  it("keeps read-model keyword branching possible via result.message", () => {
    const result = { reason: "conflict", message: "cost line already billed" };
    expect(result.message.includes("already billed")).toBe(true);
    expect(rejectionText(result)).not.toContain("already billed");
  });

  it("falls back to the caller's line when no reason and no message", () => {
    expect(rejectionText({}, "The tag was rejected.")).toBe("The tag was rejected.");
  });
});
