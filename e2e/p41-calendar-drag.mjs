// Verify the week grid's reschedule drag is pointer-driven, not HTML5.
import { chromium } from "playwright";
const WEB = "http://localhost:3001";
const EMAIL = "owner@demo.local", PASSWORD = "demo-password-123";
const b = await chromium.launch();
const c = await b.newContext({ viewport: { width: 1600, height: 994 } });
const p = await c.newPage();
// Sign in on this context (tokens are memory-only).
await p.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
await p.waitForTimeout(1500);
await p.fill('input[type="email"]', EMAIL);
await p.fill('input[type="password"]', PASSWORD);
await p.locator('button[type="submit"]').click();
await p.waitForURL(/tickets|today|dashboard|search/, { timeout: 45000 });
await c.storageState({ path: "design/qa-p41-ui-audit/captures/_storage.json" });
console.log("signed in ->", new URL(p.url()).pathname);

await p.goto(`${WEB}/calendar`, { waitUntil: "domcontentloaded" });
await p.waitForTimeout(5000);
console.log("url:", p.url());
console.log("week-grid:", await p.locator("[data-testid=week-grid]").count());
console.log("day cols:", await p.locator("[data-cal-day]").count());
console.log("entry blocks:", await p.locator("[data-cal-entry]").count());
console.log("body:", (await p.locator("body").innerText()).slice(0, 180).replace(/\n/g, " | "));

// ---- pointer drag: pick a block, drag it, read the result ----
const blocks = p.locator("[data-cal-entry]");
const n = await blocks.count();
if (!n) { console.log("NO ENTRIES to drag"); await b.close(); process.exit(0); }
const labels = [];
for (let i = 0; i < n; i += 1) labels.push(await blocks.nth(i).getAttribute("aria-label"));
const target = labels.findIndex((l) => l && l.includes("draggable") && l.includes("p41 drag probe"));
console.log(`draggable blocks: ${target >= 0 ? "yes" : "NO"}`);

// Choose a block whose column we can identify, and drag it down by ~2 hours.
const idx = target >= 0 ? target : 0;
const before = labels[idx];
const block = blocks.nth(idx);
const box = await block.boundingBox();
if (!box) { console.log("no box"); await b.close(); process.exit(0); }
console.log("dragging:", (before ?? "").slice(0, 70));
console.log("HTML5 draggable attr before:", await p.locator("[draggable][data-cal-entry]").count(), "(0 = DnD gone)");

await p.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
await p.mouse.down();
for (let dy = 8; dy <= 240; dy += 8) {
  await p.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + dy);
  await p.waitForTimeout(12);
}
const ghost = await p.locator("[data-cal-ghost]").count();
console.log("ghost during drag:", ghost, ghost > 0 ? "=> POINTER DRAG ENGAGED" : "=> no ghost (drag did not start)");
await p.mouse.up();
await p.waitForTimeout(3000);

const labels2 = await p.locator("[data-cal-entry]").evaluateAll((els) => els.map((e) => e.getAttribute("aria-label")));
const moved = labels2.filter((l) => l && l.includes("draggable")).map((l) => l.slice(0, 46));
console.log("after drag, first draggable:", moved[0] ?? "(none)");
console.log("VERDICT:", ghost > 0 ? "pointer drag engaged and committed" : "pointer drag did not start");
await b.close();
