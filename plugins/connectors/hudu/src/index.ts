//! @marula-core/connector-hudu — reference Hudu connector.
//!
//! Purpose: demonstrate the Meridian connector pattern end-to-end using
//! ONLY the public API via @marula-core/sdk and signed webhook deliveries:
//!
//!   verify   — typed SDK read (proves key + scope + connectivity)
//!   listen   — HTTP receiver for Meridian's signed webhooks; matched
//!              client events are mirrored to Hudu companies, or LOGGED
//!              ONLY in documented none mode when Hudu is unconfigured.
//!
//! Non-purpose: no Meridian internals imported, no credential vaulting,
//! no asset/KB sync (that is a Stage 3 adapter concern).

import { createServer } from "node:http";
import { loadConfig, noneMode } from "./config.js";
import { connectMeridian, verifyAccess, type ClientSummary } from "./meridian.js";
import { SIGNATURE_HEADER, verifyDelivery } from "./receiver.js";
import { buildCompany, pushCompany } from "./hudu.js";

async function commandVerify(): Promise<void> {
  const config = loadConfig();
  const client = connectMeridian(config);
  const clients: ClientSummary[] = await verifyAccess(client);
  console.log(
    `Meridian OK at ${config.meridianBaseUrl}; clients:read grants ${clients.length} client(s).`,
  );
  for (const c of clients) console.log(`  ${c.name}`);
  if (noneMode(config)) {
    console.log("Hudu unconfigured → NONE MODE (observe-only pushes).");
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
      res.writeHead(200).end();

      try {
        const envelope = JSON.parse(rawBody.toString("utf8")) as {
          type?: string;
          payload?: Record<string, unknown>;
        };
        const type = envelope.type ?? "unknown";
        if (!type.startsWith("client.")) return;
        const name =
          (envelope.payload?.["name"] as string | undefined) ??
          "(fetch via SDK)";
        void pushCompany(config, buildCompany({ id: name, name })).catch(
          (err) => console.error("push failed:", err),
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
