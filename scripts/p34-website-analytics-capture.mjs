// p34-verdicts-closeout claim 3 — prove the website's aggregate analytics
// off state against a REAL served page, not a unit test.
//
// The claim under test is a network claim: with no PostHog key configured,
// the marketing site makes zero third-party requests, and with a key it
// makes exactly one cookie-free beacon per pageview and sets no cookie. A
// unit test can only assert the functions; this asserts the browser.
//
// Usage:
//   node scripts/p34-website-analytics-capture.mjs <out-dir> [--on]
//
// Boots the site's own production build (next start) on a high port in both
// modes and records, per mode:
//   - every request the page made, split local vs third-party
//   - document.cookie before and after
//   - whether a beacon was sent, and its body
//
// `--on` builds with NEXT_PUBLIC_POSTHOG_KEY set. The capture host is
// pointed at a local sink so the "third-party request" is observable and the
// run makes no request to a real PostHog.
import { chromium } from 'playwright';
import { mkdir, writeFile } from 'fs/promises';
import { existsSync } from 'fs';
import { resolve } from 'path';
import { spawn } from 'child_process';
import { createServer } from 'http';

const OUT = resolve(process.argv[2] ?? 'design/qa-p34-verdicts-closeout/captures/analytics');
const MODE = process.argv.includes('--on') ? 'on' : 'off';
const SITE_PORT = MODE === 'on' ? 3457 : 3456;
const SINK_PORT = MODE === 'on' ? 3458 : 3459;
const SITE = `http://127.0.0.1:${SITE_PORT}`;

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

// --- the sink: stands in for PostHog so a beacon is visible, and no request
//     ever leaves the machine. Answers 200 with an empty body, which is
//     what PostHog's capture endpoint returns on success.
//
//     The CORS preflight is answered rather than ignored on purpose. A
//     beacon whose content type is not on the CORS safelist triggers an
//     OPTIONS, the OPTIONS is answered here, and the POST then succeeds
//     with an EMPTY body — the request shows up in the network log and
//     looks fine. So the sink also records the raw body and the verdict
//     below fails on an empty one, which is the only way this class of bug
//     ("the request fired, the payload did not") gets caught.
const beacons = [];
const sink = createServer((req, res) => {
  if (req.method === 'OPTIONS') {
    res.writeHead(204, {
      'access-control-allow-origin': req.headers.origin ?? '*',
      'access-control-allow-methods': 'POST, OPTIONS',
      'access-control-allow-headers': 'content-type',
      'access-control-max-age': '600',
    });
    res.end();
    return;
  }
  let body = '';
  req.on('data', (c) => { body += c; });
  req.on('end', () => {
    let parsed = null;
    let parseError = null;
    try { parsed = JSON.parse(body); } catch (e) { parseError = String(e); }
    beacons.push({
      url: req.url,
      method: req.method,
      contentType: req.headers['content-type'] ?? null,
      origin: req.headers.origin ?? null,
      bodyBytes: Buffer.byteLength(body),
      parsed,
      parseError,
      raw: body,
    });
    res.writeHead(200, {
      'content-type': 'application/json',
      'access-control-allow-origin': req.headers.origin ?? '*',
    });
    res.end('{}');
  });
});
await new Promise((r) => sink.listen(SINK_PORT, '127.0.0.1', r));

// --- serve the site
// `next dev`, not the standalone production bundle, and the reason is this
// environment rather than the claim:
//
//   - next.config.mjs sets `output: "standalone"`, and the standalone trace
//     picks its root by choosing between the several pnpm-lock.yaml files
//     visible from inside a git worktree. Here it picks the MAIN repo, so
//     the emitted standalone tree comes out empty and there is no server.js
//     to run.
//   - `next dev` reads NEXT_PUBLIC_* per request, so both modes run off one
//     server with no rebuild in between — which also means the off/on
//     comparison differs by exactly one environment variable and nothing
//     else.
//
// The claim under test is client-side network behaviour, identical in both
// servers. To assert the production bundle too, re-run this against
// `node .next/standalone/<pkg>/server.js` in the separate site package's
// checkout with a single lockfile (see deploy/preview/README.md for why
// worktrees trip this).
const env = {
  ...process.env,
  PORT: String(SITE_PORT),
  HOSTNAME: '127.0.0.1',
  NODE_ENV: 'development',
  NEXT_TELEMETRY_DISABLED: '1',
};
if (MODE === 'on') {
  env.NEXT_PUBLIC_POSTHOG_KEY = 'phc_capture_key_not_a_real_project';
  env.NEXT_PUBLIC_POSTHOG_HOST = `http://127.0.0.1:${SINK_PORT}`;
}
console.log(`[capture] starting next dev on ${SITE_PORT} (mode=${MODE})`);
const site = spawn('pnpm', ['exec', 'next', 'dev', '-p', String(SITE_PORT)], {
  cwd: resolve(import.meta.dirname, '../website'),
  env,
  stdio: ['ignore', 'pipe', 'pipe'],
});
let siteLog = '';
site.stdout.on('data', (d) => { siteLog += d; });
site.stderr.on('data', (d) => { siteLog += d; });

const cleanup = async () => {
  site.kill('SIGTERM');
  sink.close();
};
process.on('exit', () => { site.kill('SIGKILL'); });

// Wait for the server to answer before driving a browser at it.
let up = false;
for (let i = 0; i < 60; i++) {
  try {
    const r = await fetch(`${SITE}/`);
    if (r.ok) { up = true; break; }
  } catch { /* not listening yet */ }
  await new Promise((r) => setTimeout(r, 500));
}
if (!up) {
  console.error('[capture] site never came up:\n', siteLog.slice(-2000));
  await cleanup();
  process.exit(1);
}

const browser = await chromium.launch();
const result = { mode: MODE, site: SITE, sinkPort: SINK_PORT, pages: [] };

for (const path of ['/', '/compare', '/pricing']) {
  const context = await browser.newContext();
  const page = await context.newPage();
  const requests = [];
  page.on('request', (r) => requests.push({ url: r.url(), method: r.method(), resourceType: r.resourceType() }));

  await page.goto(`${SITE}${path}`, { waitUntil: 'networkidle' });
  // Give the effect a beat, then settle.
  await page.waitForTimeout(600);

  const cookies = await context.cookies();
  const localStorageKeys = await page.evaluate(() => Object.keys(localStorage));
  const sessionStorageKeys = await page.evaluate(() => Object.keys(sessionStorage));
  const documentCookie = await page.evaluate(() => document.cookie);

  const thirdParty = requests.filter((r) => {
    try { return new URL(r.url).host !== `127.0.0.1:${SITE_PORT}`; } catch { return true; }
  });

  result.pages.push({
    path,
    requestCount: requests.length,
    thirdPartyRequests: thirdParty,
    beacons: thirdParty.filter((r) => r.resourceType === 'other' || r.method === 'POST'),
    cookiesSet: cookies.map((c) => ({ name: c.name, domain: c.domain, httpOnly: c.httpOnly })),
    documentCookie,
    localStorageKeys,
    sessionStorageKeys,
  });
  await context.close();
}

await browser.close();
result.beaconsReceived = beacons;

await writeFile(resolve(OUT, `mode-${MODE}.json`), JSON.stringify(result, null, 2));

// --- the verdict, stated as the row's claim -------------------------------
const problems = [];
for (const p of result.pages) {
  if (MODE === 'off' && p.thirdPartyRequests.length > 0) {
    problems.push(`${p.path}: off mode made ${p.thirdPartyRequests.length} third-party request(s): ${p.thirdPartyRequests.map((r) => r.url).join(', ')}`);
  }
  if (p.cookiesSet.length > 0) problems.push(`${p.path}: set ${p.cookiesSet.length} cookie(s) — the cookie-free claim is broken`);
  // A cookie can exist without being in the jar the context reports (a
  // session cookie written and already expired, or one the driver filtered),
  // so document.cookie is asserted separately.
  if (p.documentCookie !== '') problems.push(`${p.path}: document.cookie is non-empty ("${p.documentCookie}")`);

  if (MODE === 'on' && p.thirdPartyRequests.length !== 1) {
    problems.push(`${p.path}: on mode made ${p.thirdPartyRequests.length} third-party request(s), expected exactly 1`);
  }
}

// The payload checks, against what the SINK actually received rather than
// what the browser reported sending. A beacon can be counted as a request
// and still deliver nothing — that is the failure that reached production
// code in this lane once already, and counting requests cannot see it.
if (MODE === 'on') {
  if (beacons.length !== result.pages.length) {
    problems.push(`sink received ${beacons.length} beacon(s) for ${result.pages.length} page(s)`);
  }
  for (const [i, b] of beacons.entries()) {
    if (b.bodyBytes === 0) {
      problems.push(`beacon ${i} arrived with an EMPTY body — the request fired, the payload did not (a non-CORS-safelisted beacon content type does exactly this)`);
    }
    if (b.parseError !== null) {
      problems.push(`beacon ${i} body is not JSON: ${b.parseError}`);
    }
    if (b.contentType !== null && !/^text\/plain/i.test(b.contentType)) {
      problems.push(`beacon ${i} content-type is "${b.contentType}", which is not CORS-safelisted — the browser drops such a payload`);
    }
  }
  // The identity claims, asserted on the real payload.
  for (const b of beacons) {
    const props = b.parsed?.properties ?? {};
    const banned = ['$current_url', '$referrer', '$referring_domain', 'distinct_id', '$user_id', 'email'];
    for (const key of banned) {
      if (key in props) problems.push(`beacon payload carries "${key}" — it must not be present under any circumstance`);
    }
    if (props.$process_person_profile !== false) {
      problems.push('beacon payload does not set $process_person_profile: false');
    }
    if (typeof props.pathname === 'string' && /[?#]/.test(props.pathname)) {
      problems.push(`beacon payload pathname carries a query string or fragment: "${props.pathname}"`);
    }
  }
}
console.log(`\n[capture] mode=${MODE}`);
for (const p of result.pages) {
  console.log(`  ${p.path.padEnd(12)} requests=${p.requestCount} third-party=${p.thirdPartyRequests.length} cookies=${p.cookiesSet.length} localStorage=${JSON.stringify(p.localStorageKeys)} sessionStorage=${JSON.stringify(p.sessionStorageKeys)}`);
}
if (MODE === 'on' && beacons.length > 0) {
  console.log(`  beacon content-type: ${beacons[0]?.contentType}  bytes: ${beacons[0]?.bodyBytes}`);
  console.log('  beacon payload:', JSON.stringify(beacons[0]?.parsed, null, 2).split('\n').map((l) => '    ' + l).join('\n'));
}
if (problems.length > 0) {
  console.error('\n[capture] CLAIM VIOLATIONS:');
  for (const p of problems) console.error('  ✗ ' + p);
  await cleanup();
  process.exit(1);
}
console.log(`\n[capture] claims hold in mode=${MODE} → ${resolve(OUT, `mode-${MODE}.json`)}`);
await cleanup();
process.exit(0);
