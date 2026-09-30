// Lane `p41-ui-operations` — brand-comparison render.
//
// Renders the lane's REAL presentational components (no mocks, no hand-written
// markup that merely resembles them) against the app's REAL compiled CSS —
// tokens.css + components.css + Tailwind utilities, the same three layers
// docs/theme.md §9 names — and shoots both themes.
//
// Purpose: the mockup pack has no state for the ticket ledger, the merge
// banner or the rota cards, so "is it on-brand?" cannot be answered by pointing
// at a reference screenshot. It has to be answered by putting our own pixels
// next to the pack and looking at them. This script exists so that comparison
// is cheap and repeatable rather than a one-off.
//
// The components are rendered with `react-dom/server`, which runs no effects:
// this is exactly the state a user sees on first paint, which is the state most
// worth checking against a static mockup.
//
// Run:  node e2e/p41-brand-render.mjs
import { chromium } from "playwright";
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const WEB = path.resolve("apps/web");
const OUT = "design/qa-p41-ui-operations/captures";
// The helper scripts live INSIDE apps/web: Node resolves modules from the
// script's own path, not from cwd, so a script in /tmp cannot see the
// workspace packages at all.
const TMP = path.resolve("apps/web/.harness");
mkdirSync(OUT, { recursive: true });
mkdirSync(TMP, { recursive: true });

// --- 1. compile the app's real CSS ------------------------------------------
const compileScript = `
const path=require('path'), fs=require('fs');
const pluginPath = require.resolve('@tailwindcss/postcss');
const postcss = require(require.resolve('postcss', {paths:[path.dirname(pluginPath)]}));
const tw = require(pluginPath).default ?? require(pluginPath);
const ui = path.resolve('../../packages/ui/src');
const dir = path.resolve('.harness'); fs.mkdirSync(dir,{recursive:true});
let css = fs.readFileSync('app/globals.css','utf8');
css = css.replace('@import "@marula-core/ui/tokens.css";', '@import "'+ui+'/tokens.css";')
         .replace('@import "@marula-core/ui/components.css";', '@import "'+ui+'/components.css";')
         .replace(/@import "@marula-core\\/layout\\/styles.css";/, '')
         .replace(/@import "\\.\\/shell.css";/, '');
fs.writeFileSync(path.join(dir,'in.css'), css);
postcss([tw()]).process(css,{from: path.join(dir,'in.css')}).then(r=>{
  fs.writeFileSync(path.join(dir,'app.css'), r.css);
  console.log('css bytes', r.css.length);
}).catch(e=>{console.error(String(e)); process.exit(1);});
`;
writeFileSync(path.join(TMP, "compile.cjs"), compileScript);
const cssOut = execFileSync("node", [path.join(TMP, "compile.cjs")], {
  cwd: WEB,
  encoding: "utf8",
});
console.log(cssOut.trim());
const css = readFileSync(path.join(TMP, "app.css"), "utf8");

// --- 2. SSR the real components ---------------------------------------------
const renderScript = `
const path = require('path');
process.chdir(${JSON.stringify(WEB)});
// TSX + the workspace packages, transpiled through esbuild-register-free path:
// tsx is a devDependency, so require it to get TS/TSX + JSX support.
require('tsx/cjs');
const { renderToStaticMarkup } = require('react-dom/server');
const React = require('react');
const { TicketTable } = require('./components/tickets/ledger-table.tsx');
const { MergeBanner } = require('./components/tickets/merge-banner.tsx');

const h = React.createElement;
const NOW = Date.now();
const iso = (ms) => new Date(NOW + ms).toISOString();
const ORG = '00000000-0000-4000-8000-00000000000f';
const t = (o) => ({
  id: '00000000-0000-4000-8000-0000000000aa', org_id: ORG,
  board_id: '00000000-0000-4000-8000-0000000000bb',
  status_id: '00000000-0000-4000-8000-0000000000cc',
  category: 'open', category_id: null, number: 'MAR-100', title: 'Untitled',
  body: null, priority: 'p2', assignee_id: null, requester_id: null,
  client_id: null, contact_id: null, site_id: null, type_id: null, due_at: null,
  created_at: iso(-172800000), updated_at: iso(-3600000), resolved_at: null,
  merged_into_ticket_id: null, custom_fields: [], ...o,
});

const ROWS = [
  t({ id:'00000000-0000-4000-8000-0000000000a1', number:'MAR-184', priority:'p1',
      category:'in_progress', title:'VPN disconnects every 20\\u201330 minutes',
      assignee_id:'00000000-0000-4000-8000-0000000000d1',
      due_at: iso(12*60000), updated_at: iso(-600000) }),
  t({ id:'00000000-0000-4000-8000-0000000000a2', number:'MAR-181', priority:'p2',
      category:'open', title:'Kitchen extractor fan rattling after filter change',
      assignee_id:'00000000-0000-4000-8000-0000000000d1',
      due_at: iso(5*3600000), updated_at: iso(-7200000) }),
  t({ id:'00000000-0000-4000-8000-0000000000a3', number:'MAR-177', priority:'p3',
      category:'waiting', title:'Awaiting part delivery from supplier',
      due_at: iso(-3*3600000), updated_at: iso(-86400000) }),
  t({ id:'00000000-0000-4000-8000-0000000000a4', number:'MAR-176', priority:'p4',
      category:'open', title:'Quarterly patch cycle \\u2014 check the notes first',
      updated_at: iso(-96000000) }),
  t({ id:'00000000-0000-4000-8000-0000000000a5', number:'MAR-175', priority:'p4',
      category:'closed', title:'Duplicate of MAR-181 \\u2014 extractor fan',
      merged_into_ticket_id:'00000000-0000-4000-8000-0000000000a2',
      updated_at: iso(-172800000) }),
];

const COLUMNS = [
  { key:'number', label:'Number', visible:true, required:true, width:116, sortable:true },
  { key:'title', label:'Subject', visible:true, required:true, width:340, sortable:true },
  { key:'category', label:'Status', visible:true, width:124, sortable:true },
  { key:'priority', label:'Priority', visible:true, width:92, sortable:true },
  { key:'assignee', label:'Assignee', visible:true, width:150, sortable:false },
  { key:'sla', label:'SLA', visible:true, width:124, sortable:false },
  { key:'updated_at', label:'Updated', visible:true, width:104, sortable:true },
];

const sla = {
  '00000000-0000-4000-8000-0000000000a1': { applied:false, breached:false, minutes_remaining:null, paused:false, state:null },
};
const people = { '00000000-0000-4000-8000-0000000000d1': 'Thabo Mokoena' };

const table = renderToStaticMarkup(h(TicketTable, {
  columns: COLUMNS, rows: ROWS, sort: { field:'updated_at', dir:'desc' },
  sla, people, myId: null,
  selected: new Set(['00000000-0000-4000-8000-0000000000a2']),
  onSort(){}, onReorderColumns(){}, onResizeColumn(){}, onToggleRow(){},
}));

const banner = renderToStaticMarkup(h(MergeBanner, {
  survivorId: '00000000-0000-4000-8000-0000000000a2', survivorLabel: 'MAR-181',
}));

writeFileSync(path.join(process.cwd(), 'fragments.json'), JSON.stringify({ table, banner }));
console.log('rendered table bytes', table.length, 'banner bytes', banner.length);
`;
writeFileSync(path.join(TMP, "render.cjs"), renderScript);
const renderOut = execFileSync("node", [path.join(TMP, "render.cjs")], {
  cwd: WEB,
  encoding: "utf8",
  stdio: ["ignore", "pipe", "inherit"],
});
console.log(renderOut.trim());
const { table, banner } = JSON.parse(readFileSync(path.join(TMP, "fragments.json"), "utf8"));

// --- 3. shoot both themes ----------------------------------------------------
const shell = (theme, body) => `<!doctype html>
<html lang="en" data-theme="${theme}">
<head>
<meta charset="utf-8">
<style>${css}</style>
<style>
  body { margin: 0; background: var(--m-background); }
  .harness-pad { padding: 24px; }
  .harness-label {
    font: 600 11px/1 var(--m-font-ui); letter-spacing: 0.06em; text-transform: uppercase;
    color: var(--m-text-muted); margin: 0 0 10px;
  }
</style>
</head>
<body class="harness-pad">
  <p class="harness-label">/tickets ledger</p>
  <div class="m-card">${table}</div>
  <p class="harness-label" style="margin-top:22px">merged-row banner</p>
  <div class="m-card">${banner}</div>
</body>
</html>`;

const browser = await chromium.launch({ args: ["--no-proxy-server"] });
for (const theme of ["light", "dark"]) {
  const file = path.join(TMP, `ledger-${theme}.html`);
  writeFileSync(file, shell(theme, ""));
  const page = await browser.newPage({ viewport: { width: 1200, height: 760 } });
  await page.goto(`file://${file}`, { waitUntil: "load" });
  await page.waitForTimeout(400);
  const out = `${OUT}/brand-ledger-${theme}.png`;
  await page.screenshot({ path: out, fullPage: true });
  console.log(`[brand] ${out}`);
  await page.close();
}
await browser.close();