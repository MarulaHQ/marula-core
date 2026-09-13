// /boards/[id] — board surface backing the left-nav tree links.
//
// Purpose: the board's status pipeline and its ticket queue rendered from
// the IF4 board read model: hydrate once from the snapshot, then live
// through board-channel WS events as per-ticket deltas (IF5 — reload the
// cache, not the page). Returning to a visited board renders from cache
// with no refetch and no loading flash.
// Non-purpose: no board administration (statuses/transitions/members are
// managed through the API's ConfigureBoard endpoints, not this page).
"use client";

import { rejectionText } from "../../../lib/error-copy";
import type { components } from "@meridian/sdk";
import {
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  StatusTag,
  Textarea,
} from "@meridian/ui";
import { useParams, useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";

import { BoardNav } from "../../../components/board-nav";
import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import {
  boardModelKey,
  makeBoardSpec,
  type BoardTicket,
} from "../../../lib/instant/read-models";
import {
  useInstantChannels,
  useInstantCollection,
} from "../../../lib/use-instant";

type BoardMemberView = components["schemas"]["BoardMemberView"];
type Priority = components["schemas"]["Priority"];

const PRIORITIES: Priority[] = ["p1", "p2", "p3", "p4"];

function categoryTone(category: string): "success" | "warning" | "neutral" {
  if (category === "closed") return "success";
  if (category === "waiting") return "warning";
  return "neutral";
}

/// New-ticket composer: the only staff create surface, wired to
/// POST /api/v1/tickets through the IF1/IF2 envelope (idempotency key,
/// read-your-write version). The board channel's `ticket.created` delta
/// lands the row in the list — no local insert needed.
function NewTicketDialog({
  boardId,
  canCreate,
}: {
  boardId: string;
  canCreate: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [priority, setPriority] = useState<Priority>("p3");
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!canCreate) return null;

  const reset = () => {
    setTitle("");
    setPriority("p3");
    setBody("");
    setError(null);
  };

  const create = async () => {
    if (title.trim() === "") return;
    setBusy(true);
    setError(null);
    const result = await runEnvelopeMutation<
      components["schemas"]["CreateTicketRequest"],
      components["schemas"]["TicketMutationResponse"]
    >({
      describe: "Create ticket",
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        board_id: boardId,
        title: title.trim(),
        priority,
        body: body.trim() === "" ? null : body,
      }),
      optimistic: () => {},
      revert: () => {},
      request: (client, requestBody) =>
        client.POST("/api/v1/tickets", { body: requestBody }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setBusy(false);
    if (!result.ok) {
      setError(rejectionText(result, "The ticket was rejected. Nothing was created."));
      return;
    }
    setOpen(false);
    reset();
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) reset();
      }}
    >
      <DialogTrigger asChild>
        <Button size="sm">New ticket</Button>
      </DialogTrigger>
      <DialogContent
        title="New ticket"
        description="Opens on this board's entry state with SLA clocks running."
      >
        <div className="grid gap-3 p-4">
          <div>
            <Label htmlFor="new-ticket-title">Title</Label>
            <Input
              id="new-ticket-title"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              autoFocus
            />
          </div>
          <div>
            <Label htmlFor="new-ticket-priority">Priority</Label>
            <Select
              value={priority}
              onValueChange={(v) => setPriority(v as Priority)}
            >
              <SelectTrigger aria-label="Priority" className="w-32">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {PRIORITIES.map((p) => (
                  <SelectItem key={p} value={p}>
                    {p.toUpperCase()}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div>
            <Label htmlFor="new-ticket-body">Description</Label>
            <Textarea
              id="new-ticket-body"
              rows={4}
              value={body}
              onChange={(e) => setBody(e.target.value)}
            />
            <FieldHint>Optional. Visible to clients on the portal.</FieldHint>
          </div>
          {error !== null ? (
            <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {error}
            </p>
          ) : null}
          <DialogActions>
            <Button variant="ghost" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button onClick={() => void create()} disabled={busy || title.trim() === ""}>
              {busy ? <Spinner /> : null} Create ticket
            </Button>
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
  );
}

const ROLES = ["viewer", "member", "admin"] as const;

/// Compact board-admin roster: existing members with inline role change.
/// Adding NEW members waits on an org user-directory endpoint (deferred;
/// see stage notes) — re-roling works fully today.
function MembersPanel({ boardId }: { boardId: string }) {
  const [members, setMembers] = useState<BoardMemberView[] | null>(null);
  const [hidden, setHidden] = useState(false);

  const refresh = useCallback(async () => {
    const res = await api.api.GET("/api/v1/boards/{id}/members", {
      params: { path: { id: boardId } },
    });
    if (res.response.status === 403) {
      setHidden(true);
      return;
    }
    if (res.error !== undefined) return;
    setMembers(res.data);
  }, [boardId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function changeRole(userId: string, role: string) {
    await api.api.PUT("/api/v1/boards/{id}/members", {
      params: { path: { id: boardId } },
      body: { user_id: userId, role: role as (typeof ROLES)[number] },
    });
    void refresh();
  }

  if (hidden || members === null) {
    return null;
  }
  return (
    <section aria-label="Board members" className="flex flex-col gap-1">
      <h2
        className="text-[10px] font-semibold uppercase tracking-wide"
        style={{ color: "var(--m-text-muted)" }}
      >
        Members ({members.length})
      </h2>
      {members.map((m) => (
        <div key={m.user_id} className="row-hover flex h-6 items-center gap-2 text-xs">
          <span className="truncate">{m.display_name}</span>
          <span className="ml-auto" />
          <select
            aria-label={`Role for ${m.display_name}`}
            className="rounded-[2px] border px-1 py-0 text-xs"
            style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
            value={m.role}
            onChange={(e) => void changeRole(m.user_id, e.target.value)}
          >
            {ROLES.map((r) => (
              <option key={r} value={r}>
                {r}
              </option>
            ))}
          </select>
        </div>
      ))}
    </section>
  );
}

export default function BoardPage() {
  const auth = useRequireAuth();
  const router = useRouter();
  const params = useParams<{ id: string }>();
  const boardId = typeof params?.id === "string" ? params.id : "";

  // IF4/IF5: hydrate-once board snapshot, deltas over the board channel.
  const boardSpec = useMemo(() => makeBoardSpec(api.api, boardId), [boardId]);
  const { data: board, loading } = useInstantCollection(boardModelKey(boardId), boardSpec);
  useInstantChannels(boardId === "" ? [] : [`board:${boardId}`]);

  if (auth.status !== "authed") {
    return (
      <main className="flex min-h-full items-center justify-center">
        <Spinner />
      </main>
    );
  }

  const statuses = board?.statuses ?? null;
  const tickets = board?.tickets ?? null;

  return (
    <div className="flex h-full flex-col">
      <div className="flex min-h-0 flex-1">
        <BoardNav />
        <main
          className="flex min-w-0 flex-1 flex-col gap-4 overflow-auto p-4"
          style={{ borderLeft: "1px solid var(--m-border)" }}
        >
          <div className="flex items-center gap-3">
            <h1 className="text-sm font-semibold">Board</h1>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => router.push(`/boards/${boardId}/fields`)}
            >
              Custom fields
            </Button>
            <span className="ml-auto" />
            <NewTicketDialog
              boardId={boardId}
              canCreate={auth.me?.permissions.includes("tickets:create") ?? false}
            />
          </div>

          <MembersPanel boardId={boardId} />

          {statuses !== null && (
            <div className="flex flex-wrap items-center gap-2">
              {[...statuses]
                .sort((a, b) => a.sort_order - b.sort_order)
                .map((s) => (
                  <StatusTag key={s.id} tone={categoryTone(s.category)}>
                    {s.name}
                  </StatusTag>
                ))}
            </div>
          )}

          {loading ? (
            <div className="flex flex-1 items-center justify-center">
              <Spinner />
            </div>
          ) : tickets !== null && tickets.length === 0 ? (
            <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              No tickets on this board yet.
            </p>
          ) : (
            <ol role="list" aria-label="Board tickets" className="flex flex-col">
              {(tickets ?? []).map((t: BoardTicket) => (
                <li key={t.id}>
                  <button
                    type="button"
                    onClick={() => router.push(`/tickets/${t.id}`)}
                    className="row-hover grid h-6 w-full grid-cols-[auto_1fr_auto_auto] items-center gap-2 text-left text-xs"
                  >
                    <span className="font-mono" style={{ color: "var(--m-text-muted)" }}>
                      {t.number}
                    </span>
                    <span className="truncate">{t.title}</span>
                    <StatusTag tone={categoryTone(t.category)}>{t.category}</StatusTag>
                    <span
                      className="tabular-nums"
                      style={{ color: "var(--m-text-secondary)" }}
                    >
                      {t.priority.toUpperCase()}
                    </span>
                  </button>
                </li>
              ))}
            </ol>
          )}
        </main>
      </div>
    </div>
  );
}
