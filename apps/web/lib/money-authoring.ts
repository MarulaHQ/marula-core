// Describe-it client for quotes and contracts.
//
// Purpose: one typed wrapper over the authoring legs. The endpoints are drafts
// only — nothing is written until the ordinary create route runs from the
// prefilled builder. The call uses the same token + refusal shape as the report
// authoring client, because the server rides the same AI admission path.
//
// Shapes mirror `crates/core-api/src/routes/money_authoring.rs`: sources in,
// a draft with provenance out, an understanding block that is the check step,
// and a change proposal that carries operations but applies nothing.
//
// Non-purpose: no draft storage, no second save path, no surface-key choice
// (the server fixes the key to the record kind), and no arithmetic the server
// did not also compute.

import { api, apiBaseUrl, loadTokens } from "./api";

export type MoneyKind = "quote" | "contract";

export type SourceKind = "typed" | "document" | "transcript" | "recording";

/** One thing the person gave the panel to read. */
export type MoneySource = {
  id?: string;
  kind: SourceKind;
  name?: string;
  text: string;
};

export type SourceSpan = { source_id: string; start: number; end: number };

/** `notes` means a located span; `catalogue` means the catalogue priced it. */
export type LineProvenance = "notes" | "assumed" | "catalogue";

export type DraftLine = {
  description: string;
  quantity_milli: number;
  unit_price_minor: number;
  unit?: string | null;
  section?: string | null;
  optional?: boolean;
  catalog_item_id?: string | null;
  source?: LineProvenance;
  source_span?: SourceSpan | null;
  quantity_assumed?: boolean;
  price_assumed?: boolean;
};

export type MatchedFact = { label: string; value: string };
export type DraftQuestion = {
  id: string;
  text: string;
  options: string[];
  chosen?: string | null;
};
export type Understanding = {
  statement: string;
  facts: MatchedFact[];
  questions: DraftQuestion[];
  assumptions: string[];
};

export type ClauseRef = {
  clause_id: string;
  key: string;
  title: string;
  body: string;
  version: number;
  status: "standard" | "variant";
  review_required: boolean;
  parameter_label?: string | null;
  parameter_value?: string | null;
  standard_value?: string | null;
};

export type ContractPriceLine = {
  label: string;
  quantity_milli: number;
  unit_price_minor: number;
  quantity_assumed?: boolean;
  price_assumed?: boolean;
  source_span?: SourceSpan | null;
};

export type QuoteDraft = {
  title: string;
  currency: string;
  valid_for_days?: number | null;
  notes?: string | null;
  client_id?: string | null;
  client_name?: string | null;
  budget_minor?: number | null;
  lines: DraftLine[];
  assumptions: string[];
  understanding?: Understanding | null;
  checks?: string[];
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
  clause_refs?: ClauseRef[];
  pricing?: ContractPriceLine[];
  client_id?: string | null;
  client_name?: string | null;
  assumptions: string[];
  understanding?: Understanding | null;
  checks?: string[];
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
  reference?: string;
};

export type QuoteAuthorResult = BaseResponse & { draft: QuoteDraft | null };
export type ContractAuthorResult = BaseResponse & { draft: ContractDraft | null };

export type ChangeOperation = {
  path: string;
  value: unknown;
  summary: string;
  kind: string;
};

export type QuoteChangeResult = {
  valid: boolean;
  saved: boolean;
  sentence: string;
  operations: ChangeOperation[];
  draft: QuoteDraft | null;
  resulting_total_minor: number;
  currency: string;
  issues: Issue[];
  usage: { prompt_tokens: number; completion_tokens: number };
  model_info: { provider: string; model: string };
  error?: string;
};

export type ContractChangeResult = Omit<QuoteChangeResult, "draft"> & {
  draft: ContractDraft | null;
};

export type ExtractedField = { key: string; value: string };

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
        : reason === "not_granted" || reason === "ai_surface_not_granted"
          ? "This surface has not been granted to you."
          : "The draft did not come back.";
  return {
    refusal: status === 403 ? (reason ?? "ai_refused") : status === 429 ? "rate_limited" : "request_failed",
    message,
  };
}

async function postJson<T>(
  path: string,
  body: unknown,
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
      body: JSON.stringify(body),
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
  const parsed = (await response.json().catch(() => null)) as T | null;
  if (parsed === null) return { ok: false, refusal: "transport", message: "The answer was not JSON." };
  return { ok: true, result: parsed };
}

export type AuthorInput = {
  /** The one composer's text. Kept for the transcript-shaped call. */
  transcript?: string;
  sources?: MoneySource[];
  instruction?: string;
  client_id?: string | null;
};

export function authorQuoteDraft(input: AuthorInput | string, signal?: AbortSignal) {
  const body = typeof input === "string" ? { transcript: input } : input;
  return postJson<QuoteAuthorResult>("/api/v1/quotes/author", body, signal);
}

export function authorContractDraft(input: AuthorInput | string, signal?: AbortSignal) {
  const body = typeof input === "string" ? { transcript: input } : input;
  return postJson<ContractAuthorResult>("/api/v1/contracts/author", body, signal);
}

/**
 * The live chips while typing. Debounce at the call site: this is a read of the
 * words the panel already has, not a model call, but it still crosses the wire.
 */
export function extractFields(
  kind: MoneyKind,
  input: { transcript?: string; sources?: MoneySource[]; client_id?: string | null },
  signal?: AbortSignal,
) {
  const path = kind === "quote" ? "/api/v1/quotes/extract" : "/api/v1/contracts/extract";
  return postJson<{ fields: ExtractedField[]; characters: number }>(path, input, signal);
}

export function proposeQuoteChange(draft: QuoteDraft, instruction: string, signal?: AbortSignal) {
  return postJson<QuoteChangeResult>("/api/v1/quotes/author/change", { draft, instruction }, signal);
}

export function proposeContractChange(
  draft: ContractDraft,
  instruction: string,
  signal?: AbortSignal,
) {
  return postJson<ContractChangeResult>(
    "/api/v1/contracts/author/change",
    { draft, instruction },
    signal,
  );
}

/**
 * The approved clause library. Read-only; it is the only source of clause text.
 * Read with plain fetch rather than the generated client: the path is new in
 * this lane and the SDK is regenerated by the integration owner, so a typed
 * call would not compile until the pins move.
 */
export async function contractClauses(): Promise<ClauseRef[]> {
  const tokens = loadTokens();
  if (tokens === null) return [];
  try {
    const response = await fetch(`${apiBaseUrl()}/api/v1/contracts/clauses`, {
      headers: { accept: "application/json", authorization: `Bearer ${tokens.access_token}` },
    });
    if (!response.ok) return [];
    const rows = (await response.json()) as ClauseRef[] | null;
    return rows ?? [];
  } catch {
    return [];
  }
}

export type SourceUploadOutcome =
  | { accepted: true; source: MoneySource; length: number }
  | { accepted: false; reason: string; message: string };

/**
 * Upload a file as a source. The server reads text only, so a PDF or a Word
 * file comes back with a typed reason rather than a silent empty source.
 */
export async function uploadSource(file: File): Promise<SourceUploadOutcome> {
  const dataBase64 = await base64Of(file);
  const outcome = await postJson<{
    accepted: boolean;
    reason?: string;
    message?: string;
    length?: number;
    source?: MoneySource;
  }>("/api/v1/money-authoring/sources", {
    name: file.name,
    content_type: file.type,
    data_base64: dataBase64,
  });
  if (!outcome.ok) {
    return { accepted: false, reason: outcome.refusal, message: outcome.message };
  }
  if (!outcome.result.accepted || outcome.result.source === undefined) {
    return {
      accepted: false,
      reason: outcome.result.reason ?? "unsupported_type",
      message: outcome.result.message ?? "That file could not be read.",
    };
  }
  return {
    accepted: true,
    source: { ...outcome.result.source, id: `upload-${file.name}-${file.size}` },
    length: outcome.result.length ?? outcome.result.source.text.length,
  };
}

async function base64Of(file: File): Promise<string> {
  const buffer = await file.arrayBuffer();
  const bytes = new Uint8Array(buffer);
  let binary = "";
  const chunk = 0x8000;
  for (let index = 0; index < bytes.length; index += chunk) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunk));
  }
  return btoa(binary);
}

/** True when this user holds the grant for the surface and a model is usable. */
export async function moneyAuthoringAvailable(surfaceKey: string): Promise<boolean> {
  const { data, error } = await api.api.GET("/api/v1/ai/me/surfaces");
  if (error !== undefined || data === undefined) return false;
  const surface = data.surfaces.find((row) => row.surface_key === surfaceKey);
  return surface !== undefined && surface.granted && surface.models_available;
}

/**
 * Talk is the browser's own speech recognition: the words become text in the
 * composer and no audio is stored anywhere. When the browser has no
 * recogniser there is no Talk control at all — a disabled microphone would be
 * a promise the product cannot keep.
 */
type SpeechRecognitionLike = {
  lang: string;
  continuous: boolean;
  interimResults: boolean;
  start: () => void;
  stop: () => void;
  abort: () => void;
  onresult: ((event: SpeechResultEventLike) => void) | null;
  onerror: ((event: { error?: string }) => void) | null;
  onend: (() => void) | null;
};

type SpeechResultEventLike = {
  resultIndex: number;
  results: ArrayLike<
    ArrayLike<{ transcript: string }> & { isFinal: boolean }
  >;
};

type SpeechWindow = Window & {
  SpeechRecognition?: new () => SpeechRecognitionLike;
  webkitSpeechRecognition?: new () => SpeechRecognitionLike;
};

export function speechRecognitionCtor(): (new () => SpeechRecognitionLike) | null {
  if (typeof window === "undefined") return null;
  const scope = window as SpeechWindow;
  return scope.SpeechRecognition ?? scope.webkitSpeechRecognition ?? null;
}

/** Money is always in minor units; this only formats, it never computes a total. */
export function moneyFromMinor(amountMinor: number, currency: string): string {
  try {
    return new Intl.NumberFormat(undefined, {
      style: "currency",
      currency,
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    }).format(amountMinor / 100);
  } catch {
    return `${(amountMinor / 100).toFixed(2)} ${currency}`;
  }
}
