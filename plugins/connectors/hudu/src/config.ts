//! Connector configuration from the environment.
//!
//! Purpose: one typed view of everything this connector needs. Missing
//! Hudu credentials never crashes the process — it degrades to documented
//! NONE MODE (observe-only) because a connector is an enhancement, never
//! a prerequisite.

export interface Config {
  /** Base URL of the Meridian core API. */
  meridianBaseUrl: string;
  /** Scoped Meridian API key (raw `mrd_…` material). */
  meridianApiKey: string;
  /** Shared secret used to verify Meridian's signed webhook deliveries. */
  meridianWebhookSecret: string;
  /** TCP port for the webhook receiver. */
  port: number;
  /** Hudu base URL (e.g. https://yourcompany.huducloud.com). */
  huduBaseUrl?: string;
  /** Hudu API key. */
  huduApiKey?: string;
}

/** NONE MODE is active when Hudu credentials are absent. */
export function noneMode(config: Config): boolean {
  return !config.huduBaseUrl || !config.huduApiKey;
}

export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
  const meridianApiKey = env.MARULA_API_KEY ?? "";
  if (!meridianApiKey) {
    throw new Error(
      "MARULA_API_KEY is required — create a scoped key under Settings → Integrations.",
    );
  }
  return {
    meridianBaseUrl: env.MARULA_BASE_URL ?? "http://localhost:8080",
    meridianApiKey,
    meridianWebhookSecret: env.MARULA_WEBHOOK_SECRET ?? "",
    port: Number(env.PORT ?? 8789),
    huduBaseUrl: env.HUDU_BASE_URL,
    huduApiKey: env.HUDU_API_KEY,
  };
}
