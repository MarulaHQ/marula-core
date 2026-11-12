// ConnectionPresence — the "● Online" line in the top bar.
//
// Purpose: the mockup draws one presence line in the header: a dot and a word.
// This is that line, and it tells the truth about the wire instead of a static
// label — green for a healthy connection, blush when the user should expect a
// delay (offline, reconnecting, rehydrating past the replay buffer). The queue
// depth rides along while there is one, because "Offline" alone leaves the
// person wondering whether their last three edits survived.
//
// Non-purpose: no retry button (replay is automatic and order-safe), no panel,
// no toast. The queue's per-operation outcomes live in OutboxDock; this is the
// state, not the ledger.
//
// Colour is never the only signal: the word is always there, and the dot is
// aria-hidden decoration behind it.

"use client";

import { useConnectionState } from "../../lib/use-instant";

type Tone = "healthy" | "attention";

const DOT: Record<Tone, string> = {
  healthy: "bg-mc-green",
  attention: "bg-mc-attn",
};

export function ConnectionPresence({ className = "" }: { className?: string }) {
  const { status, online, pending, resyncing } = useConnectionState();
  const liveInUse = status !== "closed" || pending > 0;

  // Most specific state first: the browser's own network flag outranks the
  // stream (a closed stream while offline is not "reconnecting"), and a resync
  // outranks a plain reconnect because it explains a longer wait.
  let tone: Tone = "healthy";
  let label = "Online";
  if (!online) {
    tone = "attention";
    label = pending > 0 ? `Offline · ${pending} queued` : "Offline";
  } else if (resyncing) {
    tone = "attention";
    label = "Catching up…";
  } else if (liveInUse && status !== "open") {
    tone = "attention";
    label = "Reconnecting…";
  }

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
