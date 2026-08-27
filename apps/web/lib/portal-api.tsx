// Portal API wiring for the client portal (portal v1).
//
// Purpose: token persistence under a SEPARATE localStorage family
// ("meridian-portal-tokens" — never the staff key), typed fetch helpers for
// the /api/v1/portal/* surface, and a React context owning portal auth state.
// Deliberately independent of lib/api.tsx so staff and portal sessions can
// never bleed into each other in one browser profile.
// Non-purpose: no SDK client usage (the portal is a small dedicated
// surface), no staff endpoints, no refresh scheduling beyond on-401
// rotation.
"use client";

import { useRouter } from "next/navigation";
import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";

/** Request paths on the canonical versioned surface only. The portal owns
 * hand-written transport (no generated client), so this is the narrow
 * anchor keeping it off undocumented legacy aliases forever. */
type PortalPath = `/api/v1/portal/${string}`;

const TOKENS_KEY = "meridian-portal-tokens";

export interface PortalTokens {
  access_token: string;
  refresh_token: string;
  session_id: string;
  expires_in: number;
}

export interface PortalProfile {
  id: string;
  org_id: string;
  client_id: string;
  email: string;
  display_name: string;
  role: "contact" | "approver" | "it_manager";
}

interface PortalTicket {
  id: string;
  number: string;
  title: string;
  status_id: string;
  category: "open" | "in_progress" | "waiting" | "closed";
  priority: "p1" | "p2" | "p3" | "p4";
  board_id: string;
  created_at: string;
  updated_at: string;
  resolved_at?: string | null;
}

interface PortalComment {
  id: string;
  kind: "internal" | "client_visible" | "in_house_update";
  body: string;
  created_at: string;
}

export interface PortalTicketDetail {
  ticket: PortalTicket;
  comments: PortalComment[];
  can_view_in_house_updates: boolean;
}

function loadTokens(): PortalTokens | null {
  if (typeof window === "undefined") return null;
  const raw = window.localStorage.getItem(TOKENS_KEY);
  if (raw === null) return null;
  try {
    return JSON.parse(raw) as PortalTokens;
  } catch {
    return null;
  }
}

function saveTokens(tokens: PortalTokens): void {
  window.localStorage.setItem(TOKENS_KEY, JSON.stringify(tokens));
  window.localStorage.setItem(
    `${TOKENS_KEY}-exp`,
    String(Date.now() + tokens.expires_in * 1000 - 30_000),
  );
}

export function clearPortalTokens(): void {
  window.localStorage.removeItem(TOKENS_KEY);
  window.localStorage.removeItem(`${TOKENS_KEY}-exp`);
}

const API_BASE = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8080";

/**
 * One fetch wrapper for every portal call: attaches the access token,
 * rotates the refresh token once on 401, and clears state on hard logout.
 */
async function portalFetch<T>(
  path: PortalPath,
  init: RequestInit & { tokens?: PortalTokens | null },
): Promise<T> {
  const { tokens, ...rest } = init;
  const stored = tokens ?? loadTokens();
  const response = await fetch(`${API_BASE}${path}`, {
    ...rest,
    headers: {
      "content-type": "application/json",
      ...(stored ? { authorization: `Bearer ${stored.access_token}` } : {}),
      ...rest.headers,
    },
  });
  if (response.status === 401 && stored) {
    // Single rotation attempt; a second 401 means truly logged out.
    const rotated = await fetch(`${API_BASE}/api/v1/portal/refresh`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        refresh_token: stored.refresh_token,
        session_id: stored.session_id,
      }),
    });
    if (rotated.ok) {
      const next = (await rotated.json()) as PortalTokens & { portal_user: PortalProfile };
      saveTokens(next);
      const retry = await fetch(`${API_BASE}${path}`, {
        ...rest,
        headers: {
          "content-type": "application/json",
          authorization: `Bearer ${next.access_token}`,
          ...rest.headers,
        },
      });
      if (retry.ok) return (await retry.json()) as T;
    }
    clearPortalTokens();
    throw new PortalError("unauthorized", 401);
  }
  if (!response.ok) {
    let message = `request failed (${response.status})`;
    try {
      const payload = (await response.json()) as { error?: { message?: string } };
      message = payload.error?.message ?? message;
    } catch {
      // non-JSON error body — keep the generic message
    }
    throw new PortalError(message, response.status);
  }
  return (await response.json()) as T;
}

export class PortalError extends Error {
  readonly status: number;
  constructor(message: string, status: number) {
    super(message);
    this.status = status;
  }
}

export const portalApi = {
  async login(email: string, password: string): Promise<PortalTokens & { portal_user: PortalProfile }> {
    const result = await portalFetch<
      PortalTokens & { portal_user: PortalProfile }
    >("/api/v1/portal/login", {
      method: "POST",
      tokens: null,
      body: JSON.stringify({ email, password }),
    });
    saveTokens(result);
    return result;
  },

  me(): Promise<PortalProfile> {
    return portalFetch<PortalProfile>("/api/v1/portal/me", {});
  },

  listTickets(): Promise<PortalTicket[]> {
    return portalFetch<PortalTicket[]>("/api/v1/portal/tickets?limit=200", {});
  },

  getTicket(id: string): Promise<PortalTicketDetail> {
    return portalFetch<PortalTicketDetail>(`/api/v1/portal/tickets/${id}`, {});
  },

  reply(id: string, body: string): Promise<PortalComment> {
    return portalFetch<PortalComment>(`/api/v1/portal/tickets/${id}/reply`, {
      method: "POST",
      body: JSON.stringify({ body }),
    });
  },
};

interface PortalAuthState {
  me: PortalProfile | null;
  status: "loading" | "anonymous" | "authed";
  signOut(): void;
}

const PortalAuthContext = createContext<PortalAuthState>({
  me: null,
  status: "loading",
  signOut: () => {},
});

/** Wraps the /portal tree: restores stored tokens and validates them via /api/v1/portal/me. */
export function PortalAuthProvider({ children }: { children: ReactNode }) {
  const [me, setMe] = useState<PortalProfile | null>(null);
  const [status, setStatus] = useState<PortalAuthState["status"]>("loading");

  useEffect(() => {
    let cancelled = false;
    async function restore() {
      if (loadTokens() === null) {
        setStatus("anonymous");
        return;
      }
      try {
        const profile = await portalApi.me();
        if (cancelled) return;
        setMe(profile);
        setStatus("authed");
      } catch {
        if (cancelled) return;
        setStatus("anonymous");
      }
    }
    void restore();
    return () => {
      cancelled = true;
    };
  }, []);

  const signOut = useCallback(() => {
    clearPortalTokens();
    setMe(null);
    setStatus("anonymous");
  }, []);

  const value = useMemo(
    () => ({ me, status, signOut }),
    [me, status, signOut],
  );
  return <PortalAuthContext.Provider value={value}>{children}</PortalAuthContext.Provider>;
}

export function usePortalAuth(): PortalAuthState {
  return useContext(PortalAuthContext);
}

/** Guard hook: pushes to the portal login when anonymous. */
export function useRequirePortalAuth(): void {
  const { status } = usePortalAuth();
  const router = useRouter();
  useEffect(() => {
    if (status === "anonymous") router.replace("/portal/login");
  }, [status, router]);
}
