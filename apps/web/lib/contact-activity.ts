// Contact activity: the typed read behind the client record's Last contact
// column.
//
// The server derives this from email on the client's tickets, portal replies
// and calls. Nothing is written. The client is raw-fetch because the generated
// SDK is regenerated in one batch at the end of the lane; the route is stable
// and the shape is checked here.

import { apiBaseUrl, authReady, loadTokens } from "./api";
import { relativeTime } from "./ticket-ops";

export type ContactActivityEvent = {
  at: string;
  kind: string;
  detail: string | null;
};

export type ContactActivity = {
  contact_id: string;
  last_at: string | null;
  last_kind: string | null;
  recent: ContactActivityEvent[];
};

const KIND_LABEL: Record<string, string> = {
  email_in: "Emailed us",
  email_out: "We emailed",
  portal_reply: "Portal reply",
  call_in: "Called us",
  call_out: "We called",
};

export function contactKindLabel(kind: string | null | undefined): string {
  if (kind === null || kind === undefined) return "Contact";
  return KIND_LABEL[kind] ?? "Contact";
}

/** "Portal reply · 2d ago", or "Never" when nothing has been recorded. */
export function contactActivitySummary(row: ContactActivity | undefined): string {
  if (row === undefined || row.last_at === null) return "Never";
  const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  return `${contactKindLabel(row.last_kind)} · ${relativeTime(row.last_at, zone)}`;
}

/**
 * Every contact's activity for one client, keyed by contact id.
 *
 * `null` means the read failed — not that nobody has been contacted. The
 * caller must say "unavailable", because "Never" is a claim about the client's
 * history and a 500, a 403 or a dropped connection proves nothing about it.
 */
export async function fetchContactActivity(
  clientId: string,
): Promise<Record<string, ContactActivity> | null> {
  // A full page load restores the token from the session cookie. Wait for
  // that only when there is no token yet; a seeded test session proceeds.
  if (loadTokens() === null) await authReady();
  const tokens = loadTokens();
  try {
    const response = await fetch(
      `${apiBaseUrl()}/api/v1/clients/${encodeURIComponent(clientId)}/contacts/activity`,
      {
        credentials: "include",
        headers: {
          accept: "application/json",
          ...(tokens === null ? {} : { authorization: `Bearer ${tokens.access_token}` }),
        },
      },
    );
    if (!response.ok) return null;
    const rows = (await response.json().catch(() => null)) as ContactActivity[] | null;
    if (rows === null || !Array.isArray(rows)) return null;
    return Object.fromEntries(rows.map((row) => [row.contact_id, row]));
  } catch {
    return null;
  }
}
