// Custom fields, tags and the explicit link tree, on the v2 workbench.
//
// Purpose: three facts about a ticket that used to be reachable only by leaving
// the workbench for the classic one. Each panel is read-only and each fetches
// at most once, on mount, so opening the Details tab costs one tag call and
// opening Related costs one link call — not both on every ticket visit.
//
// Custom fields are the odd one out: `TicketWithFields.custom_fields` is
// already on the ticket payload and is SELF-DESCRIBING (it carries `name`,
// `kind` and `required`), so that panel needs no fetch at all. Rendering a
// field without its definition used to be the reason this could not be drawn;
// the value payload turned out to carry the label with it.
//
// Tone: these are facts about a ticket, so they read in the neutral surface
// voice. Nothing here takes a status pair, because nothing here is a status.
"use client";

import { Badge, EmptyState, Spinner, formatDayKey, formatNumber } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import "./ticket-fields.css";

type CustomFieldValue = components["schemas"]["TicketCustomFieldValue"];
type Tag = components["schemas"]["Tag"];
type LinksView = components["schemas"]["LinksView"];

/** The wire vocabulary from `FieldKindWire` (migration 0009's CHECK set). */
export type FieldKind =
  | "text"
  | "number"
  | "date"
  | "select"
  | "multiselect"
  | "bool"
  | "url"
  | "user";

/**
 * Render one custom-field value as a label a person reads.
 *
 * Every branch answers the same question — what does this field hold — and
 * none of them prints a raw JSON blob. `value` is `unknown` because the server
 * validates it against the definition rather than the schema, so an unfamiliar
 * payload has to degrade to something readable instead of throwing.
 */
export function fieldValueLabel(kind: string, value: unknown): string {
  if (value === null || value === undefined || value === "") return "—";
  switch (kind) {
    case "bool":
      return value === true || value === "true" ? "Yes" : "No";
    case "date": {
      // A date field holds a `YYYY-MM-DD` day, not an instant
      // (`crates/core-domain/src/custom_fields.rs` parses `%Y-%m-%d`). Reading
      // it as an instant put it at midnight UTC and printed the day before for
      // every reader west of Greenwich, so it is read in its own frame.
      const text = String(value);
      return /^\d{4}-\d{2}-\d{2}$/.test(text) ? formatDayKey(text, "date") : text;
    }
    case "number": {
      const num = typeof value === "number" ? value : Number(value);
      return Number.isNaN(num) ? String(value) : formatNumber(num);
    }
    case "multiselect":
      return Array.isArray(value)
        ? value.map((v) => (typeof v === "object" && v !== null ? String(labelOf(v)) : String(v))).join(", ")
        : String(value);
    case "select":
      return labelOf(value);
    case "user":
      return labelOf(value);
    default: {
      if (Array.isArray(value)) return value.map(String).join(", ");
      if (typeof value === "object") return labelOf(value);
      return String(value);
    }
  }
}

/** A select/user payload is usually `{ id, name }`; fall back to the id. */
function labelOf(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (typeof value !== "object") return String(value);
  const record = value as Record<string, unknown>;
  for (const key of ["name", "label", "display_name", "title", "number"]) {
    const found = record[key];
    if (typeof found === "string" && found !== "") return found;
  }
  const id = record["id"];
  return typeof id === "string" && id !== "" ? id : "—";
}

/** `parent_child` / `major_incident` read as words, not as column values. */
function linkTypeLabel(linkType: string): string {
  if (linkType === "major_incident") return "Major incident member";
  if (linkType === "parent_child") return "Parent / child";
  return linkType;
}

// --- custom fields ---------------------------------------------------------

export function CustomFieldsPanel({
  fields,
}: {
  fields: CustomFieldValue[] | undefined;
}): React.JSX.Element | null {
  const list = Array.isArray(fields) ? fields : [];
  if (list.length === 0) return null;
  const ordered = [...list].sort((a, b) => (a.sort_order ?? 0) - (b.sort_order ?? 0));
  return (
    <section data-testid="wb-custom-fields">
      <h3 className="m-panel-subhead">Custom fields</h3>
      <dl className="m-field-grid">
        {ordered.map((field) => (
          <div className="m-field" key={field.field_id}>
            <dt className="m-field-label">
              {field.name}
              {field.required ? (
                <span className="m-field-required" aria-label="required">
                  *
                </span>
              ) : null}
            </dt>
            <dd className="m-field-value">
              {field.kind === "url" && typeof field.value === "string" && field.value !== "" ? (
                <a
                  href={field.value}
                  rel="noreferrer noopener"
                  target="_blank"
                  className="underline"
                  style={{ color: "var(--m-gold-text)" }}
                >
                  {fieldValueLabel(field.kind, field.value)}
                </a>
              ) : (
                fieldValueLabel(field.kind, field.value)
              )}
            </dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

// --- tags ------------------------------------------------------------------

export function TagsPanel({ ticketId }: { ticketId: string }): React.JSX.Element | null {
  const [tags, setTags] = useState<Tag[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const res = await api.api.GET("/api/v1/tickets/{id}/tags", {
          params: { path: { id: ticketId } },
        });
        if (cancelled) return;
        setTags(res.data ?? []);
      } catch {
        if (!cancelled) setTags([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [ticketId]);

  // Nothing to draw while loading and nothing to draw after: a tags row that
  // appears then vanishes is worse than one that was never there.
  if (tags === null || tags.length === 0) return null;

  return (
    <section data-testid="wb-tags">
      <h3 className="m-panel-subhead">Tags</h3>
      <ul className="m-tag-row">
        {tags.map((tag) => (
          <li key={tag.id}>
            <Badge tone="neutral">{tag.name}</Badge>
          </li>
        ))}
      </ul>
    </section>
  );
}

// --- explicit link tree + rollup ------------------------------------------

export function TicketLinksPanel({
  ticketId,
}: {
  ticketId: string;
}): React.JSX.Element {
  const [view, setView] = useState<LinksView | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const res = await api.api.GET("/api/v1/tickets/{id}/links", {
          params: { path: { id: ticketId } },
        });
        if (cancelled) return;
        setView(res.data ?? null);
      } catch {
        if (!cancelled) setView(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [ticketId]);

  if (view === null) {
    return (
      <div data-testid="wb-links-loading">
        <Spinner label="Loading linked tickets" />
      </div>
    );
  }

  // The wire types these edges as opaque objects, so the read is defensive by
  // necessity: an edge whose endpoints are missing is skipped rather than
  // rendered as a link to nowhere.
  const from = (view.from ?? []) as unknown as LinkEdge[];
  const to = (view.to ?? []) as unknown as LinkEdge[];
  const rollup = view.rollup;
  const hasEdges = from.length > 0 || to.length > 0;

  return (
    <div className="grid content-start gap-3" data-testid="wb-links">
      {rollup ? (
        <section className="m-rollup" aria-label="Rollup across linked tickets">
          <Stat label="Children" value={String(rollup.child_count ?? 0)} />
          <Stat label="Open children" value={String(rollup.child_open_count ?? 0)} />
          {/* The rollup's billable total is deliberately NOT shown here: it is
              minor units with no currency on the payload, and guessing one
              would print a confident wrong number. Money lives on the Money
              tab, where the currency is known. */}
          <Stat label="Time folded in" value={formatMinutes(rollup.time_minutes ?? 0)} />
        </section>
      ) : null}

      {hasEdges ? (
        <>
          <EdgeGroup title="Linked to" edges={from} selfId={ticketId} />
          <EdgeGroup title="Linked from" edges={to} selfId={ticketId} />
        </>
      ) : (
        <EmptyState message="This ticket has no parent, children or incident members." />
      )}
    </div>
  );
}

interface LinkEdge {
  id?: string;
  from_ticket_id?: string;
  to_ticket_id?: string;
  link_type?: string;
}

function EdgeGroup({
  title,
  edges,
  selfId,
}: {
  title: string;
  edges: LinkEdge[];
  selfId: string;
}): React.JSX.Element | null {
  if (edges.length === 0) return null;
  return (
    <section>
      <h3 className="m-panel-subhead">{title}</h3>
      <ul className="m-edge-list">
        {edges.map((edge, i) => {
          const otherId = edge.from_ticket_id === selfId ? edge.to_ticket_id : edge.from_ticket_id;
          const label = linkTypeLabel(String(edge.link_type ?? ""));
          if (typeof otherId !== "string" || otherId === "") {
            return (
              <li className="m-edge" key={edge.id ?? i}>
                <span className="m-edge-label">{label}</span>
                <span className="m-edge-meta">endpoint not readable</span>
              </li>
            );
          }
          return (
            <li className="m-edge" key={edge.id ?? i}>
              <a className="m-edge-link" href={`/tickets/${otherId}`}>
                {shortId(otherId)}
              </a>
              <span className="m-edge-label">{label}</span>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function Stat({ label, value }: { label: string; value: string }): React.JSX.Element {
  return (
    <div className="m-rollup-stat">
 <span className="m-rollup-value tabular-nums">{value}</span>
      <span className="m-rollup-label">{label}</span>
    </div>
  );
}

/** A uuid is 36 characters and unreadable in a row; the head is enough to
 *  match it against a ticket the reader already has open. */
function shortId(id: string): string {
  return id.length > 8 ? `${id.slice(0, 8)}…` : id;
}

function formatMinutes(minutes: number): string {
  if (minutes <= 0) return "0m";
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  if (h === 0) return `${m}m`;
  return m === 0 ? `${h}h` : `${h}h ${m}m`;
}
