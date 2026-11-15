// Describe-it client for quotes and contracts.
//
// Purpose: one typed wrapper over the two authoring legs. The endpoints are
// drafts only — nothing is written until the ordinary create route runs from
// the prefilled builder. The call uses the same token + refusal shape as the
// report authoring client, because the server rides the same AI admission path.
//
// Non-purpose: no draft storage, no second save path, no surface-key choice
// (the server fixes the key to the record kind).

import { api, apiBaseUrl, loadTokens } from "./api";

export type QuoteDraftLine = {
  description: string;
  quantity_milli: number;
  unit_price_minor: number;
};

export type QuoteDraft = {
  title: string;
  currency: string;
  valid_for_days?: number | null;
  notes?: string | null;
  lines: QuoteDraftLine[];
  assumptions: string[];
};

export type ContractDraft = {
  name: string;
  contract_type: string;
  billing_frequency?: string | null;
  value_minor?: number | null;
  currency: string;
  term_months?: number | null;
  effective_from?: string | null;
  notes?: string | null;
  clauses: string[];
  assumptions: string[];
};

type Issue = { field: string; message: string };

type BaseResponse = {
  valid: boolean;
  saved: boolean;
  assumptions: string[];
  issues: Issue[];
  usage: { prompt_tokens: number; completion_tokens: number };
  model_info: { provider: string; model: string };
  error?: string;
};

export type QuoteAuthorResult = BaseResponse & { draft: QuoteDraft | null };
export type ContractAuthorResult = BaseResponse & { draft: ContractDraft | null };

export type MoneyAuthorOutcome<T> =
  | { ok: true; result: T }
  | { ok: false; refusal: string; message: string };

type RefusalShape = {
  error?: { message?: unknown; reason?: unknown };
  message?: unknown;
  reason?: unknown;
};

function refusalOf(status: number, envelope: RefusalShape): { refusal: string; message: string } {
  const shape = envelope.error ?? envelope;
  const reason = typeof shape.reason === "string" ? shape.reason : undefined;
  const message =
    typeof shape.message === "string"
      ? shape.message
      : reason === "ai_features_disabled"
        ? "AI is switched off for this org."
        : reason === "not_granted"
          ? "This surface has not been granted to you."
          : "The draft did not come back.";
  return {
    refusal: status === 403 ? (reason ?? "ai_refused") : status === 429 ? "rate_limited" : "request_failed",
    message,
  };
}

async function author<T>(
  path: "/api/v1/quotes/author" | "/api/v1/contracts/author",
  transcript: string,
  signal?: AbortSignal,
): Promise<MoneyAuthorOutcome<T>> {
  const tokens = loadTokens();
  if (tokens === null) return { ok: false, refusal: "auth", message: "Not signed in." };
  let response: Response;
  try {
    response = await fetch(`${apiBaseUrl()}${path}`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        accept: "application/json",
        authorization: `Bearer ${tokens.access_token}`,
      },
      signal,
      body: JSON.stringify({ transcript }),
    });
  } catch (error) {
    return {
      ok: false,
      refusal: "transport",
      message: error instanceof Error ? error.message : "The request did not complete.",
    };
  }
  if (!response.ok) {
    const body = (await response.json().catch(() => ({}))) as RefusalShape;
    return { ok: false, ...refusalOf(response.status, body) };
  }
  const body = (await response.json().catch(() => null)) as T | null;
  if (body === null) return { ok: false, refusal: "transport", message: "The answer was not JSON." };
  return { ok: true, result: body };
}

export function authorQuoteDraft(transcript: string, signal?: AbortSignal) {
  return author<QuoteAuthorResult>("/api/v1/quotes/author", transcript, signal);
}

export function authorContractDraft(transcript: string, signal?: AbortSignal) {
  return author<ContractAuthorResult>("/api/v1/contracts/author", transcript, signal);
}

/** True when this user holds the grant for the surface and a model is usable. */
export async function moneyAuthoringAvailable(surfaceKey: string): Promise<boolean> {
  const { data, error } = await api.api.GET("/api/v1/ai/me/surfaces");
  if (error !== undefined || data === undefined) return false;
  const surface = data.surfaces.find((row) => row.surface_key === surfaceKey);
  return surface !== undefined && surface.granted && surface.models_available;
}
