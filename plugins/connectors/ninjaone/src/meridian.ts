//! Meridian side of the connector: a typed `MeridianClient` from
//! @meridian/sdk, authenticated with an x-api-key machine credential.
//!
//! Purpose: prove the extension pattern — this file talks to Meridian ONLY
//! through the published SDK surface (public REST + generated types). No
//! Meridian internals are imported anywhere in this connector.

import { MeridianClient } from "@meridian/sdk";
import type { Config } from "./config.js";

export function connectMeridian(config: Config): MeridianClient {
  const client = new MeridianClient({ baseUrl: config.meridianBaseUrl });
  // Machine auth: the scoped API key rides x-api-key on every typed call.
  client.api.use({
    async onRequest({ request }) {
      request.headers.set("x-api-key", config.meridianApiKey);
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
  client: MeridianClient,
): Promise<TicketSummary[]> {
  const { data, error } = await client.api.GET("/api/tickets", {
    params: { query: { limit: 5 } },
  });
  if (error) {
    throw new Error(`Meridian rejected the key: ${JSON.stringify(error)}`);
  }
  return (data ?? []).map((t) => ({
    id: String(t.id),
    number: t.number,
    title: t.title,
  }));
}
