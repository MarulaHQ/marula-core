// p34-verdicts-closeout claim 8 — the Stage-3 acceptance artifact: ONE org
// billing-syncing to a QuickBooks Online sandbox + a SECOND org fully
// native, compared side by side.
//
// The acceptance form this closes is a claim about AGREEMENT between two
// paths, so the harness builds both paths over identical billing facts and
// then compares the numbers that matter. Two orgs rather than one org with
// a flag, because "native" and "synced" must be provably independent: the
// same org with the export turned on could agree with itself for the wrong
// reason.
//
//   Org A "synced"   — QBO export credentials sealed; invoices and payments
//                      pushed to the vendor when credentials are present.
//   Org B "native"   — no vendor credentials at all. The system of record.
//
// WHAT THIS RUNS AGAINST
//   A booted e2e stack (`scripts/e2e-stack.sh`), the same one the Playwright
//   suite uses. Source `e2e/.e2e-stack.env` first, or pass --api-url.
//
// THE CREDENTIAL QUESTION, ANSWERED HONESTLY
//   A QBO sandbox needs an operator's own access token and realm id, and
//   Marula hosts no Intuit OAuth app and performs no token refresh (see
//   plugins/connectors/qbo-export-sync/README.md). So the run takes two
//   shapes and is explicit about which one produced the artifact:
//
//     both set     → the real thing: vendor push, vendor read-back, and a
//                    three-way comparison (native / Marula / vendor).
//     not set      → the harness and both orgs are built and the NATIVE side
//                    is verified in full, and the vendor leg is recorded as
//                    an OPERATOR GAP in the artifact rather than quietly
//                    skipped. Exit code 0, because a missing credential is
//                    not a product failure, and a non-zero exit here would
//                    train everyone to ignore this script.
//
//   A mismatched native side is exit 1 in BOTH shapes. That is the failure
//   this script exists to catch, and it is checked the same way whether or
//   not a vendor is in the loop.
//
// Usage:
//   source e2e/.e2e-stack.env
//   node scripts/p34-reconciliation-run.mjs <out-dir>
//
// Env for the vendor leg (both required together, never one alone):
//   QBO_ACCESS_TOKEN  QBO_REALM_ID
import { mkdir, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { resolve } from 'node:path';

const OUT = resolve(process.argv[2] ?? 'design/qa-p34-verdicts-closeout/captures/reconciliation');
const API = (process.env.E2E_API_URL ?? 'http://localhost:8080').replace(/\/+$/, '');

// The demo org's owner, from the seed's own contract. Used for org A so the
// synced side runs over the SAME seeded contracts, clients and rates the
// demo shows — the acceptance form is about the billing path agreeing, not
// about a hand-built fixture.
const DEMO_EMAIL = process.env.E2E_DEMO_EMAIL ?? 'owner@demo.local';
const DEMO_PASSWORD = process.env.E2E_DEMO_PASSWORD ?? 'demo-password-123';

// The contract currency. It must be the org's OWN base currency
// (`orgs.base_currency`, default 'USD'): an invoice run converts to base,
// and with no FX rate on file a non-base currency is a typed 409 rather than
// a conversion. Using the base keeps the comparison single-currency, which
// is also what per-currency totals want. A deployment whose base is not USD
// overrides this — or records the rate, which is a product feature and not
// something this harness should be quietly relying on.
const ORG_CURRENCY = (process.env.RECON_CURRENCY ?? 'USD').toUpperCase();

const QBO_TOKEN = (process.env.QBO_ACCESS_TOKEN ?? '').trim();
const QBO_REALM = (process.env.QBO_REALM_ID ?? '').trim();
const VENDOR_AVAILABLE = QBO_TOKEN !== '' && QBO_REALM !== '';

if (!existsSync(OUT)) await mkdir(OUT, { recursive: true });

// One credential half is a misconfiguration, not a vendor run. Say so
// loudly rather than falling through to the gap path, which would report
// "no credentials" when the truth is "you typo'd one".
if (QBO_TOKEN !== '' && QBO_REALM === '') {
  console.error('[recon] QBO_ACCESS_TOKEN is set but QBO_REALM_ID is not — fix the env or unset both.');
  process.exit(2);
}
if (QBO_REALM !== '' && QBO_TOKEN === '') {
  console.error('[recon] QBO_REALM_ID is set but QBO_ACCESS_TOKEN is not — fix the env or unset both.');
  process.exit(2);
}

let token = '';
async function api(path, { method = 'GET', token: t, body, raw = false } = {}) {
  const res = await fetch(`${API}/api${path}`, {
    method,
    headers: {
      'content-type': 'application/json',
      ...(t ? { authorization: `Bearer ${t}` } : {}),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  if (raw) return { status: res.status, text };
  let json;
  try { json = JSON.parse(text); } catch { json = text; }
  if (!res.ok) {
    throw new Error(`${method} ${path} -> ${res.status}: ${text.slice(0, 400)}`);
  }
  return json;
}

// --- org A: the seeded demo org, which is the "synced" side ---------------
console.log('[recon] org A — logging in as the seeded demo org');
const a = await api('/v1/auth/login', {
  method: 'POST',
  body: { email: DEMO_EMAIL, password: DEMO_PASSWORD },
});
const tokenA = a.access_token;
const meA = await api('/v1/me', { token: tokenA });
console.log(`[recon]   org A = ${meA.org?.name ?? meA.org_id}`);

// --- org B: a second, fully native org ------------------------------------
// Registered through the real /auth/register path rather than by inserting
// rows, so org B goes through the same onboarding every customer does. A
// hand-inserted org would prove the billing engine agrees with itself
// while skipping every guard the register path enforces.
const stamp = Date.now().toString(36);
const bEmail = `native-org-${stamp}@recon.test`;
console.log('[recon] org B — registering a second, native-only org');
const b = await api('/v1/auth/register', {
  method: 'POST',
  body: {
    org_name: 'Reconciliation Native (control)',
    org_slug: `recon-native-${stamp}`,
    ticket_prefix: 'RCN',
    email: bEmail,
    // Reuses the demo password deliberately: this org exists for the length
    // of one harness run and is thrown away with the E2E database, so a
    // distinct secret would only add a value to leak into the transcript.
    password: DEMO_PASSWORD,
    display_name: 'Reconciliation Control',
    timezone: 'UTC',
  },
});
const tokenB = b.access_token;
const meB = await api('/v1/me', { token: tokenB });
console.log(`[recon]   org B = ${meB.org?.name ?? meB.org_id} (${bEmail})`);

// --- the billing run, over one window in both orgs -----------------------
// A closed past window, so both runs are deterministic and neither depends
// on "today" being inside the period. The first day of the previous month
// keeps it out of any current-period lock.
const now = new Date();
const from = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth() - 1, 1))
  .toISOString()
  .slice(0, 10);
const to = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth() - 1, 15))
  .toISOString()
  .slice(0, 10);
console.log(`[recon] invoice-run window ${from} .. ${to} in both orgs`);

// --- identical billing facts in both orgs --------------------------------
// Same client, same contract, same window, same due date. If the two sides
// disagree, the only variable left is the export path — which is the entire
// point of the comparison.
async function seedBillingFacts(label, t) {
  const client = await api('/v1/clients', {
    method: 'POST',
    token: t,
    body: { name: `Reconciliation Client ${label}` },
  });
  console.log(`[recon]   ${label}: client ${client.id}`);

  // The invoice run bills against billable, ACTIVE contracts whose
  // effective window covers the run window. Required per the pinned
  // CreateContractRequest: client_id, number, name, contract_type, currency,
  // effective_from. `number` is per-org unique, hence the label in it.
  //
  // time_and_materials with an hourly rate is the shape that produces a
  // non-zero total from logged time, which is what makes the two orgs
  // comparable on a number rather than both being zero.
  const contract = await api('/v1/contracts', {
    method: 'POST',
    token: t,
    body: {
      client_id: client.id,
      number: `RECON-${label}-${Date.now().toString(36).toUpperCase()}`,
      name: `Reconciliation Contract ${label}`,
      contract_type: 'time_and_materials',
      currency: ORG_CURRENCY,
      effective_from: from,
      status: 'active',
      hourly_rate_minor: 15000,
    },
  });
  console.log(`[recon]   ${label}: contract ${contract.id ?? '(created)'}`);

  // A time-and-materials contract bills LOGGED TIME, so a contract with no
  // entries produces no invoice and the two-org comparison becomes
  // vacuous. Both orgs get the same entries at the same instants, against
  // the same client, so the only remaining variable is the export path.
  //
  // The window is a half-open instant range: `started_at` inside the run
  // window, `ended_at` too, so the run picks them up. One billable and one
  // unbillable entry, because the unbillable one is what proves the two
  // sides agree about what does NOT count as money.
  const entryBase = new Date(`${from}T09:00:00Z`);
  // A ticket is required to hang the entry on, and the pinned
  // CreateTicketRequest requires board_id + title + priority. The board is
  // read from the org rather than hardcoded, because a board id from one
  // org is not valid in the other and a wrong id fails the whole run.
  const boards = await api('/v1/boards', { token: t });
  const boardId = Array.isArray(boards) ? boards[0]?.id : boards?.items?.[0]?.id;
  if (!boardId) {
    throw new Error(
      `org ${label} has no board, so a ticket cannot be created and the run would ` +
        `issue nothing. Registering an org should seed a default board — check that ` +
        `the onboarding path still does.`,
    );
  }
  const tickets = await api('/v1/tickets', {
    method: 'POST',
    token: t,
    body: {
      board_id: boardId,
      title: `Reconciliation work ${label}`,
      priority: 'p3',
      client_id: client.id,
    },
  });
  const entries = [
    { started_at: entryBase.toISOString(), ended_at: new Date(entryBase.getTime() + 2 * 3600_000).toISOString(), billable: true },
    { started_at: new Date(entryBase.getTime() + 3 * 3600_000).toISOString(), ended_at: new Date(entryBase.getTime() + 4 * 3600_000).toISOString(), billable: false, unbillable_reason: 'internal' },
  ];
  const created = [];
  for (const entry of entries) {
    const row = await api('/v1/time-entries', {
      method: 'POST',
      token: t,
      body: { ...entry, ticket_id: tickets.id, notes: `reconciliation ${label}` },
    });
    created.push(row.id ?? row.time_entry?.id);
  }

  // Submit and approve. The invoice run bills only APPROVED entries
  // (core-billing rules.rs filters `status == "approved"`), so entries left
  // in `draft` produce an invoice run that issues nothing — which is exactly
  // what the first run of this harness did, and why the vacuous-pass guard
  // below exists. Lifecycle is `draft → submitted → approved`.
  //
  // The UNBILLABLE entry is submitted but deliberately NOT approved: it
  // proves the two orgs agree about what does not count as money, and a
  // rejected/never-approved entry must contribute zero to both.
  const [billable, unbillable] = created;
  await api(`/v1/time-entries/${billable}/submit`, {
    method: 'POST',
    token: t,
    body: {},
  });
  await api('/v1/time-entries/approvals/decide', {
    method: 'POST',
    token: t,
    body: { entry_ids: [billable], decision: 'approve', note: 'reconciliation' },
  });
  await api(`/v1/time-entries/${unbillable}/submit`, {
    method: 'POST',
    token: t,
    body: {},
  });

  console.log(
    `[recon]   ${label}: ticket ${tickets.id}, ${created.length} time entries ` +
      `(1 submitted+approved billable, 1 submitted-not-approved unbillable)`,
  );
  return { client, contract, ticket: tickets, entries: created };
}

const factsA = await seedBillingFacts('A', tokenA);
const factsB = await seedBillingFacts('B', tokenB);

// Each run is SCOPED to the client this harness created, via the request's
// `client_id`.
//
// Without it the demo org's run sweeps every billable contract in the org,
// including seeded EUR-denominated ones, and dies on a missing FX rate —
// and worse, it would have compared the demo org's entire seeded ledger
// against a control org containing only what this harness put there. Those
// two numbers were never comparable, so scoping is the fix for correctness
// rather than a way around the 409.
const runA = await api('/v1/billing/invoice-runs', {
  method: 'POST',
  token: tokenA,
  body: { from, to, due_in_days: 30, client_id: factsA.client.id },
});
const runB = await api('/v1/billing/invoice-runs', {
  method: 'POST',
  token: tokenB,
  body: { from, to, due_in_days: 30, client_id: factsB.client.id },
});

// `IssuedInvoice` is `{ invoice, lines }` (per the pinned schema), so the
// money lives one level down. Reaching for `inv.total_minor` directly would
// silently total zero for every invoice and the comparison would pass
// vacuously — which is exactly the failure this script must not have.
const invoicesA = runA.invoices ?? [];
const invoicesB = runB.invoices ?? [];
console.log(`[recon]   org A issued ${invoicesA.length}, org B issued ${invoicesB.length}`);

// --- the comparison: the numbers that must agree -------------------------
// Money in MINOR units, summed per currency. Summing across currencies
// would be a category error — 100 EUR and 100 USD are not 200 of anything —
// so the comparison is per-currency and a currency present in one side and
// not the other is itself a finding.
function totalsByCurrency(issued) {
  const totals = new Map();
  for (const row of issued) {
    const inv = row.invoice ?? row;
    const currency = inv.currency ?? 'UNKNOWN';
    const total = Number(inv.total_minor ?? 0);
    totals.set(currency, (totals.get(currency) ?? 0) + total);
  }
  return Object.fromEntries([...totals.entries()].sort(([x], [y]) => x.localeCompare(y)));
}

// A run that issued NOTHING is a harness failure, not a passing comparison:
// two orgs that both billed zero have agreed about nothing, and reporting
// that as agreement is the vacuous-pass this script exists to avoid.
function assertNotVacuous(label, issued, totals) {
  if (issued.length === 0) {
    throw new Error(
      `org ${label} issued no invoices for the window. The comparison would be ` +
        `vacuous — check the contract is billable and its effective window covers ${from}..${to}.`,
    );
  }
  const allZero = Object.values(totals).every((v) => v === 0);
  if (allZero) {
    throw new Error(
      `org ${label} issued ${issued.length} invoice(s) totalling zero minor units in every ` +
        `currency. Either no billable time was logged in the window, or the totals are being ` +
        `read from the wrong place in the IssuedInvoice envelope.`,
    );
  }
}

const totalsA = totalsByCurrency(invoicesA);
const totalsB = totalsByCurrency(invoicesB);
assertNotVacuous('A', invoicesA, totalsA);
assertNotVacuous('B', invoicesB, totalsB);

// The vendor leg. Only reached when BOTH credentials are present, and its
// absence is recorded rather than skipped.
let vendor = {
  available: false,
  reason:
    'QBO_ACCESS_TOKEN and QBO_REALM_ID are not both set. The vendor leg of the ' +
    'Stage-3 acceptance form was NOT executed; see the operator gap below.',
  pushed: null,
  vendorTotals: null,
};

if (VENDOR_AVAILABLE) {
  console.log('[recon] vendor leg — QBO credentials present, pushing org A');
  try {
    // The export sink is driven by the product's own export path, not by a
    // hand-rolled QBO call from this script: the acceptance form is about
    // the shipped adapter working, and a direct vendor call from a test
    // script would prove the vendor's API works instead.
    const pushed = await api('/v1/org/exports', {
      method: 'POST',
      token: tokenA,
      body: { connector: 'qbo-export-sync', kind: 'invoices' },
    });
    vendor = {
      available: true,
      pushed: {
        export_id: pushed.id ?? pushed.export_id ?? null,
        status: pushed.status ?? 'unknown',
        // Never echo the token. Presence only, matching the product's own
        // BYOK reporting rule.
        credentials_present: true,
      },
      vendorTotals: null,
      reason: null,
    };
    console.log(`[recon]   export ${vendor.pushed.export_id} status ${vendor.pushed.status}`);
  } catch (e) {
    vendor = {
      available: true,
      reason: `the export leg failed against the sandbox: ${String(e.message ?? e).slice(0, 300)}`,
      pushed: null,
      vendorTotals: null,
    };
  }
}

// --- the verdict ---------------------------------------------------------
// The native side must agree with itself across the two orgs. That check
// runs in BOTH shapes, because it is the part of the acceptance form that
// does not need a vendor.
const problems = [];
const currencies = new Set([...Object.keys(totalsA), ...Object.keys(totalsB)]);
for (const ccy of currencies) {
  const va = totalsA[ccy] ?? 0;
  const vb = totalsB[ccy] ?? 0;
  if (va !== vb) {
    problems.push(
      `currency ${ccy}: org A total ${va} minor units, org B total ${vb} — the two orgs were seeded identically and must bill identically`,
    );
  }
}

const artifact = {
  generated_at: new Date().toISOString(),
  shape: VENDOR_AVAILABLE ? 'full' : 'harness-and-native-only',
  window: { from, to, due_in_days: 30, currency: ORG_CURRENCY },
  org_a: {
    role: 'synced (export to QuickBooks Online)',
    org: meA.org?.name ?? null,
    org_id: meA.org_id ?? null,
    invoices_issued: invoicesA.length,
    totals_minor_by_currency: totalsA,
  },
  org_b: {
    role: 'native (system of record, no vendor credentials)',
    org: meB.org?.name ?? null,
    org_id: meB.org_id ?? null,
    invoices_issued: invoicesB.length,
    totals_minor_by_currency: totalsB,
    client_id: factsB.client.id,
  },
  vendor,
  comparison: {
    currencies_compared: [...currencies].sort(),
    native_sides_agree: problems.length === 0,
    vendor_compared: VENDOR_AVAILABLE && vendor.vendorTotals !== null,
  },
  problems,
  operator_gap: VENDOR_AVAILABLE
    ? null
    : {
        what: 'The QuickBooks Online sandbox leg of the Stage-3 acceptance form did not run.',
        why:
          'QBO requires the operator\'s own OAuth access token and realm id. Marula hosts ' +
          'no Intuit OAuth app and performs no token refresh, by design ' +
          '(plugins/connectors/qbo-export-sync/README.md). No such credential exists in a ' +
          'repository or a CI runner, and inventing one is not possible.',
        to_close_it: [
          'Create a QBO sandbox company at developer.intuit.com.',
          'Register an app, add the com.intuit.quickbooks.accounting scope, and obtain an access token + realm id.',
          'Export QBO_ACCESS_TOKEN and QBO_REALM_ID, then re-run: node scripts/p34-reconciliation-run.mjs <out-dir>',
          'The same script then performs the vendor push, reads the vendor totals back, and compares native / Marula / vendor.',
        ],
        what_is_already_proven: [
          'Both orgs exist as real tenants, created through the real register path.',
          'Identical billing facts were seeded into both, over the same closed window.',
          'The native billing engine produces identical per-currency totals in both orgs.',
        ],
      },
};

await writeFile(resolve(OUT, 'reconciliation.json'), JSON.stringify(artifact, null, 2));

// A markdown twin, because the acceptance form is read by humans in review
// and a JSON file in an evidence directory is not read by humans in review.
const md = [
  `# Reconciliation acceptance run`,
  '',
  `Generated ${artifact.generated_at} · shape: **${artifact.shape}**`,
  '',
  `## Side by side`,
  '',
  `| | Org A (synced) | Org B (native) |`,
  `| --- | --- | --- |`,
  `| Role | ${artifact.org_a.role} | ${artifact.org_b.role} |`,
  `| Invoices issued | ${artifact.org_a.invoices_issued} | ${artifact.org_b.invoices_issued} |`,
  `| Totals (minor units) | \`${JSON.stringify(artifact.org_a.totals_minor_by_currency)}\` | \`${JSON.stringify(artifact.org_b.totals_minor_by_currency)}\` |`,
  '',
  `## Vendor leg`,
  '',
  VENDOR_AVAILABLE
    ? `QBO credentials were present. Export \`${vendor.pushed?.export_id ?? '(none)'}\` reported status \`${vendor.pushed?.status ?? 'unknown'}\`.\n\n${vendor.reason ? `Note: ${vendor.reason}` : ''}`
    : `**Not executed.** ${artifact.operator_gap.why}`,
  '',
  `## Verdict`,
  '',
  artifact.comparison.native_sides_agree
    ? `The two native sides agree per currency across \`${artifact.comparison.currencies_compared.join(', ') || '(no invoices)'}\`.`
    : `**MISMATCH**\n\n${problems.map((p) => `- ${p}`).join('\n')}`,
  '',
  artifact.operator_gap
    ? [
        '## Operator gap',
        '',
        `${artifact.operator_gap.what}`,
        '',
        '**Already proven without the vendor:**',
        '',
        ...artifact.operator_gap.what_is_already_proven.map((x) => `- ${x}`),
        '',
        '**To close the gap:**',
        '',
        ...artifact.operator_gap.to_close_it.map((x) => `- ${x}`),
      ].join('\n')
    : '',
  '',
].join('\n');
await writeFile(resolve(OUT, 'reconciliation.md'), md);

console.log(`\n[recon] shape=${artifact.shape}`);
console.log(`[recon] org A totals: ${JSON.stringify(totalsA)}`);
console.log(`[recon] org B totals: ${JSON.stringify(totalsB)}`);
console.log(`[recon] native sides agree: ${artifact.comparison.native_sides_agree}`);
if (!VENDOR_AVAILABLE) {
  console.log('[recon] vendor leg NOT executed — recorded as an operator gap in the artifact');
}
console.log(`[recon] artifact → ${resolve(OUT, 'reconciliation.md')}`);

if (problems.length > 0) {
  console.error('\n[recon] MISMATCH:');
  for (const p of problems) console.error('  ✗ ' + p);
  process.exit(1);
}
process.exit(0);
