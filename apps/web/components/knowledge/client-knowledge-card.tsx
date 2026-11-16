"use client";
/**
 * ClientKnowledgeCard — the client record's door into documentation.
 *
 * Purpose: the integration owner mounts one component on the client record
 * (`apps/web/app/clients/[id]/page.tsx`, which this lane does not edit). It
 * answers the only question the client record has about documentation — "how
 * complete is this client, and what is missing?" — and links to the hub that
 * does the reading.
 *
 * Mount point: inside the client record's summary column, above or below the
 * existing cards:
 *
 *   import { ClientKnowledgeCard } from "../../../components/knowledge/client-knowledge-card";
 *   <ClientKnowledgeCard clientId={clientId} />
 *
 * Non-purpose: it is a summary, not a second editor. It fetches one endpoint,
 * renders counts and gaps, and navigates away for everything else.
 */

import Link from "next/link";
import { useEffect, useState } from "react";
import { Skeleton } from "@marula-core/ui";
import { useRequireAuth } from "../../lib/api";
import { knowledgeApi, type Hub, type KnowledgeRequestError } from "./knowledge-api";
import { CompletenessBlock } from "./completeness";

export function ClientKnowledgeCard({
  clientId,
  className,
}: {
  clientId: string;
  className?: string;
}): React.JSX.Element {
  const auth = useRequireAuth();
  const [hub, setHub] = useState<Hub | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // Wait for the token: a read that fires first 401s and would cache the
    // refusal for the life of the card.
    if (auth.status !== "authed") return;
    let cancelled = false;
    setHub(null);
    setError(null);
    knowledgeApi
      .get<Hub>(`/api/v1/knowledge/clients/${encodeURIComponent(clientId)}/hub`)
      .then((data) => {
        if (!cancelled) setHub(data);
      })
      .catch((e: KnowledgeRequestError) => {
        if (!cancelled) setError(e.message);
      });
    return () => {
      cancelled = true;
    };
  }, [auth.status, clientId]);

  return (
    <section
      className={`overflow-hidden rounded-2xl border ${className ?? ""}`}
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      aria-label="Documentation"
      data-testid="client-knowledge-card"
    >
      <div
        className="flex items-center gap-2.5 border-b px-[18px] py-3.5"
        style={{ borderColor: "var(--m-border)" }}
      >
        <h2 className="text-[15px] font-semibold">Documentation</h2>
        <span className="flex-1" />
        <Link
          href={`/kb/clients/${encodeURIComponent(clientId)}`}
          className="text-[13px] font-semibold underline underline-offset-2"
          style={{ color: "var(--m-text)" }}
        >
          Open hub
        </Link>
      </div>
      <div className="flex flex-col gap-3 px-[18px] py-3.5">
        {error !== null ? (
          <p className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {error}
          </p>
        ) : hub === null ? (
          <div className="flex flex-col gap-2" role="status" aria-label="Loading documentation">
            <Skeleton shape="line" width="70%" />
            <Skeleton shape="line" width="90%" />
            <Skeleton shape="line" width="55%" />
          </div>
        ) : (
          <>
            <CompletenessBlock value={hub.header.completeness} compact />
            <dl className="grid grid-cols-2 gap-2 text-[13px]">
              {[
                ["Configurations", hub.sections.assets_n],
                ["Documents", hub.sections.documents_n],
                ["Credentials", hub.sections.secrets_n],
                ["Procedures", hub.sections.procedures_n],
              ].map(([label, value]) => (
                <div key={String(label)} className="flex items-baseline justify-between gap-2">
                  <dt style={{ color: "var(--m-text-muted)" }}>{label}</dt>
                  <dd className="font-semibold tabular-nums">{value}</dd>
                </div>
              ))}
            </dl>
            {hub.expirations.length > 0 || hub.secrets_glance.stale_n > 0 ? (
              <p className="text-[13px]" style={{ color: "var(--m-status-due-fg)" }}>
                {hub.expirations.length > 0
                  ? `${hub.expirations.length} expiring in the next 90 days`
                  : ""}
                {hub.expirations.length > 0 && hub.secrets_glance.stale_n > 0 ? " · " : ""}
                {hub.secrets_glance.stale_n > 0
                  ? `${hub.secrets_glance.stale_n} credentials past rotation`
                  : ""}
              </p>
            ) : (
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                Nothing is expiring in the next 90 days.
              </p>
            )}
          </>
        )}
      </div>
    </section>
  );
}
