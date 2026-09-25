// Client-generated idempotency keys.
//
// Purpose: one place mints the key every queued write carries, so a replay
// (reconnect, retry, app relaunch) presents the SAME key and the server's
// idempotency gate replays the original result instead of double-writing.
//
// Non-purpose: no persistence — the key is persisted with its queued op.

export type UuidFn = () => string;

/** RFC 4122 v4 from the platform crypto (expo-crypto in the app,
 * node:crypto in tests). Injected so the core stays platform-free. */
export function makeIdempotencyKey(uuid: UuidFn): string {
  return `rn-${uuid()}`;
}
