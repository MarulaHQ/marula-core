// One-shot template actions (fork / restore) for the settings surfaces.
//
// Both ride the instant-first envelope (idempotency key, typed rejection,
// friendly copy) through `runEnvelopeMutation` — the same path the editor's
// autosave uses — so a double-click can never fork twice and a slug
// conflict comes back as copy, not a raw 409.

import { api } from "../api";
import { runEnvelopeMutation } from "../envelope-write";
import type { ForkTemplateBody, ForkedTemplate, RestoreTemplateBody, TemplateRow } from "./types";

export interface TemplateActionResult {
  ok: boolean;
  /** The new fork, on a successful fork. */
  template?: ForkedTemplate;
  /** The restored row, on a successful restore. */
  restored?: TemplateRow;
  friendly?: string;
}

/** Duplicates a template into a mutable org-owned copy. */
export async function forkTemplate(
  id: string,
  body: Omit<ForkTemplateBody, "idempotency_key">,
): Promise<TemplateActionResult> {
  // runEnvelopeMutation's return carries the verdict, not the payload —
  // onApplied is where the applied snapshot arrives (IF2 read-your-write).
  let forked: ForkedTemplate | undefined;
  const result = await runEnvelopeMutation<ForkTemplateBody, ForkedTemplate>({
    describe: "Duplicate template",
    buildBody: (base) => ({ ...body, idempotency_key: base.idempotency_key }),
    optimistic: () => {},
    revert: () => {},
    request: (client, payload) =>
      client
        .POST("/api/v1/templates/{id}/fork", {
          params: { path: { id } },
          // The generated ForkTemplateRequest shape is correct; the response
          // schema is not (see types.ts) — the cast is the collision cast.
          body: payload,
        })
        .then((r) => ({ ...r, data: r.data as unknown as ForkedTemplate })),
    onApplied: (response) => {
      forked = response;
    },
  });
  if (!result.ok) return { ok: false, friendly: result.friendly ?? result.message };
  return { ok: true, template: forked };
}

/** Resets a shipped default's content to the canonical copy (KB4). */
export async function restoreTemplate(
  row: TemplateRow,
): Promise<TemplateActionResult> {
  let restored: TemplateRow | undefined;
  const result = await runEnvelopeMutation<RestoreTemplateBody, TemplateRow>({
    describe: "Reset template to default",
    entityId: row.id,
    buildBody: (base) => ({ idempotency_key: base.idempotency_key }),
    optimistic: () => {},
    revert: () => {},
    request: (client, payload) =>
      client
        .POST("/api/v1/templates/{id}/restore", {
          params: { path: { id: row.id } },
          body: payload,
        })
        .then((r) => ({ ...r, data: r.data as unknown as TemplateRow })),
    onApplied: (response) => {
      restored = response;
    },
  });
  if (!result.ok) return { ok: false, friendly: result.friendly ?? result.message };
  return { ok: true, restored };
}
