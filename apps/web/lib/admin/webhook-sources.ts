// Pure state logic for the inbound webhook-source surface
// (/settings/integrations — Wave 43 lane 5, spec item B).
//
// Purpose: classify a source between its two shipped profiles (generic
// M2M receiver per lane 1 vs the classic alert ingestor), build the
// public ingest URL, validate the optional custom `source_ref`, and shape
// per-source ingest-log rows off the two EXISTING ledger reads — the
// cursor-paged audit trail (`webhook.ingested` audit events for M2M
// sources) and the alerts ledger (alert-profile sources). Unit-testable
// pure functions only.
//
// Non-purpose: no network I/O; the HMAC signing posture lives server-side
// (webhook_receiver.rs); no route is invented here — when the platform
// grows a dedicated ingest-log endpoint this lib swaps its row source.
import type { components } from "@marula-core/sdk";

export type AlertSourceView = components["schemas"]["AlertSourceView"];
export type AuditEvent = components["schemas"]["AuditEvent"];
export type AlertView = components["schemas"]["AlertView"];

export type DeliveryStatusTone = "success" | "warning" | "danger" | "neutral";

/** The two shipped source profiles. `event_type` set ⇒ generic M2M. */
export type SourceProfileKind = "m2m" | "alert";

export interface SourceProfile {
  kind: SourceProfileKind;
  /** Path segment under which the source receives signed POSTs. */
  ingestPath: string;
  /** What an accepted POST does, in one line for the surface. */
  effect: string;
  /** Short badge label. */
  label: string;
}

export function sourceProfile(source: AlertSourceView): SourceProfile {
  if (source.event_type !== null && source.event_type !== undefined) {
    return {
      kind: "m2m",
      ingestPath: `/api/v1/webhooks/ingest/${source.source_ref}`,
      effect: "emits a signed envelope onto the domain-event stream",
      label: "webhook",
    };
  }
  return {
    kind: "alert",
    ingestPath: `/api/v1/alerts/ingest/${source.source_ref}`,
    effect: "creates an alert row and correlates storms into an incident ticket",
    label: "alerts",
  };
}

/** Absolute ingest URL rendered from the page origin. */
export function ingestUrl(origin: string, path: string): string {
  return `${origin.replace(/\/+$/, "")}${path}`;
}

/** Custom routing id the create form accepts (server: 3–64 of [a-z0-9-]). */
export const SOURCE_REF_RE = /^[a-z0-9][a-z0-9-]{2,63}$/;

export function sourceRefProblem(raw: string): string | null {
  if (raw.trim() === "") return null; // absent ⇒ server generates one
  if (!SOURCE_REF_RE.test(raw.trim())) {
    return "Use 3–64 lowercase letters, digits or dashes.";
  }
  return null;
}

/** Grammar of the per-source declared event type (mirrors the registry). */
export const EVENT_TYPE_RE = /^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$/;

export function eventTypeProblem(raw: string): string | null {
  const value = raw.trim();
  if (value === "") return null; // absent ⇒ classic alert profile
  if (!EVENT_TYPE_RE.test(value)) {
    return "Use dotted lowercase segments, e.g. rmm.device_offline.";
  }
  return null;
}

// --- ingest log rows -----------------------------------------------------------

export interface IngestLogRow {
  id: string;
  /** ISO timestamp of the ingest. */
  at: string;
  /** Provider's per-event identity — the replay fence key. */
  dedupKey: string | null;
  /** Entity the envelope carried, when the source set one. */
  entityId: string | null;
  tone: DeliveryStatusTone;
  label: string;
}

/**
 * M2M ingest rows from the audit trail: the receiver writes one
 * `webhook.ingested` audit event per ACCEPTED envelope (duplicates hit
 * the replay fence and write nothing), with `source_ref` and `dedup_key`
 * in the detail. Cursor pagination is the caller's; filtering by
 * `source_ref` is here.
 */
export function m2mIngestRows(events: AuditEvent[], sourceRef: string): IngestLogRow[] {
  return events
    .filter((event) => {
      const detail = event.detail as { source_ref?: unknown } | null;
      return (
        event.action === "webhook.ingested" &&
        detail !== null &&
        typeof detail === "object" &&
        detail.source_ref === sourceRef
      );
    })
    .map((event) => {
      const detail = event.detail as { dedup_key?: unknown };
      return {
        id: event.id,
        at: event.occurred_at,
        dedupKey: typeof detail.dedup_key === "string" ? detail.dedup_key : null,
        entityId: event.entity_id ?? null,
        tone: "neutral" as DeliveryStatusTone,
        label: "emitted",
      };
    });
}

/** Alert-profile ingest rows from the alerts ledger, newest first. */
export function alertIngestRows(alerts: AlertView[]): IngestLogRow[] {
  return alerts.map((alert) => ({
    id: alert.id,
    at: alert.received_at,
    dedupKey: alert.dedup_key,
    entityId: alert.incident_ticket_id ?? null,
    tone: alertTone(alert.status),
    label: `${alert.severity} · ${alert.status}`,
  }));
}

/**
 * One row of `GET /api/v1/alert-sources/{id}/ingest-log` (Wave 67 lane
 * `w67-read-surfaces`). The server already filtered the audit trail to
 * `webhook.ingested` events for this source, so the shape is the envelope's
 * identity and nothing else — no client-side scan, no cross-source rows.
 */
export interface IngestLogApiRow {
  id: string;
  occurred_at: string;
  dedup_key: string | null;
  entity_id: string | null;
}

/** M2M ingest rows from the source's own log endpoint, newest first. */
export function m2mIngestLogRows(rows: IngestLogApiRow[]): IngestLogRow[] {
  return rows.map((row) => ({
    id: row.id,
    at: row.occurred_at,
    dedupKey: row.dedup_key,
    entityId: row.entity_id,
    tone: "neutral" as DeliveryStatusTone,
    label: "emitted",
  }));
}

function alertTone(status: string): DeliveryStatusTone {
  if (status === "resolved") return "success";
  if (status === "acknowledged") return "warning";
  return "neutral";
}
