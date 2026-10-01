// Brand-review render for the ticket ledger.
//
// The mockup pack has no state for the ledger or the merge banner, so "is it
// on-brand?" cannot be answered by pointing at a reference shot. It has to be
// answered by rendering OUR components against OUR compiled CSS and looking at
// the result next to the pack. This is that render, made repeatable.
//
// It is a test so that it inherits the workspace's module resolution (the
// `@marula-core/ui` package exports only an `import` condition, which the
// tsx/CJS loader cannot satisfy — the vitest resolver can).
//
// It writes NOTHING by default. Set MARULA_BRAND_CAPTURE=1 to produce the
// screenshots; otherwise it asserts the fixtures still render, which is the
// part worth having in CI: the ledger's markup cannot drift into a shape that
// throws.
//
// Prerequisite for the screenshots (regenerate after a token change):
//   cd apps/web && node .harness/compile.cjs
import { describe, expect, it } from "vitest";
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { components } from "@marula-core/sdk";
import { MergeBanner } from "../components/tickets/merge-banner";
import { CustomFieldsPanel } from "../components/ticket-workspace/ticket-fields";
import { TicketTable, type LedgerColumnDef } from "../components/tickets/ledger-table";
import type { TicketRow } from "../lib/ticket-ops";

const HERE = dirname(fileURLToPath(import.meta.url));
const WEB = resolve(HERE, "..");
const REPO = resolve(WEB, "../..");
const CSS = resolve(WEB, ".harness/app.css");
const OUT = resolve(REPO, "design/qa-p41-ui-operations/captures");

const NOW = Date.now();
const iso = (ms: number): string => new Date(NOW + ms).toISOString();
const ORG = "00000000-0000-4000-8000-00000000000f";
const MINE = "00000000-0000-4000-8000-0000000000d1";

const base = (over: Partial<TicketRow>): TicketRow =>
  ({
    id: "00000000-0000-4000-8000-0000000000aa",
    org_id: ORG,
    board_id: "00000000-0000-4000-8000-0000000000bb",
    status_id: "00000000-0000-4000-8000-0000000000cc",
    category: "open",
    category_id: null,
    number: "MAR-100",
    title: "Untitled",
    body: null,
    priority: "p2",
    assignee_id: null,
    requester_id: null,
    client_id: null,
    contact_id: null,
    site_id: null,
    type_id: null,
    due_at: null,
    created_at: iso(-172_800_000),
    updated_at: iso(-3_600_000),
    resolved_at: null,
    merged_into_ticket_id: null,
    custom_fields: [],
    ...over,
  }) as unknown as TicketRow;

/**
 * Fixtures chosen to put every status tone and every countdown branch on
 * screen at once: P1 in progress with 12 minutes left (the risk window), an
 * open P2 well clear of it, a waiting P3 already over, an open P4 with no
 * deadline at all (the "—" case), and a closed merged row.
 */
const ROWS: TicketRow[] = [
  base({
    id: "00000000-0000-4000-8000-0000000000a1",
    number: "MAR-184",
    priority: "p1",
    category: "in_progress",
    title: "VPN disconnects every 20–30 minutes",
    assignee_id: MINE,
    due_at: iso(12 * 60_000 + 30_000),
    updated_at: iso(-600_000),
  }),
  base({
    id: "00000000-0000-4000-8000-0000000000a2",
    number: "MAR-181",
    priority: "p2",
    title: "Kitchen extractor fan rattling after filter change",
    assignee_id: MINE,
    due_at: iso(5 * 3_600_000),
    updated_at: iso(-7_200_000),
  }),
  base({
    id: "00000000-0000-4000-8000-0000000000a3",
    number: "MAR-177",
    priority: "p3",
    category: "waiting",
    title: "Awaiting part delivery from supplier",
    due_at: iso(-3 * 3_600_000),
    updated_at: iso(-86_400_000),
  }),
  base({
    id: "00000000-0000-4000-8000-0000000000a4",
    number: "MAR-176",
    priority: "p4",
    title: "Quarterly patch cycle — check the notes first",
    updated_at: iso(-96_000_000),
  }),
  base({
    id: "00000000-0000-4000-8000-0000000000a5",
    number: "MAR-175",
    priority: "p4",
    category: "closed",
    title: "Duplicate of MAR-181 — extractor fan",
    merged_into_ticket_id: "00000000-0000-4000-8000-0000000000a2",
    updated_at: iso(-172_800_000),
  }),
];

const COLUMNS: LedgerColumnDef[] = [
  { key: "number", label: "Number", visible: true, required: true, width: 116, sortable: true },
  { key: "title", label: "Subject", visible: true, required: true, width: 340, sortable: true },
  { key: "category", label: "Status", visible: true, width: 124, sortable: true },
  { key: "priority", label: "Priority", visible: true, width: 92, sortable: true },
  { key: "assignee", label: "Assignee", visible: true, width: 150, sortable: false },
  { key: "sla", label: "SLA", visible: true, width: 124, sortable: false },
  { key: "updated_at", label: "Updated", visible: true, width: 104, sortable: true },
];

const SLA: Record<string, components["schemas"]["TicketSlaStatus"] | null> = {
  "00000000-0000-4000-8000-0000000000a1": {
    applied: false,
    breached: false,
    minutes_remaining: null,
    paused: false,
    state: null,
  },
};

const noop = (): void => {};

const CUSTOM_FIELDS: components["schemas"]["TicketCustomFieldValue"][] = [
  { field_id: "f1", kind: "text", name: "Site access", required: true, sort_order: 0,
    value: "Gate code 4417, ring the bell twice" },
  { field_id: "f2", kind: "select", name: "Contract", required: false, sort_order: 1,
    value: { id: "c1", name: "Gold 24/7" } },
  { field_id: "f3", kind: "multiselect", name: "Affected services", required: false, sort_order: 2,
    value: ["VPN", "Wi-Fi", "DNS"] },
  { field_id: "f4", kind: "number", name: "Downtime (mins)", required: false, sort_order: 3, value: 1450 },
  { field_id: "f5", kind: "date", name: "Confirmed outage", required: false, sort_order: 4,
    value: "2026-09-29T08:15:00Z" },
  { field_id: "f6", kind: "bool", name: "Third party involved", required: false, sort_order: 5,
    value: true },
  { field_id: "f7", kind: "text", name: "Parts ordered", required: false, sort_order: 6, value: null },
];

function renderFragments(): { table: string; banner: string; fields: string } {
  const table = renderToStaticMarkup(
    React.createElement(TicketTable, {
      columns: COLUMNS,
      rows: ROWS,
      sort: { field: "updated_at", dir: "desc" },
      sla: SLA,
      people: { [MINE]: "Thabo Mokoena" },
      clients: {},

      numbersById: Object.fromEntries(ROWS.map((r) => [r.id, r.number])),
      myId: null,
      selected: new Set(["00000000-0000-4000-8000-0000000000a2"]),
      onSort: noop,
      onReorderColumns: noop,
      onResizeColumn: noop,
      onToggleRow: noop,
    }),
  );
  const banner = renderToStaticMarkup(
    React.createElement(MergeBanner, {
      survivorId: "00000000-0000-4000-8000-0000000000a2",
      survivorLabel: "MAR-181",
    }),
  );
  const fields = renderToStaticMarkup(
    React.createElement(CustomFieldsPanel, { fields: CUSTOM_FIELDS }),
  );
  return { table, banner, fields };
}

function shell(theme: string, css: string, body: string): string {
  return `<!doctype html>
<html lang="en" data-theme="${theme}">
<head><meta charset="utf-8"><style>${css}</style>
<style>
  body { margin: 0; background: var(--m-background); }
  .harness-pad { padding: 24px; }
  /* The pack's own .lab label, and the same 12px section gap the pack uses
     between a label and what it labels (its .chips row is padding-top 12px).
     The earlier 10px left the label sitting on the card, which read as a
     product defect when it was a defect in the harness. */
  .harness-label {
    font: 600 12px/1 var(--m-font-ui); letter-spacing: .08em; text-transform: uppercase;
    color: var(--m-text-secondary); margin: 24px 0 12px;
  }
  .harness-label:first-child { margin-top: 0; }
  /* The pack's own .card padding is 16px. The surface under test is what is
     being reviewed, so the frame around it must not be tighter than the real
     thing. */
  .harness-card { padding: 16px; }
</style></head>
<body class="harness-pad">
  <p class="harness-label">/tickets ledger</p>
  <div class="m-card">${body}</div>
</body></html>`;
}

describe("ledger brand render", () => {
  it("renders every status tone and countdown branch without throwing", () => {
    const { table, banner, fields } = renderFragments();
    // The three countdown branches are the point of the fixtures. The figures
    // are matched loosely on purpose: the countdown reads the wall clock at
    // render, so a fixture sitting exactly on a minute boundary floors to a
    // different minute depending on how long the suite took to get here.
    expect(table).toMatch(/1[12]m left/);  // at risk: inside the 30-minute window
    expect(table).toMatch(/\d+h( \d+m)? over/); // breached
    expect(table).toContain("—");          // no SLA policy and no due date
    // The four status tones.
    expect(table).toContain("In progress");
    expect(table).toContain("Open");
    expect(table).toContain("Waiting");
    expect(table).toContain("Closed");
    expect(table).toContain("MAR-181");
    expect(table).toContain("Thabo Mokoena");
    // The banner must NAME the survivor: the row is on this very page, so
    // claiming it is out of scope would be a lie the reader can disprove.
    expect(table).toContain("Open MAR-181");
    expect(banner).toContain("merged");
    // fieldValueLabel: every branch has to read as a value, never as JSON.
    expect(fields).toContain("Gate code 4417");
    expect(fields).toContain("Gold 24/7");
    expect(fields).toContain("VPN, Wi-Fi, DNS");
    expect(fields).toContain("1,450");
    expect(fields).toContain("Yes");
    expect(fields).toContain("—");
  });

  it("writes both theme captures when MARULA_BRAND_CAPTURE=1", async () => {
    if (process.env.MARULA_BRAND_CAPTURE !== "1") return;
    if (!existsSync(CSS)) {
      throw new Error(
        `compiled CSS missing at ${CSS} — run: cd apps/web && node .harness/compile.cjs`,
      );
    }
    // A STALE compile is worse than a missing one: it renders the surfaces in
    // CSS that no longer matches the repo, and every screenshot taken from it
    // is a lie. This bit once already — a new `@import` in globals.css made the
    // compile fail, the script exited, and the previous app.css sat there
    // looking fine until the capture was reviewed.
    const newestSource = [
      resolve(WEB, "app/globals.css"),
      resolve(REPO, "packages/ui/src/components.css"),
      resolve(REPO, "packages/ui/src/tokens.css"),
    ]
      .filter(existsSync)
      .map((f) => statSync(f).mtimeMs)
      .reduce((a, b) => Math.max(a, b), 0);
    const compiledAt = statSync(CSS).mtimeMs;
    if (compiledAt < newestSource) {
      throw new Error(
        `compiled CSS at ${CSS} is older than its sources — recompile: ` +
          `cd apps/web && node .harness/compile.cjs`,
      );
    }
    const css = readFileSync(CSS, "utf8");
    const { table, banner, fields } = renderFragments();
    mkdirSync(OUT, { recursive: true });
    // Also drop the raw table markup for the geometry probe to measure.
    writeFileSync(resolve(WEB, ".harness/fragments.json"), JSON.stringify({ table, banner, fields }));

    const { chromium } = await import("playwright");
    const browser = await chromium.launch({ args: ["--no-proxy-server"] });
    for (const theme of ["light", "dark"]) {
      const page = await browser.newPage({ viewport: { width: 1180, height: 720 } });
      await page.setContent(
        shell(theme, css, `${table}<p class="harness-label">merged-row banner</p><div class="m-card">${banner}</div><p class="harness-label">workbench details · custom fields</p><div class="m-card"><div class="harness-card">${fields}</div></div>`),
        { waitUntil: "load" },
      );
      await page.waitForTimeout(250);
      const file = resolve(OUT, `brand-ledger-${theme}.png`);
      await page.screenshot({ path: file, fullPage: true });
      console.log(`[brand] ${file}`);
      await page.close();
    }
    await browser.close();
  });
});