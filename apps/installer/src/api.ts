/**
 * The two transports the installer runs on (Wave 60 lane W60-3, ADR 0027).
 *
 * Purpose: one interface, `Transport`, with a native implementation over Tauri
 * IPC and a fallback over loopback HTTP. The screens import `transport` and
 * never know which one they are on; the difference between the app and the
 * fallback is a shell, not a second wizard.
 *
 * Non-purpose: no state. The engine decides everything and both transports call
 * the same functions in `install::view::Session`; this file only moves bytes.
 */

import type {
  ApplyState,
  Doctor,
  Plan,
  PlanRequest,
  State,
} from "./types";

export interface Transport {
  readonly kind: "native" | "browser";
  state(): Promise<State>;
  plan(request: PlanRequest): Promise<Plan>;
  apply(request: PlanRequest): Promise<void>;
  applyState(): Promise<ApplyState>;
  doctor(profile: string): Promise<Doctor>;
  /** Opens the installed product in the operator's browser. */
  open(url: string): Promise<void>;
}

declare global {
  interface Window {
    __TAURI__?: {
      core?: { invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> };
    };
  }
}

function native(): boolean {
  return typeof window !== "undefined" && typeof window.__TAURI__?.core?.invoke === "function";
}

async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const core = window.__TAURI__?.core;
  if (!core) throw new Error("the native shell is not available");
  return core.invoke<T>(command, args);
}

const tauriTransport: Transport = {
  kind: "native",
  state: () => invoke<State>("installer_state"),
  plan: (request) => invoke<Plan>("installer_plan", { request }),
  apply: async (request) => {
    await invoke<null>("installer_apply", { request });
  },
  applyState: () => invoke<ApplyState>("installer_apply_state"),
  doctor: (profile) => invoke<Doctor>("installer_doctor", { profile }),
  open: async (url) => {
    await invoke<null>("installer_open", { url });
  },
};

/**
 * Loopback HTTP. The session cookie is HttpOnly and SameSite=Strict; the
 * `x-marula-installer` header is required by the server as well, so a page on
 * another origin cannot drive the installer with a form post or a simple
 * request, even if it somehow obtained the cookie's origin.
 */
async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    credentials: "same-origin",
    ...init,
    headers: {
      "content-type": "application/json",
      "x-marula-installer": "1",
      ...(init?.headers ?? {}),
    },
  });
  if (!response.ok) {
    let detail = `${response.status} ${response.statusText}`;
    try {
      const body = (await response.json()) as { error?: string };
      if (body.error) detail = body.error;
    } catch {
      // A non-JSON body keeps the status line, which is the honest fallback.
    }
    throw new Error(detail);
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

const httpTransport: Transport = {
  kind: "browser",
  state: () => request<State>("/api/state"),
  plan: (body) => request<Plan>("/api/plan", { method: "POST", body: JSON.stringify(body) }),
  apply: async (body) => {
    await request<null>("/api/apply", { method: "POST", body: JSON.stringify(body) });
  },
  applyState: () => request<ApplyState>("/api/apply"),
  doctor: (profile) => request<Doctor>(`/api/doctor?profile=${encodeURIComponent(profile)}`),
  open: async (url) => {
    window.open(url, "_blank", "noopener,noreferrer");
  },
};

export const transport: Transport = native() ? tauriTransport : httpTransport;

export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "something went wrong, and it did not say what";
}
