"use client";
// /kb/procedures — procedures for one client; each opens as a runbook.

import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { useEffect, useState } from "react";
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
  type Procedure,
} from "../../../components/knowledge/knowledge-api";

export default function ProceduresPage() {
  useRequireAuth();
  const search = useSearchParams();
  const clientId = search.get("client_id");
  const [rows, setRows] = useState<Procedure[] | null>(null);
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
      .get<Procedure[]>(`/api/v1/knowledge/procedures?client_id=${encodeURIComponent(clientId)}`)
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
          title="Procedures"
          description="Runbooks and checklists, so the next person repeats the work instead of rediscovering it."
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
          <ErrorNote title="Procedures did not load.">{error}</ErrorNote>
        ) : rows === null ? (
          <SkeletonRows rows={6} sentence="Loading procedures" columns={4} />
        ) : clientId === null ? (
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Choose a client to see its procedures.
          </p>
        ) : rows.length === 0 ? (
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            No procedures for this client yet. Write the runbook once and it stops being someone's
            memory.
          </p>
        ) : (
          <TableCard label="Procedures">
            <table className="w-full border-collapse text-[13px]">
              <thead>
                <tr style={{ background: "var(--m-surface-2)" }}>
                  {["Procedure", "Owner", "Checklist", "Client-visible", "Version"].map((head) => (
                    <th key={head} className={HEAD_CELL_CLASS} style={{ color: "var(--m-text-muted)" }}>
                      {head}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {rows.map((procedure) => (
                  <tr key={procedure.id} style={{ borderBottom: "1px solid var(--m-border)" }}>
                    <td className="px-3 py-2 font-semibold">
                      <Link
                        href={`/kb/procedures/${procedure.id}`}
                        className="underline underline-offset-2"
                      >
                        {procedure.title}
                      </Link>
                      {procedure.summary !== null ? (
                        <span className="block text-[12px] font-normal" style={{ color: "var(--m-text-secondary)" }}>
                          {procedure.summary}
                        </span>
                      ) : null}
                    </td>
                    <td className="px-3 py-2">{procedure.owner_name ?? "Unassigned"}</td>
                    <td className="px-3 py-2">{procedure.is_checklist ? "Steps" : "Body only"}</td>
                    <td className="px-3 py-2">{procedure.client_visible ? "Visible" : "Internal"}</td>
                    <td className="px-3 py-2 tabular-nums">v{procedure.version}</td>
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
