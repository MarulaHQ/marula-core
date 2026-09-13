// ConnectionBanner — offline / reconnecting recovery surface (Wave 13
// lane 2, instant-first doctrine beyond IF6).
//
// Purpose: ONE quiet chip next to the outbox dock telling the user the
// connection state that matters to their queued writes: offline (changes
// are saved locally and will replay), or reconnecting (the socket is
// backing off; pending changes stay queued). Gone the moment the wire is
// open and the browser is online — it is a status, never a dead end.
//
// Non-purpose: no retry buttons (replay is automatic and order-safe), no
// modal treatment; chrome-quiet tokens only per docs/theme.md.

"use client";

import { Tag } from "@meridian/ui";
import { useConnectionState, useOutbox } from "../../lib/use-instant";

export function ConnectionBanner() {
  const { status, online, pending } = useConnectionState();
  const { outcomes } = useOutbox();

  if (!online) {
    return (
      <Tag tone="warning" aria-live="polite">
        {pending > 0
          ? `Offline — ${pending} ${pending === 1 ? "change" : "changes"} will sync when you're back`
          : "Offline — changes will sync when you're back"}
      </Tag>
    );
  }
  // The live leg only exists once something subscribes; a page without
  // instant channels stays quiet (no "connecting" noise on first load).
  const liveInUse = status !== "closed" || pending > 0 || outcomes.length > 0;
  if (liveInUse && status !== "open") {
    return (
      <Tag tone="warning" aria-live="polite">
        {pending > 0
          ? `Reconnecting — ${pending} ${pending === 1 ? "change" : "changes"} queued`
          : "Reconnecting…"}
      </Tag>
    );
  }
  return null;
}
