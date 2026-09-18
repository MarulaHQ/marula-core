// /portal/kb/articles/[id] — one client-facing article (Portal v2).
// The body is plain text rendered with preserved paragraphs; drafts and
// gate-failing articles 404 on the server before this page ever renders.
//
// Wave 17 lane 8 UX pass: reading typography (opt-in serif display title,
// comfortable measure, paragraph rhythm), staggered entrance.
"use client";

import { Button, Card, EmptyState, Spinner } from "@meridian/ui";
import { useParams, useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { portalApi, useRequirePortalAuth, type PortalKbArticle } from "../../../../../lib/portal-api";

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
      <section className="m-rise">
        <EmptyState message="This guide is no longer available." />
        <div className="mt-3 text-center">
          <Button variant="secondary" size="sm" onClick={() => router.push("/portal/kb")}>
            Back to knowledge base
          </Button>
        </div>
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
    <article className="m-rise">
      <button
        type="button"
        onClick={() => router.push("/portal/kb")}
        className="text-xs transition-colors duration-100 hover:underline"
        style={{ color: "var(--m-text-muted)" }}
      >
        ← Knowledge base
      </button>
      <Card className="mt-3">
        <div className="border-b px-5 pb-3 pt-5" style={{ borderColor: "var(--m-border)" }}>
          <h1 className="text-xl font-semibold tracking-tight">{article.title}</h1>
          <p className="mt-0.5 text-xs tabular-nums" style={{ color: "var(--m-text-muted)" }}>
            Updated {updated(article.updated_at)}
          </p>
        </div>
        <div
          className="m-kb-prose whitespace-pre-wrap px-5 py-4 text-[15px] leading-7"
          style={{ color: "var(--m-text-secondary)" }}
        >
          {article.body}
        </div>
      </Card>
    </article>
  );
}
