// /portal/kb/articles/[id] — one client-facing article (Portal v2).
// The body is plain text rendered with preserved paragraphs; drafts and
// gate-failing articles 404 on the server before this page ever renders.
"use client";

import { Button, Spinner } from "@meridian/ui";
import { useParams, useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { portalApi, usePortalAuth, useRequirePortalAuth, type PortalKbArticle } from "../../../../../lib/portal-api";

function updated(iso: string): string {
  return new Date(iso).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export default function PortalKbArticlePage() {
  useRequirePortalAuth();
  const params = useParams<{ id: string }>();
  const router = useRouter();
  const [article, setArticle] = useState<PortalKbArticle | null>(null);
  const [missing, setMissing] = useState(false);

  useEffect(() => {
    let cancelled = false;
    portalApi
      .kbArticle(params.id)
      .then((a) => {
        if (!cancelled) setArticle(a);
      })
      .catch(() => {
        if (!cancelled) setMissing(true);
      });
    return () => {
      cancelled = true;
    };
  }, [params.id]);

  if (missing) {
    return (
      <section>
        <p className="text-sm" style={{ color: "var(--m-text-muted)" }}>
          This guide is no longer available.
        </p>
        <Button variant="ghost" size="sm" className="mt-3" onClick={() => router.push("/portal/kb")}>
          Back to knowledge base
        </Button>
      </section>
    );
  }
  if (article === null) {
    return (
      <div className="flex justify-center py-16">
        <Spinner aria-label="Loading article" />
      </div>
    );
  }

  return (
    <article>
      <button
        type="button"
        onClick={() => router.push("/portal/kb")}
        className="text-xs transition-colors duration-100 hover:underline"
        style={{ color: "var(--m-text-muted)" }}
      >
        Knowledge base
      </button>
      <h1 className="mt-1 text-base font-semibold">{article.title}</h1>
      <p className="mt-0.5 text-xs" style={{ color: "var(--m-text-muted)" }}>
        Updated {updated(article.updated_at)}
      </p>
      <div
        className="mt-4 whitespace-pre-wrap text-sm leading-6"
        style={{ color: "var(--m-text-secondary)" }}
      >
        {article.body}
      </div>
    </article>
  );
}
