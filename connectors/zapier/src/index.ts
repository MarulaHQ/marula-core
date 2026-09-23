//! CLI entry: `verify` (live probe or none-mode) and `print-app` (dump the
//! Zapier app definition as JSON for zapier-platform-schema validation).

import { loadConfig, noneMode } from "./config.ts";
import { MERIDIAN_EVENT_TYPES, TRIGGERS } from "./events.ts";
import { buildZapierApp } from "./zapier-app.ts";
import { connectMeridian, verifyAccess } from "./meridian.ts";

async function main(argv: string[]): Promise<void> {
  const command = argv[0] ?? "verify";
  const config = loadConfig();

  if (command === "print-app") {
    // print-app is definition-only: it works in none mode too.
    process.stdout.write(`${JSON.stringify(buildZapierApp(config), null, 2)}\n`);
    return;
  }

  if (command === "verify") {
    if (noneMode(config)) {
      console.log("[none-mode] MERIDIAN_API_KEY absent — printing the would-be probe.");
      console.log(`[none-mode] would GET ${config.meridianBaseUrl}/api/v1/tickets?limit=1 with x-api-key`);
      console.log(`[none-mode] trigger set: ${TRIGGERS.map((t) => t.key).join(", ")}`);
      console.log(`[none-mode] event catalog: ${MERIDIAN_EVENT_TYPES.length} known types`);
      return;
    }
    const summary = await verifyAccess(connectMeridian(config));
    console.log(summary);
    console.log(`triggers: ${TRIGGERS.map((t) => t.key).join(", ")}`);
    console.log(`actions: ${Object.keys(buildZapierApp(config).creates).join(", ")}`);
    return;
  }

  throw new Error(`unknown command: ${command} (expected verify | print-app)`);
}

await main(process.argv.slice(2));
