//! Signed webhook receiver for Meridian events (the IN half).
//!
//! Purpose: accept POSTs from a Meridian webhook subscription and verify
//! them EXACTLY the way first-party receivers must: constant-time HMAC-
//! SHA256 over `"{t}.{rawBody}"` with the `t=<ts>,v1=<hex>` header, plus
//! the ±10-minute anti-replay window. Raw bytes are verified BEFORE any
//! JSON parsing — signature covers bytes, not objects.

import { createHmac, timingSafeEqual } from "node:crypto";

export const SIGNATURE_HEADER = "x-meridian-signature";
/** Generous for queue delay, tight enough to kill replays (~10 min). */
export const REPLAY_TOLERANCE_SECS = 600;

export type VerifyOutcome =
  | { ok: true }
  | { ok: false; reason: "malformed-header" | "outside-window" | "bad-signature" };

function hmacHex(secret: string, message: Buffer): string {
  return createHmac("sha256", secret).update(message).digest("hex");
}

/** Receiver-side verification of one delivery's headers + raw body. */
export function verifyDelivery(
  secret: string,
  rawBody: Buffer,
  signatureHeader: string | undefined,
  nowUnix: number = Math.floor(Date.now() / 1000),
): VerifyOutcome {
  if (!secret) return { ok: false, reason: "malformed-header" };
  if (!signatureHeader) return { ok: false, reason: "malformed-header" };
  const parts = signatureHeader.trim().split(",");
  if (parts.length !== 2) return { ok: false, reason: "malformed-header" };
  const tPart = parts[0] ?? "";
  const vPart = parts[1] ?? "";
  if (!tPart.startsWith("t=") || !vPart.startsWith("v1=")) {
    return { ok: false, reason: "malformed-header" };
  }
  const ts = Number(tPart.slice(2));
  const presented = vPart.slice(3).toLowerCase();
  if (!Number.isFinite(ts) || !/^[0-9a-f]+$/.test(presented)) {
    return { ok: false, reason: "malformed-header" };
  }
  if (Math.abs(nowUnix - ts) > REPLAY_TOLERANCE_SECS) {
    return { ok: false, reason: "outside-window" };
  }
  const expected = hmacHex(
    secret,
    Buffer.concat([Buffer.from(String(ts)), Buffer.from("."), rawBody]),
  );
  const a = Buffer.from(expected, "utf8");
  const b = Buffer.from(presented.padEnd(expected.length, "\0"), "utf8");
  // Length mismatch short-circuits; lengths are not secret.
  if (presented.length !== expected.length || !timingSafeEqual(a, b)) {
    return { ok: false, reason: "bad-signature" };
  }
  return { ok: true };
}
