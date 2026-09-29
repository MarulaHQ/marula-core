// Lane-6 vision verification driver — logs into the lane stack, exercises
// the rebuilt ticket workspace, and drops screenshots for review.
// Run: node scripts/lane6-verify.mjs
import { chromium } from "../node_modules/.pnpm/playwright-core@1.63.0/node_modules/playwright-core/index.mjs";

const BASE = "http://127.0.0.1:3002";
const TICKET = "6d657269-6469-616e-0000-000000000068"; // DEMO1004, in_progress, ransomware alert
const OUT = "design/qa-ticket-workspace";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function login(page) {
  await page.goto(`${BASE}/login`, { waitUntil: "networkidle" });
  await page.fill('input[type="email"], input[name="email"]', "owner@demo.local");
  await page.fill('input[type="password"], input[name="password"]', "demo-password-123");
  await page.click('button[type="submit"]');
  await page.waitForURL(/today|\/(?!login)/, { timeout: 15000 }).catch(() => {});
  await sleep(1500);
}

const shots = [];
async function shot(page, name) {
  const path = `${OUT}/${name}.png`;
  await page.screenshot({ path, fullPage: false });
  shots.push(name);
  console.log("shot:", name);
}

const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
const page = await ctx.newPage();

await login(page);
await page.goto(`${BASE}/tickets/${TICKET}`, { waitUntil: "networkidle" });
await sleep(2500);
await shot(page, "01-detail-light-1440");

// Status bar + transitions from the bar
await page.getByTestId("ticket-status-change").click();
await sleep(400);
await shot(page, "02-status-menu-light");

// Tabs
for (const tab of ["notes", "tasks", "details", "related", "original"]) {
  await page.keyboard.press("Escape");
  await page.getByTestId(`detail-tab-${tab}`).click();
  await sleep(500);
  await shot(page, `10-tab-${tab}`);
}

// Tasks tri-state
await page.getByTestId("detail-tab-tasks").click();
await sleep(300);
const taskInput = page.getByTestId("tab-task-input");
await taskInput.fill("Verify backups restored");
await taskInput.press("Enter");
await sleep(800);
const naBtn = page.getByTestId(/^tab-task-na-/).last();
await naBtn.click();
await sleep(800);
await shot(page, "11-tasks-tristate");

// Notes composer + visibility selector
await page.getByTestId("detail-tab-notes").click();
await page.getByTestId("notes-composer-body").fill("Line one of the diagnosis.\nLine two with the fix plan.\nLine three follow-up.");
await page.getByTestId("note-kind-client_visible").click();
await sleep(300);
await shot(page, "12-notes-composer");

// Close flow — walk the steps to the status screen
await page.getByTestId("validate-close").click();
await sleep(600);
await shot(page, "20-close-merge");
const finalNote = page.getByTestId("close-final-note");
if (await finalNote.isVisible()) {
  await page.getByTestId("close-next-time").click();
  await sleep(700);
  await shot(page, "21-close-time");
  await page.getByTestId("close-next-status").click();
  await sleep(500);
  await shot(page, "22-close-status");
  await page.getByRole("button", { name: /Cancel/i }).first().click();
  await sleep(400);
}

// Scheduling pod
await page.getByTestId("scheduling-type")?.waitFor({ timeout: 8000 }).catch(() => {});
await sleep(300);
await shot(page, "13-scheduling-pod");
const createBtn = page.getByTestId("scheduling-create");
if (await createBtn.isEnabled().catch(() => false)) {
  await createBtn.click();
  await sleep(1200);
  await shot(page, "14-scheduling-copy-once");
  await page.getByTestId("scheduling-copy")?.click();
  await sleep(300);
  await page.getByTestId("scheduling-copy-done")?.click();
  await sleep(400);
  await shot(page, "15-scheduling-links");
}

// Pod admin matrix + user arrange
await page.getByRole("button", { name: "Pod visibility" }).click();
await sleep(400);
await shot(page, "30-pod-admin");
await page.getByRole("button", { name: "Done" }).last().click();
await sleep(200);
await page.getByRole("button", { name: "Arrange pods" }).click();
await sleep(400);
await shot(page, "31-pod-arrange");
await page.getByRole("button", { name: "Done" }).last().click();

// Timeline filters
await page.getByTestId("timeline-filter-notes").click();
await sleep(400);
await shot(page, "40-timeline-filter-notes");
await page.getByTestId("timeline-filter-events").click();
await sleep(400);
await shot(page, "41-timeline-filter-events");
await page.getByTestId("timeline-filter-all").click();
await page.getByTestId("timeline-visibility-client_visible").click();
await sleep(400);
await shot(page, "42-timeline-filter-visible");

// Dark parity
await page.evaluate(() => localStorage.setItem("marula-theme", "dark"));
await page.reload({ waitUntil: "networkidle" });
await sleep(2500);
await shot(page, "50-detail-dark-1440");
await page.getByTestId("detail-tab-notes").click();
await sleep(500);
await shot(page, "51-notes-dark");
await page.getByTestId("detail-tab-tasks").click();
await sleep(500);
await shot(page, "52-tasks-dark");

// Responsive
await page.setViewportSize({ width: 1024, height: 800 });
await sleep(600);
await shot(page, "60-detail-dark-1024");
await page.setViewportSize({ width: 768, height: 900 });
await sleep(800);
const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
console.log("768 horizontal overflow px:", overflow);
await shot(page, "61-detail-dark-768");

await browser.close();
console.log(JSON.stringify({ shots }, null, 1));
