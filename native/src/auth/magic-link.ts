// Magic link request — the pack's "Email me a link" leg.
//
// Purpose: POST /api/v1/auth/magic-link/request is a public endpoint the
// typed client paths expose; the login screen calls it through this helper
// so the link leg honors the same honest copy rules (IF3 envelope → the
// server's fixed human copy).

import type { MarulaClient } from "@marula-core/sdk";

import { API_URL } from "@/config";

export async function requestMagicLink(email: string): Promise<boolean> {
  try {
    const response = await fetch(`${API_URL}/api/v1/auth/magic-link/request`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ email }),
    });
    return response.ok;
  } catch {
    return false;
  }
}

/** Kept type-side so future legs reuse the client directly. */
export type MagicClient = MarulaClient;
