// One roster card: identity, one status, one action (§2).
//
// Purpose: draw a connector the way the reference rosters do — Calendly,
// Tettra and LaunchNotes each render one row per integration with a single
// action, and none of them puts a configuration control on the row. The card
// therefore stops at "what is this, is it working, and what do I do next";
// everything else lives in the connection detail.
//
// Non-purpose: no fetch, no expansion. `onOpen` is the only way into detail.
// A connector with no connections is a real card — that is what an install
// looks like before it is configured — so it gets `Connect` rather than being
// hidden, and its secondary line says "Installed but not connected yet"
// instead of doing arithmetic on zero. Connect is a link to Settings → Plugins
// because the credential step belongs to the plugin consent flow today; the
// roster does not invent a connection form (see `connection-detail.tsx`).
"use client";

import Link from "next/link";
import { Button, Card, StatusPill, statusOfConnection } from "@marula-core/ui";
import type { ConnectorCardModel } from "../../../lib/connectors/roster";
import type { ConnectionView } from "../../../lib/connectors/types";

/** The secondary line: the count, the scoped clients, and what needs attention. */
export function rosterMetaLine(card: ConnectorCardModel): string {
  let line: string;
  if (card.total === 0) {
    // "0 of 0" is arithmetic, not information. The install-first contract
    // exists to show this state, so the line says it in words.
    line = "Installed but not connected yet";
  } else if (card.total === 1) {
    line = "1 connection";
  } else if (card.unhealthy > 0) {
    line = `${card.unhealthy} of ${card.total} needing attention`;
  } else {
    line = `${card.total} connections`;
  }
  return card.scopedClients.length > 0
    ? `${line} · ${card.scopedClients.join(", ")}`
    : line;
}

export function ConnectorRosterCard({
  card,
  onOpen,
}: {
  card: ConnectorCardModel;
  onOpen: (connection: ConnectionView) => void;
}): React.JSX.Element {
  const first = card.connections[0];
  const firstUnhealthy = card.connections.find(
    (connection) => statusOfConnection(connection.status) === "needs_attention",
  );

  return (
    <Card
      data-testid={`connector-card-${card.connector}`}
      data-installation-id={card.installationId}
    >
      <div style={{ display: "flex", alignItems: "flex-start", gap: 14, padding: 16 }}>
        <span
          aria-hidden="true"
          style={{
            display: "inline-flex",
            alignItems: "center",
            justifyContent: "center",
            width: 36,
            height: 36,
            flex: "0 0 36px",
            borderRadius: "var(--m-radius-control)",
            border: "1px solid var(--m-border)",
            background: "var(--m-surface-2)",
            color: "var(--m-text-secondary)",
            fontFamily: "var(--m-font-mono)",
            fontSize: 15,
            fontWeight: 600,
            textTransform: "uppercase",
          }}
        >
          {card.name.slice(0, 1)}
        </span>

        <div style={{ display: "grid", gap: 3, flex: 1, minWidth: 0 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
            <h2
              style={{
                margin: 0,
                fontSize: 14,
                fontWeight: 600,
                color: "var(--m-text)",
              }}
            >
              {card.name}
            </h2>
            <StatusPill status={card.status} />
          </div>
          <p
            style={{
              margin: 0,
              fontSize: 13,
              color: "var(--m-text-secondary)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {card.purpose}
          </p>
          <p style={{ margin: 0, fontSize: 12, color: "var(--m-text-muted)" }}>
            {rosterMetaLine(card)}
          </p>
        </div>

        <div style={{ flex: "0 0 auto" }}>
          {card.action === "connect" || first === undefined ? (
            <Button asChild size="sm" variant="primary">
              <Link href="/settings/plugins">Connect</Link>
            </Button>
          ) : card.action === "fix" ? (
            // Fix opens the failing connection, not the connector in general:
            // the health section is where the reason and the canary live. With
            // no unhealthy row identified (an unknown status string), the
            // first connection is still the honest target.
            <Button
              size="sm"
              variant="secondary"
              onClick={() => onOpen(firstUnhealthy ?? first)}
              data-testid={`connector-fix-${card.connector}`}
            >
              Fix
            </Button>
          ) : (
            <Button
              size="sm"
              variant="secondary"
              onClick={() => onOpen(first)}
              data-testid={`connector-manage-${card.connector}`}
            >
              Manage
            </Button>
          )}
        </div>
      </div>
    </Card>
  );
}
