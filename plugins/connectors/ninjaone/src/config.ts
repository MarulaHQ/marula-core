//! Connector configuration from the environment.
//!
//! Purpose: one typed view of everything this connector needs. Missing
//! NinjaOne credentials never crashes the process — it degrades to
//! documented NONE MODE (observe-only) because a connector is an
//! enhancement, never a prerequisite.

export interface Config {
  /** Base URL of the Marula core API. */
  marulaBaseUrl: string;
  /** Scoped Marula API key (raw `mrl_…` material). */
  marulaApiKey: string;
  /** Shared secret used to verify Marula's signed webhook deliveries. */
  marulaWebhookSecret: string;
  /** TCP port for the webhook receiver. */
  port: number;
  /** NinjaOne PSAs base URL (e.g. https://app.ninjarmm.com). */
  ninjaOneBaseUrl?: string;
  /** NinjaOne OAuth2 client credentials token. */
  ninjaOneToken?: string;
}

/** NONE MODE is active when NinjaOne credentials are absent. */
export function noneMode(config: Config): boolean {
  return !config.ninjaOneBaseUrl || !config.ninjaOneToken;
}

export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
  const marulaBaseUrl = env.MARULA_BASE_URL ?? "http://localhost:8080";
  const marulaApiKey = env.MARULA_API_KEY ?? "";
  if (!marulaApiKey) {
    throw new Error(
      "MARULA_API_KEY is required — create a scoped key under Settings → Integrations.",
    );
  }
  return {
    marulaBaseUrl,
    marulaApiKey,
    marulaWebhookSecret: env.MARULA_WEBHOOK_SECRET ?? "",
    port: Number(env.PORT ?? 8788),
    ninjaOneBaseUrl: env.NINJAONE_BASE_URL,
    ninjaOneToken: env.NINJAONE_TOKEN,
  };
}
