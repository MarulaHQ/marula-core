// MoveBoardDialog — move a ticket to another board.
//
// Purpose: pick a target board, optionally pin a status on it, and decide
// whether the SLA clocks re-initialize. The server remaps the status
// through the SEMANTIC categories by default; when the target board has no
// counterpart, it returns a typed 409 (`board_move_status_conflict`)
// carrying every target-board status — this dialog renders that list inline
// as a pick-list and retries with the explicit `status_id`, so the action is
// never a dead button. Mutations ride the instant-first envelope: the
// caller's `onMove` performs the optimistic render + authoritative
// reconcile, and the dialog only owns the form and the typed-conflict UI.
// Non-purpose: no board/status administration (that lives in Settings).
"use client";

import { rejectionText } from "../../lib/error-copy";
import { Button, Checkbox, Dialog, DialogActions, DialogContent, FieldHint, Label, Spinner, StatusTag } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { api } from "../../lib/api";

type Ticket = components["schemas"]["TicketWithFields"];
type Board = components["schemas"]["Board"];
type BoardStatus = components["schemas"]["BoardStatus"];
type TargetStatusOption = components["schemas"]["TargetStatusOption"];
type TicketStatusCategory = components["schemas"]["TicketStatusCategory"];

const CATEGORY_LABEL: Record<TicketStatusCategory, string> = {
  open: "Open",
  in_progress: "In Progress",
  waiting: "Waiting",
  closed: "Closed",
};

const CATEGORY_TONE: Record<TicketStatusCategory, "neutral" | "warning" | "success"> = {
  open: "neutral",
  in_progress: "neutral",
  waiting: "warning",
  closed: "success",
};

export interface MoveBoardInput {
  targetBoardId: string;
  /** Explicit target status pin; null = remap through the semantic category. */
  statusId: string | null;
  resetSla: boolean;
  /** Best client-side remap for the optimistic render, when one exists. */
  autoStatus: { id: string; category: TicketStatusCategory } | null;
  targetStatuses: BoardStatus[];
}

export interface MoveBoardOutcome {
  ok: boolean;
  reason?: string;
  message?: string;
  /** Present on a `board_move_status_conflict` — the target board's statuses. */
  conflictStatuses?: TargetStatusOption[];
}

export interface MoveBoardDialogProps {
  open: boolean;
  onOpenChange(open: boolean): void;
  ticket: Ticket;
  onMove(input: MoveBoardInput): Promise<MoveBoardOutcome>;
  onMoved(): void;
}

/** First target-board status sharing the ticket's category (server's rule). */
export function autoRemapStatus(
  statuses: BoardStatus[],
  category: TicketStatusCategory,
): BoardStatus | null {
  const candidates = statuses
    .filter((s) => s.category === category)
    .sort(
      (a, b) =>
        a.sort_order - b.sort_order || a.name.localeCompare(b.name) || a.id.localeCompare(b.id),
    );
  return candidates[0] ?? null;
}

/** Pulls the typed target-status options off a conflict rejection body. */
export function extractTargetStatuses(details: unknown): TargetStatusOption[] {
  const raw = (details as { target_statuses?: unknown } | null)?.target_statuses;
  if (!Array.isArray(raw)) return [];
  const out: TargetStatusOption[] = [];
  for (const item of raw) {
    const o = item as { id?: unknown; name?: unknown; category?: unknown };
    if (
      typeof o.id === "string" &&
      typeof o.name === "string" &&
      (o.category === "open" || o.category === "in_progress" || o.category === "waiting" || o.category === "closed")
    ) {
      out.push({ id: o.id, name: o.name, category: o.category });
    }
  }
  return out;
}

export function MoveBoardDialog({
  open,
  onOpenChange,
  ticket,
  onMove,
  onMoved,
}: MoveBoardDialogProps): ReactNode {
  const [boards, setBoards] = useState<Board[] | null>(null);
  const [boardsError, setBoardsError] = useState<string | null>(null);
  const [targetBoardId, setTargetBoardId] = useState("");
  const [targetStatuses, setTargetStatuses] = useState<BoardStatus[]>([]);
  const [statusesLoading, setStatusesLoading] = useState(false);
  const [statusId, setStatusId] = useState("");
  const [resetSla, setResetSla] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [conflict, setConflict] = useState<TargetStatusOption[] | null>(null);

  const loadBoards = useCallback(async () => {
    const { data, error: err } = await api.api.GET("/api/v1/boards");
    if (err !== undefined || data === undefined) {
      setBoardsError("The board list could not be loaded.");
      return;
    }
    setBoardsError(null);
    setBoards(data);
  }, []);

  // Fresh form on every open; fetch the board list once.
  useEffect(() => {
    if (!open) return;
    setTargetBoardId("");
    setTargetStatuses([]);
    setStatusId("");
    setResetSla(false);
    setError(null);
    setConflict(null);
    if (boards === null) void loadBoards();
  }, [open, boards, loadBoards]);

  // Board choice drives the target-status set and the remap preview.
  useEffect(() => {
    if (targetBoardId === "") {
      setTargetStatuses([]);
      return;
    }
    let cancelled = false;
    setStatusesLoading(true);
    setConflict(null);
    setError(null);
    void api.api
      .GET("/api/v1/boards/{id}/statuses", { params: { path: { id: targetBoardId } } })
      .then(({ data, error: err }) => {
        if (cancelled) return;
        setStatusesLoading(false);
        if (err !== undefined || data === undefined) {
          setError("The target board's statuses could not be loaded.");
          setTargetStatuses([]);
          return;
        }
        setTargetStatuses(data);
      })
      .catch(() => {
        if (!cancelled) {
          setStatusesLoading(false);
          setError("The target board's statuses could not be loaded.");
        }
      });
    return () => {
      cancelled = true;
    };
  }, [targetBoardId]);

  const targetBoard = (boards ?? []).find((b) => b.id === targetBoardId) ?? null;
  const auto = autoRemapStatus(targetStatuses, ticket.category);

  const submit = async () => {
    if (targetBoardId === "") return;
    setBusy(true);
    setError(null);
    const result = await onMove({
      targetBoardId,
      statusId: statusId === "" ? null : statusId,
      resetSla,
      autoStatus: auto === null ? null : { id: auto.id, category: auto.category },
      targetStatuses,
    });
    setBusy(false);
    if (result.ok) {
      onMoved();
      onOpenChange(false);
      return;
    }
    if (result.reason === "board_move_status_conflict" && (result.conflictStatuses?.length ?? 0) > 0) {
      setConflict(result.conflictStatuses ?? []);
      // Pre-select the first option so a second click is a real retry.
      setStatusId(result.conflictStatuses?.[0]?.id ?? "");
      setError(
        rejectionText(
          result,
          `This ticket is ${CATEGORY_LABEL[ticket.category]}, and the target board has no matching status. Choose one to move to.`,
        ),
      );
      return;
    }
    setError(rejectionText(result, "The move was rejected."));
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        title="Move to another board"
        description="The status remaps through the semantic category so behavior (SLA, workflow) survives the move."
      >
        <div className="grid gap-3 p-4">
          <div>
            <Label htmlFor="move-board-target">Target board</Label>
            {boardsError !== null ? (
              <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {boardsError}
              </p>
            ) : (
              <select
                id="move-board-target"
                aria-label="Target board"
                data-testid="move-board-target"
                value={targetBoardId}
                onChange={(e) => setTargetBoardId(e.target.value)}
                className="w-full text-[13px] px-2 py-1.5"
                style={{
                  borderRadius: "var(--m-radius-control)",
                  border: "1px solid var(--m-border)",
                  background: "var(--m-surface)",
                  color: "var(--m-text)",
                }}
              >
                <option value="" disabled>
                  {boards === null ? "Loading boards…" : "Choose a board…"}
                </option>
                {(boards ?? [])
                  .filter((b) => b.id !== ticket.board_id)
                  .map((b) => (
                    <option key={b.id} value={b.id}>
                      {b.name}
                      {b.is_default ? " (default)" : ""}
                    </option>
                  ))}
              </select>
            )}
            {boards !== null && boards.filter((b) => b.id !== ticket.board_id).length === 0 ? (
              <FieldHint>This is the only board in the organization.</FieldHint>
            ) : null}
          </div>

          {targetBoardId !== "" ? (
            <div>
              <Label htmlFor="move-board-status">Target status</Label>
              <select
                id="move-board-status"
                aria-label="Target status"
                data-testid="move-board-status"
                value={statusId}
                disabled={statusesLoading}
                onChange={(e) => setStatusId(e.target.value)}
                className="w-full text-[13px] px-2 py-1.5"
                style={{
                  borderRadius: "var(--m-radius-control)",
                  border: "1px solid var(--m-border)",
                  background: "var(--m-surface)",
                  color: "var(--m-text)",
                }}
              >
                <option value="">
                  {auto === null
                    ? "Auto — no matching category (you will be asked to choose)"
                    : `Auto — remap to ${auto.name} (${CATEGORY_LABEL[auto.category]})`}
                </option>
                {targetStatuses.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.name} ({CATEGORY_LABEL[s.category]})
                  </option>
                ))}
              </select>
              <FieldHint>
                Auto keeps the ticket&apos;s semantic category. Pinning a status is the retry path
                when the target board has no matching category.
              </FieldHint>
            </div>
          ) : statusesLoading ? (
            <Spinner />
          ) : null}

          {conflict !== null ? (
            <div
              data-testid="board-move-conflict"
              role="alert"
              className="grid gap-2 rounded-[var(--m-radius-container)] border px-3 py-2"
              style={{ borderColor: "var(--m-status-warning-fg)", background: "var(--m-status-warning-bg)" }}
            >
              <p className="text-[13px]" style={{ color: "var(--m-status-warning-fg)" }}>
                {error ??
                  "No status on the target board matches this ticket's semantic category. Choose one to move to."}
              </p>
              <div className="flex flex-wrap gap-1">
                {conflict.map((s) => (
                  <Button
                    key={s.id}
                    size="sm"
                    variant={statusId === s.id ? "secondary" : "ghost"}
                    data-testid={`board-move-conflict-${s.category}`}
                    onClick={() => setStatusId(s.id)}
                  >
                    <StatusTag tone={CATEGORY_TONE[s.category]}>{s.name}</StatusTag>
                  </Button>
                ))}
              </div>
            </div>
          ) : error !== null ? (
            <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {error}
            </p>
          ) : null}

          <label className="flex items-start gap-2 text-[13px]">
            <Checkbox
              aria-label="Reset SLA clocks on arrival"
              checked={resetSla}
              onCheckedChange={(v) => setResetSla(v === true)}
            />
            <span>
              Reset SLA clocks on arrival
              <FieldHint>
                Off (default): the existing clock state carries over. On: re-initialize under the
                org&apos;s default policy from the move moment.
              </FieldHint>
            </span>
          </label>

          <DialogActions>
            <Button variant="ghost" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button
              variant="primary"
              data-testid="confirm-move-board"
              onClick={() => void submit()}
              disabled={busy || targetBoardId === ""}
            >
              {busy ? <Spinner /> : null} Move ticket
            </Button>
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
  );
}
