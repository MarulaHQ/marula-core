"use client";
// /kb/networks — the networks for one client, and the way into IPAM.

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
  type Network,
} from "../../../components/knowledge/knowledge-api";

const KIND_LABEL: Record<string, string> = {
  lan: "LAN",
  wan: "WAN",
  wifi: "Wi-Fi",
  vpn: "VPN",
  voice: "Voice",
  guest: "Guest",
  other: "Other",
};

function NetworksPage() {
  useRequireAuth();
  const search = useSearchParams();
  const clientId = search.get("client_id");
  const [rows, setRows] = useState<Network[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (clientId === null) {
      // Clearing the scope clears its failure too, or the previous client's
      // error stands in front of the "choose a client" prompt.
      setError(null);
      setRows([]);
      return;
    }
    let cancelled = false;
    setRows(null);
    setError(null);
    knowledgeApi
      .get<Network[]>(`/api/v1/knowledge/networks?client_id=${encodeURIComponent(clientId)}`)
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
          title="Networks"
          description="Subnets, VLANs and the addresses already spoken for."
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
          <ErrorNote title="Networks did not load.">{error}</ErrorNote>
        ) : rows === null ? (
          <SkeletonRows rows={6} sentence="Loading networks" columns={4} />
        ) : clientId === null ? (
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Choose a client to see its networks.
          </p>
        ) : rows.length === 0 ? (
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            No networks for this client yet. Document the subnets so the next address conflict is
            visible rather than guessed at.
          </p>
        ) : (
          <TableCard label="Networks">
            <table className="w-full border-collapse text-[13px]">
              <thead>
                <tr style={{ background: "var(--m-surface-2)" }}>
                  {["Network", "Kind", "Gateway", "DNS", "VLAN zone"].map((head) => (
                    <th key={head} className={HEAD_CELL_CLASS} style={{ color: "var(--m-text-muted)" }}>
                      {head}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {rows.map((network) => (
                  <tr key={network.id} style={{ borderBottom: "1px solid var(--m-border)" }}>
                    <td className="px-3 py-2 font-semibold">
                      <Link href={`/kb/networks/${network.id}`} className="underline underline-offset-2">
                        {network.name}
                      </Link>
                    </td>
                    <td className="px-3 py-2">{KIND_LABEL[network.kind] ?? network.kind}</td>
                    <td className="px-3 py-2 font-mono text-[12px]">{network.gateway ?? "—"}</td>
                    <td className="px-3 py-2 font-mono text-[12px]">{network.dns ?? "—"}</td>
                    <td className="px-3 py-2">{network.vlan_zone ?? "—"}</td>
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
export default function NetworksRoute() {
  return (
    <Suspense fallback={null}>
      <NetworksPage />
    </Suspense>
  );
}
