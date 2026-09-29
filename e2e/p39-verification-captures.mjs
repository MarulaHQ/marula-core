// Wave 39 lane 2 `p39-verification-run` — vision-review captures.
//
// Drives the REAL app against the live e2e stack (never mocks) so the
// screenshots show true states of the billing verification workspace:
//   1. The workspace mid-review: entries grouped by ticket, the counter
//      band, the close card, and at least one entry that blocks the sign
//      -off (unclassified / draft) so the warning tone is on the shot.
//   2. The entry action dialog: the reason-required adjust form, which is
//      the moment the reviewer actually changes a bill.
//   3. A window WITH a block contract, so the block-consumption panel is
//      captured (drawn vs remaining vs overage) rather than assumed.
//   4. The close dialog with its checklist, before signing.
//   5. The run composer's new pointer into the ritual.
//
// Each surface is captured in light and dark. Run with the stack env
// exported:
//   set -a; . e2e/.e2e-stack.env; set +a; node e2e/p39-verification-captures.mjs
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";

const API = (process.env.E2E_API_URL ?? "http://localhost:8080").replace(/\/$/, "");
const WEB = (process.env.E2E_WEB_URL ?? "http://localhost:3000").replace(/\/$/, "");
const OUT = "design/qa-p39-verification-run/captures";
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
const STAMP = Date.now() % 1_000_000;

mkdirSync(OUT, { recursive: true });

const req = await (await import("playwright")).request.newContext();
const login = await req.post(`${API}/api/v1/auth/login`, { data: { email: EMAIL, password: PASSWORD } });
if (!login.ok()) throw new Error(`login failed: ${login.status()}`);
const { access_token: token } = await login.json();
const auth = (path, init = {}) =>
  req.fetch(`${API}/api/v1${path}`, {
    ...init,
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json", ...(init.headers ?? {}) },
  });

const iso = (offsetDays) => new Date(Date.now() + offsetDays * 86400_000).toISOString().slice(0, 10);
const started = new Date(Date.now() - 7200_000).toISOString();
const at = (minutes) => new Date(Date.now() - 7200_000 + minutes * 60_000).toISOString();

// --- stage: a client, a T&M contract, and one retainer block ---------------
const client = (
  await (
    await auth("/clients", {
      method: "POST",
      data: { name: `Capture Verify ${STAMP}`, idempotency_key: `p39-cap-client-${STAMP}` },
    })
  ).json()
);
// The 10-hour prepaid block rides the CREATE body (blocks are contract
// configuration, not a separate resource). Its lower block rate is what
// makes the panel worth reading: some work draws it, the rest spills.
const contract = (
  await (
    await auth("/contracts", {
      method: "POST",
      data: {
        client_id: client.id,
        number: `C-CAP-${STAMP}`,
        name: `Capture Verification ${STAMP}`,
        contract_type: "time_and_materials",
        currency: "USD",
        hourly_rate_minor: 15000,
        effective_from: iso(-30),
        blocks: [
          {
            label: `Capture block ${STAMP}`,
            total_minutes: 600,
            sale_price_minor: 0,
            block_rate_minor: 12000,
            rollover_policy: "none",
          },
        ],
        idempotency_key: `p39-cap-contract-${STAMP}`,
      },
    })
  ).json()
);

const boards = await (await auth("/boards")).json();
const board = boards.find((b) => b.is_default) ?? boards[0];
const ticketA = (
  await (
    await auth("/tickets", {
      method: "POST",
      data: {
        board_id: board.id,
        client_id: client.id,
        title: `Capture review A ${STAMP}`,
        priority: "p2",
        idempotency_key: `p39-cap-ticket-a-${STAMP}`,
      },
    })
  ).json()
);
const ticketB = (
  await (
    await auth("/tickets", {
      method: "POST",
      data: {
        board_id: board.id,
        client_id: client.id,
        title: `Capture review B ${STAMP}`,
        priority: "p3",
        idempotency_key: `p39-cap-ticket-b-${STAMP}`,
      },
    })
  ).json()
);

// Ticket A: two classified approved entries. Ticket B: one unclassified
// approved entry (the blocker tone) plus one left in draft.
const entries = [];
for (const [ticket, minutes, workType] of [
  [ticketA, 120, "remote"],
  [ticketA, 90, "on_site"],
  [ticketB, 60, null],
  [ticketB, 45, "depot"],
]) {
  const entry = (
    await (
      await auth("/time-entries", {
        method: "POST",
        data: {
          ticket_id: ticket.id,
          started_at: started,
          ended_at: at(minutes),
          billable: true,
          ...(workType === null ? {} : { work_type: workType }),
          notes: `p39 verification capture ${STAMP}`,
          idempotency_key: `p39-cap-entry-${STAMP}-${minutes}`,
        },
      })
    ).json()
  );
  entries.push(entry);
  if (minutes !== 45) {
    await auth(`/time-entries/${entry.id}/submit`, { method: "POST", data: {} });
    await auth("/time-entries/approvals/decide", {
      method: "POST",
      data: { entry_ids: [entry.id], decision: "approve" },
    });
  }
}

// One reviewed entry, so the shot shows a badge that is neither draft
// nor unclassified — the "reviewed" state is the workspace's steady state.
await auth(`/billing/verification/entries/${entries[1].id}/action`, {
  method: "POST",
  data: {
    action: "adjust_minutes",
    minutes: 60,
    reason: "capture: trimmed the diagnostic tail",
    idempotency_key: `p39-cap-adjust-${STAMP}`,
  },
});

const window = `from=${iso(-30)}&to=${iso(1)}`;

// --- browser -------------------------------------------------------------
const browser = await chromium.launch();
let shots = 0;
for (const theme of ["light", "dark"]) {
  const ctx = await browser.newContext({ colorScheme: theme, viewport: { width: 1440, height: 1100 } });
  const page = await ctx.newPage();
  await page.goto(`${WEB}/login`);
  await page.fill('input[type="email"]', EMAIL);
  await page.fill('input[type="password"]', PASSWORD);
  await page.click('button[type="submit"]');
  await page.waitForURL(/tickets|today|dashboard/, { timeout: 45_000 });

  const shoot = async (name) => {
    await page.waitForTimeout(1200);
    await page.screenshot({ path: `${OUT}/${name}.png`, fullPage: false });
    shots += 1;
  };

  // 1: the workspace itself, loaded on the staged contract.
  await page.goto(`${WEB}/billing/verification?contract_id=${contract.id}&${window}`);
  await page.getByTestId("billing-verification-page").waitFor({ state: "visible", timeout: 30_000 });
  await page.getByTestId(`verify-group-${ticketA.id}`).waitFor({ state: "visible", timeout: 30_000 });
  await shoot(`workspace-${theme}`);

  // 2: the in-context action dialog on a classified entry.
  await page.getByTestId(`verify-open-${entries[0].id}`).click();
  await page.getByTestId(`verify-action-dialog-${entries[0].id}`).waitFor({ state: "visible", timeout: 15_000 });
  await shoot(`workspace-action-dialog-${theme}`);
  await page.keyboard.press("Escape");

  // 3: the close card, framed at the top of the viewport so the shot
  //    shows the checklist rather than another copy of the page above.
  const closeCard = page.getByTestId("verify-close-card");
  if (await closeCard.isVisible().catch(() => false)) {
    await closeCard.scrollIntoViewIfNeeded();
    await page.evaluate(() => window.scrollBy(0, 240));
    await shoot(`workspace-close-${theme}`);
  }

  // 4: the sign-off dialog. It only unlocks once the counters are
  //    clean, so the remaining staged entries get dispositioned first —
  //    otherwise this shot silently records a disabled button and the
  //    ritual is never reviewed in either mode.
  // EVERY entry has to be dispositioned or the sign-off stays locked —
  // including the two the earlier shots already reviewed, so the
  // "undispositioned" counter can actually reach zero.
  const unclassified = entries[2];
  const draft = entries[3];
  for (const [entry, target] of [
    [entries[0], 120],
    [entries[1], 60],
    [unclassified, 60],
    [draft, 45],
  ]) {
    if (entry.work_type === null) {
      await auth(`/billing/verification/entries/${entry.id}/action`, {
        method: "POST",
        data: {
          action: "recategorize",
          work_type: "remote",
          idempotency_key: `p39-cap-recat-${STAMP}-${entry.id}`,
        },
      });
    }
    if (entry.id === draft.id) {
      await auth(`/time-entries/${entry.id}/submit`, { method: "POST", data: {} });
      await auth("/time-entries/approvals/decide", {
        method: "POST",
        data: { entry_ids: [entry.id], decision: "approve" },
      });
    }
    await auth(`/billing/verification/entries/${entry.id}/action`, {
      method: "POST",
      data: {
        action: "adjust_minutes",
        minutes: target,
        reason: "capture: reviewed and accepted",
        idempotency_key: `p39-cap-adj2-${STAMP}-${entry.id}`,
      },
    });
  }
  await page.reload({ waitUntil: "networkidle" });
  await page.getByTestId("verify-close-card").waitFor({ state: "visible", timeout: 30_000 });
  const signButton = page.getByTestId("verify-open-signoff");
  if (await signButton.isEnabled().catch(() => false)) {
    await signButton.click();
    await page.getByTestId("verify-signoff-dialog").waitFor({ state: "visible", timeout: 15_000 });
    await shoot(`workspace-signoff-${theme}`);
    await page.keyboard.press("Escape");
  } else {
    console.warn(`[p39-verification-captures] ${theme}: sign-off stayed locked (counters not clean)`);
  }

  // 5: the run composer's pointer into the ritual.
  await page.goto(`${WEB}/billing`);
  await page.waitForLoadState("networkidle");
  const runBtn = page.getByRole("button", { name: /invoice run|new run|run invoices/i }).first();
  if (await runBtn.isVisible().catch(() => false)) {
    await runBtn.click();
    await page.waitForTimeout(1500);
    const pointer = page.getByTestId("run-open-verification");
    if (await pointer.isVisible().catch(() => false)) {
      await shoot(`run-composer-verification-link-${theme}`);
      await page.keyboard.press("Escape");
    }
  }

  await ctx.close();
}
await browser.close();
console.log(`[p39-verification-captures] wrote ${shots} captures to ${OUT}`);
