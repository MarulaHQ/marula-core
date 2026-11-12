// ConnectionPresence — the "● Online" line in the top bar.
//
// Purpose: the mockup draws one presence line in the header: a dot and a word.
// This is that line, and it tells the truth about the wire instead of a static
// label — green for a healthy connection, blush when the person should expect a
// delay (offline, reconnecting, rehydrating past the replay buffer). The queue
// depth rides along while there is one, because "Offline" alone leaves the
// person wondering whether their last three edits survived.
//
// The word is always there and the dot is decoration, so colour is never the
// only signal. "Reconnecting" needs history: a first connect is not a
// reconnect, and a bar that says Reconnecting on every page load trains people
// to ignore it — so that state is claimed only after this page has held an open
// stream once.
//
// Non-purpose: no retry button (replay is automatic and order-safe), no panel,
// no toast. The queue's per-operation outcomes live in OutboxDock; this is the
// state, not the ledger.

"use client";

import { useEffect, useRef } from "react";
import { useConnectionState } from "../../lib/use-instant";

type Tone = "healthy" | "attention";

const DOT: Record<Tone, string> = {
  healthy: "bg-mc-green",
  attention: "bg-mc-attn",
};

export type PresenceState = {
  online: boolean;
  status: "connecting" | "open" | "closed";
  pending: number;
  resyncing: boolean;
  /** True once this page has held an open stream, which is what makes "Reconnecting…" honest. */
  everOpen: boolean;
};

/**
 * The mapping, pure so it can be tested without a stream. Most specific state
 * first: the browser's own network flag outranks the stream (a closed stream
 * while offline is not "reconnecting"), and a resync outranks a plain reconnect
 * because it explains a longer wait. A stream that has never opened is not a
 * reconnect and shows nothing at all — the line stays Online.
 */
export function presenceOf({ online, status, pending, resyncing, everOpen }: PresenceState): {
  tone: Tone;
  label: string;
} {
  const liveInUse = status !== "closed" || pending > 0;
  if (!online) {
    return {
      tone: "attention",
      label: pending > 0 ? `Offline · ${pending} queued` : "Offline",
    };
  }
  if (resyncing) return { tone: "attention", label: "Catching up…" };
  if (everOpen && liveInUse && status !== "open") {
    return { tone: "attention", label: "Reconnecting…" };
  }
  return { tone: "healthy", label: "Online" };
}

export function ConnectionPresence({ className = "" }: { className?: string }) {
  const { status, online, pending, resyncing } = useConnectionState();
  const opened = useRef(false);
  useEffect(() => {
    if (status === "open") opened.current = true;
  }, [status]);
  const { tone, label } = presenceOf({
    online,
    status,
    pending,
    resyncing,
    everOpen: opened.current,
  });

  return (
    <span
      data-testid="connection-presence"
      data-state={tone === "healthy" ? "online" : "degraded"}
      aria-live="polite"
      className={`inline-flex items-center gap-2 whitespace-nowrap text-sm text-foreground ${className}`}
    >
      <span aria-hidden="true" className={`size-2 shrink-0 rounded-full ${DOT[tone]}`} />
      {label}
    </span>
  );
}
