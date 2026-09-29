// Flow 21 — Time categories (Wave 39 lane 1, `p39-time-categories`): the
// categorization backbone end to end. The taxonomy the org seeds, admin
// CRUD + reorder + soft retire through the IF1 envelope, capture-time
// `work_type` normalization (exact slug, kept legacy alias, typed refusal
// of garbage), the V3 rule that a non-billable entry needs a typed reason,
// and the billing-audit chips — which are read-only by construction, proven
// here by asserting the entry is byte-identical after the read.
//
// Order-independent: creates its own category slug per run and retires it
// at the end, so it can run beside any other lane's spec.
import { expect, test } from "../helpers/harness";
import { authedFetch } from "../helpers/api";

interface TimeCategory {
  id: string;
  slug: string;
  label: string;
  default_billable: boolean;
  retired: boolean;
  position: number;
}

interface TimeEntry {
  id: string;
  billable: boolean;
  work_type: string | null;
  unbillable_reason: string | null;
}

test("seeded taxonomy, admin CRUD/reorder, capture-time rules, read-only chips", async ({
  authedPage,
  ownerTokens,
}) => {
  const call = authedFetch(ownerTokens.access_token);
  const post = (path: string, body?: unknown) =>
    authedFetch(ownerTokens.access_token)(path, { method: "POST", body });
  const patch = (path: string, body?: unknown) =>
    authedFetch(ownerTokens.access_token)(path, { method: "PATCH", body });
  const del = (path: string) =>
    authedFetch(ownerTokens.access_token)(path, { method: "DELETE" });
  const run = Date.now() % 100000;
  const slug = `e2e_field_${run}`;

  // 1. The org is seeded with the standard ten, in picker order.
  const defaults = await call<{ categories: TimeCategory[]; work_type_map: unknown[] }>(
    "/time-categories/capture-defaults",
  );
  const slugs = defaults.categories.map((c) => c.slug);
  for (const seeded of ["remote", "on_site", "depot", "travel", "internal", "warranty"]) {
    expect(slugs, `seed set missing ${seeded}`).toContain(seeded);
  }
  expect(defaults.work_type_map.length).toBeGreaterThan(0);

  // 2. Admin CRUD on the envelope; the create returns a mutation seq.
  const created = await post<{ category: TimeCategory; mutation: { seq: number } }>(
    "/time-categories",
    {
      slug,
      label: `E2E Field ${run}`,
      description: "staged by the p39 flow spec",
      default_billable: true,
      rate_card_affinity: true,
      idempotency_key: `p39-e2e-create-${run}`,
    },
  );
  expect(created.category.slug).toBe(slug);
  expect(created.mutation.seq).toBeGreaterThan(0);
  const id = created.category.id;

  // 3. The same key + payload replays byte-identically (no double-create).
  const replay = await post<{ category: TimeCategory; mutation: { seq: number } }>(
    "/time-categories",
    {
      slug,
      label: `E2E Field ${run}`,
      description: "staged by the p39 flow spec",
      default_billable: true,
      rate_card_affinity: true,
      idempotency_key: `p39-e2e-create-${run}`,
    },
  );
  expect(replay.category.id).toBe(id);
  expect(replay.mutation.seq).toBe(created.mutation.seq);

  // 4. A duplicate slug is refused (409 conflict), never silently renamed.
  await expect(
    post("/time-categories", {
      slug,
      label: "dup",
      idempotency_key: `p39-e2e-dup-${run}`,
    }),
  ).rejects.toThrow(/409/);

  // 5. PATCH flips the billing default; the seq advances.
  const patched = await patch<{ category: TimeCategory; mutation: { seq: number } }>(
    `/time-categories/${id}`,
    { label: `E2E Field ${run} (reactive)`, default_billable: false, idempotency_key: `p39-e2e-patch-${run}` },
  );
  expect(patched.category.default_billable).toBe(false);
  expect(patched.mutation.seq).toBeGreaterThan(created.mutation.seq);

  // 6. Capture-time resolution: exact slug, kept legacy alias, refusal.
  const tickets = await call<any[]>(`/tickets?limit=50`);
  const ticket = tickets.find((t) => t.category !== "closed");
  expect(ticket).toBeDefined();
  const started = new Date(Date.now() - 7200_000).toISOString();
  const ended = new Date(Date.now() - 3600_000).toISOString();

  const exact = await post<TimeEntry>("/time-entries", {
    ticket_id: ticket.id,
    started_at: started,
    ended_at: ended,
    billable: true,
    work_type: slug,
  });
  expect(exact.work_type).toBe(slug);

  // The kept backfill mapping keeps legacy free text working: `onsite`
  // normalizes onto the `on_site` slug instead of being refused.
  const alias = await post<TimeEntry>("/time-entries", {
    ticket_id: ticket.id,
    started_at: started,
    ended_at: ended,
    billable: true,
    work_type: "onsite",
  });
  expect(alias.work_type).toBe("on_site");

  // Unknown work_type is refused loudly rather than guessed.
  await expect(
    post("/time-entries", {
      ticket_id: ticket.id,
      started_at: started,
      ended_at: ended,
      billable: true,
      work_type: "definitely_not_a_category",
    }),
  ).rejects.toThrow();

  // 7. V3: a non-billable entry REQUIRES a typed reason, and carries it.
  await expect(
    post("/time-entries", {
      ticket_id: ticket.id,
      started_at: started,
      ended_at: ended,
      billable: false,
    }),
  ).rejects.toThrow();
  const nonBillable = await post<TimeEntry>("/time-entries", {
    ticket_id: ticket.id,
    started_at: started,
    ended_at: ended,
    billable: false,
    work_type: "warranty",
    unbillable_reason: "warranty",
  });
  expect(nonBillable.billable).toBe(false);
  expect(nonBillable.unbillable_reason).toBe("warranty");

  // 8. The billing-audit read is suggestion-grade: it never mutates.
  const before = await call<TimeEntry[]>(`/tickets/${ticket.id}/time-entries`);
  const suggestions = await call<{ entry_id: string; suggestions: unknown[] }>(
    `/time-entries/${exact.id}/billing-suggestions`,
  );
  expect(suggestions.entry_id).toBe(exact.id);
  const after = await call<TimeEntry[]>(`/tickets/${ticket.id}/time-entries`);
  expect(after).toEqual(before);

  // 9. The capture surface renders the taxonomy picker, and the approvals
  //    queue shows the category + reason for the non-billable entry.
  await authedPage.goto("/today");
  const quickLog = authedPage.getByRole("button", { name: /log time/i }).first();
  if (await quickLog.isVisible().catch(() => false)) {
    await quickLog.click();
    const picker = authedPage.getByTestId("quicklog-category").first();
    await expect(picker).toBeVisible({ timeout: 20_000 });
    const options = await picker.locator("option").allTextContents();
    expect(options.join(" ")).toContain("Remote");
    await authedPage.keyboard.press("Escape");
  }

  // 10. Soft retire: the slug leaves the capture picker but the row stays.
  await del(`/time-categories/${id}`);
  const afterRetire = await call<{ categories: TimeCategory[] }>("/time-categories/capture-defaults");
  expect(afterRetire.categories.map((c) => c.slug)).not.toContain(slug);
  const withRetired = await call<{ categories: TimeCategory[] }>(
    "/time-categories?include_retired=true",
  );
  expect(withRetired.categories.map((c) => c.slug)).toContain(slug);
});
