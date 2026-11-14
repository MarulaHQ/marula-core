//! @marula-core/connector-ninjaone — reference NinjaOne connector.
//!
//! Purpose: demonstrate the Marula connector pattern end-to-end using
//! ONLY the public API via @marula-core/sdk and signed webhook deliveries:
//!
//!   verify   — typed SDK read (proves key + scope + connectivity)
//!   listen   — HTTP receiver for Marula's signed webhooks; matched
//!              ticket events are pushed to NinjaOne, or LOGGED ONLY in
//!              documented none mode when NinjaOne is unconfigured. The
//!              delivery is acknowledged only after the push settles.
//!
//! Non-purpose: no Marula internals imported, no ticket writes back
//! into Marula beyond the scoped key's grants, no credential vaulting.

import { createServer } from "node:http";
import { loadConfig, noneMode } from "./config.js";
import { forwardTicketEvent } from "./delivery.js";
import { connectMarula, verifyAccess } from "./marula.js";
import { WEBHOOK_PATH, createWebhookHandler } from "./server.js";

async function commandVerify(): Promise<void> {
  const config = loadConfig();
  const client = connectMarula(config);
  const tickets = await verifyAccess(client);
  console.log(
    `Marula OK at ${config.marulaBaseUrl}; tickets:read grants ${tickets.length} recent ticket(s).`,
  );
  for (const t of tickets) console.log(`  ${t.number}  ${t.title}`);
  if (noneMode(config)) {
    console.log("NinjaOne unconfigured → NONE MODE (observe-only pushes).");
  }
}

async function commandListen(): Promise<void> {
  const config = loadConfig();
  if (!config.marulaWebhookSecret) {
    throw new Error(
      "MARULA_WEBHOOK_SECRET is required for listen — copy it from webhook creation.",
    );
  }
  const server = createServer(
    createWebhookHandler({
      secret: config.marulaWebhookSecret,
      forward: async (rawBody) => {
        await forwardTicketEvent(config, rawBody);
      },
    }),
  );
  server.listen(config.port, () => {
    console.log(
      `receiver listening on :${config.port}${WEBHOOK_PATH}` +
        (noneMode(config) ? " (NONE MODE)" : ""),
    );
  });
}

const command = process.argv[2] ?? "verify";
if (command === "verify") await commandVerify();
else if (command === "listen") await commandListen();
else {
  console.error(`unknown command ${command} — use "verify" or "listen"`);
  process.exitCode = 2;
}
