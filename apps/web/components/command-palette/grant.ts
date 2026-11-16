// The general_chat grant read.
//
// Purpose: one boolean the palette and panel must both respect. Deny-by-
// default: until GET /api/v1/ai/me/surfaces answers, the answer is "no", so
// no Ask switch, tab, hint or empty gap can flash before the read settles.
// A surface counts as usable only when it is granted AND a model is available;
// a granted surface with no model would answer 403 on the first turn.
//
// Non-purpose: no model picker here (the chat rides the org/user default), no
// retries on failure — a failed read is simply no grant, which is honest.

"use client";

import { useEffect, useState } from "react";
import { api } from "../../lib/api";

export const GENERAL_CHAT_SURFACE = "general_chat";

export type GrantStatus = "loading" | "granted" | "denied";

export function useGeneralChatGrant(): GrantStatus {
  const [status, setStatus] = useState<GrantStatus>("loading");

  useEffect(() => {
    let cancelled = false;
    void api.api
      .GET("/api/v1/ai/me/surfaces")
      .then(({ data, error }) => {
        if (cancelled) return;
        if (error !== undefined || data === undefined) {
          setStatus("denied");
          return;
        }
        const surface = data.surfaces.find((row) => row.surface_key === GENERAL_CHAT_SURFACE);
        setStatus(
          surface !== undefined && surface.granted && surface.models_available
            ? "granted"
            : "denied",
        );
      })
      .catch(() => {
        if (!cancelled) setStatus("denied");
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return status;
}
