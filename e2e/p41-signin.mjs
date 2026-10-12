// Sign in on a fresh context and persist the session for the other p41 probes.
// The access token is memory-only, so a storage state round trip works only for
// the httpOnly cookie; each consumer therefore signs in on its own context.
import { chromium } from "playwright";
import { writeFileSync } from "node:fs";

const WEB = process.env.E2E_WEB_URL || "http://localhost:3002";
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";
const OUT = "design/qa-p41-ui-audit/captures/_storage.json";

const b = await chromium.launch();
const c = await b.newContext({ viewport: { width: 1600, height: 994 } });
const p = await c.newPage();

await p.goto(`${WEB.replace(/\/$/, "")}/login`, { waitUntil: "domcontentloaded" });
await p.waitForTimeout(3000);
await p.fill('input[type="email"]', EMAIL);
await p.fill('input[type="password"]', PASSWORD);
await p.locator('button[type="submit"]').click();
await p.waitForTimeout(9000);

const landed = new URL(p.url()).pathname;
console.log("landed:", landed, landed === "/login" ? "(FAILED)" : "(ok)");

await c.storageState({ path: OUT });
writeFileSync("design/qa-p41-ui-audit/captures/_port.txt", WEB);
console.log("storage ->", OUT);
await b.close();
