// /portal/kb — knowledge-base self-service (Portal v2): search plus the
// client-facing category tree. Only articles passing the effective
// client-facing gate (published AND article flag AND category flag) can
// ever appear here — the server enforces it, the UI just reflects it.
//
// Wave 17 lane 8 UX pass: search-first landing (help-center pattern),
// topic chips with descriptions as accessible titles, article rows with
// real previews, staggered entrance. Header nav is the SPA's spine — the
// old "back to requests" footer affordance is gone.
"use client";

import { friendlyMutationText } from "../../../lib/error-copy";
import { Button, EmptyState, Input, Spinner } from "@marula-core/ui";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import {
  portalApi,
  useRequirePortalAuth,
  type PortalKbArticle,
  type PortalKbCategory,
} from "../../../lib/portal-api";

export default function PortalKbPage() {
  useRequirePortalAuth();
  const router = useRouter();
  const [categories, setCategories] = useState<PortalKbCategory[] | null>(null);
  const [articles, setArticles] = useState<PortalKbArticle[] | null>(null);
  const [category, setCategory] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<PortalKbArticle[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    portalApi
      .kbCategories()
      .then((cats) => {
        if (!cancelled) setCategories(cats);
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(friendlyMutationText(err, "Failed to load topics"));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    setArticles(null);
    portalApi
      .kbArticles(category ?? undefined)
      .then((rows) => {
        if (!cancelled) setArticles(rows);
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(friendlyMutationText(err, "Failed to load articles"));
      });
    return () => {
      cancelled = true;
    };
  }, [category]);

  async function search(e: React.FormEvent) {
    e.preventDefault();
    const term = query.trim();
    if (term === "") {
      setHits(null);
      return;
    }
    setSearching(true);
    setError(null);
    try {
      const results = await portalApi.kbSearch(term);
      setHits(results.map((hit) => hit.article));
    } catch (err: unknown) {
      setError(friendlyMutationText(err, "Search failed; try again."));
    } finally {
      setSearching(false);
    }
  }

  const shown = hits ?? articles;

  return (
    <section className="m-rise">
      <div className="mb-4">
        <h1 className="text-xl font-semibold tracking-tight">Knowledge base</h1>
        <p className="mt-0.5 text-sm" style={{ color: "var(--m-text-muted)" }}>
          Guides your provider published for clients. Search across
          everything you can see, or browse by topic.
        </p>
      </div>

      <form onSubmit={search} className="mb-4 flex max-w-xl gap-2" role="search">
        <Input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search guides…"
          className="w-full"
          aria-label="Search the knowledge base"
        />
        <Button type="submit" size="sm" className="shrink-0" disabled={searching}>
          {searching ? "Searching…" : "Search"}
        </Button>
      </form>

      {hits !== null && (
        <div className="mb-4 flex items-center gap-2 text-xs">
          <span className="tabular-nums" style={{ color: "var(--m-text-muted)" }}>
            {hits.length} result{hits.length === 1 ? "" : "s"} for “{query.trim()}”
          </span>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setQuery("");
              setHits(null);
            }}
          >
            Clear
          </Button>
        </div>
      )}

      {error !== null && (
        <p className="mb-3 text-sm" style={{ color: "var(--m-status-danger)" }}>
          {error}
        </p>
      )}

      {categories !== null && categories.length > 0 && hits === null && (
        <div className="mb-4 flex flex-wrap gap-2" role="group" aria-label="Filter by topic">
          <TopicChip
            label="All topics"
            active={category === null}
            onClick={() => setCategory(null)}
          />
          {categories.map((c) => (
            <TopicChip
              key={c.id}
              label={c.name}
              title={c.description !== "" ? c.description : c.name}
              active={category === c.id}
              onClick={() => setCategory(c.id)}
            />
          ))}
        </div>
      )}

      {shown === null ? (
        <div className="flex justify-center py-16">
          <Spinner aria-label="Loading articles" />
        </div>
      ) : shown.length === 0 ? (
        <EmptyState
          message={
            hits !== null
              ? "Nothing matched that search — try different words."
              : "No published guides yet."
          }
        />
      ) : (
        <ul
          className="overflow-hidden rounded-[0.75rem] border"
          style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
        >
          {shown.map((a, i) => (
            <li key={a.id} className="m-rise" style={{ animationDelay: `${Math.min(i * 30, 240)}ms` }}>
              <button
                type="button"
                onClick={() => router.push(`/portal/kb/articles/${a.id}`)}
                className="w-full border-b px-3 py-3 text-left transition-colors duration-100 last:border-b-0 hover:bg-[var(--m-surface-hover)]"
                style={{ borderColor: "var(--m-border)" }}
              >
                <div className="text-sm font-medium">{a.title}</div>
                <div
                  className="mt-0.5 line-clamp-1 text-xs"
                  style={{ color: "var(--m-text-muted)" }}
                >
                  {a.body}
                </div>
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function TopicChip({
  label,
  title,
  active,
  onClick,
}: {
  label: string;
  title?: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      title={title}
      className="rounded-full border px-3 py-1 text-xs transition-colors duration-100"
      style={{
        borderColor: active ? "var(--m-accent)" : "var(--m-border)",
        color: active ? "var(--m-accent)" : "var(--m-text-secondary)",
        background: "var(--m-surface)",
      }}
    >
      {label}
    </button>
  );
}
