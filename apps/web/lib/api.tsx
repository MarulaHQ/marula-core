// API client wiring for the web app.
//
// Purpose: one MeridianClient bound to NEXT_PUBLIC_API_URL with browser
// token persistence (localStorage, Stage-1 scope), a React context that
// owns auth state, and helpers for guarded pages + 401 refresh rotation.
// Non-purpose: no server-side data fetching and no token refresh scheduling
// beyond on-401 rotation.
"use client";

import { MeridianClient } from "@meridian/sdk";
import { useRouter } from "next/navigation";
import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";

const TOKENS_KEY = "meridian-tokens";

declare global {
  interface Window {
    /** Injected by the desktop shell (desktop/src/main.rs, ADR 0005). */
    __MERIDIAN_API_URL__?: string;
  }
}

interface StoredTokens {
  access_token: string;
  refresh_token: string;
  session_id: string;
  expires_in: number;
}

export function loadTokens(): StoredTokens | null {
  if (typeof window === "undefined") return null;
  const raw = window.localStorage.getItem(TOKENS_KEY);
  if (raw === null) return null;
  try {
    return JSON.parse(raw) as StoredTokens;
  } catch {
    return null;
  }
}

export function saveTokens(tokens: StoredTokens): void {
  window.localStorage.setItem(TOKENS_KEY, JSON.stringify(tokens));
}

export function clearTokens(): void {
  window.localStorage.removeItem(TOKENS_KEY);
}

/** Desktop shell (ADR 0005) injects the embedded API base before page scripts run. */
export function resolveApiBaseUrl(): string {
  if (typeof window !== "undefined" && window.__MERIDIAN_API_URL__) {
    return window.__MERIDIAN_API_URL__;
  }
  return process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8080";
}

function createClient(): MeridianClient {
  return new MeridianClient({ baseUrl: resolveApiBaseUrl() });
}

/** App-wide singleton; safe in the browser only (this app is fully client-rendered). */
export const api = createClient();

export interface Me {
  user_id: string;
  org_id: string;
  email: string;
  display_name: string;
  timezone: string;
  permissions: readonly string[];
}

interface AuthState {
  me: Me | null;
  status: "loading" | "anonymous" | "authed";
  logout(): Promise<void>;
}

const AuthContext = createContext<AuthState>({
  me: null,
  status: "loading",
  logout: async () => {},
});

/**
 * Wraps the app: restores stored tokens, fetches /api/v1/me once, and rotates
 * the refresh token when the access token has expired. Exposes the identity
 * via {@link useAuth}.
 */
export function AuthProvider({ children }: { children: ReactNode }) {
  const [me, setMe] = useState<Me | null>(null);
  const [status, setStatus] = useState<AuthState["status"]>("loading");

  useEffect(() => {
    let cancelled = false;
    async function restore() {
      const stored = loadTokens();
      if (stored === null) {
        setStatus("anonymous");
        return;
      }
      api.setTokens(stored);
      // Proactive rotation when the access token is at/past expiry keeps
      // long-open tabs working without waiting for a failed request.
      const expiresAtMs = Number(window.localStorage.getItem(`${TOKENS_KEY}-exp`) ?? 0);
      if (Date.now() >= expiresAtMs) {
        try {
          const rotated = await api.refresh();
          saveTokens(rotated);
          window.localStorage.setItem(
            `${TOKENS_KEY}-exp`,
            String(Date.now() + rotated.expires_in * 1000 - 30_000),
          );
        } catch {
          clearTokens();
          setStatus("anonymous");
          return;
        }
      }
      const { data, error } = await api.api.GET("/api/v1/me");
      if (cancelled) return;
      if (error !== undefined) {
        clearTokens();
        setStatus("anonymous");
        return;
      }
      setMe(data);
      setStatus("authed");
    }
    void restore();
    return () => {
      cancelled = true;
    };
  }, []);

  const logout = useCallback(async () => {
    try {
      await api.logout();
    } finally {
      clearTokens();
      setMe(null);
      setStatus("anonymous");
    }
  }, []);

  const value = useMemo(() => ({ me, status, logout }), [me, status, logout]);
  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthState {
  return useContext(AuthContext);
}

/** Client-side guard: redirects to /login unless authenticated. */
export function useRequireAuth(): AuthState {
  const auth = useAuth();
  const router = useRouter();
  useEffect(() => {
    if (auth.status === "anonymous") {
      router.replace("/login");
    }
  }, [auth.status, router]);
  return auth;
}

// --- attachments (TASKLIST 2.14) --------------------------------------------
// Raw-byte surfaces (octet-stream upload, streamed download) bypass the
// generated client on purpose: openapi-fetch models JSON bodies, while
// attachments are binary with sidecar headers.
//
// The URL literals below are `satisfies`-anchored to the canonical /api/v1
// contract, so a spec rename fails `tsc` here instead of silently 404ing.

import type { MeridianPaths } from "@meridian/sdk";

const TICKET_ATTACHMENTS_PATH = "/api/v1/tickets/{id}/attachments" satisfies keyof MeridianPaths;
const ATTACHMENT_DOWNLOAD_PATH =
  "/api/v1/attachments/{id}/download" satisfies keyof MeridianPaths;
const PLUGIN_ZIP_STAGE_PATH = "/api/v1/plugins/distribution/staged/zip" satisfies keyof MeridianPaths;

/** Auth header for manual fetches against the API base URL. */
function authHeaders(): HeadersInit {
  const tokens = loadTokens();
  return tokens === null ? {} : { authorization: `Bearer ${tokens.access_token}` };
}

export function apiBaseUrl(): string {
  return resolveApiBaseUrl();
}

/** Kept as API_BASE for callers binding to the documented constant. */
export const API_BASE = resolveApiBaseUrl();

export interface UploadedAttachment {
  id: string;
  filename: string;
  content_type: string;
  size_bytes: number;
}

/**
 * Uploads one file against a ticket (composer flow: upload first, link the
 * returned ids when the comment is posted). Throws with the server's
 * message on validation failures — callers surface it verbatim.
 */
export async function uploadAttachment(ticketId: string, file: File): Promise<UploadedAttachment> {
  const response = await fetch(
    `${apiBaseUrl()}${TICKET_ATTACHMENTS_PATH.replace("{id}", ticketId)}`,
    {
    method: "POST",
    headers: {
      ...authHeaders(),
      "content-type": "application/octet-stream",
      "x-filename": file.name,
    },
    body: file,
  });
  if (!response.ok) {
    const detail = (await response.json().catch(() => null)) as
      | { error?: { message?: string } }
      | null;
    throw new Error(detail?.error?.message ?? `Upload failed (${response.status})`);
  }
  return (await response.json()) as UploadedAttachment;
}

/** Streams an attachment through the API and triggers a browser download. */
export async function downloadAttachment(attachmentId: string, filename: string): Promise<void> {
  const response = await fetch(
    `${apiBaseUrl()}${ATTACHMENT_DOWNLOAD_PATH.replace("{id}", attachmentId)}`,
    { headers: authHeaders() },
  );
  if (!response.ok) throw new Error(`Download failed (${response.status})`);
  const blob = await response.blob();
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.click();
  URL.revokeObjectURL(url);
}

export interface StagedItemResponse {
  id: string;
  artifact_kind: string;
  source_kind: string;
}

/**
 * Uploads one plugin/skill zip to the staged sandbox import (raw binary
 * `application/zip` — same bypass as attachments: the generated client
 * models JSON bodies while this route is binary).
 */
export async function uploadPluginZip(
  file: File,
): Promise<{ ok: true; staged: StagedItemResponse } | { ok: false; status: number; message?: string }> {
  const response = await fetch(`${apiBaseUrl()}${PLUGIN_ZIP_STAGE_PATH}`, {
    method: "POST",
    headers: { ...authHeaders(), "content-type": "application/zip" },
    body: file,
  });
  const typed = response.headers.get("content-type")?.includes("application/json") ?? false;
  const detail = typed ? ((await response.json().catch(() => null)) as { error?: { message?: string; reason?: string } } | null) : null;
  if (!response.ok) {
    return {
      ok: false,
      status: response.status,
      message:
        detail?.error?.message ??
        `The archive was rejected during the sandbox import (${response.status}).`,
    };
  }
  return { ok: true, staged: (await response.json()) as StagedItemResponse };
}
