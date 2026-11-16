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
import { useRequirePortalAuth } from "../../../lib/portal-api";
import { PortalCard, PortalRow } from "../../../components/portal/portal-ui";

interface PortalDoc {
  id: string;
  kind: "document" | "article";
  title: string;
  summary: string;
  href: string;
  updated_at: string;
}

export default function PortalKnowledgePage() {
  useRequirePortalAuth();
  const [docs, setDocs] = useState<PortalDoc[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    fetch(`${process.env.NEXT_PUBLIC_API_URL?.replace(/\/+$/, "") ?? ""}/api/v1/portal/knowledge/documents`, {
      credentials: "include",
    })
      .then(async (response) => {
        if (!response.ok) throw new Error(`request failed (${response.status})`);
        return (await response.json()) as PortalDoc[];
      })
      .then((data) => {
        if (!cancelled) setDocs(data);
      })
      .catch(() => {
        if (!cancelled) {
          setError("We could not load your documents just now. Try again in a moment.");
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div className="flex flex-col gap-4">
      <header>
        <h1 className="m-0 text-[26px] font-semibold">Documentation</h1>
        <p className="mt-1 text-[14px]" style={{ color: "var(--m-text-secondary)" }}>
          The documents and guides we have published for you.
        </p>
      </header>

      {error !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </p>
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
