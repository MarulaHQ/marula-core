//! @marula-core/connector-ninjaone — reference NinjaOne connector.
//!
//! Purpose: demonstrate the Meridian connector pattern end-to-end using
//! ONLY the public API via @marula-core/sdk and signed webhook deliveries:
//!
//!   verify   — typed SDK read (proves key + scope + connectivity)
//!   listen   — HTTP receiver for Meridian's signed webhooks; matched
//!              ticket events are pushed to NinjaOne, or LOGGED ONLY in
//!              documented none mode when NinjaOne is unconfigured.
//!
//! Non-purpose: no Meridian internals imported, no ticket writes back
//! into Meridian beyond the scoped key's grants, no credential vaulting.

import { createServer } from "node:http";
import { loadConfig, noneMode } from "./config.js";
import { connectMeridian, verifyAccess } from "./meridian.js";
import { SIGNATURE_HEADER, verifyDelivery } from "./receiver.js";
import { buildActivity, pushActivity } from "./ninjaone.js";

async function commandVerify(): Promise<void> {
  const config = loadConfig();
  const client = connectMeridian(config);
  const tickets = await verifyAccess(client);
  console.log(
    `Meridian OK at ${config.meridianBaseUrl}; tickets:read grants ${tickets.length} recent ticket(s).`,
  );
  for (const t of tickets) console.log(`  ${t.number}  ${t.title}`);
  if (noneMode(config)) {
    console.log("NinjaOne unconfigured → NONE MODE (observe-only pushes).");
  }
}

async function commandListen(): Promise<void> {
  const config = loadConfig();
  if (!config.meridianWebhookSecret) {
    throw new Error(
      "MARULA_WEBHOOK_SECRET is required for listen — copy it from webhook creation.",
    );
  }
  const server = createServer((req, res) => {
    if (req.method !== "POST") {
      res.writeHead(405).end();
      return;
    }
    const chunks: Buffer[] = [];
    req.on("data", (c: Buffer) => chunks.push(c));
    req.on("end", () => {
      const rawBody = Buffer.concat(chunks);
      // Verify RAW BYTES before parsing anything.
      const presented = req.headers[SIGNATURE_HEADER];
      const signatureHeader = Array.isArray(presented) ? presented[0] : presented;
      const outcome = verifyDelivery(
        config.meridianWebhookSecret,
        rawBody,
        signatureHeader,
      );
      if (!outcome.ok) {
        console.warn(`rejected delivery: ${outcome.reason}`);
        res.writeHead(401).end();
        return;
      }
      res.writeHead(200).end(); // ACK fast; processing is best-effort below.

      try {
        const envelope = JSON.parse(rawBody.toString("utf8")) as {
          type?: string;
          payload?: { ticket_id?: string };
        };
        const type = envelope.type ?? "unknown";
        if (!type.startsWith("ticket.")) return;
        const activity = buildActivity({
          type,
          ticket: envelope.payload?.ticket_id
            ? {
                id: envelope.payload.ticket_id,
                number: "(fetch via SDK)",
                title: type,
              }
            : undefined,
        });
        void pushActivity(config, "unassigned", activity).catch((err) =>
          console.error("push failed:", err),
        );
      } catch (err) {
        console.error("malformed JSON after valid signature:", err);
      }
    });
  });
  server.listen(config.port, () => {
    console.log(
      `receiver listening on :${config.port}` +
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
