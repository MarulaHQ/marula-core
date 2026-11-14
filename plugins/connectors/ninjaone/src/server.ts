//! The signed webhook receiver, as a handler a test can drive.
//!
//! Purpose: verify the signature over the raw bytes, cap the body while it is
//! still being read, forward the event, and answer only once the forward has
//! settled. The order matters: a 200 is a promise that the work happened, and a
//! 5xx is what makes the sender retry, because there is no queue in this design.
//!
//! Separation: this module knows HTTP and nothing about NinjaOne. `delivery.ts`
//! owns the mapping.

import type { IncomingMessage, ServerResponse } from "node:http";

import { DeliveryRefusal } from "./delivery.js";
import { SIGNATURE_HEADER, verifyDelivery } from "./receiver.js";

/** The one path a delivery may arrive on. Documented in README.md. */
export const WEBHOOK_PATH = "/marula/hook";

/**
 * Hard ceiling on one delivery body, enforced while accumulating.
 *
 * The signature covers the whole body, so nothing can be verified until the
 * body is complete; without a ceiling an unauthenticated peer could make the
 * process buffer without limit before that check ever runs. A Marula event
 * envelope is a single record and runs to a few kilobytes, so 64 KiB leaves
 * room for a large ticket or client payload and still bounds the damage.
 */
export const MAX_WEBHOOK_BYTES = 64 * 1024;

export interface WebhookOptions {
  /** Shared secret from the webhook subscription. */
  secret: string;
  /**
   * Performs the work for one verified delivery. Throws `DeliveryRefusal` for a
   * delivery that cannot be built, and anything else for a failed forward.
   */
  forward: (rawBody: Buffer) => Promise<void>;
  logger?: { warn: (message: string) => void };
}

interface JsonBody {
  [key: string]: unknown;
}

function respond(
  response: ServerResponse,
  status: number,
  body: JsonBody,
): void {
  if (response.headersSent) return;
  const payload = JSON.stringify(body);
  response.writeHead(status, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(payload),
  });
  response.end(payload);
}

/**
 * Builds the request handler. The returned function is safe to hand straight to
 * `createServer`, which is what makes the status codes testable without a
 * process.
 */
export function createWebhookHandler(
  options: WebhookOptions,
): (request: IncomingMessage, response: ServerResponse) => void {
  const logger = options.logger ?? console;

  return function handle(
    request: IncomingMessage,
    response: ServerResponse,
  ): void {
    if (request.method !== "POST") {
      respond(response, 405, { error: "method not allowed" });
      return;
    }
    const path = (request.url ?? "").split("?")[0];
    if (path !== WEBHOOK_PATH) {
      respond(response, 404, { error: "unknown path", path: WEBHOOK_PATH });
      return;
    }

    const chunks: Buffer[] = [];
    let received = 0;
    let tooLarge = false;

    // A client that goes away mid-body must not bring the process down.
    request.on("error", () => undefined);
    response.on("error", () => undefined);

    request.on("data", (chunk: Buffer) => {
      if (tooLarge) return;
      received += chunk.length;
      if (received > MAX_WEBHOOK_BYTES) {
        tooLarge = true;
        // Answer before closing the read side: the peer may still be sending,
        // and the point of the 413 is that it can read the reason.
        response.once("finish", () => request.destroy());
        respond(response, 413, {
          error: "body too large",
          field: "body",
          limit: MAX_WEBHOOK_BYTES,
        });
        return;
      }
      chunks.push(chunk);
    });

    request.on("end", () => {
      if (tooLarge) return;
      // Signature first, and over the raw bytes: nothing is parsed before it.
      const presented = request.headers[SIGNATURE_HEADER];
      const signatureHeader = Array.isArray(presented)
        ? presented[0]
        : presented;
      const outcome = verifyDelivery(
        options.secret,
        Buffer.concat(chunks),
        signatureHeader,
      );
      if (!outcome.ok) {
        logger.warn(`rejected delivery: ${outcome.reason}`);
        respond(response, 401, {
          error: "signature rejected",
          reason: outcome.reason,
        });
        return;
      }
      void deliver(Buffer.concat(chunks));
    });

    async function deliver(rawBody: Buffer): Promise<void> {
      try {
        await options.forward(rawBody);
      } catch (error) {
        if (error instanceof DeliveryRefusal) {
          respond(response, 400, {
            error: error.message,
            field: error.field,
          });
          return;
        }
        logger.warn(`forward failed: ${String(error)}`);
        respond(response, 502, {
          error: "forward failed",
          detail: error instanceof Error ? error.message : String(error),
        });
        return;
      }
      // Only now is the delivery acknowledged: the vendor's retry is the only
      // retry this design has.
      respond(response, 200, { ok: true });
    }
  };
}
