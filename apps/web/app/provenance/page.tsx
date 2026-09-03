// /provenance — the provenance browser (Wave 9 lane 7 close-out, extends
// the 2.3 audit viewer patterns).
//
// Purpose: queryable who/what/field/before-after history over applied
// writes: filter by actor kind, record coordinates (entity/id/field),
// time window, and page through with the keyset cursor. Every row shows
// the before → after pair, the actor attribution, and the write-trail seq.
//
// Non-purpose: no editing — provenance is a read model; undo/revert
// actions live on the ticket surface.
"use client";

import {
  Button,
  Card,
  EmptyState,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
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

type ProvenancePage2 = components["schemas"]["ProvenancePage"];
type ProvenanceEvent = components["schemas"]["ProvenanceEvent"];

const ACTOR_KINDS = ["", "human", "agent", "import"] as const;

function valueCell(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

export default function ProvenancePage() {
  const auth = useRequireAuth();
  const [events, setEvents] = useState<ProvenanceEvent[] | null>(null);
  const [cursor, setCursor] = useState<string | null>(null);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [actorKind, setActorKind] = useState<string>("");
  const [entity, setEntity] = useState<string>("");
  const [field, setField] = useState<string>("");
  const [from, setFrom] = useState<string>("");
  const [to, setTo] = useState<string>("");

  const load = useCallback(
    async (opts: { cursor?: string | null }) => {
      setLoadError(null);
      const { data, error } = await api.api.GET("/api/v1/provenance", {
        params: {
          query: {
            actor_kind: actorKind === "" ? null : actorKind,
            entity: entity.trim() === "" ? null : entity.trim(),
            field: field.trim() === "" ? null : field.trim(),
            from: from === "" ? null : new Date(from).toISOString(),
            to: to === "" ? null : new Date(to).toISOString(),
            cursor: opts.cursor ?? null,
            limit: 50,
          },
        },
      });
      if (error !== undefined || data === undefined) {
        setLoadError("Failed to load provenance.");
        return;
      }
      const page = data as ProvenancePage2;
      setEvents(page.events);
      setCursor(opts.cursor ?? null);
      setNextCursor(page.next_cursor ?? null);
    },
    [actorKind, entity, field, from, to],
  );

  useEffect(() => {
    if (auth.status === "authed") void load({ cursor: null });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [auth.status]);

  if (auth.status !== "authed") return null;

  return (
    <div className="grid gap-6">
      <div>
        <h1 className="text-lg font-semibold">Provenance</h1>
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Who changed what, field by field, across every applied write —
          including agent sessions and imports. This is a query surface over
          the write trail; it never rewrites history.
        </p>
      </div>

      <Card>
        <form
          className="grid gap-3 p-4"
          onSubmit={(e) => {
            e.preventDefault();
            void load({ cursor: null });
          }}
        >
          <div className="flex flex-wrap items-end gap-3">
            <div>
              <Label htmlFor="pv-kind">Actor kind</Label>
              <Select value={actorKind} onValueChange={setActorKind}>
                <SelectTrigger id="pv-kind" className="w-36" aria-label="Actor kind">
                  <SelectValue placeholder="Any" />
                </SelectTrigger>
                <SelectContent>
                  {ACTOR_KINDS.map((k) => (
                    <SelectItem key={k} value={k}>
                      {k === "" ? "Any" : k}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div>
              <Label htmlFor="pv-entity">Entity</Label>
              <Input
                id="pv-entity"
                placeholder="ticket"
                value={entity}
                onChange={(e) => setEntity(e.target.value)}
              />
            </div>
            <div>
              <Label htmlFor="pv-field">Field</Label>
              <Input
                id="pv-field"
                placeholder="status"
                value={field}
                onChange={(e) => setField(e.target.value)}
              />
            </div>
            <div>
              <Label htmlFor="pv-from">From</Label>
              <Input
                id="pv-from"
                type="datetime-local"
                value={from}
                onChange={(e) => setFrom(e.target.value)}
              />
            </div>
            <div>
              <Label htmlFor="pv-to">To</Label>
              <Input
                id="pv-to"
                type="datetime-local"
                value={to}
                onChange={(e) => setTo(e.target.value)}
              />
            </div>
            <Button type="submit">Apply</Button>
          </div>
        </form>
      </Card>

      {loadError !== null ? (
        <p role="alert" className="text-[13px]">{loadError}</p>
      ) : events === null ? (
        <Spinner />
      ) : events.length === 0 ? (
        <Card>
          <div className="p-4">
            <EmptyState message="No provenance rows match these filters. Widen the window or clear a filter." />
          </div>
        </Card>
      ) : (
        <Card>
          <Table>
            <thead>
              <tr>
                <Th>When</Th>
                <Th>Actor</Th>
                <Th>Record</Th>
                <Th>Field</Th>
                <Th>Before</Th>
                <Th>After</Th>
                <Th className="text-right">Seq</Th>
              </tr>
            </thead>
            <tbody>
              {events.map((e) => (
                <Tr key={`${e.write_id ?? e.idempotency_key ?? e.occurred_at}-${e.seq}-${e.field}`}>
                  <Td>{new Date(e.occurred_at).toLocaleString()}</Td>
                  <Td>
                    <Tag>{e.actor_kind}</Tag>
                  </Td>
                  <Td>
                    {e.entity} {e.op}
                  </Td>
                  <Td>{e.field}</Td>
                  <Td className="max-w-52 truncate" title={valueCell(e.before)}>
                    {valueCell(e.before)}
                  </Td>
                  <Td className="max-w-52 truncate" title={valueCell(e.after)}>
                    {valueCell(e.after)}
                  </Td>
                  <Td className="text-right tabular-nums">{e.seq}</Td>
                </Tr>
              ))}
            </tbody>
          </Table>
          <div className="flex items-center justify-between p-4">
            <Button
              variant="ghost"
              size="sm"
              disabled={cursor === null}
              onClick={() => void load({ cursor: null })}
            >
              First page
            </Button>
            <Button
              variant="secondary"
              size="sm"
              disabled={nextCursor === null}
              onClick={() => void load({ cursor: nextCursor })}
            >
              Next page
            </Button>
          </div>
        </Card>
      )}
    </div>
  );
}
