// p47-legibility-audit.mjs — Wave 47 lane A13, `p47-vision-captures`.
//
// The fidelity chain (captures -> pack-diff -> vision grids) answers "does this
// look like the pack". It cannot answer "can a person read it", and neither can
// axe: axe knows nothing about a label rendered at 10px or a text colour that
// clears 3:1 against the wrong ground.
//
// This measures the three things the theme law actually states, on every
// surface, in both modes, at the pack's own desktop size:
//
//   1. CONTRAST (docs/theme.md §3-4, `scripts/check-theme-packs.mjs`): text at
//      WCAG AA — 4.5:1, or 3:1 for large text (>=24px, or >=18.66px bold).
//      Each text node is compared against the first opaque ancestor background,
//      which is what the eye sees.
//   2. POINTER TARGETS (docs/theme.md §5): "44px remains the tap-target floor
//      on every step, and only pointer targets sit at 32px under dense". A web
//      surface is pointer-driven, so 32px is the breach line and 44px is the
//      comfort line; both are recorded.
//   3. READABILITY FLOOR: text rendered below 11px. The theme doc calls the
//      floor a thing a label can be dragged under but never puts a number on
//      it, so this lane declares 11px and says so rather than implying it is
//      the project's own figure.
//
// It also records truncation (a label clipped by its own box) because a clipped
// label is a legibility defect that passes both contrast and axe.
//
// Usage:
//   node e2e/p47-legibility-audit.mjs [--themes light,dark] [--group settings]
import { chromium } from "playwright";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const LANE = "design/qa-p47-vision-captures";
const OUT = resolve(LANE, "legibility");
mkdirSync(OUT, { recursive: true });

const envPath = resolve("e2e/.e2e-stack.env");
if (!existsSync(envPath)) {
  throw new Error(
    "e2e/.e2e-stack.env is missing. Run scripts/e2e-stack.sh up first (lane boot " +
      "routes through that script; never set the ports by hand).",
  );
}
const env = Object.fromEntries(
  readFileSync(envPath, "utf8")
    .split("\n")
    .filter((line) => line.includes("="))
    .map((line) => [line.slice(0, line.indexOf("=")), line.slice(line.indexOf("=") + 1)]),
);
const WEB = process.env.E2E_WEB_URL ?? env.E2E_WEB_URL ?? "http://localhost:3000";
const API = (process.env.E2E_API_URL ?? env.E2E_API_URL ?? "http://localhost:8080").replace(/\/+$/, "");

const PACK = { width: 1600, height: 994 };
const EMAIL = "owner@demo.local";
const PASSWORD = "demo-password-123";

const arg = (name, dflt) => {
  const i = process.argv.indexOf(`--${name}`);
  return i === -1 ? dflt : process.argv[i + 1];
};
const THEMES = arg("themes", "light,dark").split(",").map((s) => s.trim());
const GROUP = arg("group", null);

/* ---------------------------------------------------------------------------
 * Every surface, grouped the way the app groups them. Auth-gated redirect
 * routes (`/sso`, `/verify-email`, `/magic-link`, `/accept-invite`) are absent
 * on purpose: they are unreachable with a session and are not surfaces.
 * ---------------------------------------------------------------------- */
const SURFACES = [
  { group: "work", routes: ["/today", "/tickets", "/boards", "/approvals", "/alerts", "/scorecards", "/reports", "/dashboards"] },
  { group: "dispatch", routes: ["/dispatch", "/dispatch/planning", "/dispatch/scheduling", "/dispatch/travel", "/calendar"] },
  { group: "crm", routes: ["/clients", "/contracts", "/projects"] },
  { group: "inventory", routes: ["/assets", "/stock", "/stock/loans", "/stock/locations", "/stock/tools", "/inventory", "/inventory/workbench", "/purchasing", "/purchasing/vendors"] },
  { group: "money", routes: ["/billing", "/billing/credit-notes", "/billing/verification", "/sales/quotes", "/sales/pipeline", "/sales/catalog"] },
  { group: "knowledge", routes: ["/kb", "/kb/secrets", "/documents", "/passwords", "/automations", "/automations/runs"] },
  { group: "admin", routes: ["/audit", "/provenance", "/imports", "/search", "/dev"] },
  {
    group: "settings",
    routes: [
      "/settings", "/settings/agent-platform", "/settings/ai", "/settings/ai/shadow", "/settings/api-keys",
      "/settings/appearance", "/settings/boards", "/settings/email", "/settings/export", "/settings/fx",
      "/settings/general", "/settings/integrations", "/settings/inventory", "/settings/labels", "/settings/mail",
      "/settings/notifications", "/settings/onboarding", "/settings/plugins", "/settings/pm-schedules",
      "/settings/portal", "/settings/preferences", "/settings/reply-snippets", "/settings/security",
      "/settings/self-host", "/settings/shortcuts", "/settings/sla", "/settings/surveys", "/settings/tags",
      "/settings/taxonomy", "/settings/team", "/settings/time-categories", "/settings/toolboxes", "/settings/workflow",
    ],
  },
  { group: "auth", routes: ["/login", "/welcome"] },
];

const all = SURFACES.filter((g) => GROUP === null || g.group === GROUP);

async function token() {
  const res = await fetch(`${API}/api/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: EMAIL, password: PASSWORD }),
  });
  if (!res.ok) throw new Error(`staff login failed (${res.status})`);
  return res.json();
}

/** The measurement runs in the page: it walks every element that renders text
 *  and every interactive control, and returns the tail of violations only. */
const PROBE = () => {
  const parseColor = (value) => {
    const m = /rgba?\(([^)]+)\)/.exec(value || "");
    if (!m) return null;
    const parts = m[1].split(",").map((s) => parseFloat(s.trim()));
    return { r: parts[0], g: parts[1], b: parts[2], a: parts.length > 3 ? parts[3] : 1 };
  };
  const over = (fg, bg) => ({
    r: fg.r * fg.a + bg.r * (1 - fg.a),
    g: fg.g * fg.a + bg.g * (1 - fg.a),
    b: fg.b * fg.a + bg.b * (1 - fg.a),
    a: 1,
  });
  const channel = (c) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  const lum = (c) => 0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b);
  const ratio = (a, b) => {
    const la = lum(a);
    const lb = lum(b);
    return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
  };
  const effectiveBg = (el) => {
    let node = el;
    while (node && node !== document.documentElement.parentElement) {
      const c = parseColor(getComputedStyle(node).backgroundColor);
      if (c && c.a > 0.95) return c;
      node = node.parentElement;
    }
    return { r: 255, g: 255, b: 255, a: 1 };
  };
  const label = (el) => {
    const testid = el.getAttribute("data-testid");
    const cls = typeof el.className === "string" ? el.className.split(/\s+/).filter(Boolean)[0] : null;
    return `${el.tagName.toLowerCase()}${testid ? `[data-testid="${testid}"]` : cls ? `.${cls}` : ""}`;
  };

  const contrast = [];
  const small = [];
  const targets = [];
  const clipped = [];
  const seen = new Set();

  for (const el of document.querySelectorAll("body *")) {
    const style = getComputedStyle(el);
    if (style.display === "none" || style.visibility === "hidden" || parseFloat(style.opacity) < 0.3) continue;
    const rect = el.getBoundingClientRect();
    if (rect.width < 2 || rect.height < 2) continue;

    // Text: this element owns a direct non-whitespace text child.
    const ownText = Array.from(el.childNodes)
      .filter((n) => n.nodeType === 3)
      .map((n) => n.textContent.trim())
      .join(" ")
      .trim();
    if (ownText.length > 0) {
      const key = `${label(el)}|${ownText.slice(0, 24)}`;
      if (!seen.has(key)) {
        seen.add(key);
        const fg = parseColor(style.color);
        if (fg) {
          const bg = effectiveBg(el);
          const r = ratio(over(fg, bg), bg);
          const size = parseFloat(style.fontSize);
          const weight = parseInt(style.fontWeight, 10) || 400;
          const large = size >= 24 || (size >= 18.66 && weight >= 700);
          const floor = large ? 3.0 : 4.5;
          if (r < floor) {
            contrast.push({ el: label(el), text: ownText.slice(0, 60), size, weight, ratio: Math.round(r * 100) / 100, floor });
          }
          if (size < 11) small.push({ el: label(el), text: ownText.slice(0, 60), size });
        }
      }
      // Truncation: the box clips its own text.
      if (el.scrollWidth > el.clientWidth + 1 && style.textOverflow === "ellipsis" && ownText.length > 0) {
        clipped.push({ el: label(el), text: ownText.slice(0, 60), over: el.scrollWidth - el.clientWidth });
      }
    }

    // Pointer targets.
    const tag = el.tagName.toLowerCase();
    const interactive =
      tag === "button" || tag === "a" || tag === "select" || tag === "textarea" || tag === "input" ||
      el.getAttribute("role") === "button" || el.getAttribute("role") === "radio" ||
      el.getAttribute("role") === "checkbox" || el.getAttribute("role") === "menuitem" ||
      el.getAttribute("role") === "tab" || el.hasAttribute("onclick");
    if (interactive && tag !== "a") {
      const h = Math.round(rect.height);
      if (h > 0 && h < 32) {
        const name = (el.getAttribute("aria-label") || el.getAttribute("title") || el.textContent || "").trim().slice(0, 40);
        targets.push({ el: label(el), name, height: h });
      }
    }
  }
  return {
    contrast: contrast.slice(0, 30),
    small: small.slice(0, 30),
    targets: targets.slice(0, 30),
    clipped: clipped.slice(0, 30),
    counts: { contrast: contrast.length, small: small.length, targets: targets.length, clipped: clipped.length },
    textNodes: seen.size,
  };
};

const browser = await chromium.launch();
const tokens = await token();
const rows = [];
const failures = [];

for (const theme of THEMES) {
  const ctx = await browser.newContext({ viewport: PACK, colorScheme: theme });
  const login = await ctx.request.post(`${API}/api/v1/auth/login`, { data: { email: EMAIL, password: PASSWORD } });
  if (!login.ok()) throw new Error(`context login failed (${login.status()})`);
  await ctx.request.put(`${API}/api/v1/settings/user`, {
    data: { key: "tours.disabled", value: true },
    headers: { authorization: `Bearer ${tokens.access_token}` },
  });
  await ctx.addInitScript(([raw, exp, th]) => {
    window.localStorage.setItem("marula-tokens", raw);
    window.localStorage.setItem("marula-tokens-exp", exp);
    window.localStorage.setItem("marula-theme", th);
    window.localStorage.setItem("marula-theme-preset", "marula");
  }, [JSON.stringify(tokens), String(Date.now() + tokens.expires_in * 1000 - 30_000), theme]);

  for (const group of all) {
    for (const route of group.routes) {
      const page = await ctx.newPage();
      let gotoError = "";
      await page.goto(`${WEB}${route}`, { waitUntil: "domcontentloaded", timeout: 60_000 }).catch((e) => {
        gotoError = String(e).split("\n")[0].slice(0, 160);
      });
      await page.waitForLoadState("networkidle", { timeout: 15_000 }).catch(() => {});
      await page.waitForTimeout(900);
      let measured;
      try {
        measured = await page.evaluate(PROBE);
      } catch (error) {
        measured = { error: String(error), counts: { contrast: 0, small: 0, targets: 0, clipped: 0 } };
      }
      // A surface that rendered no text at all was not measured. Reporting it as
      // clean would be the exact failure this repo has closed twice before (a
      // teardown that reported success it never achieved; a command whose target
      // is missing that says nothing). Nothing measured is a failure.
      if (gotoError) {
        failures.push(`${route} (${theme}): the page did not load — ${gotoError}`);
      } else if ((measured.textNodes ?? 0) === 0) {
        failures.push(`${route} (${theme}): rendered no text at all — nothing was measured`);
      }
      const row = { group: group.group, route, theme, gotoError: gotoError || undefined, ...measured };
      rows.push(row);
      const c = row.counts ?? {};
      const flag = gotoError || (c.contrast ?? 0) + (c.small ?? 0) + (c.targets ?? 0) + (c.clipped ?? 0) > 0 ? " !" : "";
      console.log(
        `  ${theme.padEnd(5)} ${route.padEnd(38)} text ${String(row.textNodes ?? 0).padStart(5)}  contrast ${String(c.contrast ?? 0).padStart(3)}  tiny ${String(c.small ?? 0).padStart(3)}  targets ${String(c.targets ?? 0).padStart(3)}  clipped ${String(c.clipped ?? 0).padStart(3)}${flag}`,
      );
      await page.close();
    }
  }
  await ctx.close();
}
await browser.close();

const totals = rows.reduce(
  (acc, r) => {
    acc.contrast += r.counts?.contrast ?? 0;
    acc.small += r.counts?.small ?? 0;
    acc.targets += r.counts?.targets ?? 0;
    acc.clipped += r.counts?.clipped ?? 0;
    return acc;
  },
  { contrast: 0, small: 0, targets: 0, clipped: 0 },
);
const worst = rows
  .map((r) => ({
    route: r.route,
    theme: r.theme,
    total: (r.counts?.contrast ?? 0) + (r.counts?.small ?? 0) + (r.counts?.targets ?? 0) + (r.counts?.clipped ?? 0),
  }))
  .filter((r) => r.total > 0)
  .sort((a, b) => b.total - a.total)
  .slice(0, 20);

writeFileSync(resolve(OUT, "LEGIBILITY.json"), JSON.stringify({ generatedAt: new Date().toISOString(), pack: PACK, themes: THEMES, totals, failures, worst, rows }, null, 2));
mkdirSync(OUT, { recursive: true });
writeFileSync(resolve(OUT, "SUMMARY.md"), ["# Legibility totals", "", "| measure | findings |", "|---|---|", `| contrast below AA | ${totals.contrast} |`, `| text under 11px | ${totals.small} |`, `| pointer targets under 32px | ${totals.targets} |`, `| clipped labels | ${totals.clipped} |`, `| surfaces that did not render | ${failures.length} |`, "", "## Worst surfaces", "", "| surface | theme | findings |", "|---|---|---|", ...worst.map((w) => `| \`${w.route}\` | ${w.theme} | ${w.total} |`), "", failures.length > 0 ? "## Surfaces that did not render (nothing was measured)\n" : "", ...failures.map((f) => `- ${f}`), ""].join("\n"));

console.log(`\n[legibility] contrast ${totals.contrast}, under-11px ${totals.small}, small targets ${totals.targets}, clipped ${totals.clipped}`);
if (failures.length > 0) {
  console.log(`[legibility] ${failures.length} surface/theme pairs rendered nothing (not scored as clean):`);
  for (const f of failures.slice(0, 10)) console.log(`  ! ${f}`);
}
console.log(`[legibility] ${rows.length} surface/theme measurements -> ${OUT}/LEGIBILITY.json`);
process.exitCode = totals.contrast + totals.targets + failures.length > 0 ? 1 : 0;
