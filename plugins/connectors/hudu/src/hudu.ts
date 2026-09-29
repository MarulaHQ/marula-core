//! Hudu edge — deliberately THIN.
//!
//! Purpose: translate Marula client events into Hudu company API calls.
//! In NONE MODE (no Hudu credentials) the same code path runs and only
//! logs the action it WOULD take, so operators can validate wiring with
//! zero third-party subscriptions (standalone-first doctrine).

import type { Config } from "./config.js";
import { noneMode } from "./config.js";

export interface CompanyPayload {
  name: string;
  nickname?: string;
}

/** Builds the Hudu company payload for one Marula client event. */
export function buildCompany(client: { id: string; name: string }): CompanyPayload {
  return { name: client.name, nickname: `marula-${client.id.slice(0, 8)}` };
}

/**
 * Upserts the company in Hudu, or logs it in none mode.
 * Returns true when an external call actually happened.
 */
export async function pushCompany(
  config: Config,
  company: CompanyPayload,
): Promise<boolean> {
  if (noneMode(config)) {
    console.log(
      `[none-mode] would upsert Hudu company:`,
      JSON.stringify(company),
    );
    return false;
  }
  const url = `${config.huduBaseUrl}/api/v1/companies`;
  const response = await fetch(url, {
    method: "POST",
    headers: {
      "x-api-key": config.huduApiKey ?? "",
      "content-type": "application/json",
    },
    body: JSON.stringify({ company }),
  });
  // Hudu returns 200 for create-or-merge on name match.
  if (!response.ok && response.status !== 422 /* already exists */) {
    throw new Error(`Hudu push failed (${response.status})`);
  }
  return true;
}
