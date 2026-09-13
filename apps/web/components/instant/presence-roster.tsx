// PresenceRoster — who else is on this ticket, right now (Wave 13 lane 9).
//
// Purpose: one quiet roster row in the ticket detail: viewer names with
// an honest state each — "editing" (active heartbeat + editing flag),
// "viewing", or "idle" (heartbeat stopped for 90s+; also how a departed
// viewer reads before the server's TTL prune drops them). Ephemeral by
// design: nothing here is trail, contract, or history.
//
// Non-purpose: no avatars/avatars-service, no editing locks — presence
// informs, it never blocks another editor. Chrome-quiet tokens only per
// docs/theme.md, light+dark parity, no motion beyond opacity hovers.

"use client";

import { usePresenceRoster } from "../../lib/use-presence";

const STATE_LABEL: Record<string, string> = {
  editing: "editing",
  viewing: "viewing",
  idle: "idle",
};

function stateTone(state: string): string {
  if (state === "editing") return "var(--m-status-warning-fg, var(--m-text))";
  return "var(--m-text-muted)";
}

export function PresenceRoster({ ticketId }: { ticketId: string }) {
  const { viewers } = usePresenceRoster(ticketId);
  if (viewers.length === 0) return null;
  return (
    <div
      className="flex flex-wrap items-center gap-x-2 gap-y-1 px-3 pt-1 text-[11px]"
      data-testid="presence-roster"
      aria-label="Who is viewing this ticket"
    >
      <span style={{ color: "var(--m-text-muted)" }}>On this ticket:</span>
      {viewers.map((v) => (
        <span
          key={v.user_id}
          className="flex items-center gap-1"
          data-testid="presence-viewer"
          data-presence-state={v.state}
          title={
            v.state === "idle"
              ? `${v.name} · idle (no activity for ${v.ageSeconds}s)`
              : v.state === "editing"
                ? `${v.name} · editing now`
                : `${v.name} · viewing`
          }
        >
          <span
            aria-hidden
            className="inline-block h-1.5 w-1.5 rounded-full"
            style={{
              background: v.state === "editing" ? "var(--m-accent, var(--m-text))" : "var(--m-border)",
              opacity: v.state === "idle" ? 0.5 : 1,
            }}
          />
          <span style={{ color: v.state === "idle" ? "var(--m-text-muted)" : "var(--m-text)" }}>
            {v.name}
          </span>
          <span style={{ color: stateTone(v.state) }}>({STATE_LABEL[v.state]})</span>
        </span>
      ))}
    </div>
  );
}
