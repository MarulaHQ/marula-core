// Per-client workflow overrides tab (Wave 12 lane 5 backends, 0063).
//
// Purpose: the client's resolved workflow document, one entry per registry
// declaration with layer provenance (client override → org default →
// unset), render-ready detail names, and per-key actions: set (typed
// value), disable (explicit deny), clear (DELETE → inherit). Toggles are
// two buttons; reference kinds pick from their own lists (boards, board
// statuses, SLA policies). Rejections surface inline (role="alert").
//
// Non-purpose: no org-default editing here (org defaults ride the settings
// registry surfaces); the resolver stays the single source of truth.
"use client";

import { rejectionText } from "../../../lib/error-copy";
import {
  Button,
  Card,
  EmptyState,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  StatusTag,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useState, type ReactNode } from "react";

import { api } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";

type ClientWorkflowResolution = components["schemas"]["ClientWorkflowResolution"];
type OverrideEntry = components["schemas"]["OverrideEntry"];
type Board = components["schemas"]["Board"];
type BoardStatus = components["schemas"]["BoardStatus"];
type SlaPolicy = components["schemas"]["PolicyView"];

const SOURCE_LABEL: Record<string, string> = {
  client_override: "client override",
  org_default: "org default",
  unset: "unset",
};

function detailText(entry: OverrideEntry): string | null {
  const d = entry.detail;
  if (d === null || d === undefined) return null;
  if (d.kind === "toggle") return d.enabled ? "enabled" : "off";
  if (d.kind === "board") return `Board “${d.name}”`;
  if (d.kind === "status") return `Status “${d.name}” (${d.category})`;
  if (d.kind === "sla_policy") return `SLA policy “${d.name}”`;
  return "set";
}

function overrideRowText(entry: OverrideEntry): string | null {
  const row = entry.client_override;
  if (row === null || row === undefined) return null;
  const who = row.changed_by !== null && row.changed_by !== undefined ? row.changed_by.slice(0, 8) : "—";
  const when = new Date(row.updated_at).toLocaleString();
  return `set by ${who} · ${when}`;
}

export default function WorkflowOverridesTab({ clientId }: { clientId: string }): ReactNode {
  const [resolution, setResolution] = useState<ClientWorkflowResolution | null>(null);
  const [boards, setBoards] = useState<Board[] | null>(null);
  const [slaPolicies, setSlaPolicies] = useState<SlaPolicy[] | null>(null);
  const [statusesByBoard, setStatusesByBoard] = useState<Map<string, BoardStatus[]>>(new Map());
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [draftBoards, setDraftBoards] = useState<Map<string, string>>(new Map());

  const refresh = useCallback(async () => {
    const { data } = await api.api.GET("/api/v1/clients/{id}/workflow-overrides", {
      params: { path: { id: clientId } },
    });
    if (data !== undefined) setResolution(data);
    else setResolution({ client_id: clientId, entries: [] });
  }, [clientId]);

  useEffect(() => {
    void refresh();
    void api.api.GET("/api/v1/boards").then(({ data }) => setBoards(data ?? []));
    void api.api.GET("/api/v1/sla/policies").then(({ data }) => setSlaPolicies(data ?? []));
  }, [refresh]);

  const statusesForBoard = useCallback(
    (boardId: string): BoardStatus[] => {
      return statusesByBoard.get(boardId) ?? [];
    },
    [statusesByBoard],
  );


  const ensureStatuses = useCallback(
    (boardId: string): void => {
      if (boardId === "" || statusesByBoard.has(boardId)) return;
      void api.api.GET("/api/v1/boards/{id}/statuses", { params: { path: { id: boardId } } }).then(
        ({ data }) => setStatusesByBoard((m) => new Map(m).set(boardId, data ?? [])),
      );
    },
    [statusesByBoard],
  );

  async function put(key: string, state: "set" | "disabled", value?: boolean | string): Promise<void> {
    setBusyKey(key);
    setError(null);
    const result = await runEnvelopeMutation({
      describe: `Set override ${key}`,
      buildBody: (base) =>
        state === "set" ? { state, value, ...base } : { state, ...base },
      optimistic: () => {},
      revert: () => {},
      request: (client, body) =>
        client.PUT("/api/v1/clients/{id}/workflow-overrides/{key}", {
          params: { path: { id: clientId, key } },
          body: body as components["schemas"]["PutOverrideRequest"],
        }),
    });
    setBusyKey(null);
    if (result.ok === false) {
      setError(`${key}: ${rejectionText(result, "the server rejected this override.")}`);
      return;
    }
    await refresh();
  }

  async function clearKey(key: string): Promise<void> {
    setBusyKey(key);
    setError(null);
    const { error: delError } = await api.api.DELETE("/api/v1/clients/{id}/workflow-overrides/{key}", {
      params: { path: { id: clientId, key } },
    });
    setBusyKey(null);
    if (delError !== undefined) {
      setError(`${key}: the server rejected clearing this override.`);
      return;
    }
    await refresh();
  }

  if (resolution === null) {
    return (
      <div className="p-4">
        <Spinner />
      </div>
    );
  }

  return (
    <Card>
      <h2 className="p-4 pb-0 text-sm font-semibold">Workflow overrides</h2>
      <p className="px-4 pt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
        Per-client overrides on top of the org defaults. Provenance shows which
        layer supplied the effective value; clearing reverts the client to
        inheritance.
      </p>
      {error !== null && (
        <p className="px-4 pt-2 text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert" data-testid="overrides-error">
          {error}
        </p>
      )}
      <ul className="grid gap-2 p-4">
        {resolution.entries.map((entry) => (
          <li
            key={entry.key}
            className="grid gap-2 rounded-[3px] border p-3"
            style={{ borderColor: "var(--m-border)" }}
            data-testid={`override-${entry.key}`}
          >
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-sm font-medium">{entry.label}</span>
              <StatusTag
                tone={entry.source === "client_override" ? "success" : entry.source === "unset" ? "neutral" : "warning"}
              >
                {SOURCE_LABEL[entry.source] ?? entry.source}
              </StatusTag>
              {detailText(entry) !== null && (
                <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                  {detailText(entry)}
                </span>
              )}
            </div>
            <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
              {entry.description}
            </p>
            {overrideRowText(entry) !== null && (
              <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                {overrideRowText(entry)}
              </p>
            )}
            {entry.issues.length > 0 && (
              <p className="text-[11px]" style={{ color: "var(--m-warning)" }}>
                {entry.issues.join(" · ")}
              </p>
            )}
            <div className="flex flex-wrap items-center gap-2">
              {entry.value_kind === "toggle" && (
                <>
                  <Button
                    size="sm"
                    disabled={busyKey === entry.key}
                    onClick={() => void put(entry.key, "set", true)}
                    data-testid={`override-set-${entry.key}`}
                  >
                    Enable
                  </Button>
                  <Button size="sm" variant="secondary" disabled={busyKey === entry.key} onClick={() => void put(entry.key, "set", false)}>
                    Turn off
                  </Button>
                  {entry.allow_disable && (
                    <Button size="sm" variant="secondary" disabled={busyKey === entry.key} onClick={() => void put(entry.key, "disabled")}>
                      Deny for this client
                    </Button>
                  )}
                </>
              )}
              {entry.value_kind === "board_ref" && (
                <Select
                  value={
                    entry.detail !== null && entry.detail !== undefined && entry.detail.kind === "board"
                      ? entry.detail.board_id
                      : ""
                  }
                  onValueChange={(v) => void put(entry.key, "set", v)}
                >
                  <SelectTrigger data-testid={`override-board-${entry.key}`}>
                    <SelectValue placeholder="Pick a board…" />
                  </SelectTrigger>
                  <SelectContent>
                    {(boards ?? []).map((b) => (
                      <SelectItem key={b.id} value={b.id}>
                        {b.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              )}
              {entry.value_kind === "status_ref" && (
                <div className="flex flex-wrap items-center gap-2">
                  {(() => {
                    const resolved =
                      entry.detail !== null && entry.detail !== undefined && entry.detail.kind === "status"
                        ? entry.detail
                        : null;
                    const draftBoard = draftBoards.get(entry.key) ?? resolved?.board_id ?? "";
                    const draftStatus = resolved?.status_id ?? "";
                    return (
                      <>
                        <Select
                          value={draftBoard}
                          onValueChange={(v) => {
                            setDraftBoards((m) => new Map(m).set(entry.key, v));
                            ensureStatuses(v);
                          }}
                        >
                          <SelectTrigger data-testid={`override-boardfor-${entry.key}`}>
                            <SelectValue placeholder="Board…" />
                          </SelectTrigger>
                          <SelectContent>
                            {(boards ?? []).map((b) => (
                              <SelectItem key={b.id} value={b.id}>
                                {b.name}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                        <Select
                          value={draftStatus}
                          onValueChange={(v) => void put(entry.key, "set", v)}
                        >
                          <SelectTrigger data-testid={`override-status-${entry.key}`}>
                            <SelectValue placeholder={draftBoard === "" ? "Pick a board first…" : "Status…"} />
                          </SelectTrigger>
                          <SelectContent>
                            {statusesForBoard(draftBoard).map((s) => (
                              <SelectItem key={s.id} value={s.id}>
                                {s.name}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </>
                    );
                  })()}
                </div>
              )}
              {entry.value_kind === "sla_policy_ref" && (
                <Select
                  value={
                    entry.detail !== null && entry.detail !== undefined && entry.detail.kind === "sla_policy"
                      ? entry.detail.policy_id
                      : ""
                  }
                  onValueChange={(v) => void put(entry.key, "set", v)}
                >
                  <SelectTrigger data-testid={`override-sla-${entry.key}`}>
                    <SelectValue placeholder="Pick a policy…" />
                  </SelectTrigger>
                  <SelectContent>
                    {(slaPolicies ?? []).map((p) => (
                      <SelectItem key={p.id} value={p.id}>
                        {p.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              )}
              {entry.source === "client_override" && (
                <Button size="sm" variant="ghost" disabled={busyKey === entry.key} onClick={() => void clearKey(entry.key)} data-testid={`override-clear-${entry.key}`}>
                  Clear (inherit)
                </Button>
              )}
            </div>
          </li>
        ))}
        {resolution.entries.length === 0 && (
          <li>
            <EmptyState message="The workflow registry is empty for this org." />
          </li>
        )}
      </ul>
    </Card>
  );
}
