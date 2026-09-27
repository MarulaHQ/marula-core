// Flow 11 — the guided merge workbench (Wave 22 lane 2): the
// defer/skip, reject, and undo decision paths over the REAL workbench
// UI, staged through the public API (feed upserts create candidates;
// the UI only decides).
//
// Paths covered:
// 1. Devices queue: DEFER parks the candidate without deciding; RESTORE
//    brings it back to pending (typed skip, never silent).
// 2. REJECT keeps both records — no fold.
// 3. ACCEPT folds the candidate; the undo contract lives on /inventory's
//    merge ledger ("Undo (split)") and the absorbed item comes back.
// 4. Contacts: the phone-only candidate accepts (audited fold) and the
//    workbench ledger card offers the split (undo).
import { expect, test } from "../helpers/harness";
import { authedFetch } from "../helpers/api";

const API =
  process.env["MERIDIAN_API_URL"] ??
  process.env["E2E_API_URL"]?.replace(/\/+$/, "") ??
  "http://localhost:8080";

async function apiPost(token: string, path: string, body: unknown): Promise<any> {
  const response = await fetch(`${API}${path}`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${token}`,
      "content-type": "application/json",
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!response.ok) {
    throw new Error(`apiPost ${path} -> ${response.status}: ${await response.text()}`);
  }
  return response.json();
}

function suffix(): string {
  return String(Date.now() % 1000000);
}

test("workbench devices queue: defer/restore, reject, accept + undo (split)", async ({
  authedPage,
  ownerTokens,
}) => {
  const get = authedFetch(ownerTokens.access_token);
  const token = ownerTokens.access_token;
  const s = suffix();
  const clients = await get<any[]>("/clients");
  const clientId = clients[0].id;
  // Unique per-run MACs: a re-run must not collide with stale devices.
  const randOct = () => Math.floor(Math.random() * 256).toString(16).padStart(2, "0");
  const m1 = `aa:bb:cc:dd:${randOct()}:${randOct()}`;
  const m2 = `aa:bb:cc:ee:${randOct()}:${randOct()}`;
  const mac = (n: number) => (n === 1 ? m1 : m2);

  // Stage ONE ambiguous device pair (partial MAC overlap ⇒ 0.6 < 0.85 ⇒
  // fresh item + pending candidate).
  await apiPost(token, "/api/config-items/feed-upsert", {
    source_system: "ninjaone",
    external_id: `E2E-X-${s}-1`,
    client_id: clientId,
    tag: `E2E-${s}`,
    name: "Device A",
    identity_keys: { macs: [mac(1), mac(2)] },
    observed_fields: {},
    idempotency_key: `e2e-feed-1-${s}`,
  });
  const second = await apiPost(token, "/api/config-items/feed-upsert", {
    source_system: "auvix",
    external_id: `E2E-Y-${s}-2`,
    client_id: clientId,
    tag: `E2E-${s}`,
    name: "Device B",
    identity_keys: { macs: [mac(2)] },
    observed_fields: {},
    idempotency_key: `e2e-feed-2-${s}`,
  });
  const queued = second.queued_candidates as any[];
  expect(queued.length).toBe(1);
  const candidateId = queued[0].id;

  await authedPage.goto("/inventory/workbench");
  const row = authedPage.getByTestId(`workbench-row-${candidateId}`);
  await expect(row).toBeVisible({ timeout: 30_000 });

  // Open the side-by-side detail.
  await row.getByRole("button").click();
  await expect(authedPage.getByTestId("detail-primary-card")).toBeVisible({ timeout: 15_000 });

  // DEFER (the skip path): parks the candidate without deciding.
  await authedPage
    .getByTestId("entity-detail")
    .or(authedPage.getByTestId("workbench-detail"))
    .getByRole("button", { name: "Defer", exact: true })
    .click();
  await expect(authedPage.getByTestId("detail-restore")).toBeVisible({ timeout: 15_000 });

  // RESTORE (un-defer) returns it to the queue — the row drops its
  // deferred chip back to a pending row (pending rows show no chip).
  await authedPage.getByTestId("detail-restore").click();
  await expect(row.getByText("deferred")).toHaveCount(0, { timeout: 15_000 });

  // REJECT keeps both records — the row never folds and no ledger row
  // appears for it on /inventory.
  await authedPage
    .getByTestId("workbench-detail")
    .getByRole("button", { name: "Keep separate", exact: true })
    .click();
  await expect(row.getByText("rejected").first()).toBeVisible({ timeout: 15_000 });
  const candidates = await get<any[]>("/config-items/merge-candidates");
  const rejected = candidates.find((c: any) => c.id === candidateId);
  expect(rejected?.status).toBe("rejected");
});

test("workbench devices queue: accept folds and the split undoes it", async ({
  authedPage,
  ownerTokens,
}) => {
  const get = authedFetch(ownerTokens.access_token);
  const token = ownerTokens.access_token;
  const s = suffix();
  const clients = await get<any[]>("/clients");
  const clientId = clients[0].id;
  // Unique per-run MACs (a re-run must not collide with stale devices):
  // two fresh random octet pairs, memoized so the overlap is exact.
  const randOct = () => Math.floor(Math.random() * 256).toString(16).padStart(2, "0");
  const m1 = `aa:bb:cc:dd:${randOct()}:${randOct()}`;
  const m2 = `aa:bb:cc:ee:${randOct()}:${randOct()}`;
  const mac = (n: number) => (n === 1 ? m1 : m2);

  await apiPost(token, "/api/config-items/feed-upsert", {
    source_system: "ninjaone",
    external_id: `E2E-U-${s}-1`,
    client_id: clientId,
    tag: `E2EU-${s}`,
    name: "Device A",
    identity_keys: { macs: [mac(1), mac(2)] },
    observed_fields: {},
    idempotency_key: `e2e-u-1-${s}`,
  });
  const second = await apiPost(token, "/api/config-items/feed-upsert", {
    source_system: "auvix",
    external_id: `E2E-U-${s}-2`,
    client_id: clientId,
    tag: `E2EU-${s}`,
    name: "Device B",
    identity_keys: { macs: [mac(2)] },
    observed_fields: {},
    idempotency_key: `e2e-u-2-${s}`,
  });
  const queued = second.queued_candidates as any[];
  expect(queued.length).toBe(1);
  const candidateId = queued[0].id;

  await authedPage.goto("/inventory/workbench");
  const row = authedPage.getByTestId(`workbench-row-${candidateId}`);
  await expect(row).toBeVisible({ timeout: 30_000 });
  await row.getByRole("button").click();
  await expect(authedPage.getByTestId("detail-primary-card")).toBeVisible({ timeout: 15_000 });

  // ACCEPT: the audited fold.
  await authedPage
    .getByTestId("workbench-detail")
    .getByRole("button", { name: "Accept merge", exact: true })
    .click();
  // The absorbed item vanishes from the ledger (optimistic) — the
  // candidate row flips to accepted.
  await expect(row.getByText("accepted").first()).toBeVisible({ timeout: 15_000 });

  // UNDO: the split on /inventory's merge ledger restores the item.
  await authedPage.goto("/inventory");
  const ledgerRow = authedPage
    .getByRole("row")
    .filter({ has: authedPage.getByText(`E2EU-${s}`) })
    .first();
  await expect(ledgerRow).toBeVisible({ timeout: 30_000 });
  await ledgerRow.getByRole("button", { name: "Undo (split)" }).click();
  await expect(
    authedPage
      .getByRole("row")
      .filter({ has: authedPage.getByText("Device B") })
      .first(),
  ).toBeVisible({ timeout: 20_000 });
});

test("workbench contacts queue: accept folds and the ledger card undoes", async ({
  authedPage,
  ownerTokens,
}) => {
  const get = authedFetch(ownerTokens.access_token);
  const token = ownerTokens.access_token;
  const s = suffix();
  const clients = await get<any[]>("/clients");
  const clientId = clients[0].id;

  // Contact A (phone only) then B matching the phone only (0.75 < 0.85 ⇒
  // reviewable candidate).
  const a = await apiPost(token, "/api/contacts/feed-upsert", {
    source_system: "ninjaone",
    external_id: `E2E-P-${s}-1`,
    client_id: clientId,
    name: "Phone Person",
    phone: `555 012 ${s.slice(0, 4)}`,
    idempotency_key: `e2e-p-1-${s}`,
  });
  const b = await apiPost(token, "/api/contacts/feed-upsert", {
    source_system: "hudu",
    external_id: `E2E-Q-${s}-2`,
    client_id: clientId,
    name: "Phone Person B",
    phone: `555012${s.slice(0, 4)}`,
    idempotency_key: `e2e-p-2-${s}`,
  });
  const queued = (b.queued_candidates ?? []) as any[];
  expect(queued.length).toBe(1);
  const candidateId = queued[0].id;
  const aId = a.contact.id;
  void aId;

  await authedPage.goto("/inventory/workbench");
  await authedPage.getByTestId("workbench-tab-contacts").click();
  const row = authedPage.getByTestId(`workbench-row-${candidateId}`);
  await expect(row).toBeVisible({ timeout: 30_000 });
  await row.getByRole("button").click();
  await expect(authedPage.getByTestId("entity-detail")).toBeVisible({ timeout: 15_000 });

  // ACCEPT the contact fold.
  await authedPage
    .getByTestId("entity-detail")
    .getByRole("button", { name: "Accept merge", exact: true })
    .click();
  await expect(authedPage.getByTestId("entity-merge-ledger")).toBeVisible({ timeout: 15_000 });

  // UNDO via the ledger card's split.
  await authedPage
    .getByTestId("entity-merge-ledger")
    .getByRole("button", { name: "Undo", exact: true })
    .click();
  await expect(authedPage.getByTestId("entity-merge-ledger").getByText("undone")).toBeVisible({
    timeout: 15_000,
  });
});
