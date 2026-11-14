//! @marula-core/connector-hudu — reference Hudu connector.
//!
//! Purpose: demonstrate the Marula connector pattern end-to-end using
//! ONLY the public API via @marula-core/sdk and signed webhook deliveries:
//!
//!   verify   — typed SDK read (proves key + scope + connectivity)
//!   listen   — HTTP receiver for Marula's signed webhooks; matched
//!              client events are mirrored to Hudu companies, or LOGGED
//!              ONLY in documented none mode when Hudu is unconfigured.
//!              The delivery is acknowledged only after the push settles.
//!
//! Non-purpose: no Marula internals imported, no credential vaulting,
//! no asset/KB sync (that is a Stage 3 adapter concern).

import { createServer } from "node:http";
import { loadConfig, noneMode } from "./config.js";
import { forwardClientEvent } from "./delivery.js";
import { connectMarula, verifyAccess, type ClientSummary } from "./marula.js";
import { WEBHOOK_PATH, createWebhookHandler } from "./server.js";

async function commandVerify(): Promise<void> {
  const config = loadConfig();
  const client = connectMarula(config);
  const clients: ClientSummary[] = await verifyAccess(client);
  console.log(
    `Marula OK at ${config.marulaBaseUrl}; clients:read grants ${clients.length} client(s).`,
  );
  for (const c of clients) console.log(`  ${c.name}`);
  if (noneMode(config)) {
    console.log("Hudu unconfigured → NONE MODE (observe-only pushes).");
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
        await forwardClientEvent(config, rawBody);
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
