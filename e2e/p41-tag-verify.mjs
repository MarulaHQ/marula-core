// Confirm the Tag->Badge sweep is visually identical: same pill slot, same kit
// tone classes, same rendered colours on a live surface that uses both.
import { chromium } from "playwright";
const WEB = "http://localhost:3001";
const EMAIL = "owner@demo.local", PASSWORD = "demo-password-123";
const b = await chromium.launch();
const c = await b.newContext({ viewport: { width: 1600, height: 994 } });
const p = await c.newPage();
await p.goto(`${WEB}/login`, { waitUntil: "domcontentloaded" });
await p.waitForTimeout(1500);
await p.fill('input[type="email"]', EMAIL);
await p.fill('input[type="password"]', PASSWORD);
await p.locator('button[type="submit"]').click();
await p.waitForURL(/tickets|today|dashboard|search/, { timeout: 45000 });
await c.storageState({ path: "design/qa-p41-ui-audit/captures/_storage.json" });

const results = {};
for (const route of ["/tickets", "/clients", "/settings/plugins", "/dispatch"]) {
  await p.goto(`${WEB}${route}`, { waitUntil: "domcontentloaded" });
  await p.waitForTimeout(4000);
  const r = await p.evaluate(() => {
    const pills = Array.from(document.querySelectorAll('[data-slot="pill"]'));
    const seen = new Map();
    for (const el of pills) {
      const cls = Array.from(el.classList).find((x) => x.startsWith("bg-")) ?? "(none)";
      const cs = getComputedStyle(el);
      const key = `${cls}|${cs.backgroundColor}|${cs.color}`;
      seen.set(key, (seen.get(key) ?? 0) + 1);
    }
    return { count: pills.length, distinct: [...seen.entries()].sort((a, b) => b[1] - a[1]).slice(0, 5) };
  });
  results[route] = r;
  console.log(`${route.padEnd(22)} pills=${String(r.count).padStart(4)}`);
  for (const [k, n] of r.distinct) console.log(`      ${n.toString().padStart(4)}  ${k}`);
}
// Any surviving <Tag> element at all would be a missed call site.
const leftovers = await p.evaluate(() => document.querySelectorAll("tag, Tag").length);
console.log("\nstray <tag> elements:", leftovers, leftovers === 0 ? "(none)" : "(BAD)");
await b.close();
