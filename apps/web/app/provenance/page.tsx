// /provenance — the provenance browser (Wave 9 lane 7 close-out; Wave 17
// lane 6 browser pass).
//
// Purpose: queryable who/what/field/before-after history over applied
// writes: actor kind facets as taps, record coordinates (entity/id/field),
// time window, keyset cursor paging. Each row renders the before → after
// pair as mono diff cells; ticket rows link straight into the record.
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
  Spinner,
  Table,
  Tag,
  Td,
  Th,
  Tr,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";
import { rejectionText } from "../../lib/error-copy";

type ProvenanceEvent = components["schemas"]["ProvenanceEvent"];

const ACTOR_KINDS = ["", "human", "agent", "import"] as const;

const ACTOR_TONE: Record<string, "success" | "neutral" | "warning" | "danger"> = {
  human: "neutral",
  agent: "success",
  import: "warning",
};

/** Values render as readable short text with the full value on title. */
function valueCell(value: unknown): string {
  if (value === null || value === undefined) return "∅";
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

function entityHref(e: ProvenanceEvent): string | null {
  if (e.entity === "ticket") return `/tickets/${e.entity_id}`;
  return null;
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
        setLoadError(rejectionText(error as never, "Failed to load provenance."));
        return;
      }
      setEvents(data.events);
      setCursor(opts.cursor ?? null);
      setNextCursor(data.next_cursor ?? null);
    },
    [actorKind, entity, field, from, to],
  );

  useEffect(() => {
    if (auth.status === "authed") void load({ cursor: null });
  }, [auth.status, actorKind, entity, field, from, to, load]);

  if (auth.status !== "authed") return null;

  const grouped = (events ?? []).reduce<Record<string, number>>((acc, e) => {
    acc[e.actor_kind] = (acc[e.actor_kind] ?? 0) + 1;
    return acc;
  }, {});

  return (
    <div className="mx-auto grid w-full max-w-[1300px] gap-6 p-4 md:p-6">
      <div>
        <h1 className="text-lg font-semibold">Provenance</h1>
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Who changed what, field by field, across every applied write —
          including agent sessions and imports. A query surface over the write
          trail; it never rewrites history.
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
              <Label htmlFor="pv-entity">Entity</Label>
              <Input id="pv-entity" placeholder="ticket" value={entity} onChange={(e) => setEntity(e.target.value)} />
            </div>
            <div>
              <Label htmlFor="pv-field">Field</Label>
              <Input id="pv-field" placeholder="status" value={field} onChange={(e) => setField(e.target.value)} />
            </div>
            <div>
              <Label htmlFor="pv-from">From</Label>
              <Input id="pv-from" type="datetime-local" value={from} onChange={(e) => setFrom(e.target.value)} />
            </div>
            <div>
              <Label htmlFor="pv-to">To</Label>
              <Input id="pv-to" type="datetime-local" value={to} onChange={(e) => setTo(e.target.value)} />
            </div>
            <Button type="submit">Apply</Button>
            <Button
              type="button"
              variant="ghost"
              onClick={() => {
                setActorKind("");
                setEntity("");
                setField("");
                setFrom("");
                setTo("");
              }}
            >
              Clear
            </Button>
          </div>
          <div className="flex flex-wrap gap-1" role="group" aria-label="Actor kind facets">
            {ACTOR_KINDS.map((k) => (
              <button
                key={k}
                type="button"
                className="rounded-full border px-2.5 py-0.5 text-[11px] transition-opacity duration-100 hover:opacity-80"
                style={{
                  borderColor: actorKind === k ? "var(--m-accent)" : "var(--m-border)",
                  background: actorKind === k ? "var(--m-accent)" : "transparent",
                  color: actorKind === k ? "var(--m-background)" : "var(--m-text-secondary)",
                }}
                aria-pressed={actorKind === k}
                onClick={() => setActorKind(k)}
              >
                {k === "" ? "any actor" : k}
                {k !== "" && grouped[k] !== undefined ? (
                  <span className="ml-1 tabular-nums" style={{ opacity: 0.8 }}>
                    {grouped[k]}
                  </span>
                ) : null}
              </button>
            ))}
          </div>
        </form>
      </Card>

      {loadError !== null ? (
        <p role="alert" className="text-[13px]">{loadError}</p>
      ) : events === null ? (
        <div className="flex justify-center py-10"><Spinner /></div>
      ) : events.length === 0 ? (
        <Card>
          <div className="p-4">
            <EmptyState message="No provenance rows match these filters. Widen the window or clear a filter." />
          </div>
        </Card>
      ) : (
        <Card className="overflow-x-auto p-0">
          <Table>
            <thead>
              <tr>
                <Th>When</Th>
                <Th>Actor</Th>
                <Th>Record</Th>
                <Th>Field</Th>
                <Th>Before → After</Th>
                <Th className="text-right">Seq</Th>
              </tr>
            </thead>
            <tbody>
              {events.map((e) => {
                const href = entityHref(e);
                return (
                  <Tr key={`${e.write_id ?? e.idempotency_key ?? e.occurred_at}-${e.seq}-${e.field}`}>
                    <Td className="whitespace-nowrap text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                      {new Date(e.occurred_at).toLocaleString()}
                    </Td>
                    <Td>
                      <Tag tone={ACTOR_TONE[e.actor_kind] ?? "neutral"}>{e.actor_kind}</Tag>
                    </Td>
                    <Td className="text-[12px]">
                      {href !== null ? (
                        <Link
                          href={href}
                          className="underline decoration-[var(--m-border)] underline-offset-2 hover:text-[var(--m-accent)]"
                        >
                          {e.entity} {e.op} · {e.entity_id.slice(0, 8)}
                        </Link>
                      ) : (
                        <span>
                          {e.entity} {e.op} · {e.entity_id.slice(0, 8)}
                        </span>
                      )}
                    </Td>
                    <Td className="font-mono text-[12px]">{e.field}</Td>
                    <Td>
                      <span className="flex min-w-0 items-center gap-2 font-mono text-[11px]">
                        <span
                          className="max-w-56 truncate rounded px-1.5 py-0.5"
                          style={{ background: "color-mix(in srgb, var(--m-status-danger-fg) 8%, transparent)" }}
                          title={`before: ${valueCell(e.before)}`}
                        >
                          {valueCell(e.before)}
                        </span>
                        <span aria-hidden="true" style={{ color: "var(--m-text-muted)" }}>→</span>
                        <span
                          className="max-w-56 truncate rounded px-1.5 py-0.5"
                          style={{ background: "color-mix(in srgb, var(--m-status-success-fg) 10%, transparent)" }}
                          title={`after: ${valueCell(e.after)}`}
                        >
                          {valueCell(e.after)}
                        </span>
                      </span>
                    </Td>
                    <Td className="text-right font-mono tabular-nums text-[11px]">{e.seq}</Td>
                  </Tr>
                );
              })}
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
