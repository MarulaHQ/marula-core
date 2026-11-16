"use client";
// /kb — the client-first knowledge home.
//
// Purpose: the pack's K01. The question this page answers first is "which
// client", not "which article": one big search across every documentation
// store, your clients as cards with their completeness and what needs
// attention, org knowledge (shared articles, templates, the org vault) beside
// them, and the expiries that are closest. The article ledger still exists, one
// click away at /kb/articles, because writing an article is a different job
// from finding a client.
//
// Non-purpose: no editor. Creating a record happens on the record's own page,
// and the hub is a reading surface (§ brief, Phase 1).

import Link from "next/link";
import { useEffect, useMemo, useRef, useState } from "react";
import { Spinner, SurfaceHeader } from "@marula-core/ui";
import { useRequireAuth } from "../../lib/api";
import { LocalTime } from "../../components/local-time";
import {
  AlertIcon,
  BookIcon,
  ErrorNote,
  FileIcon,
  LockIcon,
  PageFrame,
  SkeletonRows,
} from "../../components/knowledge/ledger";
import {
  knowledgeApi,
  type ExpiryRow,
  type KnowledgeHome,
  type KnowledgeRequestError,
  type SearchHit,
} from "../../components/knowledge/knowledge-api";
import { dueWords } from "../../components/knowledge/expiry-table";

const SCOPES = ["All", "Clients", "Documents", "Configurations", "Secrets"] as const;
type Scope = (typeof SCOPES)[number];

const SCOPE_KINDS: Record<Scope, ReadonlySet<string> | null> = {
  All: null,
  Clients: new Set(["client"]),
  Documents: new Set(["article", "document"]),
  Configurations: new Set(["asset", "rack", "network", "domain", "procedure"]),
  Secrets: new Set(["secret"]),
};

const KIND_LABEL: Record<string, string> = {
  client: "Client",
  article: "Article",
  document: "Document",
  asset: "Configuration",
  rack: "Rack",
  network: "Network",
  domain: "Domain",
  procedure: "Procedure",
  secret: "Credential",
};

export default function KnowledgeHomePage() {
  useRequireAuth();
  const [home, setHome] = useState<KnowledgeHome | null>(null);
  const [expiries, setExpiries] = useState<ExpiryRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [scope, setScope] = useState<Scope>("All");
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    let cancelled = false;
    setError(null);
    Promise.all([
      knowledgeApi.get<KnowledgeHome>("/api/v1/knowledge/home"),
      knowledgeApi.get<ExpiryRow[]>("/api/v1/knowledge/expirations?days=60"),
    ])
      .then(([loadedHome, loadedExpiries]) => {
        if (cancelled) return;
        setHome(loadedHome);
        setExpiries(loadedExpiries);
      })
      .catch((e: KnowledgeRequestError) => {
        if (!cancelled) setError(e.message);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // "/" focuses the search, as the pack prints on the field.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      const typing =
        target !== null &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.isContentEditable);
      if (event.key === "/" && !typing) {
        event.preventDefault();
        searchRef.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    const needle = query.trim();
    if (needle.length < 2) {
      setHits(null);
      setSearching(false);
      setSearchError(null);
      return;
    }
    let cancelled = false;
    setSearching(true);
    // A new search clears the previous failure; the answer below is either a
    // result, a true empty result, or this error — never an empty result
    // standing in for a failure.
    setSearchError(null);
    const timer = window.setTimeout(() => {
      knowledgeApi
        .get<SearchHit[]>(`/api/v1/knowledge/search?q=${encodeURIComponent(needle)}&limit=24`)
        .then((rows) => {
          if (!cancelled) {
            setHits(rows);
            setSearching(false);
          }
        })
        .catch((e: KnowledgeRequestError) => {
          if (!cancelled) {
            setHits([]);
            setSearching(false);
            setSearchError(e.message);
          }
        });
    }, 200);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [query]);

  const filtered = useMemo(() => {
    const allowed = SCOPE_KINDS[scope];
    return (hits ?? []).filter((hit) => allowed === null || allowed.has(hit.kind));
  }, [hits, scope]);

  const attention = useMemo(
    () =>
      [...expiries]
        .sort((a, b) => a.due_on.localeCompare(b.due_on))
        .filter((row) => dueWords(row.due_on).tone !== "neutral")
        .slice(0, 5),
    [expiries],
  );

  return (
    <PageFrame>
      <div className="flex flex-col gap-5 overflow-y-auto pb-6 pt-5">
        <SurfaceHeader
          title="Knowledge"
          description="Everything you document about every client, and the secrets that go with it."
          actions={
            <>
              <div className="flex items-center gap-2">
              <Link
              href="/kb/expirations"
              className="inline-flex h-11 items-center gap-2 rounded-[12px] border px-4 text-[13px] font-semibold"
              style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
              >
              <AlertIcon size={16} />
              Expirations
              {home !== null && home.expiring_soon_n > 0 ? (
              <span className="tabular-nums">{home.expiring_soon_n}</span>
              ) : null}
              </Link>
              <Link
              href="/kb/articles"
              className="inline-flex h-11 items-center gap-2 rounded-[12px] px-4 text-[13px] font-semibold"
              style={{ background: "var(--m-ink)", color: "var(--m-gold)" }}
              >
              <BookIcon size={16} />
              All articles
              </Link>
              </div>
            </>
          }
        />

        {error !== null ? <ErrorNote title="Knowledge did not load.">{error}</ErrorNote> : null}

        <div className="flex flex-col gap-3">
          <div
            className="flex h-14 items-center gap-3 rounded-[16px] border px-[18px]"
            style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
          >
            <svg
              aria-hidden="true"
              width={20}
              height={20}
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.75"
              strokeLinecap="round"
            >
              <circle cx="11" cy="11" r="7" />
              <path d="M20 20l-3.5-3.5" />
            </svg>
            <input
              ref={searchRef}
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search clients, documents, devices and secrets"
              aria-label="Search knowledge"
              className="min-w-0 flex-1 border-none bg-transparent text-[16px] outline-none"
              style={{ color: "var(--m-text)" }}
            />
            {searching ? <Spinner label="Searching" /> : null}
            <kbd
              className="rounded-[5px] border px-1.5 text-[12px] font-semibold"
              style={{ borderColor: "var(--m-border)", color: "var(--m-text-secondary)" }}
            >
              /
            </kbd>
          </div>

          <div
            className="flex w-fit items-center gap-1 rounded-[12px] p-[3px]"
            style={{ background: "var(--m-surface-2)" }}
            role="group"
            aria-label="Search scope"
          >
            {SCOPES.map((name) => (
              <button
                key={name}
                type="button"
                aria-pressed={scope === name}
                onClick={() => setScope(name)}
                className="h-[38px] rounded-[9px] px-3.5 text-[13px] font-semibold transition-colors duration-100"
                style={{
                  background: scope === name ? "var(--m-surface)" : "transparent",
                  color: scope === name ? "var(--m-text)" : "var(--m-text-secondary)",
                }}
              >
                {name}
              </button>
            ))}
          </div>
        </div>

        {query.trim().length >= 2 ? (
          <section aria-label="Search results" className="flex flex-col gap-2">
            {searchError !== null ? (
              <ErrorNote title="Search did not run.">{searchError}</ErrorNote>
            ) : filtered.length === 0 && !searching ? (
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                Nothing matches “{query.trim()}” in this scope. Secrets are found by their record
                only, never by their value.
              </p>
            ) : (
              <ul
                className="overflow-hidden rounded-[16px] border"
                style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
              >
                {filtered.map((hit) => (
                  <li key={`${hit.kind}:${hit.id}`}>
                    <Link
                      href={hit.href}
                      className="flex items-center gap-3 px-4 py-3 text-[13px]"
                      style={{ color: "var(--m-text)" }}
                    >
                      <span
                        className="w-[92px] shrink-0 text-[12px] font-semibold uppercase tracking-[0.06em]"
                        style={{ color: "var(--m-text-muted)" }}
                      >
                        {KIND_LABEL[hit.kind] ?? hit.kind}
                      </span>
                      <span className="min-w-0 flex-1 truncate font-semibold">{hit.title}</span>
                      <span className="hidden min-w-0 flex-1 truncate sm:block" style={{ color: "var(--m-text-secondary)" }}>
                        {hit.subtitle}
                        {hit.client_name !== null ? ` · ${hit.client_name}` : ""}
                      </span>
                    </Link>
                  </li>
                ))}
              </ul>
            )}
          </section>
        ) : null}

        {home === null || error !== null ? (
          home === null && error === null ? (
            <SkeletonRows rows={6} sentence="Loading your clients" columns={3} />
          ) : null
        ) : (
          <>
            <section className="flex flex-col gap-2">
              <div className="flex items-center justify-between">
                <h2
                  className="m-0 text-[12px] font-semibold uppercase tracking-[0.06em]"
                  style={{ color: "var(--m-text-muted)" }}
                >
                  Your clients
                </h2>
                <span className="tabular-nums text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                  {home.clients.length}
                </span>
              </div>
              <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
                {home.clients.map((client) => (
                  <Link
                    key={client.id}
                    href={`/kb/clients/${client.id}`}
                    className="rounded-[14px] border p-3.5 transition-colors duration-100 hover:bg-[var(--m-surface-2)]"
                    style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                    data-testid="knowledge-client-card"
                  >
                    <div className="flex items-start gap-2.5">
                      <span
                        aria-hidden="true"
                        className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[10px] text-[13px] font-bold"
                        style={{ background: "var(--m-surface-2)", color: "var(--m-text-secondary)" }}
                      >
                        {client.name
                          .split(/\s+/)
                          .slice(0, 2)
                          .map((word) => word[0]?.toUpperCase() ?? "")
                          .join("")}
                      </span>
                      <div className="min-w-0 flex-1">
                        <p className="truncate text-[14px] font-semibold">{client.name}</p>
                        <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                          {client.assets_n} {client.assets_n === 1 ? "configuration" : "configurations"} · {client.sites_n} {client.sites_n === 1 ? "site" : "sites"}
                        </p>
                      </div>
                      {client.expiring_n > 0 ? (
                        <span
                          className="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[12px] font-semibold"
                          style={{
                            background: "var(--m-status-due-bg)",
                            color: "var(--m-status-due-fg)",
                          }}
                        >
                          <AlertIcon size={12} />
                          {client.expiring_n} to review
                        </span>
                      ) : null}
                    </div>
                    <div
                      className="mt-2.5 flex flex-wrap items-center gap-x-2 text-[12px]"
                      style={{ color: "var(--m-text-secondary)" }}
                    >
                      <span className="tabular-nums font-semibold">
                        {client.completeness.complete} of {client.completeness.total} sections
                      </span>
                      <span aria-hidden="true">·</span>
                      <span>
                        {client.last_activity !== null ? (
                          <LocalTime value={client.last_activity} kind="date" />
                        ) : (
                          "No documentation yet"
                        )}
                      </span>
                    </div>
                  </Link>
                ))}
              </div>
            </section>

            <section className="grid gap-4 lg:grid-cols-2">
              <div className="flex flex-col gap-2">
                <h2
                  className="m-0 text-[12px] font-semibold uppercase tracking-[0.06em]"
                  style={{ color: "var(--m-text-muted)" }}
                >
                  Org knowledge
                </h2>
                <div className="grid gap-3 sm:grid-cols-2">
                  {[
                    {
                      icon: <BookIcon size={16} />,
                      title: "Knowledge base",
                      detail: `${home.org.published_articles_n} published · ${home.org.draft_articles_n} ${home.org.draft_articles_n === 1 ? "draft" : "drafts"}`,
                      href: "/kb/articles",
                    },
                    {
                      icon: <FileIcon size={16} />,
                      title: "Templates",
                      detail:
                        home.org.templates_n > 0
                          ? `${home.org.templates_n} defined — no manager yet`
                          : "None yet",
                      href: null,
                    },
                    {
                      icon: <LockIcon size={16} />,
                      title: "Org vault",
                      detail: `${home.org.org_secrets_n} org-level secrets`,
                      href: "/kb/secrets",
                    },
                    {
                      icon: <FileIcon size={16} />,
                      title: "Org documents",
                      detail: `${home.org.org_documents_n} not tied to a client`,
                      href: "/documents",
                    },
                  ].map((card) => (
                    <OrgCard key={card.title} href={card.href}>
                      <span
                        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[9px]"
                        style={{ background: "var(--m-surface-2)" }}
                      >
                        {card.icon}
                      </span>
                      <span className="min-w-0">
                        <span className="block text-[13px] font-semibold">{card.title}</span>
                        <span className="block text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                          {card.detail}
                        </span>
                      </span>
                    </OrgCard>
                  ))}
                </div>
              </div>

              <div className="flex flex-col gap-2">
                <div className="flex items-center justify-between">
                  <h2
                    className="m-0 text-[12px] font-semibold uppercase tracking-[0.06em]"
                    style={{ color: "var(--m-text-muted)" }}
                  >
                    Needs attention
                  </h2>
                  <Link
                    href="/kb/expirations"
                    className="text-[12px] font-semibold underline underline-offset-2"
                  >
                    All expirations
                  </Link>
                </div>
                <div
                  className="overflow-hidden rounded-[16px] border"
                  style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                >
                  {attention.length === 0 ? (
                    <p className="px-4 py-5 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                      Nothing is due in the next 30 days. Add a domain or a certificate and its
                      expiry will appear here.
                    </p>
                  ) : (
                    attention.map((row, index) => {
                      const words = dueWords(row.due_on);
                      return (
                        <div
                          key={`${row.kind}:${row.source_id ?? row.title}`}
                          className="flex items-center gap-3 px-4 py-2.5 text-[13px]"
                          style={{
                            borderTop: index === 0 ? undefined : "1px solid var(--m-border)",
                          }}
                        >
                          <span
                            className="shrink-0"
                            style={{
                              color:
                                words.tone === "danger"
                                  ? "var(--m-status-danger-fg)"
                                  : "var(--m-status-due-fg)",
                            }}
                          >
                            <AlertIcon size={16} />
                          </span>
                          <span className="min-w-0 flex-1">
                            <span className="font-semibold">{row.title}</span>
                            <span className="ml-2 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                              {row.client_name}
                            </span>
                          </span>
                          <span
                            className="shrink-0 text-[12px] font-semibold"
                            style={{
                              color:
                                words.tone === "danger"
                                  ? "var(--m-status-danger-fg)"
                                  : "var(--m-status-due-fg)",
                            }}
                          >
                            {words.text}
                          </span>
                        </div>
                      );
                    })
                  )}
                </div>
              </div>
            </section>
          </>
        )}
      </div>
    </PageFrame>
  );
}

function OrgCard({
  href,
  children,
}: {
  href: string | null;
  children: React.ReactNode;
}): React.JSX.Element {
  const className =
    "flex items-center gap-2.5 rounded-[14px] border p-3.5";
  const style = { borderColor: "var(--m-border)", background: "var(--m-surface)" };
  if (href === null) {
    return (
      <div className={className} style={style}>
        {children}
      </div>
    );
  }
  return (
    <Link href={href} className={className} style={style}>
      {children}
    </Link>
  );
}
