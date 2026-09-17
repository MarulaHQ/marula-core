// /audit — the filterable audit-trail browser (Wave 11 lane 8; Wave 17
// lane 6 browser pass).
//
// Purpose: keyset-paged, filterable append-only ledger (actor, action
// prefix, date range) with expandable event rows — each row expands in
// place to the full structured detail (actor attribution, entity
// coordinates, IP, JSON payload) instead of a separate drawer.
//
// Non-purpose: no editing, no export (the audit ledger is append-only;
// exports are an operator concern).
"use client";

import {
  Button,
  Card,
  EmptyState,
  Input,
  Label,
  Spinner,
  Table,
  Tag,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { Fragment, useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";
import { rejectionText } from "../../lib/error-copy";

type AuditEvent = components["schemas"]["AuditEvent"];

const ACTION_PRESETS = ["auth", "secret", "plugin", "portal", "billing"] as const;

/** The serialized `Actor` union renders as `kind · short-id`, never raw JSON. */
function actorLabel(actor: AuditEvent["actor"] | null | undefined): string | null {
  if (actor === null || actor === undefined) return null;
  if (typeof actor !== "object") return String(actor);
  if (!("id" in actor) || typeof actor.id !== "string") return actor.kind;
  return `${actor.kind} ${actor.id.length > 64 ? actor.id.slice(0, 12) : actor.id}`;
}

function kindOfActor(actor: AuditEvent["actor"]): string {
  if (typeof actor === "object" && actor !== null && "kind" in actor) return String(actor.kind);
  return "other";
}

const KIND_TONE: Record<string, "success" | "neutral" | "warning" | "danger"> = {
  human: "neutral",
  agent: "success",
  system: "neutral",
  api_key: "warning",
};

export default function AuditPage() {
  const auth = useRequireAuth();

  const [events, setEvents] = useState<AuditEvent[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  // Filters
  const [actor, setActor] = useState("");
  const [actionPrefix, setActionPrefix] = useState("");
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");

  // Cursors
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [cursor, setCursor] = useState<string | null>(null);

  // Per-row expansion (event detail in place)
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const loadEvents = useCallback(
    async (targetCursor: string | null = null) => {
      setLoading(true);
      setErrorMsg(null);
      const { data, error } = await api.api.GET("/api/v1/audit-events", {
        params: {
          query: {
            actor: actor.trim() === "" ? undefined : actor.trim(),
            action: actionPrefix.trim() === "" ? undefined : actionPrefix.trim(),
            from: from === "" ? undefined : new Date(from).toISOString(),
            to: to === "" ? undefined : new Date(to).toISOString(),
            cursor: targetCursor ?? undefined,
            limit: 50,
          },
        },
      });
      setLoading(false);
      if (error !== undefined || data === undefined) {
        setErrorMsg(rejectionText(error as never, "Failed to load audit events."));
        return;
      }
      setEvents(data.events);
      setCursor(targetCursor);
      setNextCursor(data.next_cursor ?? null);
      setExpandedId(null);
    },
    [actor, actionPrefix, from, to],
  );

  useEffect(() => {
    if (auth.status === "authed") void loadEvents(null);
  }, [auth.status, loadEvents]);

  if (auth.status !== "authed") return null;

  return (
    <div className="mx-auto grid w-full max-w-[1100px] gap-6 p-4 md:p-6">
      <header>
        <h1 className="text-lg font-semibold">Audit trail</h1>
        <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Append-only ledger of security, administrative, and domain lifecycle events.
          Every row expands in place to its structured detail.
        </p>
      </header>

      <Card>
        <form
          className="grid gap-3 p-4"
          onSubmit={(e) => {
            e.preventDefault();
            void loadEvents(null);
          }}
        >
          <div className="flex flex-wrap items-end gap-3">
            <div className="w-48">
              <Label htmlFor="audit-action">Action prefix</Label>
              <Input
                id="audit-action"
                placeholder="e.g. auth, alert, portal"
                value={actionPrefix}
                onChange={(e) => setActionPrefix(e.target.value)}
              />
            </div>
            <div className="w-56">
              <Label htmlFor="audit-actor">Actor (UUID or prefix)</Label>
              <Input
                id="audit-actor"
                placeholder="user UUID · left blank = any"
                value={actor}
                onChange={(e) => setActor(e.target.value)}
              />
            </div>
            <div className="w-36">
              <Label htmlFor="audit-from">From</Label>
              <Input id="audit-from" type="date" value={from} onChange={(e) => setFrom(e.target.value)} />
            </div>
            <div className="w-36">
              <Label htmlFor="audit-to">To</Label>
              <Input id="audit-to" type="date" value={to} onChange={(e) => setTo(e.target.value)} />
            </div>
            <Button type="submit" variant="secondary">
              Apply filters
            </Button>
            <Button
              type="button"
              variant="ghost"
              onClick={() => {
                setActionPrefix("");
                setActor("");
                setFrom("");
                setTo("");
                void loadEvents(null);
              }}
            >
              Clear
            </Button>
          </div>
          <div className="flex flex-wrap gap-1" role="group" aria-label="Common action prefixes">
            {ACTION_PRESETS.map((p) => (
              <button
                key={p}
                type="button"
                className="rounded-full border px-2.5 py-0.5 text-[11px] transition-opacity duration-100 hover:opacity-80"
                style={{
                  borderColor: actionPrefix === p ? "var(--m-accent)" : "var(--m-border)",
                  background: actionPrefix === p ? "var(--m-accent)" : "transparent",
                  color: actionPrefix === p ? "var(--m-background)" : "var(--m-text-secondary)",
                }}
                aria-pressed={actionPrefix === p}
                onClick={() => {
                  const next = actionPrefix === p ? "" : p;
                  setActionPrefix(next);
                  void loadEvents(null);
                }}
              >
                {p}.*
              </button>
            ))}
          </div>
        </form>
      </Card>

      {errorMsg !== null ? (
        <EmptyState message={errorMsg} />
      ) : (
        <Card className="overflow-x-auto p-0">
          {loading ? (
            <div className="flex justify-center p-6">
              <Spinner />
            </div>
          ) : events === null ? (
            <div className="grid" />
          ) : events.length === 0 ? (
            <div className="p-6">
              <EmptyState message="No audit events matched these filters. Widen the window or clear a filter." />
            </div>
          ) : (
            <Table>
              <thead>
                <tr>
                  <Th>When</Th>
                  <Th>Event</Th>
                  <Th>Actor</Th>
                  <Th>Entity</Th>
                  <Th className="text-right">Detail</Th>
                </tr>
              </thead>
              <tbody>
                {events.map((ev) => {
                  const expanded = expandedId === ev.id;
                  return (
                    <Fragment key={ev.id}>
                      <Tr
                        className="cursor-pointer"
                        onClick={() => setExpandedId(expanded ? null : ev.id)}
                      >
                        <Td className="text-[12px] whitespace-nowrap" style={{ color: "var(--m-text-secondary)" }}>
                          {new Date(ev.occurred_at).toLocaleString()}
                        </Td>
                        <Td className="text-[12px]">
                          <Tag tone={KIND_TONE[kindOfActor(ev.actor)] ?? "neutral"}>
                            {kindOfActor(ev.actor)}
                          </Tag>
                        </Td>
                        <Td className="min-w-0 truncate text-[12px]">
                          {ev.action}
                        </Td>
                        <Td className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                          {ev.entity_type !== null && ev.entity_type !== undefined
                            ? `${ev.entity_type}${ev.entity_id ? ` · ${ev.entity_id.slice(0, 8)}` : ""}`
                            : "—"}
                        </Td>
                        <Td className="text-right text-[12px]" style={{ color: "var(--m-accent)" }}>
                          {expanded ? "Hide ↑" : "Inspect ↓"}
                        </Td>
                      </Tr>
                      {expanded ? (
                        <Tr>
                          <Td className="p-0" />
                          <Td colSpan={4} className="border-b p-3 align-top">
                            <div
                              className="grid gap-2 rounded-md border p-3 text-[12px]"
                              style={{ borderColor: "var(--m-border)" }}
                            >
                              <div className="grid gap-1 md:grid-cols-2">
                                <span>
                                  <span className="font-semibold">Event id:</span>{" "}
                                  <code className="font-mono">{ev.id}</code>
                                </span>
                                <span>
                                  <span className="font-semibold">Actor:</span>{" "}
                                  {actorLabel(ev.actor) ?? "—"}
                                </span>
                                <span>
                                  <span className="font-semibold">Entity:</span>{" "}
                                  {ev.entity_type ?? "—"} {ev.entity_id ?? ""}
                                </span>
                                <span>
                                  <span className="font-semibold">IP:</span>{" "}
                                  {ev.ip ?? "—"}
                                </span>
                              </div>
                              <div>
                                <span className="mb-1 block font-semibold">Payload</span>
                                <pre
                                  className="overflow-x-auto rounded-md p-3 font-mono text-[11px]"
                                  style={{ background: "var(--m-surface)", border: "1px solid var(--m-border)" }}
                                >
                                  {JSON.stringify(ev.detail, null, 2)}
                                </pre>
                              </div>
                            </div>
                          </Td>
                        </Tr>
                      ) : null}
                    </Fragment>
                  );
                })}
              </tbody>
            </Table>
          )}
          <div className="flex items-center justify-between p-4 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
            <Button
              variant="ghost"
              size="sm"
              disabled={cursor === null}
              onClick={() => void loadEvents(null)}
            >
              First page
            </Button>
            <span aria-live="polite">
              {events?.length ?? 0} loaded{nextCursor !== null ? " · more available" : ""}
            </span>
            <Button
              variant="secondary"
              size="sm"
              disabled={nextCursor === null}
              onClick={() => void loadEvents(nextCursor)}
            >
              Next page
            </Button>
          </div>
        </Card>
      )}
    </div>
  );
}
