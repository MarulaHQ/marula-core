// TicketUndo — the IF7 undo surface on the ticket workspace (Wave 9 lane
// 5 close-out).
//
// Purpose: loads the record's undo stack (GET /tickets/{id}/undo-stack),
// binds Ctrl/Cmd+Z to step the record back one applied write
// (POST /tickets/{id}/revert with the target seq and the record's current
// version), and respects the record's `mode`: `open` offers near-total
// whole-record undo; `reverts_only` (closed records) offers explicit
// per-field reverts only. The revert is a normal authorized write — it is
// audited and never rewrites the trail.
//
// Non-purpose: no redo stack (the server's revert is itself a new write);
// no undo for other entities in v1.
"use client";

import { Button, Label, Spinner, StatusTag, Tag } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { api } from "../lib/api";
import { newIdempotencyKey } from "../lib/instant/instant-write";

type UndoStackResponse = components["schemas"]["UndoStackResponse"];
type UndoStep = components["schemas"]["UndoStep"];
type TicketRevertResponse = components["schemas"]["TicketRevertResponse"];

const UNDO_STACK_PATH = "/api/v1/tickets/{id}/undo-stack";
const REVERT_PATH = "/api/v1/tickets/{id}/revert";

function describeStep(step: UndoStep): string {
  const when = new Date(step.created_at).toLocaleString();
  const who = step.actor_kind === "human" ? "you" : step.actor_kind;
  return `${step.op} by ${who} · ${when}`;
}

export function TicketUndo({
  ticketId,
  onChanged,
}: {
  ticketId: string;
  /** Called after a successful revert so the detail model re-snapshots. */
  onChanged: () => void;
}): ReactNode {
  const [stack, setStack] = useState<UndoStackResponse | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [selectedFields, setSelectedFields] = useState<Set<string>>(new Set());
  const [expanded, setExpanded] = useState(false);

  const refresh = useCallback(async () => {
    const { data, error } = await api.api.GET(UNDO_STACK_PATH, {
      params: { path: { id: ticketId } },
    });
    if (error !== undefined || data === undefined) {
      setLoadError("Undo history is not available for this ticket yet.");
      return;
    }
    setStack(data);
    setLoadError(null);
  }, [ticketId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const revert = useCallback(
    async (seq: number, fields?: string[]) => {
      setBusy(true);
      setMessage(null);
      const { data, error } = await api.api.POST(REVERT_PATH, {
        params: { path: { id: ticketId } },
        body: {
          idempotency_key: newIdempotencyKey(),
          version: stack?.current_version,
          seq,
          fields: fields === undefined || fields.length === 0 ? null : fields,
        },
      });
      setBusy(false);
      if (error !== undefined || data === undefined) {
        const detail = (error as { error?: { message?: string; code?: string } } | undefined)?.error;
        setMessage(
          detail?.code === "undo_reverts_only"
            ? "This record is closed — step back individual fields instead."
            : (detail?.message ?? "The revert was rejected."),
        );
        return;
      }
      const response = data as TicketRevertResponse;
      setMessage(
        response.revert.whole_record
          ? `Stepped back write #${response.revert.reverted_seq}.`
          : `Restored ${response.revert.fields.join(", ")}.`,
      );
      setSelectedFields(new Set());
      await refresh();
      onChanged();
    },
    [ticketId, stack, refresh, onChanged],
  );

  // Ctrl/Cmd+Z: step back the newest restorable whole-record write while
  // the record is open. Native input undo wins when focus is in a field.
  useEffect(() => {
    if (stack === null || stack.mode !== "open") return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "z") return;
      const target = event.target as HTMLElement | null;
      if (
        target !== null &&
        (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)
      ) {
        return;
      }
      const newest = stack.steps.find((s) => s.restorable);
      if (newest === undefined) return;
      event.preventDefault();
      void revert(newest.seq);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [stack, revert]);

  if (loadError !== null) return null;

  if (stack === null) return <Spinner />;

  const newestRestorable = stack.steps.find((s) => s.restorable) ?? null;
  const changedFields = Array.from(
    new Set(stack.fields.filter((f) => f.changed).map((f) => f.field)),
  );

  return (
    <section className="grid gap-2" aria-label="Undo history">
      <div className="flex items-center gap-2">
        <Label>Undo</Label>
        <Tag>{stack.mode === "open" ? "open record" : "closed · reverts only"}</Tag>
      </div>

      {stack.steps.length === 0 ? (
        <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
          No applied writes on this record yet.
        </p>
      ) : stack.mode === "open" ? (
        <>
          <Button
            size="sm"
            disabled={busy || newestRestorable === null}
            onClick={() => newestRestorable !== null && void revert(newestRestorable.seq)}
            title="Ctrl/Cmd+Z"
          >
            {busy ? <Spinner /> : null}
            {newestRestorable !== null
              ? `Undo: ${describeStep(newestRestorable)}`
              : "Nothing left to undo"}
          </Button>
        </>
      ) : (
        <>
          <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
            This record is closed. Pick the fields to step back to their values
            at a chosen write — whole-record undo is not available.
          </p>
          {changedFields.length === 0 ? (
            <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              No field changes to revert.
            </p>
          ) : (
            <div className="flex flex-wrap gap-2">
              {changedFields.map((f) => (
                <label key={f} className="flex items-center gap-1 text-xs">
                  <input
                    type="checkbox"
                    checked={selectedFields.has(f)}
                    onChange={(e) => {
                      setSelectedFields((prev) => {
                        const next = new Set(prev);
                        if (e.target.checked) next.add(f);
                        else next.delete(f);
                        return next;
                      });
                    }}
                  />
                  {f}
                </label>
              ))}
            </div>
          )}
          <Button
            size="sm"
            variant="secondary"
            disabled={busy || newestRestorable === null || selectedFields.size === 0}
            onClick={() =>
              newestRestorable !== null && void revert(newestRestorable.seq, [...selectedFields])
            }
          >
            {busy ? <Spinner /> : null} Revert selected fields
          </Button>
        </>
      )}

      {stack.steps.length > 0 ? (
        <>
          <Button variant="ghost" size="sm" onClick={() => setExpanded((v) => !v)} aria-expanded={expanded}>
            {expanded ? "Hide history" : `Show history (${stack.steps.length})`}
          </Button>
          {expanded ? (
            <ol className="grid gap-1 text-xs" style={{ color: "var(--m-text-secondary)" }}>
              {stack.steps.map((s) => (
                <li key={s.write_id} className="flex items-center gap-2">
                  <StatusTag tone={s.restorable ? "neutral" : "warning"}>{`#${s.seq}`}</StatusTag>
                  <span>{describeStep(s)}</span>
                  {!s.restorable ? <Tag>not value-restorable</Tag> : null}
                </li>
              ))}
            </ol>
          ) : null}
        </>
      ) : null}

      {message !== null ? (
        <p role="status" className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
          {message}
        </p>
      ) : null}
    </section>
  );
}
