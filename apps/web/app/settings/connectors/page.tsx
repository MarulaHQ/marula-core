// /settings/connectors — the connector roster (§2).
//
// Purpose: S1 of `docs/reference/connector-surface.md` — one card per
// connector, one status pill, one state-dependent action, search by name and
// purpose, and a connection detail reached from the card. The layout follows
// the reference rosters (Calendly, Tettra, LaunchNotes) rather than a table:
// a table forces a column decision per vendor and does not survive the
// marketplace, which is the opposite of what this surface is for.
//
// Non-purpose: no connector installation or consent flow (that lives under
// Settings → Plugins), no destination editor and no settings-schema renderer
// this round. The detail panel names each of those gaps where it reaches one
// rather than drawing a control that cannot act.
//
// This page is not yet in `lib/settings/ia.ts`: navigation for the settings
// area is owned by another lane. The route is real and reachable by URL.
"use client";

import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Button,
  EmptyState,
  Input,
  Skeleton,
  SkeletonGroup,
} from "@marula-core/ui";
import { SettingsHeader } from "../../../components/settings/settings-shell";
import { apiBaseUrl, authHeaders, useRequireAuth } from "../../../lib/api";
import { friendlyMutationText } from "../../../lib/error-copy";
import { buildRoster, filterRoster } from "../../../lib/connectors/roster";
import type {
  ConnectorInstallationView,
  ConnectorsRosterResponse,
} from "../../../lib/connectors/types";
import { ConnectorRosterCard } from "./roster-card";
import { ConnectionDetail } from "./connection-detail";

/** The loading shape repeats the card's geometry so nothing jumps on arrival. */
function RosterSkeleton(): React.JSX.Element {
  return (
    <SkeletonGroup label="Loading connectors">
      <div style={{ display: "grid", gap: 12 }} aria-hidden="true">
        {[0, 1, 2].map((index) => (
          <div
            key={index}
            style={{
              display: "flex",
              alignItems: "flex-start",
              gap: 14,
              padding: 16,
              border: "1px solid var(--m-border)",
              borderRadius: "var(--m-radius-container)",
              background: "var(--m-surface)",
            }}
          >
            <Skeleton shape="circle" width={36} height={36} />
            <div style={{ display: "grid", gap: 7, flex: 1 }}>
              <Skeleton width="38%" />
              <Skeleton width="68%" height={10} />
              <Skeleton width="22%" height={10} />
            </div>
            <Skeleton shape="block" width={76} height={32} />
          </div>
        ))}
      </div>
    </SkeletonGroup>
  );
}

export default function ConnectorsSettingsPage(): React.JSX.Element | null {
  const auth = useRequireAuth();
  const [connectors, setConnectors] = useState<ConnectorInstallationView[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoadError(null);
    try {
      // Installation-first (§2): every installed connector is a card, with its
      // connections nested. A connector with none still appears, which is the
      // state an install starts in.
      const res = await fetch(`${apiBaseUrl()}/api/v1/connectors/roster`, {
        headers: authHeaders(),
        cache: "no-store",
      });
      if (!res.ok) {
        throw new Error(`The connector roster request failed (${String(res.status)}).`);
      }
      const body = (await res.json()) as Partial<ConnectorsRosterResponse>;
      if (!Array.isArray(body.connectors)) {
        throw new Error("The connector roster response carried no connectors list.");
      }
      setConnectors(body.connectors);
    } catch (error) {
      setLoadError(friendlyMutationText(error, "Failed to load connectors."));
    }
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void load();
  }, [auth.status, load]);

  const roster = useMemo(
    () => buildRoster(connectors ?? []),
    [connectors],
  );
  const visible = useMemo(() => filterRoster(roster, query), [roster, query]);

  const selectedInstallation =
    selectedId === null
      ? undefined
      : (connectors ?? []).find((installation) =>
          installation.connections.some((connection) => connection.id === selectedId),
        );
  const selected = selectedInstallation?.connections.find(
    (connection) => connection.id === selectedId,
  );

  if (auth.status !== "authed") return null;

  // A connection opens as a detail step, not a modal (§3). The back control
  // returns to the roster with its search still in place.
  if (selected !== undefined && selectedInstallation !== undefined) {
    return (
      <div className="grid gap-6 max-w-3xl">
        <ConnectionDetail
          connection={selected}
          siblings={selectedInstallation.connections}
          onSelect={(connection) => setSelectedId(connection.id)}
          onBack={() => setSelectedId(null)}
        />
      </div>
    );
  }

  return (
    <div className="grid gap-6 max-w-3xl">
      <SettingsHeader
        title="Connectors"
        description="The systems this org can work in, what each one is for, and whether it is working. A connection is not a webhook — signed delivery logs live under Integrations."
      />

      <div>
        <label htmlFor="connector-search" className="sr-only">
          Search connectors
        </label>
        <Input
          id="connector-search"
          type="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search connectors"
          aria-label="Search connectors"
        />
      </div>

      {loadError !== null ? (
        <div
          role="alert"
          data-testid="connectors-error"
          style={{
            display: "flex",
            alignItems: "center",
            gap: 12,
            padding: "10px 12px",
            borderRadius: "var(--m-radius-container)",
            background: "var(--m-status-danger-bg)",
            color: "var(--m-status-danger-fg)",
            fontSize: 13,
          }}
        >
          <span>{loadError}</span>
          <Button size="sm" variant="secondary" onClick={() => void load()}>
            Retry
          </Button>
        </div>
      ) : connectors === null ? (
        <RosterSkeleton />
      ) : roster.length === 0 ? (
        <EmptyState
          message="No connectors installed yet. Marula ships the standard set — RMM, remote access, documentation, notifications — and the marketplace adds the rest."
          action={
            <Button asChild size="sm" variant="primary">
              <Link href="/settings/plugins">Open the plugin marketplace</Link>
            </Button>
          }
        />
      ) : visible.length === 0 ? (
        <div
          style={{ display: "grid", gap: 8, justifyItems: "start" }}
          data-testid="connectors-no-matches"
        >
          <p style={{ margin: 0, fontSize: 13, color: "var(--m-text-secondary)" }}>
            No connectors match “{query.trim()}”.
          </p>
          <Button size="sm" variant="ghost" onClick={() => setQuery("")}>
            Clear search
          </Button>
        </div>
      ) : (
        <div style={{ display: "grid", gap: 12 }} data-testid="connector-roster">
          {visible.map((card) => (
            <ConnectorRosterCard
              // `installation_id` identifies the card: two installations of
              // the same connector are two roster entries, not one.
              key={card.installationId}
              card={card}
              onOpen={(connection) => setSelectedId(connection.id)}
            />
          ))}
        </div>
      )}
    </div>
  );
}
