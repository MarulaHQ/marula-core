"use client";
/**
 * StatusPill — the one four-state connector pill.
 *
 * Purpose: connectors speak a status vocabulary the REST contract does not
 * spell the same way the screen does (`connected`, `degraded`, `auth_failed`,
 * `disabled`, and whatever a future adapter adds). Drawing that mapping at
 * every call site is how a roster, a chip menu and a detail panel end up
 * disagreeing about whether a connection is "Degraded" or "Needs attention".
 * The mapping lives here; the pill is the same four words everywhere.
 *
 * It is a thin wrapper over `Badge`, not a new pill. `Badge` already renders
 * the design system's status pair at the right size and radius, and a second
 * status primitive would be a second set of tones to keep in sync. What
 * `Badge` cannot own is the semantics — `Badge` takes a tone, and a caller
 * should not have to know that "auth_failed" is warning-coloured. That
 * decision is this component's whole job.
 *
 * Severity order, for a connector with several connections: any unhealthy
 * connection wins (`Needs attention`), then any healthy one (`Connected`),
 * then `Disabled`, then `Not connected`. A connector with three healthy
 * client connections and one failed one is not "connected with a footnote";
 * the one failure is the thing an operator must act on.
 *
 * Design research (Calendly, Tettra, LaunchNotes): a roster row carries one
 * status and one action, and the status is a word, not a colour swatch. Superlist
 * and Glide spend their single accent on one element per view; this pill is
 * usually that element, so it uses the fixed status pairs and never a new
 * accent of its own (docs/theme.md §3.4, Ember ≤5%).
 */
import type { ReactNode } from "react";
import { Badge, type BadgeTone } from "./badge";
import { cn } from "../lib/cn";

/** The four states a connector card, a chip and a detail panel all name. */
export type ConnectorStatus = "connected" | "needs_attention" | "not_connected" | "disabled";

interface StatusPresentation {
  label: string;
  tone: BadgeTone;
}

const PRESENTATION: Record<ConnectorStatus, StatusPresentation> = {
  connected: { label: "Connected", tone: "success" },
  // `warning` is the amber "look at this" pair, not `due`: due is a date and
  // an auth failure is not a date (docs/theme.md §3.4).
  needs_attention: { label: "Needs attention", tone: "warning" },
  not_connected: { label: "Not connected", tone: "neutral" },
  disabled: { label: "Disabled", tone: "offline" },
};

/**
 * The raw status of one connection, folded into the four the UI names.
 * `needs_attachment` is the contract's "a step is missing" state (a consent,
 * a channel, a credential), and it is repair work: it reads Needs attention,
 * not Not connected, because the connection exists and is half-configured.
 * Unknown strings are `not_connected` rather than an error: a connector from a
 * marketplace release newer than this screen must still render, and the
 * conservative reading ("we cannot prove it is healthy") is the honest one.
 */
export function statusOfConnection(raw: string): ConnectorStatus {
  switch (raw.trim().toLowerCase()) {
    case "connected":
    case "ok":
    case "healthy":
      return "connected";
    case "degraded":
    case "auth_failed":
    case "needs_attachment":
    case "error":
    case "failed":
    case "needs_attention":
      return "needs_attention";
    case "disabled":
    case "paused":
      return "disabled";
    default:
      return "not_connected";
  }
}

/**
 * The worst state across a connector's connections. Empty means the connector
 * is installed with nothing bound yet, which reads `Not connected`.
 */
export function statusOfConnector(
  connections: readonly { status: string }[],
): ConnectorStatus {
  if (connections.length === 0) return "not_connected";
  const states = connections.map((connection) => statusOfConnection(connection.status));
  if (states.includes("needs_attention")) return "needs_attention";
  if (states.includes("connected")) return "connected";
  if (states.includes("disabled")) return "disabled";
  return "not_connected";
}

export interface StatusPillProps {
  status: ConnectorStatus;
  className?: string;
}

export function StatusPill({ status, className }: StatusPillProps): ReactNode {
  const presentation = PRESENTATION[status];
  return (
    <Badge
      tone={presentation.tone}
      className={cn(className)}
      data-status={status}
      data-testid="status-pill"
    >
      {presentation.label}
    </Badge>
  );
}
