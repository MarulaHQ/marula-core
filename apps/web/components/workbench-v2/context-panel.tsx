// Workbench v2 — the left Ticket Context panel (Wave 15 lane 7).
//
// Purpose: the mockup's context rail on real records — customer, contact,
// device (asset), agreement (contract), SLA ring, documentation links
// (KB), related tickets, and the client's recent devices. Every section
// reads live data through the typed SDK; a section with no backing record
// renders a typed "not available yet" line — never fake data. The panel
// COLLAPSES as the view narrows: persistent ≥1280px, a toggleable drawer
// 1024–1279px, and overlay-only below 1024px — never persistently visible
// in narrow layouts (binding constraint).
// Non-purpose: no editing here (client/contact changes stay in the
// classic pods until this surface earns them).

"use client";

import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useEffect, useState, type ReactNode } from "react";

import { keywordsOf } from "./agent-panel";
import { api } from "../../lib/api";

type Client = components["schemas"]["Client"];
type Contact = components["schemas"]["Contact"];
type DeviceAsset = components["schemas"]["DeviceAsset"];
type Contract = components["schemas"]["Contract"];
type TicketSlaStatus = components["schemas"]["TicketSlaStatus"];
type KbArticle = components["schemas"]["KbArticle"];
type Ticket = components["schemas"]["Ticket"];

/**
 * Anti-hang guard for a fetch promise: if it never settles (a hung route
 * mock, a wedged proxy), resolve to the empty fallback after 5s so the
 * panel shows a typed state instead of a forever "Loading…".
 */
function neverHang<T>(p: Promise<T>, empty: T): Promise<T> {
  return Promise.race([p, new Promise<T>((rl) => setTimeout(() => rl(empty), 5000))]);
}

function SectionHead({ label, action }: { label: string; action?: ReactNode }): ReactNode {
  return (
    <div className="mb-2 flex items-center justify-between">
      <span className="text-[12px] font-bold" style={{ color: "var(--m-text)" }}>{label}</span>
      {action}
    </div>
  );
}

function Gap({ children, testId }: { children: ReactNode; testId?: string }): ReactNode {
  return (
    <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }} data-testid={testId}>
      {children}
    </p>
  );
}

function Label({ children }: { children: ReactNode }): ReactNode {
  return (
    <div className="mb-1.5 text-[9.5px] font-bold uppercase tracking-[0.075em]" style={{ color: "var(--m-text-muted)" }}>
      {children}
    </div>
  );
}

/** SLA ring: conic fill proportional to the remaining fraction of the budget. */
function SlaRing({ sla }: { sla: TicketSlaStatus | null }): ReactNode {
  if (sla === null || !sla.applied) return null;
  // The API exposes minutes remaining, not the original budget — the ring
  // renders an honest qualitative fill (healthy / low / breach), not a
  // fabricated percentage.
  const mins = sla.minutes_remaining ?? null;
  const frac = mins === null ? 0 : mins <= 0 ? 1 : mins > 240 ? 0.75 : mins > 60 ? 0.5 : 0.25;
  const fg = sla.breached || (mins !== null && mins <= 0) ? "var(--m-status-danger-fg)" : mins !== null && mins <= 60 && !sla.paused ? "var(--m-status-warning-fg)" : "var(--m-status-success-fg)";
  return (
    <div
      aria-hidden="true"
      className="relative h-9 w-9 shrink-0 rounded-full"
      style={{ background: `conic-gradient(${fg} ${Math.round(frac * 100)}%, var(--m-border) 0)` }}
    >
      <div className="absolute inset-[5px] rounded-full" style={{ background: "var(--m-surface)" }} />
    </div>
  );
}

function slaText(sla: TicketSlaStatus | null): { fg: string; text: string } | null {
  if (sla === null || !sla.applied) return null;
  const mins = sla.minutes_remaining ?? null;
  const fmt = (m: number) => (m >= 60 ? `${Math.floor(m / 60)}h ${m % 60}m` : `${m}m`);
  if (sla.breached || (mins !== null && mins < 0)) {
    return { fg: "var(--m-status-danger-fg)", text: mins !== null ? `SLA over ${fmt(-mins)}` : "SLA breached" };
  }
  if (mins === null) return { fg: "var(--m-status-success-fg)", text: sla.paused ? "SLA paused" : "SLA on track" };
  return {
    fg: mins <= 60 && !sla.paused ? "var(--m-status-warning-fg)" : "var(--m-status-success-fg)",
    text: `${sla.paused ? "Paused · " : ""}${fmt(mins)} remaining`,
  };
}

export interface WbContextPanelProps {
  clientId: string | null;
  contactId: string | null;
  ticketId: string;
  ticketNumber: string;
  /** Ticket title — drives the shared search for Related Tickets. */
  ticketTitle: string;
  open: boolean;
  onClose(): void;
}

export function WbContextPanel({ clientId, contactId, ticketId, ticketNumber, ticketTitle, open, onClose }: WbContextPanelProps): ReactNode {
  const [client, setClient] = useState<Client | null>(null);
  const [clientLoaded, setClientLoaded] = useState(false);
  const [contact, setContact] = useState<Contact | null>(null);
  const [assets, setAssets] = useState<DeviceAsset[] | null>(null);
  const [contract, setContract] = useState<Contract | null>(null);
  const [sla, setSla] = useState<TicketSlaStatus | null>(null);
  const [articles, setArticles] = useState<KbArticle[] | null>(null);
  const [related, setRelated] = useState<Ticket[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    setClient(null);
    setClientLoaded(false);
    setContact(null);
    setAssets(null);
    setContract(null);
    if (clientId === null) {
      setClientLoaded(true);
      return;
    }
    void (async () => {
      try {
        const [c, contacts, contracts] = await Promise.all([
          api.api.GET("/api/v1/clients/{id}", { params: { path: { id: clientId } } }),
          api.api.GET("/api/v1/clients/{id}/contacts", { params: { path: { id: clientId } } }),
          api.api.GET("/api/v1/contracts", { params: { query: { client_id: clientId, status: "active" } } }),
        ]);
        if (cancelled) return;
        setClient(c.data ?? null);
        setClientLoaded(true);
        const match = contactId !== null ? (contacts.data ?? []).find((x) => x.id === contactId) : undefined;
        setContact(match ?? (contacts.data ?? []).find((x) => x.is_primary) ?? (contacts.data ?? [])[0] ?? null);
        setContract((contracts.data ?? [])[0] ?? null);
      } catch {
        // Fetch failure resolves each section to its typed gap.
        if (cancelled) return;
        setClient(null);
        setClientLoaded(true);
        setContract(null);
      }
      try {
        const a = await api.api.GET("/api/v1/assets", { params: { query: { client_id: clientId } } });
        if (!cancelled) setAssets(a.data ?? []);
      } catch {
        if (!cancelled) setAssets([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [clientId, contactId]);

  useEffect(() => {
    let cancelled = false;
    void neverHang(
      api.api
        .GET("/api/v1/tickets/{id}/sla", { params: { path: { id: ticketId } } }),
      { data: null } as never,
    ).then(({ data }) => {
        if (!cancelled) setSla(data ?? null);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [ticketId]);

  // Documentation: real published KB articles (latest three). The honest
  // empty state renders when the KB is empty.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const { data } = await api.api.GET("/api/v1/kb/articles", {
          params: { query: { status: "published" } },
        });
        if (!cancelled) setArticles((data ?? []).slice(0, 3));
      } catch {
        // A failed KB read resolves to the honest empty state — never a
        // stuck Loading row.
        if (!cancelled) setArticles([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [ticketId, ticketNumber]);

  // Related tickets: SAME source as the Agent panel — the unified search
  // over this ticket's title words (ticket-kind hits) — so the two panels
  // never disagree. 5s race keeps the section loading-proof.
  useEffect(() => {
    let cancelled = false;
    const q = keywordsOf(ticketTitle);
    if (q === "") {
      setRelated([]);
      return;
    }
    void neverHang(
      api.api
        .GET("/api/v1/search", { params: { query: { q, limit: 10 } } })
        .then(({ data }) => {
          if (!cancelled) {
            setRelated((data?.results ?? []).filter((r) => r.entity_kind === "ticket" && r.entity_id !== ticketId).slice(0, 3) as unknown as Ticket[]);
          }
        }),
      undefined,
    ).catch(() => {
      if (!cancelled) setRelated([]);
    });
    return () => {
      cancelled = true;
    };
  }, [ticketId, ticketTitle]);

  const slaReadout = slaText(sla);

  return (
    <aside
      aria-label="Ticket context"
      data-testid="wb-context"
      data-open={open}
      className="m-wb-context min-h-0 shrink-0 overflow-y-auto border-r pb-16"
    >
      <div className="flex items-center justify-between px-3 pt-3">
        <span className="text-[12px] font-bold" style={{ color: "var(--m-text)" }}>Ticket Context</span>
        <button
          type="button"
          data-testid="wb-context-close"
          aria-label="Hide ticket context panel"
          className="cursor-pointer border-none bg-transparent text-[13px]"
          style={{ color: "var(--m-text-muted)" }}
          onClick={onClose}
        >
          ×
        </button>
      </div>

      <div className="grid gap-3 px-4 pt-3">
        {/* Customer */}
        <section className="m-wb-card">
          <Label>Customer</Label>
          {clientLoaded && client !== null ? (
            <>
              <div className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>{client.name}</div>
              <Link href={`/clients/${client.id}`} className="text-[11px] underline" style={{ color: "var(--m-accent)" }}>
                Open client record ↗
              </Link>
            </>
          ) : clientLoaded ? (
            <Gap testId="wb-context-no-client">No client is linked to this ticket.</Gap>
          ) : (
            <Gap>Loading…</Gap>
          )}
        </section>

        {/* Contact */}
        <section className="m-wb-card">
          <Label>Contact</Label>
          {contact !== null ? (
            <>
              <div className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>{contact.name}</div>
              {contact.title != null && <div className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>{contact.title}</div>}
              {contact.email != null && <div className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>{contact.email}</div>}
              {contact.phone != null && <div className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>{contact.phone}</div>}
            </>
          ) : (
            <Gap>No contact on this ticket{clientLoaded ? "" : " yet"}.</Gap>
          )}
        </section>

        {/* Device */}
        <section className="m-wb-card">
          <Label>Device</Label>
          {assets !== null && assets.length > 0 && assets[0] !== undefined ? (
            <>
              <div className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>{assets[0].name}</div>
              <div className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                {assets[0].kind} · {assets[0].status}
              </div>
              <Link href="/assets" className="text-[11px] underline" style={{ color: "var(--m-accent)" }}>
                View inventory ↗
              </Link>
            </>
          ) : assets !== null ? (
            <Gap>No device linked to this client&apos;s inventory.</Gap>
          ) : (
            <Gap>Loading…</Gap>
          )}
        </section>

        {/* Agreement */}
        <section className="m-wb-card">
          <Label>Agreement</Label>
          {contract !== null ? (
            <>
              <div className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>{contract.contract_type}</div>
              <div className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                {contract.billing_frequency ?? "one-off"} · effective {contract.effective_from}
              </div>
            </>
          ) : (
            <Gap>No active agreement on record for this client.</Gap>
          )}
        </section>

        {/* SLA */}
        <section className="m-wb-card">
          <Label>SLA</Label>
          {slaReadout !== null ? (
            <div className="flex items-center gap-3">
              <SlaRing sla={sla} />
              <div>
                <div className="text-[13px] font-semibold" style={{ color: slaReadout.fg }}>{slaReadout.text}</div>
                {sla?.state?.resolution_due != null && (
                  <div className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                    Due {new Date(sla.state.resolution_due).toLocaleString()}
                  </div>
                )}
              </div>
            </div>
          ) : (
            <Gap testId="wb-context-no-sla">No SLA policy applies to this ticket.</Gap>
          )}
        </section>

        {/* Documentation */}
        <section className="m-wb-card">
          <SectionHead label="Documentation" />
          {articles !== null && articles.length > 0 ? (
            articles.map((a) => (
              <Link key={a.id} href={`/kb/${a.id}`} className="m-wb-linkrow flex items-center gap-1.5 py-1 text-[12px]">
                <svg aria-hidden width="12" height="12" viewBox="0 0 18 18" fill="none" style={{ color: "var(--m-text-muted)" }}>
                  <path d="M3 4.5c2-1 4-1 6 0v9c-2-1-4-1-6 0v-9Z" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
                  <path d="M15 4.5c-2-1-4-1-6 0v9c2-1 4-1 6 0v-9Z" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
                </svg>
                <span className="truncate">{a.title}</span>
              </Link>
            ))
          ) : articles !== null ? (
            <Gap>No knowledge-base articles published yet.</Gap>
          ) : (
            <Gap>Loading…</Gap>
          )}
        </section>

        {/* Related tickets */}
        <section className="m-wb-card">
          <SectionHead label="Related Tickets" />
          {related !== null && related.length > 0 ? (
            related.map((t) => (
              <Link key={t.id} href={`/tickets/${t.id}`} className="m-wb-minirow flex items-center justify-between py-1 text-[12px]">
                <strong className="" style={{ color: "var(--m-text)" }}>{t.number}</strong>
                <span className="min-w-0 truncate pl-2" style={{ color: "var(--m-text-muted)" }}>{t.title}</span>
              </Link>
            ))
          ) : related !== null ? (
            <Gap>No other tickets for this client.</Gap>
          ) : (
            <Gap>Loading…</Gap>
          )}
        </section>

        {/* Recent devices */}
        <section className="m-wb-card">
          <SectionHead label="Recent Devices" />
          {assets !== null && assets.length > 1 ? (
            assets.slice(1).map((d) => (
              <div key={d.id} className="m-wb-minirow flex items-center justify-between py-1 text-[12px]">
                <span style={{ color: "var(--m-text-secondary)" }}>{d.name}</span>
                <span
                  className="inline-block h-1.5 w-1.5 rounded-full"
                  aria-hidden="true"
                  style={{ background: d.status === "deployed" ? "var(--m-status-success-fg)" : "var(--m-text-muted)" }}
                  title={d.status}
                />
              </div>
            ))
          ) : assets !== null ? (
            <Gap>Only the primary device is on inventory.</Gap>
          ) : (
            <Gap>Loading…</Gap>
          )}
        </section>
      </div>
    </aside>
  );
}
