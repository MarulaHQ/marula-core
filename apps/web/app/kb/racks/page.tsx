"use client";
// /kb/racks — every rack for one client, and the way into its elevation.

import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState } from "react";
import { useRequireAuth } from "../../../lib/api";
import { SurfaceHeader } from "@marula-core/ui";
import {
  ErrorNote,
  HEAD_CELL_CLASS,
  PageFrame,
  SkeletonRows,
  TableCard,
} from "../../../components/knowledge/ledger";
import { ClientScopeSelect } from "../../../components/knowledge/client-scope-select";
import {
  knowledgeApi,
  type KnowledgeRequestError,
  type Rack,
} from "../../../components/knowledge/knowledge-api";

function RacksPage() {
  useRequireAuth();
  const search = useSearchParams();
  const clientId = search.get("client_id");
  const [rows, setRows] = useState<Rack[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (clientId === null) {
      setRows([]);
      return;
    }
    let cancelled = false;
    setRows(null);
    setError(null);
    knowledgeApi
      .get<Rack[]>(`/api/v1/knowledge/racks?client_id=${encodeURIComponent(clientId)}`)
      .then((data) => {
        if (!cancelled) setRows(data);
      })
      .catch((e: KnowledgeRequestError) => {
        if (!cancelled) setError(e.message);
      });
    return () => {
      cancelled = true;
    };
  }, [clientId]);

  return (
    <PageFrame>
      <div className="flex flex-col gap-3 pt-5">
        <SurfaceHeader
          title="Racks"
          description="Physical layouts: what is installed, in which unit, on which face."
          actions={
            <>
              <ClientScopeSelect clientId={clientId} />
              <Link href="/kb" className="text-[13px] font-semibold underline underline-offset-2">
              Knowledge
              </Link>
            </>
          }
        />

        {error !== null ? (
          <ErrorNote title="Racks did not load.">{error}</ErrorNote>
        ) : rows === null ? (
          <SkeletonRows rows={6} sentence="Loading racks" columns={4} />
        ) : clientId === null ? (
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Choose a client to see its racks.
          </p>
        ) : rows.length === 0 ? (
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            No racks for this client yet. A rack record shows the elevation, its power load and the
            devices installed in it.
          </p>
        ) : (
          <TableCard label="Racks">
            <table className="w-full border-collapse text-[13px]">
              <thead>
                <tr style={{ background: "var(--m-surface-2)" }}>
                  {["Rack", "Location", "Size", "Power", "Updated"].map((head) => (
                    <th key={head} className={HEAD_CELL_CLASS} style={{ color: "var(--m-text-muted)" }}>
                      {head}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {rows.map((rack) => (
                  <tr key={rack.id} style={{ borderBottom: "1px solid var(--m-border)" }}>
                    <td className="px-3 py-2 font-semibold">
                      <Link href={`/kb/racks/${rack.id}`} className="underline underline-offset-2">
                        {rack.name}
                      </Link>
                      {rack.code !== null ? (
                        <span className="ml-2 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                          {rack.code}
                        </span>
                      ) : null}
                    </td>
                    <td className="px-3 py-2">{rack.location_note ?? "—"}</td>
                    <td className="px-3 py-2 tabular-nums">{rack.units}U</td>
                    <td className="px-3 py-2">
                      {rack.power_used_kw !== null && rack.power_capacity_kw !== null
                        ? `${rack.power_used_kw} of ${rack.power_capacity_kw} kW`
                        : "Not recorded"}
                    </td>
                    <td className="px-3 py-2 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                      {rack.updated_at.slice(0, 10)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </TableCard>
        )}
      </div>
    </PageFrame>
  );
}

// The route reads its filters from the query string, which bails the page out
// of prerendering. Next requires a Suspense boundary for that at build time.
export default function RacksRoute() {
  return (
    <Suspense fallback={null}>
      <RacksPage />
    </Suspense>
  );
}
