"use client";
// /portal/knowledge — the documents and articles this client may read.
//
// Purpose: the client-facing half of Phase 3. It lists exactly what the staff
// side marked client-visible — documents (`client_visible`) and published
// articles — and nothing else. Articles open in the existing portal reader at
// /portal/kb/articles/[id]; documents are listed with their review date.
//
// Non-purpose: no vault, no drafts, no internal notes. A client portal that
// leaks one internal document is worse than one with fewer documents.

import Link from "next/link";
import { useEffect, useState } from "react";
import { Spinner } from "@marula-core/ui";
import { portalApi, useRequirePortalAuth, type PortalKnowledgeDoc } from "../../../lib/portal-api";
import { PortalCard, PortalRow } from "../../../components/portal/portal-ui";

export default function PortalKnowledgePage() {
  useRequirePortalAuth();
  const [docs, setDocs] = useState<PortalKnowledgeDoc[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setDocs(null);
    setError(null);
    // Through `portalApi`, not a bare fetch: the portal session is read from
    // the Authorization header only, and the client owns the rotation.
    portalApi
      .knowledgeDocuments()
      .then((data) => {
        if (cancelled) return;
        // A 200 with a body that is not a list would throw during render.
        if (!Array.isArray(data)) {
          setError("We could not read your documents just now. Try again in a moment.");
          return;
        }
        setDocs(data);
      })
      .catch(() => {
        if (!cancelled) {
          setError("We could not load your documents just now. Try again in a moment.");
        }
      });
    return () => {
      cancelled = true;
    };
  }, [attempt]);

  return (
    <div className="flex flex-col gap-4">
      <header>
        <h1 className="m-0 text-[26px] font-semibold">Documentation</h1>
        <p className="mt-1 text-[14px]" style={{ color: "var(--m-text-secondary)" }}>
          The documents and guides we have published for you.
        </p>
      </header>

      {error !== null ? (
        <div className="flex flex-col items-start gap-2">
          <p role="alert" className="m-0 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {error}
          </p>
          <button
            type="button"
            className="portal-btn"
            onClick={() => setAttempt((count) => count + 1)}
          >
            Try again
          </button>
        </div>
      ) : docs === null ? (
        <div className="flex items-center gap-2 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          <Spinner label="Loading documents" />
          Loading your documents…
        </div>
      ) : docs.length === 0 ? (
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          No documents have been published to your portal yet. Your service desk publishes them as
          they are written.
        </p>
      ) : (
        <PortalCard title="Published for you">
          {docs.map((doc) => (
            <PortalRow
              key={`${doc.kind}:${doc.id}`}
              title={doc.title}
              meta={doc.summary}
              trailing={
                doc.kind === "article" ? (
                  <Link
                    href={`/portal/kb/articles/${doc.id}`}
                    className="text-[13px] font-semibold underline underline-offset-2"
                  >
                    Read
                  </Link>
                ) : (
                  <span className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                    Updated {doc.updated_at.slice(0, 10)}
                  </span>
                )
              }
            />
          ))}
          <p className="portal-hint">
            Documents are listed for reference — ask your service desk for a copy.
          </p>
        </PortalCard>
      )}

      <p className="text-[13px]">
        <Link href="/portal/kb" className="underline underline-offset-2">
          Browse the help centre
        </Link>
      </p>
    </div>
  );
}
