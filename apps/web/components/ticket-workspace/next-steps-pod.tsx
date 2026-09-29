// "Next steps" dedicated pod.
//
// Purpose: a dedicated per-ticket space for the tech's next steps, stored
// as a board-scoped TEXT custom field named "Next steps" (the API's
// purpose-built store for per-ticket structured text that must survive
// reloads, be visible to the whole team, and fold into final notes at
// close). When the board lacks the definition, an owner/admin gets a
// one-click "define it" (POST /boards/{id}/fields, board-admin gated);
// everyone else gets a typed "not available yet" state — never a dead
// editor. Value writes ride the existing custom-field value endpoint.
// Non-purpose: no taxonomy/board-admin UI beyond this one definition.

"use client";

import type { components } from "@marula-core/sdk";
import { Button, Spinner, Textarea } from "@marula-core/ui";
import { useEffect, useRef, useState, type ReactNode } from "react";

import { api } from "../../lib/api";

type TicketCustomFieldValue = components["schemas"]["TicketCustomFieldValue"];
type FieldDefView = components["schemas"]["FieldDefView"];

export const NEXT_STEPS_FIELD_NAME = "Next steps";

export interface NextStepsPodProps {
  boardId: string;
  values: TicketCustomFieldValue[];
  /** Whether the viewer holds board-admin (special:manage_boards). */
  canManageBoards: boolean;
  /** Applies a custom-field value through the page's instant-first mutate. */
  onSaveValue(fieldId: string, value: string): Promise<void>;
  onDefinitionCreated(): void;
}

type DefState =
  | { status: "loading" }
  | { status: "error" }
  | { status: "absent" }
  | { status: "ready"; fieldId: string };

export function NextStepsPod({
  boardId,
  values,
  canManageBoards,
  onSaveValue,
  onDefinitionCreated,
}: NextStepsPodProps): ReactNode {
  const [def, setDef] = useState<DefState>({ status: "loading" });
  const [draft, setDraft] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [creatingDef, setCreatingDef] = useState(false);
  const savedDraftRef = useRef<string | null>(null);

  const existing = values.find(
    (v) => v.name.toLowerCase() === NEXT_STEPS_FIELD_NAME.toLowerCase(),
  );

  useEffect(() => {
    if (existing !== undefined) {
      setDef({ status: "ready", fieldId: existing.field_id });
      return;
    }
    // Values carry only assigned fields; a defined-but-empty field won't
    // appear — probe the board's active definitions before saying "absent".
    let cancelled = false;
    setDef({ status: "loading" });
    api.api
      .GET("/api/v1/boards/{id}/fields", { params: { path: { id: boardId } } })
      .then(({ data, error }) => {
        if (cancelled) return;
        if (error !== undefined) {
          setDef({ status: "error" });
          return;
        }
        const match = (data ?? []).find(
          (f) => f.name.toLowerCase() === NEXT_STEPS_FIELD_NAME.toLowerCase(),
        );
        setDef(match !== undefined ? { status: "ready", fieldId: match.id } : { status: "absent" });
      })
      .catch(() => !cancelled && setDef({ status: "error" }));
    return () => {
      cancelled = true;
    };
  }, [boardId, existing]);

  // Draft tracks the stored value until the tech types.
  useEffect(() => {
    if (savedDraftRef.current !== null) return;
    setDraft(existing !== undefined && existing.value != null ? String(existing.value) : "");
  }, [existing]);

  async function createDefinition(): Promise<void> {
    setCreatingDef(true);
    const { data, error } = await api.api.POST("/api/v1/boards/{id}/fields", {
      params: { path: { id: boardId } },
      body: { name: NEXT_STEPS_FIELD_NAME, kind: "text", required: false },
    });
    setCreatingDef(false);
    if (error !== undefined || data === undefined) {
      setDef({ status: "error" });
      return;
    }
    onDefinitionCreated();
  }

  async function save(): Promise<void> {
    if (def.status !== "ready" || draft === null) return;
    setBusy(true);
    setError(null);
    try {
      await onSaveValue(def.fieldId, draft);
      savedDraftRef.current = null;
    } catch {
      setError("Save rejected");
    } finally {
      setBusy(false);
    }
  }

  return (
    <section
      className="grid content-start gap-2 rounded-[var(--m-radius-container)] border p-3"
      style={{ borderColor: "var(--m-border)" }}
      data-testid="pod-next-steps"
      aria-label="Next steps"
    >
      <h2 className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
        Next steps
      </h2>
      {def.status === "loading" && <Spinner />}
      {def.status === "error" && (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }}>
          Could not load the Next steps field for this board.
        </p>
      )}
      {def.status === "absent" && (
        <div className="grid gap-2">
          <p className="text-xs" style={{ color: "var(--m-text-muted)" }} data-testid="next-steps-gap">
            This board has no &ldquo;Next steps&rdquo; field yet.
          </p>
          {canManageBoards ? (
            <Button size="sm" disabled={creatingDef} onClick={() => void createDefinition()} data-testid="next-steps-define">
              {creatingDef ? "Creating…" : "Define the Next steps field on this board"}
            </Button>
          ) : (
            <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
              Ask a board admin to define it — the field cannot be created from this account.
            </p>
          )}
        </div>
      )}
      {def.status === "ready" && (
        <>
          <Textarea
            rows={3}
            value={draft ?? ""}
            onChange={(e) => setDraft(e.target.value)}
            placeholder="What happens after this ticket…"
            aria-label="Next steps"
            data-testid="next-steps-input"
          />
          <div className="flex justify-end gap-2">
            <Button size="sm" disabled={busy || draft === null} onClick={() => void save()} data-testid="next-steps-save">
              Save
            </Button>
          </div>
          <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            Merged into the final notes when the ticket is validated for close.
          </p>
        </>
      )}
      {error !== null && (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">{error}</p>
      )}
    </section>
  );
}
