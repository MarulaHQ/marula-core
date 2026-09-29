// /settings/reply-snippets — the canned-response library (Wave 34 lane 3,
// `p34-canned-templates`).
//
// Purpose: staff write the replies; this page owns the library. Snippets
// are short reusable reply bodies the ticket composer inserts rendered
// against a ticket. Each snippet can be scoped to specific boards
// (empty = offered everywhere). Fork duplicates one into a new editable
// copy and records the origin (fork-on-edit provenance, templates
// pattern).
//
// Non-purpose: no per-ticket insertion here (surfaces carry the picker);
// ticket TEMPLATES stay their own library under their own surface.

"use client";

import { Button, Card, EmptyState, Input, Label, Spinner, Textarea } from "@marula-core/ui";
import { useCallback, useEffect, useState } from "react";

import { SettingsHeader } from "../../../components/settings/settings-shell";
import { api, useRequireAuth } from "../../../lib/api";
import { friendlyMutationText } from "../../../lib/error-copy";
import {
  createReplySnippet,
  deleteReplySnippet,
  forkReplySnippet,
  listReplySnippets,
  updateReplySnippet,
  type ReplySnippet,
} from "../../../lib/reply-snippets";

interface BoardOption {
  id: string;
  name: string;
}

export default function ReplySnippetsSettingsPage(): React.ReactNode {
  const auth = useRequireAuth();
  const [snippets, setSnippets] = useState<ReplySnippet[] | null>(null);
  const [boards, setBoards] = useState<BoardOption[]>([]);
  const [name, setName] = useState("");
  const [body, setBody] = useState("");
  const [boardIds, setBoardIds] = useState<string[]>([]);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const result = await listReplySnippets({});
      setSnippets(result.snippets);
    } catch (e) {
      setError(friendlyMutationText(e, "The snippet library could not be loaded."));
      setSnippets([]);
    }
  }, []);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refresh();
    api.api
      .GET("/api/v1/boards")
      .then(({ data }) => {
        const rows = (data as { boards?: BoardOption[] } | undefined)?.boards ?? [];
        setBoards(rows);
      })
      .catch(() => setBoards([]));
  }, [auth.status, refresh]);

  function resetForm() {
    setName("");
    setBody("");
    setBoardIds([]);
    setEditingId(null);
  }

  async function save() {
    if (saving) return;
    setSaving(true);
    setError(null);
    try {
      if (editingId !== null) {
        await updateReplySnippet(editingId, {
          name,
          body,
          board_ids: boardIds,
          idempotency_key: `patch-${crypto.randomUUID()}`,
        });
      } else {
        await createReplySnippet({ name, body, board_ids: boardIds, idempotency_key: `create-${crypto.randomUUID()}` });
      }
      resetForm();
      await refresh();
    } catch (e) {
      setError(friendlyMutationText(e, "The snippet could not be saved."));
    } finally {
      setSaving(false);
    }
  }

  async function fork(snippet: ReplySnippet) {
    setError(null);
    try {
      await forkReplySnippet(snippet.id, `${snippet.name} (copy)`);
      await refresh();
    } catch (e) {
      setError(friendlyMutationText(e, "The snippet could not be duplicated."));
    }
  }

  async function remove(snippet: ReplySnippet) {
    setError(null);
    try {
      await deleteReplySnippet(snippet.id);
      if (editingId === snippet.id) resetForm();
      await refresh();
    } catch (e) {
      setError(friendlyMutationText(e, "The snippet could not be removed."));
    }
  }

  function edit(snippet: ReplySnippet) {
    setEditingId(snippet.id);
    setName(snippet.name);
    setBody(snippet.body);
    setBoardIds(snippet.board_ids);
  }

  if (auth.status !== "authed") return null;

  return (
    <div>
      <SettingsHeader
        title="Reply snippets"
        description="Short canned replies your team inserts into ticket replies. Tokens like {{contact.name}} fill in from the ticket when a snippet is inserted."
      />
      {error !== null && (
        <p className="mb-3 text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </p>
      )}

      <Card className="mb-4 p-4">
        <h2 className="mb-3 text-sm font-semibold" style={{ color: "var(--m-text)" }}>
          {editingId !== null ? "Edit snippet" : "New snippet"}
        </h2>
        <div className="grid gap-3">
          <div className="grid gap-1">
            <Label htmlFor="snippet-name">Name</Label>
            <Input
              id="snippet-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Wifi triage"
              data-testid="snippet-name"
            />
          </div>
          <div className="grid gap-1">
            <Label htmlFor="snippet-body">Body</Label>
            <Textarea
              id="snippet-body"
              rows={4}
              value={body}
              onChange={(e) => setBody(e.target.value)}
              placeholder="Hi {{contact.name}}, we are looking at {{ticket.summary}} now."
              data-testid="snippet-body"
            />
            <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
              Tokens resolve from the ticket — contact, client, ticket, assignee, org.
            </p>
          </div>
          {boards.length > 0 && (
            <div className="grid gap-1">
              <Label>Offer on boards</Label>
              <div className="flex flex-wrap gap-2">
                {boards.map((b) => (
                  <label
                    key={b.id}
                    className="flex cursor-pointer items-center gap-1.5 rounded-[var(--m-radius-container)] border px-2 py-0.5 text-[11px]"
                    style={{
                      borderColor: boardIds.includes(b.id) ? "var(--m-accent)" : "var(--m-border)",
                      color: boardIds.includes(b.id) ? "var(--m-accent)" : "var(--m-text-muted)",
                    }}
                  >
                    <input
                      type="checkbox"
                      className="hidden"
                      checked={boardIds.includes(b.id)}
                      onChange={(e) =>
                        setBoardIds((prev) => (e.target.checked ? [...prev, b.id] : prev.filter((id) => id !== b.id)))
                      }
                    />
                    {b.name}
                  </label>
                ))}
              </div>
              <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                Leave empty to offer the snippet on every board.
              </p>
            </div>
          )}
          <div className="flex gap-2">
            <Button
              variant="primary"
              size="sm"
              disabled={saving || name.trim() === "" || body.trim() === ""}
              onClick={() => void save()}
              data-testid="snippet-save"
            >
              {editingId !== null ? "Save changes" : "Add snippet"}
            </Button>
            {editingId !== null && (
              <Button variant="secondary" size="sm" onClick={resetForm}>
                Cancel
              </Button>
            )}
          </div>
        </div>
      </Card>

      {snippets === null ? (
        <div className="flex justify-center py-10">
          <Spinner aria-label="Loading snippets" />
        </div>
      ) : snippets.length === 0 ? (
        <div className="text-sm">
          <EmptyState message="No snippets yet. Write your first canned reply above — the composer picks it up immediately." />
        </div>
      ) : (
        <div className="grid gap-2" data-testid="snippet-list">
          {snippets.map((s) => (
            <Card key={s.id} className="flex flex-wrap items-start justify-between gap-2 p-3">
              <div className="min-w-0">
                <p className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
                  {s.name}
                  {s.forked_from !== null && (
                    <span className="ml-2 text-[10px] font-normal" style={{ color: "var(--m-text-muted)" }}>
                      forked copy
                    </span>
                  )}
                </p>
                <p className="mt-0.5 whitespace-pre-wrap text-[11px]" style={{ color: "var(--m-text-secondary)" }}>
                  {s.body}
                </p>
                <p className="mt-1 text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                  {s.board_ids.length === 0
                    ? "Every board"
                    : `${s.board_ids.length} board${s.board_ids.length === 1 ? "" : "s"}`}
                </p>
              </div>
              <div className="flex shrink-0 gap-1.5">
                <Button variant="secondary" size="sm" onClick={() => edit(s)}>
                  Edit
                </Button>
                <Button variant="secondary" size="sm" onClick={() => void fork(s)}>
                  Fork
                </Button>
                <Button variant="secondary" size="sm" onClick={() => void remove(s)}>
                  Remove
                </Button>
              </div>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
