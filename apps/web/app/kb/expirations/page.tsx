"use client";
// /kb/expirations — every date that is running out, across the whole org.
//
// Purpose: one question, one answer. Domains, certificates, document reviews,
// warranties and credential rotation report here with the date, the owner and
// where the date came from. A typed warranty never reads like a registry fact.

import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { useEffect, useMemo, useState } from "react";
import { useRequireAuth } from "../../../lib/api";
import { SurfaceHeader } from "@marula-core/ui";
import { ErrorNote, PageFrame, SkeletonRows } from "../../../components/knowledge/ledger";
import { ExpiryTable } from "../../../components/knowledge/expiry-table";
import {
  knowledgeApi,
  type ExpiryRow,
  type KnowledgeRequestError,
} from "../../../components/knowledge/knowledge-api";

const KINDS = [
  { id: "all", label: "Everything" },
  { id: "domain_registrar", label: "Domain registration" },
  { id: "domain_tls", label: "TLS certificates" },
  { id: "document_review", label: "Document reviews" },
  { id: "asset_warranty", label: "Warranties" },
  { id: "secret_rotation", label: "Credential rotation" },
] as const;

export default function ExpirationsPage() {
  useRequireAuth();
  const search = useSearchParams();
  const clientId = search.get("client_id");
  const [rows, setRows] = useState<ExpiryRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [kind, setKind] = useState<string>("all");
  const [days, setDays] = useState(90);

  useEffect(() => {
    let cancelled = false;
    setRows(null);
    setError(null);
    const params = new URLSearchParams({ days: String(days) });
    if (clientId !== null) params.set("client_id", clientId);
    knowledgeApi
      .get<ExpiryRow[]>(`/api/v1/knowledge/expirations?${params.toString()}`)
      .then((data) => {
        if (!cancelled) setRows(data);
      })
      .catch((e: KnowledgeRequestError) => {
        if (!cancelled) setError(e.message);
      });
    return () => {
      cancelled = true;
    };
  }, [clientId, days]);

  const filtered = useMemo(
    () => (rows ?? []).filter((row) => kind === "all" || row.kind === kind),
    [rows, kind],
  );

  return (
    <PageFrame>
      <div className="flex flex-col gap-3 pt-5">
        <SurfaceHeader
          title="Expirations"
          description="Every date that matters, and who owns it. Blush means soon; red means it has already lapsed."
          actions={
            <>
              <label className="flex items-center gap-2 text-[13px]">
              <span style={{ color: "var(--m-text-secondary)" }}>Window</span>
              <select
              value={days}
              onChange={(event) => setDays(Number(event.target.value))}
              className="h-10 rounded-[10px] border px-2.5 text-[13px]"
              style={{
              borderColor: "var(--m-border)",
              background: "var(--m-surface)",
              color: "var(--m-text)",
              }}
              >
              {[30, 60, 90, 180, 365].map((value) => (
              <option key={value} value={value}>
              Next {value} days
              </option>
              ))}
              </select>
              </label>
              <Link href="/kb" className="text-[13px] font-semibold underline underline-offset-2">
              Knowledge
              </Link>
            </>
          }
        />

        <div className="flex flex-wrap items-center gap-2" role="group" aria-label="Filter by kind">
          {KINDS.map((option) => (
            <button
              key={option.id}
              type="button"
              aria-pressed={kind === option.id}
              onClick={() => setKind(option.id)}
              className="h-8 rounded-full border px-3 text-[13px] font-semibold transition-colors duration-100"
              style={{
                borderColor: kind === option.id ? "transparent" : "var(--m-border)",
                background: kind === option.id ? "var(--m-accent)" : "transparent",
                color: kind === option.id ? "var(--m-on-accent)" : "var(--m-text)",
              }}
            >
              {option.label}
            </button>
          ))}
        </div>

        {error !== null ? <ErrorNote title="Expirations did not load.">{error}</ErrorNote> : null}
        {rows === null ? (
          <SkeletonRows rows={8} sentence="Collecting expiries" columns={5} />
        ) : (
          <>
            <p className="text-[13px] tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
              {filtered.length} {filtered.length === 1 ? "item" : "items"} in the next {days} days
              {clientId !== null ? " for this client" : ""}.
            </p>
            <ExpiryTable rows={filtered} showClient={clientId === null} />
          </>
        )}
      </div>
    </PageFrame>
  );
}
