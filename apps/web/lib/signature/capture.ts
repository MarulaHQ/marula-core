// Sign-off capture — the web half of the consent/queue/transport seam.
//
// Purpose: submit one captured signature and, when the network is down, hand
// the exact same op to the instant runtime's durable OfflineQueue so it
// replays in order with the same idempotency key. A signature taken at the
// door with no signal must survive, so this path deliberately does NOT use
// the envelope-write helper (which surfaces the error and abandons the
// write) — it uses the same durable queue the rest of the board rides, and
// the exact same transport the queue replays through.
//
// Non-purpose: no optimistic render. A signature is evidence, and rendering
// an unconfirmed one on a ticket would be inventing a fact; the pad shows
// its own idle/ready/queued state instead.

import { api } from "../api";
import { instantRuntime } from "../instant/runtime";
import { newIdempotencyKey } from "../instant/instant-write";
import type { components, MarulaPaths } from "@marula-core/sdk";

/** Consent id + version the pad must send back, so the server can refuse a
 *  stale pad that is about to agree to wording the signer never read.
 *
 *  Derived from the generated SDK rather than declared structurally, so a
 *  server-side change to the consent shape is a typecheck failure here instead
 *  of a silent field rename. */
/** The consent, narrowed to the two fields the capture actually needs, taken
 *  from the generated schema. */
export type ConsentPin = Pick<
  components["schemas"]["SignOffConsent"],
  "id" | "version" | "body"
>;

export interface CaptureArgs {
  ticketId: string;
  /** Base64 PNG, or "" for a typed-name agreement. */
  imageBase64: string;
  /** Who signed, as they typed it. */
  signerName: string;
  /** Device-clock reading when the stroke finished. */
  capturedAt: string;
  consent: ConsentPin;
  /** Set when this capture corrects an earlier sign-off. */
  supersedesSignOffId?: string;
}

export type CaptureResult =
  | { status: "accepted" }
  | { status: "queued"; key: string }
  | { status: "rejected"; reason: string; message?: string };

const SIGN_OFF_PATH = "/api/v1/tickets/{id}/sign-offs";
const CONSENT_PATH = "/api/v1/org/sign-off-consents/current";

/** Reads the consent wording the pad must show before capture. */
export async function currentConsent(): Promise<ConsentPin> {
  const { data, error } = await api.api.GET(CONSENT_PATH);
  if (error !== undefined || data === undefined) {
    throw new Error("The consent wording could not be loaded.");
  }
  return { id: data.id, version: data.version, body: data.body };
}

/**
 * Submits one signature. A network failure queues it durably and replays;
 * everything else is a typed rejection the caller renders.
 */
export async function captureSignOff(args: CaptureArgs): Promise<CaptureResult> {
  const key = newIdempotencyKey();
  // The body is the generated request type, not a bag of strings: a field the
  // server requires and the pad stopped sending is a typecheck failure here.
  type SignOffBody = NonNullable<
    MarulaPaths["/api/v1/tickets/{id}/sign-offs"]["post"]["requestBody"]
  >["content"]["application/json"];
  const body: SignOffBody = {
    image_base64: args.imageBase64,
    signer_name: args.signerName,
    captured_at: args.capturedAt,
    consent_version: args.consent.version,
    consent_id: args.consent.id,
    idempotency_key: key,
    ...(args.supersedesSignOffId !== undefined
      ? { supersedes_sign_off_id: args.supersedesSignOffId }
      : {}),
  };

  // openapi-fetch RESOLVES with `{ data, error }` for every HTTP status and
  // only THROWS when the request never reached the server, so the throw is
  // exactly the network leg that belongs in the durable queue.
  const POST = "/api/v1/tickets/{id}/sign-offs" as const;
  const outcome = await api.api
    .POST(POST, { params: { path: { id: args.ticketId } }, body })
    .catch((cause: unknown) => {
      void cause;
      return null;
    });
  if (outcome === null) return enqueue(key, body, args);

  if (outcome.error !== undefined) {
    const detail = errorDetail(outcome.error);
    return { status: "rejected", reason: detail.reason, message: detail.message };
  }
  return { status: "accepted" };
}

function enqueue(key: string, body: unknown, args: CaptureArgs): { status: "queued"; key: string } {
  const runtime = instantRuntime();
  runtime.queue.enqueue({
    key,
    method: "POST",
    path: SIGN_OFF_PATH,
    body,
    describe: `Sign-off for ${args.ticketId}`,
    entityId: args.ticketId,
  });
  // Replay immediately when the wire is in fact back; the queue's own
  // reentrancy guard makes this a no-op when it is not, and the reconnect
  // hook covers the rest of the session.
  void runtime.queue.replay();
  return { status: "queued", key };
}

interface ErrorDetail {
  reason: string;
  message?: string;
}

function errorDetail(error: unknown): ErrorDetail {
  if (typeof error !== "object" || error === null) {
    return { reason: "conflict", message: "The signature was rejected." };
  }
  const carrier = error as { reason?: unknown; message?: unknown; error?: unknown };
  const reason =
    typeof carrier.reason === "string"
      ? carrier.reason
      : typeof carrier.error === "string"
        ? carrier.error
        : "conflict";
  const message =
    typeof carrier.message === "string" ? carrier.message : "The signature was rejected.";
  return { reason, message };
}
