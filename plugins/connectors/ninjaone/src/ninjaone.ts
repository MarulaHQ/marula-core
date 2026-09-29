//! NinjaOne edge — deliberately THIN.
//!
//! Purpose: translate Meridian ticket events into NinjaOne API calls. In
//! NONE MODE (no NinjaOne credentials) the same code path runs and only
//! logs the action it WOULD take, so operators can validate wiring with
//! zero third-party subscriptions (standalone-first doctrine).

import type { Config } from "./config.js";
import { noneMode } from "./config.js";
import type { TicketSummary } from "./meridian.js";

interface NinjaActivity {
  activityType: string;
  activityContent: string;
}

/** Builds the NinjaOne device-activity note for one Meridian event. */
export function buildActivity(event: {
  type: string;
  ticket?: TicketSummary;
}): NinjaActivity {
  const ticket = event.ticket
    ? `[${event.ticket.number}] ${event.ticket.title}`
    : "(ticket details not included)";
  return {
    activityType: "MARULA_EVENT",
    activityContent: `Meridian event ${event.type} for ${ticket}`,
  };
}

/**
 * Pushes one activity note to NinjaOne, or logs it in none mode.
 * Returns true when an external call actually happened.
 */
export async function pushActivity(
  config: Config,
  deviceId: string,
  activity: NinjaActivity,
): Promise<boolean> {
  if (noneMode(config)) {
    console.log(
      `[none-mode] would POST activity to NinjaOne device ${deviceId}:`,
      JSON.stringify(activity),
    );
    return false;
  }
  const url = `${config.ninjaOneBaseUrl}/api/v2/device/activities`;
  const response = await fetch(url, {
    method: "POST",
    headers: {
      "authorization": `Bearer ${config.ninjaOneToken}`,
      "content-type": "application/json",
    },
    body: JSON.stringify({ deviceId, ...activity }),
  });
  if (!response.ok) {
    throw new Error(`NinjaOne push failed (${response.status})`);
  }
  return true;
}
