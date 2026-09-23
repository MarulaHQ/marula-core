//! Connector configuration from the environment.
//!
//! Purpose: one typed view of everything this connector needs. Missing
//! Meridian credentials never crashes the process — definition-only
//! commands (print-app) and none-mode still work, because a reference
//! integration is an enhancement, never a prerequisite.

export interface Config {
  /** Base URL of the Meridian core API. */
  meridianBaseUrl: string;
  /** Scoped Meridian API key (raw `mrd_…` material) — the Zapier
   * authentication field maps to this. */
  meridianApiKey?: string;
}

/** NONE MODE is active when the API key is absent: field/action
 * definitions still render, every live call prints instead of fires. */
export function noneMode(config: Config): boolean {
  return !config.meridianApiKey;
}

export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
  return {
    meridianBaseUrl: env.MERIDIAN_BASE_URL ?? "http://localhost:8080",
    meridianApiKey: env.MERIDIAN_API_KEY,
  };
}
