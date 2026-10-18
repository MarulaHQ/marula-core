// /portal/kb/articles/[id] — a help article, in the same column.
//
// Purpose: derived from the help list (DERIVATION-GUIDE.md: "Help article |
// Help | A reading page in the same column"). It is a reading page, so the
// measure is short, the type is the interface's 16px on a 1.65 line, and the
// only navigation is back to the list.
//
// The body arrives as markdown from the service desk. It is rendered with the
// small subset a help page actually uses — headings, paragraphs, lists, bold,
// inline code and links — and nothing else. There is no HTML pass-through, so a
// published article cannot inject markup into the portal.
"use client";

import { useParams, useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { friendlyMutationText } from "../../../../../lib/error-copy";
import { Spinner } from "@marula-core/ui";
import { portalApi, useRequirePortalAuth, type PortalKbArticle } from "../../../../../lib/portal-api";
import { useTimeFormat } from "../../../../../lib/use-time-format";
import { longDate } from "../../../../../lib/portal-copy";
import { DESK_NAME } from "../../../../../components/portal/portal-shell";
import { PortalCard, PortalIcon } from "../../../../../components/portal/portal-ui";

/** The markdown subset a help page uses, rendered without an HTML pass. */
function ArticleBody({ body }: { body: string }) {
  const blocks: React.ReactNode[] = [];
  let list: string[] = [];

  const inline = (text: string): React.ReactNode[] =>
    text.split(/(\*\*[^*]+\*\*|`[^`]+`|\[[^\]]+\]\([^)]+\))/g).filter(Boolean).map((part, i) => {
      if (part.startsWith("**") && part.endsWith("**")) return <strong key={i}>{part.slice(2, -2)}</strong>;
      if (part.startsWith("`") && part.endsWith("`")) return <code key={i}>{part.slice(1, -1)}</code>;
      const link = /^\[([^\]]+)\]\(([^)]+)\)$/.exec(part);
      const href = link?.[2];
      if (link !== null && href !== undefined) {
        // Only http(s) and mailto survive; anything else renders as plain text.
        return /^https?:\/\//i.test(href) || href.startsWith("mailto:") ? (
          <a key={i} href={href} rel="noreferrer noopener">
            {link[1]}
          </a>
        ) : (
          <span key={i}>{link[1]}</span>
        );
      }
      return <span key={i}>{part}</span>;
    });

  const flushList = (key: string) => {
    if (list.length === 0) return;
    blocks.push(
      <ul key={key}>
        {list.map((item, i) => (
          <li key={i}>{inline(item)}</li>
        ))}
      </ul>,
    );
    list = [];
  };

  for (const raw of body.split("\n")) {
    const line = raw.trim();
    const key = `${blocks.length}-${line.slice(0, 12)}`;
    if (line.startsWith("## ")) {
      flushList(`l${key}`);
      blocks.push(<h2 key={key}>{inline(line.slice(3))}</h2>);
    } else if (line.startsWith("# ")) {
      flushList(`l${key}`);
      blocks.push(<h2 key={key}>{inline(line.slice(2))}</h2>);
    } else if (line.startsWith("- ") || line.startsWith("* ")) {
      list.push(line.slice(2));
    } else if (line === "") {
      flushList(`l${key}`);
    } else {
      flushList(`l${key}`);
      blocks.push(<p key={key}>{inline(line)}</p>);
    }
  }
  flushList("l-final");

  return <div className="portal-article">{blocks}</div>;
}

export default function PortalKbArticlePage() {
  useRequirePortalAuth();
  const { timeZone } = useTimeFormat();
  const params = useParams<{ id: string }>();
  const router = useRouter();
  const [article, setArticle] = useState<PortalKbArticle | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setArticle(await portalApi.kbArticle(params.id));
    } catch (err) {
      setError(friendlyMutationText(err, "We couldn't open that help page."));
    }
  }, [params.id]);

  useEffect(() => {
    void load();
  }, [load]);

  if (error !== null) {
    return (
      <PortalCard>
        <div className="portal-card__body">
          <p className="portal-field__error" role="alert">
            <PortalIcon name="alert" size={14} />
            <span>{error}</span>
          </p>
          <p>
            <button type="button" className="portal-link" onClick={() => router.push("/portal/kb")}>
              Back to help
            </button>
          </p>
        </div>
      </PortalCard>
    );
  }

  if (article === null) {
    return (
      <div style={{ display: "flex", justifyContent: "center", padding: "64px 0" }}>
        <Spinner aria-label="Loading this help page" />
      </div>
    );
  }

  return (
    <>
      <div>
        <button type="button" className="portal-back" onClick={() => router.push("/portal/kb")}>
          <PortalIcon name="back" size={14} />
          Help
        </button>
      </div>
      <div>
        <h1 className="portal-title">{article.title}</h1>
        <p className="portal-lead">
          Published by {DESK_NAME} · Updated {longDate(article.updated_at, timeZone)}
        </p>
      </div>
      <PortalCard testId="portal-kb-article">
        <div className="portal-card__body">
          <ArticleBody body={article.body} />
        </div>
      </PortalCard>
    </>
  );
}
