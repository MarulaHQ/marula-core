// /audit — Filterable Audit Log Viewer (Wave 11 lane 8).
//
// Purpose:
//  - Filterable append-only audit trail: actor, action prefix, date range, keyset cursor pagination.
//  - Detail drawer / expandable view showing payload metadata & JSON details.
//
// Instant-first: cursor paging, inline errors, typed empty states,
// Workbench tokens, light+dark, <=120ms transitions.
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
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";

type AuditEvent = components["schemas"]["AuditEvent"];

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

  // Selected event for payload inspection
  const [selectedEvent, setSelectedEvent] = useState<AuditEvent | null>(null);

  const loadEvents = useCallback(
    async (targetCursor: string | null = null) => {
      setLoading(true);
      setErrorMsg(null);

      const { data, error } = await api.api.GET("/api/v1/audit-events", {
        params: {
          query: {
            actor: actor.trim() ? actor.trim() : undefined,
            action: actionPrefix.trim() ? actionPrefix.trim() : undefined,
            from: from ? new Date(from).toISOString() : undefined,
            to: to ? new Date(to).toISOString() : undefined,
            cursor: targetCursor ?? undefined,
            limit: 50,
          },
        },
      });

      setLoading(false);
      if (error !== undefined || !data) {
        setErrorMsg("Failed to load audit events.");
        return;
      }

      setEvents(data.events);
      setCursor(targetCursor);
      setNextCursor(data.next_cursor ?? null);
    },
    [actor, actionPrefix, from, to],
  );

  useEffect(() => {
    if (auth.status === "authed") {
      void loadEvents(null);
    }
  }, [auth.status, loadEvents]);

  const handleFilterSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    void loadEvents(null);
  };

  if (auth.status !== "authed") return null;

  return (
    <div className="grid gap-6 max-w-5xl mx-auto p-6">
      <header>
        <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
          Audit Trail
        </h1>
        <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Append-only ledger of security, administrative, and domain lifecycle events.
        </p>
      </header>

      {/* Filter Bar */}
      <Card>
        <form onSubmit={handleFilterSubmit} className="p-4 grid gap-3">
          <div className="flex flex-wrap items-end gap-3">
            <div className="w-48">
              <Label htmlFor="audit-action">Action Prefix</Label>
              <Input
                id="audit-action"
                placeholder="e.g. auth, alert, portal"
                value={actionPrefix}
                onChange={(e) => setActionPrefix(e.target.value)}
              />
            </div>

            <div className="w-48">
              <Label htmlFor="audit-actor">Actor (UUID or system)</Label>
              <Input
                id="audit-actor"
                placeholder="User UUID"
                value={actor}
                onChange={(e) => setActor(e.target.value)}
              />
            </div>

            <div className="w-40">
              <Label htmlFor="audit-from">From</Label>
              <Input
                id="audit-from"
                type="date"
                value={from}
                onChange={(e) => setFrom(e.target.value)}
              />
            </div>

            <div className="w-40">
              <Label htmlFor="audit-to">To</Label>
              <Input
                id="audit-to"
                type="date"
                value={to}
                onChange={(e) => setTo(e.target.value)}
              />
            </div>

            <Button type="submit" variant="secondary">
              Apply Filters
            </Button>
          </div>
        </form>
      </Card>

      {errorMsg && (
        <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {errorMsg}
        </p>
      )}

      {/* Selected event drawer / viewer */}
      {selectedEvent && (
        <Card className="p-4 border-l-4" style={{ borderLeftColor: "var(--m-accent)" }}>
          <div className="flex items-center justify-between mb-2">
            <div className="font-semibold text-sm">
              Event Detail: <code>{selectedEvent.action}</code>
            </div>
            <Button size="sm" variant="ghost" onClick={() => setSelectedEvent(null)}>
              Close
            </Button>
          </div>
          <div className="grid grid-cols-2 gap-2 text-[12px] mb-3" style={{ color: "var(--m-text-secondary)" }}>
            <div><strong>Event ID:</strong> {selectedEvent.id}</div>
            <div><strong>Occurred:</strong> {new Date(selectedEvent.occurred_at).toLocaleString()}</div>
            <div><strong>Entity:</strong> {selectedEvent.entity_type ?? "—"} ({selectedEvent.entity_id ?? "—"})</div>
            <div><strong>Actor:</strong> {typeof selectedEvent.actor === "object" ? JSON.stringify(selectedEvent.actor) : String(selectedEvent.actor)}</div>
          </div>
          <div className="text-[12px] font-semibold mb-1">Payload Detail:</div>
          <pre
            className="text-[11px] p-3 rounded overflow-x-auto"
            style={{
              background: "var(--m-surface)",
              border: "1px solid var(--m-border)",
            }}
          >
            {JSON.stringify(selectedEvent.detail, null, 2)}
          </pre>
        </Card>
      )}

      <Card className="overflow-x-auto">
        {loading ? (
          <div className="p-6 flex justify-center">
            <Spinner />
          </div>
        ) : events === null || events.length === 0 ? (
          <div className="p-6">
            <EmptyState message="No audit events matched your filters." />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Timestamp</Th>
                <Th>Action</Th>
                <Th>Actor</Th>
                <Th>Entity</Th>
                <Th className="text-right">Detail</Th>
              </tr>
            </thead>
            <tbody>
              {events.map((ev) => (
                <Tr key={ev.id}>
                  <Td className="text-[12px] whitespace-nowrap" style={{ color: "var(--m-text-secondary)" }}>
                    {new Date(ev.occurred_at).toLocaleString()}
                  </Td>
                  <Td className="font-medium text-[13px]">
                    <code>{ev.action}</code>
                  </Td>
                  <Td className="text-[12px]">
                    {typeof ev.actor === "object" ? JSON.stringify(ev.actor) : String(ev.actor)}
                  </Td>
                  <Td className="text-[12px]">
                    {ev.entity_type ? (
                      <Tag tone="neutral">{ev.entity_type}</Tag>
                    ) : (
                      <span style={{ color: "var(--m-text-muted)" }}>—</span>
                    )}
                  </Td>
                  <Td className="text-right">
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => setSelectedEvent(ev)}
                    >
                      Inspect
                    </Button>
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </Card>

      {/* Keyset Cursor Pagination */}
      <div className="flex justify-between items-center text-[12px]">
        <Button
          variant="secondary"
          size="sm"
          disabled={!cursor}
          onClick={() => void loadEvents(null)}
        >
          Reset to Start
        </Button>

        {nextCursor && (
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void loadEvents(nextCursor)}
          >
            Next Page →
          </Button>
        )}
      </div>
    </div>
  );
}
