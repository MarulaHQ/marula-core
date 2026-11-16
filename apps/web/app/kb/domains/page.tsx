"use client";
// /kb/domains — domains, websites and the two dates that matter about them.

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
import { dueWords } from "../../../components/knowledge/expiry-table";
import {
  knowledgeApi,
  type Domain,
  type KnowledgeRequestError,
} from "../../../components/knowledge/knowledge-api";

export default function DomainsPage() {
  // A full page load restores the session after mount; a request fired before
  // that carries no bearer, so it waits for the authenticated status.
  const { status } = useRequireAuth();
  const search = useSearchParams();
  const clientId = search.get("client_id");
  const [rows, setRows] = useState<Domain[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (status !== "authed") return;
    let cancelled = false;
    setRows(null);
    setError(null);
    const query = clientId === null ? "" : `?client_id=${encodeURIComponent(clientId)}`;
    knowledgeApi
      .get<Domain[]>(`/api/v1/knowledge/domains${query}`)
      .then((data) => {
        if (!cancelled) setRows(data);
      })
      .catch((e: KnowledgeRequestError) => {
        if (!cancelled) setError(e.message);
      });
    return () => {
      cancelled = true;
    };
  }, [clientId, status]);

  return (
    <PageFrame>
      <div className="flex flex-col gap-3 pt-5">
        <SurfaceHeader
          title="Domains and SSL"
          description="Registration and certificate dates. Blush means close; red means it has already expired."
          actions={
            <>
              <ClientScopeSelect clientId={clientId} label="Filter by client" />
              <Link href="/kb/expirations" className="text-[13px] font-semibold underline underline-offset-2">
              Expirations
              </Link>
            </>
          }
        />

        {error !== null ? (
          <ErrorNote title="Domains did not load.">{error}</ErrorNote>
        ) : rows === null ? (
          <SkeletonRows rows={6} sentence="Loading domains" columns={5} />
        ) : rows.length === 0 ? (
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            No domains recorded. Add one and its registrar and certificate dates report into the
            expiry rollup.
          </p>
        ) : (
          <TableCard label="Domains and certificates">
            <table className="w-full border-collapse text-[13px]">
              <thead>
                <tr style={{ background: "var(--m-surface-2)" }}>
                  {["Domain", "Registrar", "Registration", "TLS certificate", "Hosting"].map((head) => (
                    <th key={head} className={HEAD_CELL_CLASS} style={{ color: "var(--m-text-muted)" }}>
                      {head}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {rows.map((domain) => {
                  const registrar = domain.registrar_expiry;
                  const ssl = domain.ssl_expiry;
                  const registrarWords = registrar !== null ? dueWords(registrar) : null;
                  const sslWords = ssl !== null ? dueWords(ssl) : null;
                  return (
                    <tr key={domain.id} style={{ borderBottom: "1px solid var(--m-border)" }}>
                      <td className="px-3 py-2 font-semibold">
                        {domain.domain}
                        <span className="ml-2 text-[12px] font-normal" style={{ color: "var(--m-text-muted)" }}>
                          {domain.kind === "website" ? "Website" : "Domain"}
                        </span>
                      </td>
                      <td className="px-3 py-2">{domain.registrar ?? "—"}</td>
                      <td
                        className="px-3 py-2"
                        style={{
                          color:
                            registrarWords?.tone === "danger"
                              ? "var(--m-status-danger-fg)"
                              : registrarWords?.tone === "warning"
                                ? "var(--m-status-due-fg)"
                                : undefined,
                        }}
                      >
                        {registrar === null ? "Not recorded" : `${registrar} · ${registrarWords?.text}`}
                      </td>
                      <td
                        className="px-3 py-2"
                        style={{
                          color:
                            sslWords?.tone === "danger"
                              ? "var(--m-status-danger-fg)"
                              : sslWords?.tone === "warning"
                                ? "var(--m-status-due-fg)"
                                : undefined,
                        }}
                      >
                        {ssl === null ? "Not recorded" : `${ssl} · ${sslWords?.text}`}
                      </td>
                      <td className="px-3 py-2">{domain.hosting ?? "—"}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </TableCard>
        )}
      </div>
    </PageFrame>
  );
}
