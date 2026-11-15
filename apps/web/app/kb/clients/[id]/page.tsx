"use client";
// /kb/clients/[id] — the client documentation hub.
//
// Purpose: the pack's K02, and the front door the brief asks for. Three columns
// in the client record's shape: who this client is and how complete their
// documentation is on the left, what they have in the middle as sections that
// each link to their own full list, and what needs attention on the right.
//
// This is a READING surface. It creates nothing and edits nothing; a section
// is a summary of three records and a count, never a second ledger.
//
// Non-purpose: no assistant column. There is no AI grant on this surface, and
// no grant means no trace (AGENTS.md). The right column is a plain summary.

import Link from "next/link";
import { useParams } from "next/navigation";
import { useEffect, useState } from "react";
import { useRequireAuth } from "../../../../lib/api";
import { SurfaceHeader } from "@marula-core/ui";
import { LocalTime } from "../../../../components/local-time";
import {
  ErrorNote,
  PageFrame,
  SkeletonRows,
} from "../../../../components/knowledge/ledger";
import { CompletenessBlock } from "../../../../components/knowledge/completeness";
import { dueWords } from "../../../../components/knowledge/expiry-table";
import {
  knowledgeApi,
  type Domain,
  type Hub,
  type KnowledgeRequestError,
  type Network,
  type Procedure,
  type Rack,
} from "../../../../components/knowledge/knowledge-api";
import {
  RecordFrame,
  RecordRow,
  RecordSection,
  SummaryCard,
} from "../../../../components/records/record-frame";

interface SectionData {
  racks: Rack[];
  networks: Network[];
  procedures: Procedure[];
  domains: Domain[];
}

export default function ClientKnowledgeHubPage() {
  useRequireAuth();
  const params = useParams<{ id: string }>();
  const clientId = params.id;
  const [hub, setHub] = useState<Hub | null>(null);
  const [sections, setSections] = useState<SectionData | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (clientId === undefined) return;
    let cancelled = false;
    setError(null);
    const base = `/api/v1/knowledge`;
    Promise.all([
      knowledgeApi.get<Hub>(`${base}/clients/${clientId}/hub`),
      knowledgeApi.get<Rack[]>(`${base}/racks?client_id=${clientId}`),
      knowledgeApi.get<Network[]>(`${base}/networks?client_id=${clientId}`),
      knowledgeApi.get<Procedure[]>(`${base}/procedures?client_id=${clientId}`),
      knowledgeApi.get<Domain[]>(`${base}/domains?client_id=${clientId}`),
    ])
      .then(([loadedHub, racks, networks, procedures, domains]) => {
        if (cancelled) return;
        setHub(loadedHub);
        setSections({ racks, networks, procedures, domains });
      })
      .catch((e: KnowledgeRequestError) => {
        if (!cancelled) setError(e.message);
      });
    return () => {
      cancelled = true;
    };
  }, [clientId]);

  if (error !== null) {
    return (
      <PageFrame>
        <div className="pt-6">
          <ErrorNote title="The client hub did not load.">{error}</ErrorNote>
        </div>
      </PageFrame>
    );
  }

  if (hub === null || sections === null) {
    return (
      <PageFrame>
        <div className="pt-6">
          <SkeletonRows rows={8} sentence="Loading the client's documentation" columns={4} />
        </div>
      </PageFrame>
    );
  }

  const { header, sections: counts, expirations, recent_changes: recent, secrets_glance: glance } =
    hub;

  const attention = expirations.filter((row) => dueWords(row.due_on).tone !== "neutral");

  return (
    <PageFrame>
      <div className="flex flex-col gap-3 pb-4 pt-5">
        <nav aria-label="Breadcrumb" className="flex items-center gap-1.5 text-[13px]">
          <Link href="/kb" className="underline underline-offset-2" style={{ color: "var(--m-text-secondary)" }}>
            Knowledge
          </Link>
          <span aria-hidden="true" style={{ color: "var(--m-text-muted)" }}>
            /
          </span>
          <span className="font-semibold">{header.name}</span>
        </nav>

        <RecordFrame
          context={
            <>
              <RecordSection label="Client">
                <p className="text-[18px] font-semibold leading-tight">{header.name}</p>
                <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                  Documentation hub
                </p>
              </RecordSection>
              <RecordSection label="Completeness">
                <CompletenessBlock value={header.completeness} />
              </RecordSection>
              <RecordSection label="Owner">
                <RecordRow label="Documentation owner">
                  {header.owner_name ?? "Unassigned"}
                </RecordRow>
                <RecordRow label="Last activity">
                  {header.last_activity !== null ? (
                    <LocalTime value={header.last_activity} />
                  ) : (
                    "None yet"
                  )}
                </RecordRow>
              </RecordSection>
              {header.review_note !== null && header.review_note !== "" ? (
                <RecordSection label="Read first">
                  <p className="text-[13px] leading-relaxed">{header.review_note}</p>
                </RecordSection>
              ) : null}
            </>
          }
          content={
            <div className="flex flex-col gap-4 pt-2">
<SurfaceHeader
          title={header.name}
          meta={
            <span
              className="rounded-full px-2.5 py-1 text-[12px] font-semibold tabular-nums"
              style={{ background: "var(--m-accent-soft)", color: "var(--m-on-accent)" }}
            >
              {counts.assets_n} configurations
            </span>
          }
          actions={
            <Link
              href={`/kb/expirations?client_id=${clientId}`}
              className="text-[13px] font-semibold underline underline-offset-2"
            >
              Expirations
            </Link>
          }
        />

              <section aria-label="Documentation sections" className="grid gap-3 sm:grid-cols-2">
                <HubSection
                  title="Articles"
                  count={counts.articles_n}
                  href="/kb/articles"
                  empty="No client-visible articles yet."
                  items={[]}
                />
                <HubSection
                  title="Documents"
                  count={counts.documents_n}
                  href="/documents"
                  empty="No documents. Write the process down once and it stops living in chat."
                  items={[]}
                />
                <HubSection
                  title="Configurations"
                  count={counts.assets_n}
                  href="/assets"
                  empty="No configurations. Add a server or a firewall and its expiry is tracked."
                  items={[]}
                />
                <HubSection
                  title="Credentials"
                  count={counts.secrets_n}
                  href="/kb/secrets"
                  empty="No credentials in the vault."
                  items={[]}
                  note={
                    glance.stale_n > 0
                      ? `${glance.stale_n} past rotation`
                      : undefined
                  }
                />
                <HubSection
                  title="Domains and SSL"
                  count={counts.domains_n}
                  href={`/kb/domains?client_id=${clientId}`}
                  empty="No domains. Add one and its registration and certificate dates report here."
                  items={sections.domains.slice(0, 3).map((domain) => ({
                    id: domain.id,
                    label: domain.domain,
                    href: `/kb/domains?client_id=${clientId}`,
                    meta:
                      domain.ssl_expiry !== null
                        ? `TLS ${domain.ssl_expiry}`
                        : domain.registrar_expiry !== null
                          ? `Registrar ${domain.registrar_expiry}`
                          : "No date",
                  }))}
                />
                <HubSection
                  title="Networks"
                  count={counts.networks_n}
                  href={`/kb/networks?client_id=${clientId}`}
                  empty="No networks. Document the subnets so an address conflict is visible."
                  items={sections.networks.slice(0, 3).map((network) => ({
                    id: network.id,
                    label: network.name,
                    href: `/kb/networks/${network.id}`,
                    meta: network.kind.toUpperCase(),
                  }))}
                />
                <HubSection
                  title="Racks"
                  count={counts.racks_n}
                  href={`/kb/racks?client_id=${clientId}`}
                  empty="No racks. A rack record shows what is actually installed, and where."
                  items={sections.racks.slice(0, 3).map((rack) => ({
                    id: rack.id,
                    label: rack.name,
                    href: `/kb/racks/${rack.id}`,
                    meta: `${rack.units}U`,
                  }))}
                />
                <HubSection
                  title="Procedures"
                  count={counts.procedures_n}
                  href={`/kb/procedures?client_id=${clientId}`}
                  empty="No procedures. Write the runbook once, so the next person can repeat it."
                  items={sections.procedures.slice(0, 3).map((procedure) => ({
                    id: procedure.id,
                    label: procedure.title,
                    href: `/kb/procedures/${procedure.id}`,
                    meta: `v${procedure.version}`,
                  }))}
                />
              </section>

              <section aria-label="Recent documentation changes">
                <h2
                  className="m-0 mb-2 text-[12px] font-semibold uppercase tracking-[0.06em]"
                  style={{ color: "var(--m-text-muted)" }}
                >
                  Recent changes
                </h2>
                <div
                  className="overflow-hidden rounded-[16px] border"
                  style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                >
                  {recent.length === 0 ? (
                    <p className="px-4 py-4 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                      Nothing has changed yet.
                    </p>
                  ) : (
                    recent.map((change, index) => (
                      <div
                        key={`${change.kind}:${change.entity_id}`}
                        className="flex items-center gap-3 px-4 py-2.5 text-[13px]"
                        style={{ borderTop: index === 0 ? undefined : "1px solid var(--m-border)" }}
                      >
                        <span
                          className="w-[86px] shrink-0 text-[12px] font-semibold uppercase tracking-[0.06em]"
                          style={{ color: "var(--m-text-muted)" }}
                        >
                          {change.kind}
                        </span>
                        <span className="min-w-0 flex-1 truncate font-semibold">{change.title}</span>
                        <span className="shrink-0 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                          {change.actor_name ?? "System"} · <LocalTime value={change.changed_at} />
                        </span>
                      </div>
                    ))
                  )}
                </div>
              </section>
            </div>
          }
          summary={
            <>
              <SummaryCard title="Needs attention" trailing={<span className="tabular-nums text-[13px]">{attention.length}</span>}>
                {attention.length === 0 ? (
                  <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                    Nothing is expiring in the next 90 days.
                  </p>
                ) : (
                  attention.slice(0, 5).map((row) => {
                    const words = dueWords(row.due_on);
                    return (
                      <div key={`${row.kind}:${row.source_id ?? row.title}`} className="flex items-start gap-2 py-1 text-[13px]">
                        <span
                          aria-hidden="true"
                          className="mt-1.5 inline-block h-2 w-2 shrink-0 rounded-full"
                          style={{
                            background:
                              words.tone === "danger"
                                ? "var(--m-destructive)"
                                : "var(--m-blush)",
                          }}
                        />
                        <span className="min-w-0 flex-1">
                          <span className="font-semibold">{row.title}</span>
                          <span className="block text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                            {words.text} · {row.provenance}
                          </span>
                        </span>
                      </div>
                    );
                  })
                )}
              </SummaryCard>

              <SummaryCard title="Secrets at a glance">
                <RecordRow label="In the vault">{glance.total_n}</RecordRow>
                <RecordRow label="Past rotation">{glance.stale_n}</RecordRow>
                <RecordRow label="Expiring">{glance.expiring_n}</RecordRow>
                <RecordRow label="Revealed this week">{glance.revealed_week_n}</RecordRow>
                <Link
                  href="/kb/secrets"
                  className="mt-1 text-[13px] font-semibold underline underline-offset-2"
                >
                  Open the vault
                </Link>
              </SummaryCard>

              <SummaryCard title="Where things live">
                <RecordRow label="Sites">{counts.sites_n}</RecordRow>
                <RecordRow label="Contacts">{counts.contacts_n}</RecordRow>
                <RecordRow label="Articles">{counts.articles_n}</RecordRow>
                <RecordRow label="Documents">{counts.documents_n}</RecordRow>
              </SummaryCard>
            </>
          }
        />
      </div>
    </PageFrame>
  );
}

function HubSection({
  title,
  count,
  href,
  empty,
  items,
  note,
}: {
  title: string;
  count: number;
  href: string;
  empty: string;
  items: { id: string; label: string; href: string; meta: string }[];
  note?: string;
}): React.JSX.Element {
  return (
    <div
      className="flex flex-col gap-2 rounded-[16px] border p-3.5"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      data-testid={`hub-section-${title.toLowerCase().replaceAll(/\s+/g, "-")}`}
    >
      <div className="flex items-center gap-2">
        <h2 className="m-0 text-[15px] font-semibold">{title}</h2>
        <span className="tabular-nums text-[13px]" style={{ color: "var(--m-text-muted)" }}>
          {count}
        </span>
        <span className="flex-1" />
        {note !== undefined ? (
          <span className="text-[12px] font-semibold" style={{ color: "var(--m-status-due-fg)" }}>
            {note}
          </span>
        ) : null}
        <Link href={href} className="text-[12px] font-semibold underline underline-offset-2">
          Open
        </Link>
      </div>
      {items.length === 0 ? (
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {count === 0 ? empty : `${count} records, none listed here.`}
        </p>
      ) : (
        <ul className="flex flex-col">
          {items.map((item, index) => (
            <li
              key={item.id}
              style={{ borderTop: index === 0 ? undefined : "1px solid var(--m-border)" }}
            >
              <Link
                href={item.href}
                className="flex items-center gap-2 py-1.5 text-[13px]"
                style={{ color: "var(--m-text)" }}
              >
                <span className="min-w-0 flex-1 truncate">{item.label}</span>
                <span className="shrink-0 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                  {item.meta}
                </span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
