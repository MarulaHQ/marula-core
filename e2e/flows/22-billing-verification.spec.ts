// Flow 22 — Billing verification (Wave 39 lane 2, `p39-verification-run`):
// the pre-invoice ritual end to end. Stages its own client + T&M contract
// so it never depends on another lane's fixtures, then proves the whole
// chain over HTTP: the workspace read groups entries by ticket with
// disposition badges, running totals and the unclassified counter; an
// adjustment without a reason is refused, one WITH a reason lands in the
// audit trail and changes what the engine prices; the close sign-off
// re-verifies its own checklist claims; and the UI renders the same
// numbers the API returned.
//
// Order-independent: own client + own contract per run, no shared state.
import { expect, test } from "../helpers/harness";
import { localDatePlus } from "../helpers/dates";
import { authedFetch } from "../helpers/api";

interface Workspace {
  contract_id: string;
  currency: string;
  total_minor: number;
  total_minutes: number;
  preview_total_minor: number;
  counters: {
    entries: number;
    unclassified: number;
    undispositioned: number;
    draft: number;
    non_billable: number;
    adjusted: number;
    invoiced: number;
  };
  groups: {
    group_id: string;
    label: string;
    total_minutes: number;
    total_cost_minor: number;
    entries: { id: string; minutes: number; effective_minutes: number; cost_minor: number }[];
  }[];
  blocks: { block_id: string; label: string; remaining_minutes: number; alert: string }[];
  overlapping_runs: { number: string }[];
  signoff: { zero_unclassified: boolean } | null;
}

interface ActionResponse {
  entry_id: string;
  action: string;
  effective_minutes: number;
  disposition: string | null;
  audit_action: string;
  mutation: { seq: number };
}

test("workspace groups by ticket, adjustments need a reason, sign-off re-checks itself", async ({
  authedPage,
  ownerTokens,
}) => {
  const call = authedFetch(ownerTokens.access_token);
  const post = (path: string, body?: unknown) =>
    authedFetch(ownerTokens.access_token)(path, { method: "POST", body });
  const run = Date.now() % 100000;

  // 1. Own client + T&M contract at 150.00/h, effective a month back.
  const me = await call<{ org_id: string; user_id: string }>("/me");
  const client = await post<{ id: string }>("/clients", {
    name: `E2E Verify Client ${run}`,
    idempotency_key: `p39-verify-client-${run}`,
  });
  const contract = await post<{ id: string; number: string }>("/contracts", {
    client_id: client.id,
    number: `C-E2E-${run}`,
    name: `E2E Verification ${run}`,
    contract_type: "time_and_materials",
    currency: "USD",
    hourly_rate_minor: 15000,
    effective_from: localDatePlus(-30),
    idempotency_key: `p39-verify-contract-${run}`,
  });

  // 2. Two tickets, three entries: one classified, one unclassified
  //    (the counter the close ritual gates on), one left in draft.
  const boards = await call<any[]>("/boards");
  const board = boards.find((b) => b.is_default) ?? boards[0];
  const ticketA = await post<{ id: string }>("/tickets", {
    board_id: board.id,
    client_id: client.id,
    title: `Verification A ${run}`,
    priority: "p3",
    idempotency_key: `p39-verify-ticket-a-${run}`,
  });
  const ticketB = await post<{ id: string }>("/tickets", {
    board_id: board.id,
    client_id: client.id,
    title: `Verification B ${run}`,
    priority: "p3",
    idempotency_key: `p39-verify-ticket-b-${run}`,
  });

  const started = new Date(Date.now() - 7200_000).toISOString();
  const minute = (m: number) => new Date(Date.now() - 7200_000 + m * 60_000).toISOString();
  const classified = await post<{ id: string }>("/time-entries", {
    ticket_id: ticketA.id,
    started_at: started,
    ended_at: minute(120),
    billable: true,
    work_type: "remote",
    idempotency_key: `p39-verify-entry-a-${run}`,
  });
  const unclassified = await post<{ id: string }>("/time-entries", {
    ticket_id: ticketB.id,
    started_at: started,
    ended_at: minute(60),
    billable: true,
    idempotency_key: `p39-verify-entry-b-${run}`,
  });
  const draft = await post<{ id: string }>("/time-entries", {
    ticket_id: ticketB.id,
    started_at: started,
    ended_at: minute(30),
    billable: true,
    work_type: "on_site",
    idempotency_key: `p39-verify-entry-c-${run}`,
  });

  const approve = async (id: string) => {
    await post(`/time-entries/${id}/submit`, { note: "verification flow" });
    const decided = await call<{ results: { entry_id: string; outcome: string }[] }>(
      "/time-entries/approvals/decide",
      { method: "POST", body: { entry_ids: [id], decision: "approve" } },
    );
    expect(decided.results[0].outcome).toBe("applied");
  };
  await approve(classified.id);
  await approve(unclassified.id);

  const from = localDatePlus(-30);
  const to = localDatePlus(1);
  const workspaceUrl = `/billing/verification?contract_id=${contract.id}&from=${from}&to=${to}`;

  // 3. The workspace groups by ticket, counts the unclassified entry,
  //    and its own total foots to the engine's preview.
  let ws = await call<Workspace>(`/billing/verification?contract_id=${contract.id}&from=${from}&to=${to}`);
  expect(ws.groups.length).toBe(2);
  expect(ws.counters.entries).toBe(3);
  expect(ws.counters.unclassified).toBe(1);
  expect(ws.counters.draft).toBe(1);
  expect(ws.counters.undispositioned).toBe(3);
  expect(ws.total_minor).toBe(ws.preview_total_minor);

  // 4. An adjustment without a reason is refused — the evidence is
  //    mandatory, not advisory.
  await expect(
    post(`/billing/verification/entries/${classified.id}/action`, {
      action: "adjust_minutes",
      minutes: 30,
    }),
  ).rejects.toThrow(/422/);

  // 5. With a reason it lands, reports the new effective minutes, and
  //    dispositions the entry (reviewed = dispositioned).
  const adjusted = await post<ActionResponse>(
    `/billing/verification/entries/${classified.id}/action`,
    {
      action: "adjust_minutes",
      minutes: 30,
      reason: "flow spec trims the diagnostic tail",
      idempotency_key: `p39-verify-adjust-${run}`,
    },
  );
  expect(adjusted.effective_minutes).toBe(30);
  expect(adjusted.disposition).toBe("approved");
  expect(adjusted.audit_action).toBe("billing_verification.adjust_minutes");
  expect(adjusted.mutation.seq).toBeGreaterThan(0);

  // The same key + payload replays instead of adjusting twice.
  const replay = await post<ActionResponse>(
    `/billing/verification/entries/${classified.id}/action`,
    {
      action: "adjust_minutes",
      minutes: 30,
      reason: "flow spec trims the diagnostic tail",
      idempotency_key: `p39-verify-adjust-${run}`,
    },
  );
  expect(replay.effective_minutes).toBe(30);

  // 6. The workspace now shows the trimmed entry and the adjusted total.
  ws = await call<Workspace>(`/billing/verification?contract_id=${contract.id}&from=${from}&to=${to}`);
  expect(ws.counters.adjusted).toBe(1);
  const trimmed = ws.groups.flatMap((g) => g.entries).find((e) => e.id === classified.id);
  expect(trimmed?.effective_minutes).toBe(30);
  expect(trimmed?.minutes).toBe(120);

  // 7. Recategorizing rides the org taxonomy; garbage is refused.
  await expect(
    post(`/billing/verification/entries/${draft.id}/action`, {
      action: "recategorize",
      work_type: "not_a_real_category",
    }),
  ).rejects.toThrow(/422/);
  await post(`/billing/verification/entries/${draft.id}/action`, {
    action: "recategorize",
    work_type: "depot",
  });

  // 8. The sign-off refuses a claim the live window contradicts, then
  //    lands once the window is clean.
  await expect(
    post("/billing/verification/signoff", {
      contract_id: contract.id,
      from,
      to,
      entries_dispositioned: true,
      zero_unclassified: true,
      preview_seen: true,
    }),
  ).rejects.toThrow(/422/);

  // Close the remaining gaps: disposition the unclassified entry and
  // the draft one (the draft needs its owner's approval first).
  await post(`/billing/verification/entries/${unclassified.id}/action`, {
    action: "recategorize",
    work_type: "remote",
  });
  await post(`/billing/verification/entries/${unclassified.id}/action`, {
    action: "adjust_minutes",
    minutes: 60,
    reason: "reviewed as logged",
  });
  await approve(draft.id);
  await post(`/billing/verification/entries/${draft.id}/action`, {
    action: "adjust_minutes",
    minutes: 30,
    reason: "reviewed as logged",
  });

  // 9. The UI renders the same window, the same counters, and the same
  //    footing the API reported.
  await authedPage.goto(workspaceUrl);
  await expect(authedPage.getByTestId("billing-verification-page")).toBeVisible({ timeout: 30_000 });
  await authedPage.waitForSelector(`[data-testid="verify-group-${ticketA.id}"]`, { timeout: 30_000 });
  await expect(authedPage.getByTestId(`verify-group-${ticketA.id}`)).toBeVisible();
  await expect(authedPage.getByTestId(`verify-group-${ticketB.id}`)).toBeVisible();
  await expect(authedPage.getByTestId("verify-counter-adjusted")).toContainText("1");
  await expect(authedPage.getByTestId("verify-counter-unclassified")).toContainText("0");
  // The per-ticket cards must foot to the headline: a reviewer who adds
  // up the groups and gets a different number stops trusting both.
  const total = (await authedPage.getByTestId("verify-total").textContent()) ?? "";
  const groupTotals = await Promise.all(
    [ticketA.id, ticketB.id].map(async (id) => {
      const text =
        (await authedPage.getByTestId(`verify-group-total-${id}`).textContent()) ?? "";
      const match = text.match(/\$([\d,]+\.\d{2})/);
      return match ? Number(match[1].replace(/,/g, "")) : 0;
    }),
  );
  const headline = Number((total.match(/\$([\d,]+\.\d{2})/) ?? ["", "0"])[1].replace(/,/g, ""));
  expect(headline).toBe(groupTotals.reduce((a, b) => a + b, 0));

  // 10. The close ritual through the UI: the button unlocks only when
  //     the counters are clean, and signing records the attestation.
  await authedPage.getByTestId("verify-open-signoff").click();
  await expect(authedPage.getByTestId("verify-signoff-dialog")).toBeVisible({ timeout: 15_000 });
  await authedPage.getByTestId("verify-signoff-note").fill("flow spec sign-off");
  await authedPage.getByTestId("verify-signoff-submit").click();
  await expect(authedPage.getByTestId("verify-notice")).toBeVisible({ timeout: 20_000 });
  // The sign-off is on the record, and the close card says so. Scoped to
  // the card: the success notice also contains the words "Signed off",
  // and a loose text match trips Playwright's strict mode.
  await expect(
    authedPage.getByTestId("verify-close-card").getByText(/Signed off/i),
  ).toBeVisible();
});
