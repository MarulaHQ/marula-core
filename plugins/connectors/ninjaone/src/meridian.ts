//! Marula side of the connector: a typed `MarulaClient` from
//! @marula-core/sdk, authenticated with an x-api-key machine credential.
//!
//! Purpose: prove the extension pattern — this file talks to Marula ONLY
//! through the published SDK surface (public REST + generated types). No
//! Marula internals are imported anywhere in this connector.

import { MarulaClient } from "@marula-core/sdk";
import type { Config } from "./config.js";

export function connectMarula(config: Config): MarulaClient {
  const client = new MarulaClient({ baseUrl: config.marulaBaseUrl });
  // Machine auth: the scoped API key rides x-api-key on every typed call.
  client.api.use({
    async onRequest({ request }) {
      request.headers.set("x-api-key", config.marulaApiKey);
      return request;
    },
  });
  return client;
}

export interface TicketSummary {
  id: string;
  number: string;
  title: string;
}

/** Typed read through the SDK: proves scope + connectivity in one call. */
export async function verifyAccess(
  client: MarulaClient,
): Promise<TicketSummary[]> {
  const { data, error } = await client.api.GET("/api/tickets", {
    params: { query: { limit: 5 } },
  });
  if (error) {
    throw new Error(`Marula rejected the key: ${JSON.stringify(error)}`);
  }
  return (data ?? []).map((t) => ({
    id: String(t.id),
    number: t.number,
    title: t.title,
  }));
}
