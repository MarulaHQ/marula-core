// Regenerates src/generated/schema.d.ts from the committed openapi.json
// snapshot. Re-run after core-api route changes:
//   1. boot core-api, `curl /openapi.json > packages/sdk/openapi.json`
//   2. `pnpm --filter @meridian/sdk generate`
// The committed snapshot format is COMPACT (single-line minified JSON):
// step 1's curl output is pretty-printed, so this script minifies the
// snapshot in place before generating. Never commit the pretty form.
// Non-purpose: this script never hand-edits the generated output.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import openapiTS, { astToString } from "openapi-typescript";

const specPath = new URL("../openapi.json", import.meta.url);
const outPath = new URL("../src/generated/schema.d.ts", import.meta.url);

const raw = readFileSync(specPath, "utf8");
const spec = JSON.parse(raw);
const compact = JSON.stringify(spec);
if (raw !== compact) {
  writeFileSync(specPath, compact);
}
const ast = await openapiTS(spec, { pathParamsAsTypes: false });
mkdirSync(new URL("../src/generated", import.meta.url), { recursive: true });
writeFileSync(outPath, astToString(ast));
