// /portal/kb — the service desk's help pages.
//
// Purpose: the pack's state 10. A search field, and the articles the service
// desk published — the pack labels the card "Published by Brightline IT",
// because these are THEIR words, not Marula's. That attribution is the point:
// a client asking why the password policy changed wants the answer from the
// people who set it.
//
// A search that finds nothing says so in a sentence and points at the service
// desk. It never suggests an AI, because there is no grant anywhere on this
// surface (rule 7 of the DERIVATION-GUIDE contract: no grant means no trace).
//
// Non-purpose: no categories rail of staff taxonomy, no drafts, no edit. A
// client reads; the service desk writes.
"use client";

import { useEffect, useState } from "react";
import { friendlyMutationText } from "../../../lib/error-copy";
import { Spinner } from "@marula-core/ui";
import {
  portalApi,
  useRequirePortalAuth,
  type PortalKbArticle,
} from "../../../lib/portal-api";
import { DESK_NAME } from "../../../components/portal/portal-shell";
import { PortalCard, PortalIcon, PortalRow } from "../../../components/portal/portal-ui";

export default function PortalKbPage() {
  useRequirePortalAuth();
  const [articles, setArticles] = useState<PortalKbArticle[] | null>(null);
  const [query, setQuery] = useState("");
  const [term, setTerm] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const trimmed = term.trim();
    const request =
      trimmed === ""
        ? portalApi.kbArticles()
        : portalApi
            .kbSearch(trimmed)
            .then((hits) => hits.map((h) => h.article))
            .catch(() => {
              if (!cancelled) setError("That search didn't work. Try a shorter word.");
              return [] as PortalKbArticle[];
            });
    request
      .then((list) => {
        if (!cancelled) {
          setArticles(list);
          setError(null);
        }
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setError(friendlyMutationText(err, "We couldn't load the help pages."));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [term]);

  return (
    <>
      <div>
        <h1 className="portal-title">Help</h1>
        <p className="portal-lead">Answers from your service desk. Most take a couple of minutes.</p>
      </div>

      <form
        onSubmit={(e) => {
          e.preventDefault();
          setTerm(query);
        }}
        role="search"
      >
        <label htmlFor="portal-kb-search" className="sr-only">
          Search the help pages
        </label>
        <div style={{ position: "relative" }}>
          <span
            aria-hidden
            style={{
              position: "absolute",
              top: 15,
              left: 14,
              color: "var(--m-text-secondary)",
            }}
          >
            <PortalIcon name="search" size={18} />
          </span>
          <input
            id="portal-kb-search"
            className="portal-input"
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search, for example VPN or password"
            style={{ height: 52, paddingLeft: 42 }}
            data-testid="portal-kb-search"
          />
        </div>
      </form>

      {error !== null && (
        <p className="portal-field__error" role="alert">
          <PortalIcon name="alert" size={14} />
          <span>{error}</span>
        </p>
      )}

      <PortalCard
        title={term.trim() === "" ? "Popular" : "Results"}
        aside={<span className="portal-hint">Published by {DESK_NAME}</span>}
        testId="portal-kb-list"
      >
        {articles === null ? (
          <div style={{ display: "flex", justifyContent: "center", padding: "32px 0" }}>
            <Spinner aria-label="Loading the help pages" />
          </div>
        ) : articles.length === 0 ? (
          <div className="portal-empty" style={{ padding: "32px 24px" }}>
            <p>
              {term.trim() === ""
                ? "No help pages published yet. Contact your service desk and they'll answer directly."
                : "Nothing matches. Try a shorter word, or ask your service desk."}
            </p>
            <a className="portal-btn" href="mailto:support@brightline.example">
              Contact us
            </a>
          </div>
        ) : (
          articles.map((a) => (
            <PortalRow
              key={a.id}
              title={a.title}
              meta={a.body.replace(/[#*`>]/g, "").slice(0, 120)}
              columns="minmax(0, 1fr) 24px"
              to={`/portal/kb/articles/${a.id}`}
              trailing={
                <span style={{ color: "var(--m-text-secondary)" }}>
                  <PortalIcon name="chev" size={18} />
                </span>
              }
              testId={`portal-kb-${a.id}`}
            />
          ))
        )}
      </PortalCard>
    </>
  );
}
