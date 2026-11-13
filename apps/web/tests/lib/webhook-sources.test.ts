// Unit tests: /lib/admin/webhook-sources — the inbound source surface's
// pure logic: profile classification, ingest-URL building, source_ref and
// event-type grammar checks, and the two ingest-log row shapers (audit
// trail for generic receivers, alerts ledger for the alert profile).
import { describe, expect, it } from "vitest";
import {
  alertIngestRows,
  eventTypeProblem,
  ingestUrl,
  m2mIngestLogRows,
  sourceProfile,
  sourceRefProblem,
} from "../../lib/admin/webhook-sources";
import type { components } from "@marula-core/sdk";

type AlertSourceView = components["schemas"]["AlertSourceView"];
type AlertView = components["schemas"]["AlertView"];

const source = (patch: Partial<AlertSourceView> = {}): AlertSourceView => ({
  id: "70000000-0000-4000-8000-000000000001",
  org_id: "org-1",
  name: "RMM production",
  source_ref: "rmm-prod",
  board_id: "board-1",
  client_id: null,
  correlation_window_secs: 900,
  enabled: true,
  event_type: null,
  payload_schema_ref: null,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  ...patch,
});


const alert = (patch: Partial<AlertView> = {}): AlertView => ({
  id: "al-1",
  org_id: "org-1",
  source_id: "70000000-0000-4000-8000-000000000001",
  title: "Disk almost full",
  description: null,
  severity: "warning",
  status: "open",
  dedup_key: "prov-123",
  correlation_key: "k-1",
  incident_ticket_id: null,
  received_at: "2026-01-02T03:04:05Z",
  occurred_at: "2026-01-02T03:04:05Z",
  acked_at: null,
  acked_by: null,
  resolved_at: null,
  resolved_by: null,
  attributes: {},
  ...patch,
});

describe("sourceProfile — the two shipped profiles", () => {
  it("a declared event type selects the generic receiver path", () => {
    const profile = sourceProfile(source({ event_type: "rmm.device_offline" }));
    expect(profile.kind).toBe("m2m");
    expect(profile.ingestPath).toBe("/api/v1/webhooks/ingest/rmm-prod");
    expect(profile.label).toBe("webhook");
  });

  it("no event type keeps the classic alert ingestor path", () => {
    const profile = sourceProfile(source());
    expect(profile.kind).toBe("alert");
    expect(profile.ingestPath).toBe("/api/v1/alerts/ingest/rmm-prod");
    expect(profile.label).toBe("alerts");
  });
});

describe("ingestUrl", () => {
  it("joins the origin with the path and tolerates trailing slashes", () => {
    expect(ingestUrl("https://marula.example.test/", "/api/v1/webhooks/ingest/x")).toBe(
      "https://marula.example.test/api/v1/webhooks/ingest/x",
    );
    expect(ingestUrl("https://marula.example.test", "/api/v1/alerts/ingest/x")).toBe(
      "https://marula.example.test/api/v1/alerts/ingest/x",
    );
  });
});

describe("form grammars", () => {
  it("source ref is optional and 3–64 of [a-z0-9-]", () => {
    expect(sourceRefProblem("")).toBeNull();
    expect(sourceRefProblem("rmm-prod-01")).toBeNull();
    expect(sourceRefProblem("ab")).toMatch(/3–64/);
    expect(sourceRefProblem("RMM_Prod")).toMatch(/3–64/);
  });

  it("event type is optional and dotted-lowercase", () => {
    expect(eventTypeProblem("")).toBeNull();
    expect(eventTypeProblem("rmm.device_offline")).toBeNull();
    expect(eventTypeProblem("RMM.DeviceOffline")).toMatch(/dotted/);
    expect(eventTypeProblem("rmm")).toMatch(/dotted/);
    expect(eventTypeProblem("rmm..offline")).toMatch(/dotted/);
  });
});

describe("alertIngestRows — the alerts-ledger reader", () => {
  it("tones rows by status and carries the incident ticket when correlated", () => {
    const rows = alertIngestRows([
      alert({ id: "al-1", status: "resolved" }),
      alert({ id: "al-2", status: "acknowledged" }),
      alert({ id: "al-3", status: "open", incident_ticket_id: "ticket-9", severity: "critical" }),
    ]);
    expect(rows.map((r) => r.tone)).toEqual(["success", "warning", "neutral"]);
    expect(rows[2]).toMatchObject({
      id: "al-3",
      dedupKey: "prov-123",
      entityId: "ticket-9",
      label: "critical · open",
    });
    expect(rows[0]?.label).toBe("warning · resolved");
  });
});

describe("m2mIngestLogRows — the source's own log read", () => {
  it("carries the envelope identity off the server rows, in order", () => {
    const rows = m2mIngestLogRows([
      {
        id: "ev-1",
        occurred_at: "2026-01-02T03:04:05Z",
        dedup_key: "prov-123",
        entity_id: "envelope-1",
      },
      { id: "ev-2", occurred_at: "2026-01-02T02:00:00Z", dedup_key: null, entity_id: null },
    ]);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({
      id: "ev-1",
      at: "2026-01-02T03:04:05Z",
      dedupKey: "prov-123",
      entityId: "envelope-1",
      tone: "neutral",
      label: "emitted",
    });
    expect(rows[1]?.dedupKey).toBeNull();
  });

  it("an empty log is an empty list, not an error", () => {
    expect(m2mIngestLogRows([])).toEqual([]);
  });
});
