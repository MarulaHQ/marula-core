//! Meridian side of the connector: a typed `MeridianClient` from
//! @marula-core/sdk, authenticated with an x-api-key machine credential.
//!
//! Purpose: prove the extension pattern — this file talks to Meridian ONLY
//! through the published SDK surface (public REST + generated types). No
//! Meridian internals are imported anywhere in this connector.

import { MeridianClient } from "@marula-core/sdk";
import type { Config } from "./config.js";

export function connectMeridian(config: Config): MeridianClient {
  const client = new MeridianClient({ baseUrl: config.meridianBaseUrl });
  client.api.use({
    async onRequest({ request }) {
      request.headers.set("x-api-key", config.meridianApiKey);
      return request;
    },
  });
  return client;
}

export interface ClientSummary {
  id: string;
  name: string;
}

/** Typed read through the SDK: proves scope + connectivity in one call. */
export async function verifyAccess(
  client: MeridianClient,
): Promise<ClientSummary[]> {
  const { data, error } = await client.api.GET("/api/clients");
  if (error) {
    throw new Error(`Meridian rejected the key: ${JSON.stringify(error)}`);
  }
  return (data ?? []).map((c) => ({ id: String(c.id), name: c.name }));
}
