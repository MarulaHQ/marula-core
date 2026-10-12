// ConnectionBanner — offline / reconnecting / resyncing recovery surface
// (Wave 13 lane 2 + lane 9).
//
// Purpose: ONE quiet chip next to the outbox dock telling the user the
// connection state that matters to their queued writes: offline (changes
// are saved locally and will replay), reconnecting (the stream is
// backing off; pending changes stay queued), or resyncing (a reconnect
// with a Last-Event-ID cursor is rehydrating caches past the replay
// buffer — a brief honest "catching up", never an error). Gone the
// moment the wire is open and the browser is online — a status, never a
// dead end.
//
// Non-purpose: no retry buttons (replay is automatic and order-safe), no
// modal treatment; chrome-quiet tokens only per docs/theme.md.

"use client";

import {
  Badge,
} from "@marula-core/ui";
import { useConnectionState, useOutbox } from "../../lib/use-instant";

export function ConnectionBanner() {
  const { status, online, pending, resyncing } = useConnectionState();
  const { outcomes } = useOutbox();

  if (!online) {
    return (
      <Badge tone="warning" aria-live="polite">
        {pending > 0
          ? `Offline — ${pending} ${pending === 1 ? "change" : "changes"} will sync when you're back`
          : "Offline — changes will sync when you're back"}
      </Badge>
    );
  }
  if (resyncing) {
    return (
      <Badge tone="warning" aria-live="polite">
        Catching up with recent changes…
      </Badge>
    );
  }
  // The live leg only exists once something subscribes; a page without
  // instant channels stays quiet (no "connecting" noise on first load).
  const liveInUse = status !== "closed" || pending > 0 || outcomes.length > 0;
  if (liveInUse && status !== "open") {
    return (
      <Badge tone="warning" aria-live="polite">
        {pending > 0
          ? `Reconnecting — ${pending} ${pending === 1 ? "change" : "changes"} queued`
          : "Reconnecting…"}
      </Badge>
    );
  }
  return null;
}
