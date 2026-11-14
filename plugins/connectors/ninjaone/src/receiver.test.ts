import { createHmac } from "node:crypto";
import assert from "node:assert/strict";
import { test } from "node:test";

import {
  REPLAY_TOLERANCE_SECS,
  SIGNATURE_HEADER,
  verifyDelivery,
} from "./receiver.js";

const SECRET = "whsec_test_not_a_real_secret";
const NOW = 1_700_000_000;
const BODY = Buffer.from(
  JSON.stringify({
    type: "ticket.created",
    payload: { ticket_id: "t-1001" },
  }),
);

function headerFor(
  secret: string,
  rawBody: Buffer,
  timestamp: number = NOW,
): string {
  const hex = createHmac("sha256", secret)
    .update(
      Buffer.concat([
        Buffer.from(String(timestamp)),
        Buffer.from("."),
        rawBody,
      ]),
    )
    .digest("hex");
  return `t=${timestamp},v1=${hex}`;
}

test("header name is the one Marula signs with", () => {
  assert.equal(SIGNATURE_HEADER, "x-marula-signature");
});

test("accepts a delivery signed over the raw bytes", () => {
  assert.deepEqual(verifyDelivery(SECRET, BODY, headerFor(SECRET, BODY), NOW), {
    ok: true,
  });
});

// The rejection paths are the point of this file: a receiver that accepts
// an unsigned payload is the bug worth stopping.

test("rejects a delivery with no signature header", () => {
  assert.deepEqual(verifyDelivery(SECRET, BODY, undefined, NOW), {
    ok: false,
    reason: "malformed-header",
  });
});

test("rejects every delivery when the configured secret is empty", () => {
  // An unset MARULA_WEBHOOK_SECRET must not degrade into "accept anything".
  assert.deepEqual(verifyDelivery("", BODY, headerFor("", BODY), NOW), {
    ok: false,
    reason: "malformed-header",
  });
});

test("rejects a signature made with a different secret", () => {
  assert.deepEqual(
    verifyDelivery(SECRET, BODY, headerFor("some-other-secret", BODY), NOW),
    { ok: false, reason: "bad-signature" },
  );
});

test("rejects a body changed after it was signed", () => {
  const signature = headerFor(SECRET, BODY);
  const tampered = Buffer.from(
    BODY.toString("utf8").replace("t-1001", "t-9999"),
  );
  assert.deepEqual(verifyDelivery(SECRET, tampered, signature, NOW), {
    ok: false,
    reason: "bad-signature",
  });
});

test("rejects a body with only its whitespace reserialized", () => {
  // Same JSON value, different bytes: the signature covers bytes, not objects.
  const spaced = Buffer.from('{"type": "ticket.created", "payload": {}}');
  const signature = headerFor(SECRET, spaced);
  const compact = Buffer.from(JSON.stringify(JSON.parse(spaced.toString())));
  assert.notDeepEqual(compact, spaced);
  assert.deepEqual(verifyDelivery(SECRET, compact, signature, NOW), {
    ok: false,
    reason: "bad-signature",
  });
});

test("accepts a timestamp at the edge of the replay window", () => {
  const timestamp = NOW - REPLAY_TOLERANCE_SECS;
  assert.deepEqual(
    verifyDelivery(SECRET, BODY, headerFor(SECRET, BODY, timestamp), NOW),
    { ok: true },
  );
});

test("rejects a stale timestamp one second beyond the window", () => {
  const timestamp = NOW - REPLAY_TOLERANCE_SECS - 1;
  assert.deepEqual(
    verifyDelivery(SECRET, BODY, headerFor(SECRET, BODY, timestamp), NOW),
    { ok: false, reason: "outside-window" },
  );
});

test("rejects a far-future timestamp", () => {
  const timestamp = NOW + REPLAY_TOLERANCE_SECS + 1;
  assert.deepEqual(
    verifyDelivery(SECRET, BODY, headerFor(SECRET, BODY, timestamp), NOW),
    { ok: false, reason: "outside-window" },
  );
});

test("rejects malformed header shapes", () => {
  const good = headerFor(SECRET, BODY);
  const value = good.split(",")[1] ?? "";
  const digest = value.slice(3);
  const malformed = [
    "",
    "   ",
    `t=${NOW}`, // one part
    `${good},v1=deadbeef`, // three parts
    `${value},t=${NOW}`, // wrong order
    `t=${NOW},v2=${digest}`, // wrong scheme version
    `${NOW},${digest}`, // no prefixes
    `t=not-a-number,v1=${digest}`, // unparsable timestamp
    `t=${NOW},v1=not-hex`, // unparsable digest
  ];
  for (const header of malformed) {
    assert.deepEqual(
      verifyDelivery(SECRET, BODY, header, NOW),
      { ok: false, reason: "malformed-header" },
      `expected ${JSON.stringify(header)} to be refused as malformed`,
    );
  }
});

test("rejects a digest of the wrong length", () => {
  // Parsed as hex, inside the window, but not 64 characters: refused as a
  // bad signature rather than a malformed header.
  const good = headerFor(SECRET, BODY);
  const digest = (good.split(",")[1] ?? "").slice(3);
  assert.deepEqual(
    verifyDelivery(SECRET, BODY, `t=${NOW},v1=${digest}0`, NOW),
    { ok: false, reason: "bad-signature" },
  );
});

test("accepts an upper-case digest", () => {
  const good = headerFor(SECRET, BODY);
  const digest = (good.split(",")[1] ?? "").slice(3);
  assert.deepEqual(
    verifyDelivery(SECRET, BODY, `t=${NOW},v1=${digest.toUpperCase()}`, NOW),
    { ok: true },
  );
});

test("verification does not parse the body", () => {
  // A body that is not JSON still verifies: parsing happens after the ACK,
  // and a parse failure is logged rather than retried (see index.ts).
  const notJson = Buffer.from("this is not json at all");
  assert.deepEqual(
    verifyDelivery(SECRET, notJson, headerFor(SECRET, notJson), NOW),
    { ok: true },
  );
});
