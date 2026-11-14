//! NinjaOne edge — deliberately THIN.
//!
//! Purpose: translate Marula ticket events into NinjaOne API calls. In
//! NONE MODE (no NinjaOne credentials) the same code path runs and only
//! logs the action it WOULD take, so operators can validate wiring with
//! zero third-party subscriptions (standalone-first doctrine).

import type { Config } from "./config.js";
import { noneMode } from "./config.js";

export interface TicketReference {
  /** Marula ticket id. */
  id: string;
  /** Human ticket number, when the event carries it. */
  number?: string;
  /** Ticket title, when the event carries it. */
  title?: string;
}

export interface NinjaActivity {
  activityType: string;
  activityContent: string;
}

/**
 * The closest label the event can honestly support: number and title when both
 * arrive, the number alone, then the title with its id, then the id. An absent
 * field shortens the label; nothing is invented to fill the gap.
 */
function ticketLabel(ticket: TicketReference): string {
  if (ticket.number && ticket.title) {
    return `[${ticket.number}] ${ticket.title}`;
  }
  if (ticket.number) return `[${ticket.number}]`;
  if (ticket.title) return `${ticket.title} (${ticket.id})`;
  return ticket.id;
}

/** Builds the NinjaOne device-activity note for one Marula event. */
export function buildActivity(event: {
  type: string;
  ticket: TicketReference;
}): NinjaActivity {
  return {
    activityType: "MARULA_EVENT",
    activityContent: `Marula event ${event.type} for ${ticketLabel(event.ticket)}`,
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
