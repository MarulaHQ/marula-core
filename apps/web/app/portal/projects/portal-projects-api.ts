// The portal's project routes need a small transport of their own: the portal
// token family is separate from the staff session, and the shared portal client
// does not declare these paths yet.
//
// Rotation is the same shape the portal client uses: the httpOnly cookie pair
// drives a POST /portal/refresh, the returned bearer lives in memory, and a 401
// retries once with a fresh token. Nothing is written to document storage.

"use client";

import { resolveApiBaseUrl } from "../../../lib/api";

interface PortalTokens {
  access_token: string;
  expires_in: number;
}

let tokens: PortalTokens | null = null;
let expiresAt = 0;
let rotation: Promise<PortalTokens> | null = null;

function csrf(): string | null {
  if (typeof document === "undefined") return null;
  const match = document.cookie.match(/(?:^|;\s*)marula_csrf=([^;]+)/);
  return match ? decodeURIComponent(match[1] ?? "") : null;
}

async function rotate(): Promise<PortalTokens> {
  if (rotation !== null) return rotation;
  const attempt = (async () => {
    const header = csrf();
    const response = await fetch(`${resolveApiBaseUrl()}/api/v1/portal/refresh`, {
      method: "POST",
      credentials: "include",
      headers: {
        "content-type": "application/json",
        ...(header === null ? {} : { "x-marula-csrf": header }),
      },
      body: "{}",
    });
    if (!response.ok) throw new Error("Your portal session has ended. Sign in again.");
    const next = (await response.json()) as PortalTokens;
    tokens = next;
    expiresAt = Date.now() + next.expires_in * 1000 - 30_000;
    return next;
  })();
  rotation = attempt;
  const release = () => {
    if (rotation === attempt) rotation = null;
  };
  attempt.then(release, release);
  return attempt;
}

async function bearer(): Promise<string> {
  if (tokens !== null && Date.now() < expiresAt) return tokens.access_token;
  return (await rotate()).access_token;
}

export async function portalProjectFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const call = async (token: string): Promise<Response> =>
    fetch(`${resolveApiBaseUrl()}${path}`, {
      ...init,
      credentials: "include",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${token}`,
        ...init?.headers,
      },
    });

  let response = await call(await bearer());
  if (response.status === 401) {
    tokens = null;
    response = await call((await rotate()).access_token);
  }
  if (!response.ok) {
    let message = response.status === 404 ? "We could not find that project." : "Something went wrong.";
    try {
      const body = (await response.json()) as { error?: { message?: string } };
      if (body.error?.message) message = body.error.message;
    } catch {
      // keep the plain message
    }
    throw new Error(message);
  }
  return (await response.json()) as T;
}

export interface PortalProjectSummary {
  id: string;
  name: string;
  status: string;
  status_text: string;
  due_date: string | null;
  latest_update_at: string | null;
  waiting_for_you: boolean;
}

export interface PortalProjectDetail {
  id: string;
  name: string;
  status: string;
  status_text: string;
  hold_reason: string | null;
  due_date: string | null;
  steps: {
    label: string;
    status: string;
    status_text: string;
    start_date: string | null;
    end_date: string | null;
  }[];
  updates: { id: string; sent_at: string | null; body: string }[];
  change_orders: {
    id: string;
    reference: string;
    title: string;
    summary: string | null;
    amount_minor: number | null;
    currency: string | null;
    changes_date: boolean;
    acknowledged_at: string | null;
  }[];
  plan_shared: boolean;
}

export function listPortalProjects(): Promise<{ projects: PortalProjectSummary[] }> {
  return portalProjectFetch("/api/v1/portal/projects");
}

export function getPortalProject(id: string): Promise<PortalProjectDetail> {
  return portalProjectFetch(`/api/v1/portal/projects/${id}`);
}

export function acknowledgeChangeOrder(
  projectId: string,
  changeOrderId: string,
): Promise<{ acknowledged: boolean }> {
  return portalProjectFetch(
    `/api/v1/portal/projects/${projectId}/change-orders/${changeOrderId}/acknowledge`,
    { method: "POST", body: "{}" },
  );
}
