// Regenerates src/generated/schema.d.ts from the committed openapi.json
// snapshot. Re-run after core-api route changes:
//   1. boot core-api, `curl /openapi.json > packages/sdk/openapi.json`
//   2. `pnpm --filter @meridian/sdk generate`
// Non-purpose: this script never hand-edits the generated output.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import openapiTS, { astToString } from "openapi-typescript";

const specPath = new URL("../openapi.json", import.meta.url);
const outPath = new URL("../src/generated/schema.d.ts", import.meta.url);

const spec = JSON.parse(readFileSync(specPath, "utf8"));
const ast = await openapiTS(spec, { pathParamsAsTypes: false });
mkdirSync(new URL("../src/generated", import.meta.url), { recursive: true });
writeFileSync(outPath, astToString(ast));
