// /assets — the devices list (Wave 15 lane 6).
//
// Purpose: the icon rail's Devices target — a real, searchable list of
// device assets backed by the typed SDK (GET /api/v1/assets with the q
// substring filter), so the rail's Devices entry is never a dead link.
// Rows deep-link into the existing asset detail at /assets/{id}.
//
// Non-purpose: no asset editing (client detail + settings own those
// flows); no client/site facets beyond the API's q filter here.
"use client";

import { Button, Card, EmptyState, Input, Spinner } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";

type Asset = components["schemas"]["DeviceAsset"];

export default function AssetsPage() {
  const auth = useRequireAuth();
  const [q, setQ] = useState("");
  const [debouncedQ, setDebouncedQ] = useState("");
  const [assets, setAssets] = useState<Asset[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const t = window.setTimeout(() => setDebouncedQ(q.trim()), 250);
    return () => window.clearTimeout(t);
  }, [q]);

  const refresh = useCallback(async (query: string) => {
    setAssets(null);
    setError(null);
    const result = await api.api.GET("/api/v1/assets", {
      params: { query: query.length > 0 ? { q: query } : {} },
    });
    if (result.error !== undefined || result.data === undefined) {
      setError("Failed to load devices. Check your connection and retry.");
      setAssets([]);
      return;
    }
    setAssets([...result.data].sort((a, b) => a.name.localeCompare(b.name)));
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh(debouncedQ);
  }, [auth.status, debouncedQ, refresh]);

  if (auth.status !== "authed") return null;

  return (
    <div className="mx-auto grid max-w-4xl gap-6 p-6">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold">Devices</h1>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Every managed asset in the org. Open one for its detail, links, and vault.
          </p>
        </div>
        <div className="w-64">
          <Input
            value={q}
            onChange={(event) => setQ(event.target.value)}
            placeholder="Search name, tag, serial…"
            aria-label="Search devices"
          />
        </div>
      </div>

      {assets === null ? (
        <Card>
          <div className="p-4">
            <Spinner />
          </div>
        </Card>
      ) : error !== null ? (
        <Card>
          <div className="flex items-center justify-between gap-3 p-4">
            <p role="alert" className="text-[13px]">
              {error}
            </p>
            <Button variant="secondary" size="sm" onClick={() => void refresh(debouncedQ)}>
              Retry
            </Button>
          </div>
        </Card>
      ) : assets.length === 0 ? (
        <Card>
          <div className="p-4">
            <EmptyState
              message={
                debouncedQ.length > 0
                  ? `No devices match “${debouncedQ}”.`
                  : "No devices yet. Assets appear here once connected."
              }
            />
          </div>
        </Card>
      ) : (
        <Card>
          <ul className="grid">
            {assets.map((asset) => (
              <li
                key={asset.id}
                className="row-hover border-t first:border-t-0"
                style={{ borderColor: "var(--m-border)" }}
              >
                <Link
                  href={`/assets/${asset.id}`}
                  className="flex items-center gap-3 px-4 py-2 no-underline"
                  style={{ minHeight: "var(--m-density-row)" }}
                >
                  <span
                    className="w-24 shrink-0 font-mono text-[12.5px]"
                    style={{ color: "var(--m-text-muted)" }}
                  >
                    {asset.tag}
                  </span>
                  <span
                    className="min-w-0 flex-1 truncate text-[13px]"
                    style={{ color: "var(--m-text)" }}
                  >
                    {asset.name}
                  </span>
                  <span
                    className="hidden shrink-0 text-xs capitalize sm:inline"
                    style={{ color: "var(--m-text-muted)" }}
                  >
                    {asset.kind}
                  </span>
                  <span
                    className="w-20 shrink-0 text-right text-xs"
                    style={{ color: "var(--m-text-muted)" }}
                  >
                    {asset.status}
                  </span>
                </Link>
              </li>
            ))}
          </ul>
        </Card>
      )}
    </div>
  );
}
