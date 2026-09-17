// /kb — staff knowledge-base authoring (native KB v1 + docs-platform),
// Wave 17 lane 6 reader-experience pass.
//
// Purpose: the internal authoring + reading surface. Category rail with
// per-category counts, article list with status filter + client-side
// search, article reader with server-rendered sanitized content, an
// outline (TOC) built from the rendered headings, article metadata and
// the draft/published/archived posture, plus the client-facing
// visibility flags (article AND category both must pass the portal
// gate). The Tiptap editing wiring is the shipped lane-2 envelope
// (ADR 0007) — this file never re-wires it.
//
// Non-purpose: no new editor architecture; no portal logic (the portal's
// own gated search owns client visibility enforcement).
"use client";

import { rejectionText } from "../../lib/error-copy";
import { KbArticleEditor } from "../../lib/editor/kb-article-editor";
import {
  Button,
  Card,
  EmptyState,
  Input,
  KbProse,
  Dialog,
  DialogActions,
  DialogContent,
  FieldHint,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  Tag,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import Link from "next/link";
import { api, useRequireAuth } from "../../lib/api";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  rememberVersion,
  runEnvelopeMutation,
} from "../../lib/envelope-write";

type KbCategory = components["schemas"]["KbCategory"];
type KbArticle = components["schemas"]["KbArticle"];
type KbArticleStatus = components["schemas"]["KbArticleStatus"];
type ArticleDetail = components["schemas"]["ArticleDetail"];

const STATUSES: KbArticleStatus[] = ["draft", "published", "archived"];

const STATUS_TONE: Record<KbArticleStatus, "success" | "warning" | "neutral"> = {
  published: "success",
  draft: "warning",
  archived: "neutral",
};

const STATUS_FILTERS = ["all", "draft", "published", "archived"] as const;
type StatusFilter = (typeof STATUS_FILTERS)[number];

/** Extracts heading entries from the sanitized preview HTML for the TOC. */
function headingsOf(html: string | null | undefined): { level: number; text: string }[] {
  if (html === null || html === undefined || html === "") return [];
  const doc = new DOMParser().parseFromString(html, "text/html");
  const out: { level: number; text: string }[] = [];
  for (const el of doc.querySelectorAll("h2, h3")) {
    const text = el.textContent?.trim() ?? "";
    if (text !== "") out.push({ level: el.tagName === "H2" ? 2 : 3, text });
  }
  return out;
}

export default function KbPage() {
  const auth = useRequireAuth();
  const [categories, setCategories] = useState<KbCategory[] | null>(null);
  const [articles, setArticles] = useState<KbArticle[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [query, setQuery] = useState("");

  const [editing, setEditing] = useState<KbArticle | null>(null);
  const [editorOpen, setEditorOpen] = useState(false);
  const [viewOpen, setViewOpen] = useState(false);
  const [viewing, setViewing] = useState<KbArticle | null>(null);
  const [viewContent, setViewContent] = useState<ArticleDetail | null>(null);
  const [viewLoading, setViewLoading] = useState(false);
  const [categoryOpen, setCategoryOpen] = useState(false);
  const [editingCategory, setEditingCategory] = useState<KbCategory | null>(null);

  const refresh = useCallback(async () => {
    const [c, a] = await Promise.all([
      api.api.GET("/api/v1/kb/categories"),
      api.api.GET("/api/v1/kb/articles"),
    ]);
    if (c.error !== undefined || a.error !== undefined) {
      setLoadError("Failed to load the knowledge base.");
      return;
    }
    setCategories(c.data ?? []);
    setArticles(a.data ?? []);
    setLoadError(null);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  const categoryById = useMemo(
    () => new Map((categories ?? []).map((c) => [c.id, c])),
    [categories],
  );

  const sortedCategories = useMemo(
    () =>
      (categories ?? [])
        .slice()
        .sort((a, b) => (a.position ?? 0) - (b.position ?? 0)),
    [categories],
  );

  const countByCategory = useMemo(() => {
    const counts = new Map<string, number>();
    for (const a of articles ?? []) {
      if (a.category_id !== null && a.category_id !== undefined) {
        counts.set(a.category_id, (counts.get(a.category_id) ?? 0) + 1);
      }
    }
    return counts;
  }, [articles]);

  const visible = useMemo(() => {
    if (articles === null) return null;
    const q = query.trim().toLowerCase();
    return articles
      .filter((a) => selectedCategory === null || a.category_id === selectedCategory)
      .filter((a) => statusFilter === "all" || a.status === statusFilter)
      .filter(
        (a) =>
          q === "" ||
          a.title.toLowerCase().includes(q) ||
          a.slug.toLowerCase().includes(q) ||
          (a.body ?? "").toLowerCase().includes(q),
      )
      .sort((a, b) => (a.updated_at < b.updated_at ? 1 : -1));
  }, [articles, selectedCategory, statusFilter, query]);

  if (auth.status !== "authed") return null;

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-8 p-4 md:p-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold">Knowledge base</h1>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Internal runbooks and how-tos. Articles flagged client-facing (in a
            client-facing category) appear in the portal.
          </p>
        </div>
        <div className="flex gap-2">
          <Button
            variant="secondary"
            onClick={() => {
              setEditingCategory(null);
              setCategoryOpen(true);
            }}
          >
            New category
          </Button>
          <Button
            onClick={() => {
              setEditing(null);
              setEditorOpen(true);
            }}
          >
            New article
          </Button>
        </div>
      </div>

      {loadError !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {loadError}
        </p>
      ) : null}

      <div className="grid gap-6 lg:grid-cols-[260px_1fr]">
        <Card className="self-start">
          <h2 className="p-4 pb-2 text-sm font-semibold">Categories</h2>
          <ul className="grid gap-1 px-2 pb-4">
            <li>
              <CategoryChip
                name="All articles"
                count={articles?.length ?? 0}
                selected={selectedCategory === null}
                onSelect={() => setSelectedCategory(null)}
              />
            </li>
            {sortedCategories.map((c) => (
              <li key={c.id} className="flex items-center justify-between gap-1">
                <CategoryChip
                  name={c.name}
                  count={countByCategory.get(c.id) ?? 0}
                  selected={selectedCategory === c.id}
                  clientVisible={c.client_visible}
                  onSelect={() => setSelectedCategory(c.id)}
                />
                <button
                  className="shrink-0 pr-1 text-[11px]"
                  style={{ color: "var(--m-text-muted)" }}
                  onClick={() => {
                    setEditingCategory(c);
                    setCategoryOpen(true);
                  }}
                >
                  Edit
                </button>
              </li>
            ))}
          </ul>
          <p className="px-4 pb-4 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            <Link href="/kb/secrets" className="underline decoration-[var(--m-border)] underline-offset-2">
              Secret categories
            </Link>{" "}
            live beside the KB (ADR 0006).
          </p>
        </Card>

        <Card>
          <div className="flex flex-wrap items-center justify-between gap-2 p-4 pb-2">
            <h2 className="text-sm font-semibold">
              {selectedCategory !== null
                ? (categoryById.get(selectedCategory)?.name ?? "Articles")
                : "All articles"}
            </h2>
            <div className="flex flex-wrap items-center gap-2">
              <Input
                type="search"
                aria-label="Search articles"
                placeholder="Search title, slug, body…"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                className="w-56"
              />
              <div className="flex gap-1" role="group" aria-label="Status filter">
                {STATUS_FILTERS.map((s) => (
                  <button
                    key={s}
                    className={`rounded-full border px-2.5 py-0.5 text-[11px] transition-opacity duration-100 hover:opacity-80 ${statusFilter === s ? "font-semibold" : ""}`}
                    style={{
                      borderColor: "var(--m-border)",
                      background:
                        statusFilter === s
                          ? "var(--m-accent)"
                          : "transparent",
                      color: statusFilter === s ? "var(--m-background)" : "var(--m-text-secondary)",
                      opacity: statusFilter === s ? 1 : 0.75,
                    }}
                    onClick={() => setStatusFilter(s)}
                  >
                    {s === "all" ? "All" : s}
                  </button>
                ))}
              </div>
            </div>
          </div>
          {visible === null ? (
            <div className="p-4">
              <Spinner />
            </div>
          ) : visible.length === 0 ? (
            <div className="p-4">
              <EmptyState message="No articles match. Draft articles are staff-only — staff search (and the FTS preview) sees them; publishing makes an article (with the client-facing flags) portal-visible through the portal's own gated search." />
            </div>
          ) : (
            <ul className="grid gap-2 px-4 pb-4">
              {visible.map((a) => {
                const category =
                  a.category_id !== null && a.category_id !== undefined
                    ? categoryById.get(a.category_id)
                    : null;
                return (
                  <li
                    key={a.id}
                    className="grid gap-1 rounded-md border p-3 transition-transform duration-100 hover:-translate-y-px"
                    style={{ borderColor: "var(--m-border)" }}
                  >
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      <p className="min-w-0 truncate text-[13px] font-medium">{a.title}</p>
                      <div className="flex shrink-0 items-center gap-2">
                        <Tag tone={STATUS_TONE[a.status]}>{a.status}</Tag>
                        {articlePortalGated(a, categoryById) ? (
                          <Tag tone="success">client</Tag>
                        ) : a.client_visible ? (
                          <Tag tone="warning">client · ungated</Tag>
                        ) : null}
                      </div>
                    </div>
                    <p className="line-clamp-1 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                      {firstLineOf(a)}
                    </p>
                    <div className="flex flex-wrap items-center justify-between gap-1">
                      <p className="truncate text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                        /{a.slug}
                        {a.category_id !== null && category !== null && category !== undefined
                          ? ` · ${category.name}`
                          : ""}{" "}
                        · updated {new Date(a.updated_at).toLocaleDateString()}
                      </p>
                      <div className="flex gap-1">
                        <Button
                          variant="ghost"
                          onClick={() => {
                            setViewing(a);
                            setViewContent(null);
                            setViewOpen(true);
                          }}
                        >
                          View
                        </Button>
                        <Button
                          variant="ghost"
                          onClick={() => {
                            setEditing(a);
                            setEditorOpen(true);
                          }}
                        >
                          Edit
                        </Button>
                      </div>
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
          <p className="px-4 pb-4 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
            Portal view:{" "}
            <Link href="/portal/kb" className="underline decoration-[var(--m-border)] underline-offset-2">
              /portal/kb
            </Link>
          </p>
        </Card>
      </div>

      <Dialog open={viewOpen} onOpenChange={setViewOpen}>
        <DialogContent
          title={viewing?.title ?? "Article"}
          description="Read-only, server-rendered from the canonical Tiptap JSON."
        >
          <ArticleViewer
            article={viewing}
            detail={viewContent}
            loading={viewLoading}
            category={viewing?.category_id ? categoryById.get(viewing.category_id) : undefined}
            onClose={() => setViewOpen(false)}
            onOpen={() => {
              if (viewing === null) return;
              setViewLoading(true);
              api.api
                .GET("/api/v1/kb/articles/{id}", { params: { path: { id: viewing.id } } })
                .then(({ data }) => {
                  setViewContent(data ?? null);
                  setViewLoading(false);
                });
            }}
          />
        </DialogContent>
      </Dialog>

      <Dialog open={editorOpen} onOpenChange={setEditorOpen}>
        <DialogContent
          title={editing === null ? "New article" : "Edit article"}
          description="Content edits save instantly. Use the FTS preview to check how the article surfaces in search before publishing."
        >
          <ArticleEditor
            article={editing}
            categories={categories ?? []}
            onDone={() => {
              setEditorOpen(false);
              void refresh();
            }}
          />
        </DialogContent>
      </Dialog>

      <Dialog open={categoryOpen} onOpenChange={setCategoryOpen}>
        <DialogContent
          title={editingCategory === null ? "New category" : "Edit category"}
          description="Categories order the tree and gate portal visibility for their articles."
        >
          <CategoryForm
            category={editingCategory}
            onDone={() => {
              setCategoryOpen(false);
              void refresh();
            }}
          />
        </DialogContent>
      </Dialog>
    </div>
  );
}

/** The effective portal gate needs BOTH flags plus a live category. */
function articlePortalGated(
  a: KbArticle,
  categoryById: Map<string, KbCategory>,
): boolean {
  if (!a.client_visible || a.category_id === null || a.category_id === undefined) return false;
  return categoryById.get(a.category_id)?.client_visible === true;
}

function firstLineOf(a: KbArticle): string {
  const source = (a.body ?? "").trim();
  if (source === "") return "No summary yet — open the editor to add content.";
  const line = source.split("\n").find((l) => l.trim() !== "");
  return line ?? "Untitled body";
}

function CategoryChip({
  name,
  count,
  selected,
  clientVisible = false,
  onSelect,
}: {
  name: string;
  count: number;
  selected: boolean;
  clientVisible?: boolean;
  onSelect(): void;
}) {
  return (
    <button
      className="w-full min-w-0 truncate rounded-md px-2 py-1 text-left text-[13px] transition-transform duration-100 hover:-translate-y-px"
      style={{
        background: selected ? "var(--m-accent)" : "transparent",
        color: selected ? "var(--m-background)" : "var(--m-text)",
        fontWeight: selected ? 600 : 400,
      }}
      onClick={onSelect}
    >
      {name}
      <span
        className="ml-2 inline-block rounded-full px-1.5 text-[10px] tabular-nums"
        style={{
          background: selected ? "rgba(255,255,255,.2)" : "var(--m-border)",
          color: selected ? "var(--m-background)" : "var(--m-text-secondary)",
        }}
      >
        {count}
      </span>
      {clientVisible ? " · client" : ""}
    </button>
  );
}

function ArticleViewer({
  article,
  detail,
  loading,
  category,
  onOpen,
  onClose,
}: {
  article: KbArticle | null;
  detail: ArticleDetail | null;
  loading: boolean;
  category: KbCategory | undefined;
  onOpen: () => void;
  onClose: () => void;
}) {
  useEffect(() => {
    if (article !== null && detail === null) onOpen();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [article?.id]);

  const headings = useMemo(() => headingsOf(detail?.content_html), [detail?.content_html]);
  const proseRef = useRef<HTMLDivElement>(null);

  const jumpTo = (index: number) => {
    const prose = proseRef.current;
    if (prose === null) return;
    const nodes = prose.querySelectorAll("h2, h3");
    const target = nodes[index];
    if (target !== undefined) target.scrollIntoView({ block: "start" });
  };

  return (
    <div className="grid gap-3 p-4">
      {article !== null ? (
        <div className="flex flex-wrap items-center gap-2 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
          <Tag tone={STATUS_TONE[article.status]}>{article.status}</Tag>
          {category !== undefined ? (
            <Tag tone="neutral">
              {category.name}
              {category.client_visible ? " · client-facing category" : ""}
            </Tag>
          ) : null}
          {article.client_visible ? <Tag tone={category?.client_visible ? "success" : "warning"}>client-facing</Tag> : null}
          <span>
            /{article.slug} · created {new Date(article.created_at).toLocaleDateString()} · updated{" "}
            {new Date(article.updated_at).toLocaleDateString()}
          </span>
        </div>
      ) : null}
      {loading || (article !== null && detail === null) ? (
        <div className="flex items-center gap-2 text-[13px]">
          <Spinner /> Rendering…
        </div>
      ) : (
        <>
          {headings !== null && headings.length > 1 ? (
            <nav aria-label="On this page" className="rounded-md border p-3" style={{ borderColor: "var(--m-border)" }}>
              <p className="mb-1 text-[11px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                On this page
              </p>
              <ul className="grid gap-0.5">
                {headings.map((h, i) => (
                  <li key={i}>
                    <button
                      className="text-left text-[12px] underline-offset-2 hover:underline"
                      style={{
                        paddingLeft: h.level === 3 ? 12 : 0,
                        color: "var(--m-text-secondary)",
                      }}
                      onClick={() => jumpTo(i)}
                    >
                      {h.text}
                    </button>
                  </li>
                ))}
              </ul>
            </nav>
          ) : null}
          <div ref={proseRef}>
            {detail?.content_html !== undefined && detail.content_html !== null && detail.content_html !== "" ? (
              <KbProse html={detail.content_html} />
            ) : (
              <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                {detail !== null
                  ? "This article's canonical content is empty — open the editor to write the first paragraph."
                  : "The rendered content is not available right now."}
              </p>
            )}
          </div>
        </>
      )}
      <DialogActions>
        <Button variant="ghost" onClick={onClose}>
          Close
        </Button>
      </DialogActions>
    </div>
  );
}

function CategoryForm({ category, onDone }: { category: KbCategory | null; onDone: () => void }) {
  const [name, setName] = useState(category?.name ?? "");
  const [position, setPosition] = useState(
    category?.position !== null && category?.position !== undefined ? String(category.position) : "",
  );
  const [description, setDescription] = useState(category?.description ?? "");
  const [clientVisible, setClientVisible] = useState(category?.client_visible ?? false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    setBusy(true);
    setError(null);
    const parsedPosition =
      position.trim() === "" ? null : Number.parseInt(position, 10);
    const positionField: { position?: number | null } =
      parsedPosition === null || Number.isNaN(parsedPosition)
        ? { position: null }
        : { position: parsedPosition };
    const result =
      category === null
        ? await runEnvelopeMutation<
            components["schemas"]["CreateCategoryRequest"],
            components["schemas"]["CategoryMutationResponse"]
          >({
            describe: "Create KB category",
            buildBody: (base) => ({
              idempotency_key: base.idempotency_key,
              name,
              description,
              client_visible: clientVisible,
              ...positionField,
            }),
            optimistic: () => {},
            revert: () => {},
            request: (client, body) => client.POST("/api/v1/kb/categories", { body }),
            versions: (response) => [[response.id, response.mutation.version]],
          })
        : await runEnvelopeMutation<
            components["schemas"]["UpdateCategoryRequest"],
            components["schemas"]["CategoryMutationResponse"]
          >({
            describe: "Update KB category",
            entityId: category.id,
            buildBody: (base) => ({
              idempotency_key: base.idempotency_key,
              name,
              description,
              client_visible: clientVisible,
              ...positionField,
            }),
            optimistic: () => {},
            revert: () => {},
            request: (client, body) =>
              client.PATCH("/api/v1/kb/categories/{id}", {
                params: { path: { id: category.id } },
                body,
              }),
            versions: (response) => [[response.id, response.mutation.version]],
          });
    setBusy(false);
    if (!result.ok) {
      setError(rejectionText(result, "The category was rejected."));
      return;
    }
    onDone();
  };

  return (
    <div className="grid gap-3 p-4">
      <div>
        <Label htmlFor="cat-name">Name</Label>
        <Input id="cat-name" value={name} onChange={(e) => setName(e.target.value)} />
      </div>
      <div>
        <Label htmlFor="cat-desc">Description</Label>
        <Input id="cat-desc" value={description} onChange={(e) => setDescription(e.target.value)} />
      </div>
      <div>
        <Label htmlFor="cat-position">Sort position</Label>
        <Input
          id="cat-position"
          inputMode="numeric"
          placeholder="Lower sorts first"
          value={position}
          onChange={(e) => setPosition(e.target.value)}
        />
        <FieldHint>Empty places the category last.</FieldHint>
      </div>
      <label className="flex items-start gap-2 text-[13px]">
        <input
          type="checkbox"
          className="mt-0.5"
          checked={clientVisible}
          onChange={(e) => setClientVisible(e.target.checked)}
          aria-describedby="cat-visibility-note"
        />
        <span>
          Client-facing category
          <span
            id="cat-visibility-note"
            className="block text-[11px]"
            style={{ color: "var(--m-text-muted)" }}
          >
            The portal gate needs the client-facing flag on BOTH this category
            and each article — staff-only otherwise.
          </span>
        </span>
      </label>
      {error !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </p>
      ) : null}
      <DialogActions>
        <Button variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button onClick={() => void submit()} disabled={busy || name.trim() === ""}>
          {busy ? <Spinner /> : null} Save
        </Button>
      </DialogActions>
    </div>
  );
}

function ArticleEditor({
  article,
  categories,
  onDone,
}: {
  article: KbArticle | null;
  categories: KbCategory[];
  onDone: () => void;
}) {
  // New articles create FIRST (instant-first: the create is autosave too
  // — one idempotent POST), then the editor mounts immediately; every
  // content transaction after that autosaves under the IF1–IF3 envelope.
  const [preview, setPreview] = useState<components["schemas"]["SearchResult"][] | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [title, setTitle] = useState(article?.title ?? "");
  const [categoryId, setCategoryId] = useState(article?.category_id ?? "");
  const [status, setStatus] = useState<KbArticleStatus>(article?.status ?? "draft");
  const [clientVisible, setClientVisible] = useState(article?.client_visible ?? false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const versionRef = useRef<number | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const patchMetadata = useCallback(
    async (
      id: string,
      patch: Partial<Record<"title" | "category_id" | "status" | "client_visible", unknown>>,
    ) => {
      const result = await runEnvelopeMutation<
        components["schemas"]["UpdateArticleRequest"],
        components["schemas"]["ArticleMutationResponse"]
      >({
        describe: "KB article metadata autosave",
        entityId: id,
        buildBody: (base) =>
          ({
            idempotency_key: base.idempotency_key,
            ...(versionRef.current !== null ? { version: versionRef.current } : {}),
            ...patch,
          }) as unknown as components["schemas"]["UpdateArticleRequest"],
        optimistic: () => {},
        revert: () => {},
        request: (client, body) =>
          client.PATCH("/api/v1/kb/articles/{id}", {
            params: { path: { id } },
            body,
          }),
        versions: (response) => {
          versionRef.current = response.mutation.version;
          return [[id, response.mutation.version]];
        },
      });
      if (!result.ok) {
        setError(rejectionText(result, "The article metadata was rejected."));
      }
      return result.ok;
    },
    [],
  );

  // Debounced metadata autosave (title rides it; category/status flips
  // commit immediately — status flips are explicit lifecycle actions).
  const debounceTitle = useCallback(
    (next: string, id: string | null) => {
      if (id === null) return;
      if (timerRef.current !== null) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => {
        timerRef.current = null;
        void patchMetadata(id, { title: next });
      }, 700);
    },
    [patchMetadata],
  );

  useEffect(
    () => () => {
      if (timerRef.current !== null) clearTimeout(timerRef.current);
    },
    [],
  );

  const previewFts = async () => {
    setPreviewing(true);
    setPreview(null);
    const { data } = await api.api.GET("/api/v1/search", {
      params: { query: { q: title, limit: 5 } },
    });
    setPreviewing(false);
    if (data !== undefined) setPreview(data.results);
  };

  const submit = async () => {
    setBusy(true);
    setError(null);
    const shared = {
      title,
      category_id: categoryId === "" ? null : categoryId,
      status,
      client_visible: clientVisible,
    };
    const result =
      article === null
        ? await runEnvelopeMutation<
            components["schemas"]["CreateArticleRequest"],
            components["schemas"]["ArticleMutationResponse"]
          >({
            describe: "Create KB article",
            buildBody: (base) => ({
              idempotency_key: base.idempotency_key,
              ...shared,
              content_jsonb: {
                schema_ver: 1,
                doc: { type: "doc", content: [{ type: "paragraph" }] },
              },
            }) as unknown as components["schemas"]["CreateArticleRequest"],
            optimistic: () => {},
            revert: () => {},
            request: (client, body) => client.POST("/api/v1/kb/articles", { body }),
            versions: (response) => {
              versionRef.current = response.mutation.version;
              rememberVersion(response.id, response.mutation.version);
              return [[response.id, response.mutation.version]];
            },
          })
        : await runEnvelopeMutation<
            components["schemas"]["UpdateArticleRequest"],
            components["schemas"]["ArticleMutationResponse"]
          >({
            describe: "Update KB article",
            entityId: article.id,
            buildBody: (base) => ({
              idempotency_key: base.idempotency_key,
              version: base.version,
              ...shared,
            }) as unknown as components["schemas"]["UpdateArticleRequest"],
            optimistic: () => {},
            revert: () => {},
            request: (client, body) =>
              client.PATCH("/api/v1/kb/articles/{id}", { params: { path: { id: article.id } }, body }),
            versions: (response) => {
              versionRef.current = response.mutation.version;
              rememberVersion(article.id, response.mutation.version);
              return [[article.id, response.mutation.version]];
            },
          });
    setBusy(false);
    if (!result.ok) {
      setError(rejectionText(result, "The article was rejected."));
      return;
    }
    onDone();
  };

  const category = categories.find((c) => c.id === categoryId);
  const portalGate = clientVisible && (category?.client_visible ?? false);

  return (
    <div className="grid gap-3 p-4">
      <div>
        <Label htmlFor="art-title">Title</Label>
        <Input
          id="art-title"
          value={title}
          onChange={(e) => {
            setTitle(e.target.value);
            debounceTitle(e.target.value, article?.id ?? null);
          }}
        />
      </div>
      <div className="grid grid-cols-2 gap-3">
        <div>
          <Label htmlFor="art-category">Category</Label>
          <Select
            value={categoryId}
            onValueChange={(v) => {
              setCategoryId(v);
              const id = article?.id;
              if (id !== undefined) {
                void patchMetadata(id, {
                  category_id: v === "" ? null : v,
                });
              }
            }}
          >
            <SelectTrigger id="art-category">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="">Uncategorized</SelectItem>
              {categories.map((c) => (
                <SelectItem key={c.id} value={c.id}>
                  {c.name}
                  {c.client_visible ? " (client-facing)" : ""}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div>
          <Label htmlFor="art-status">Status</Label>
          <Select
            value={status}
            onValueChange={(v) => {
              setStatus(v as KbArticleStatus);
              const id = article?.id;
              if (id !== undefined) void patchMetadata(id, { status: v });
            }}
          >
            <SelectTrigger id="art-status">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {STATUSES.map((s) => (
                <SelectItem key={s} value={s}>
                  {s}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>
      {article === null ? (
        <>
          <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            Creating the article now — the rich editor opens instantly after the
            create (instant-first: no Save clicks for content).
          </p>
          {error !== null ? (
            <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {error}
            </p>
          ) : null}
          <DialogActions>
            <Button variant="ghost" onClick={onDone}>
              Cancel
            </Button>
            <Button onClick={() => void submit()} disabled={busy || title.trim() === ""}>
              {busy ? <Spinner /> : null} Create article
            </Button>
          </DialogActions>
        </>
      ) : (
        <>
          <label className="flex items-start gap-2 text-[13px]">
            <input
              type="checkbox"
              className="mt-0.5"
              checked={clientVisible}
              onChange={(e) => {
                setClientVisible(e.target.checked);
                void patchMetadata(article.id, { client_visible: e.target.checked });
              }}
              aria-describedby="art-visibility-note"
            />
            <span>
              Client-facing article
              <span
                id="art-visibility-note"
                className="block text-[11px]"
                style={{
                  color: portalGate ? "var(--m-status-success-fg)" : "var(--m-text-muted)",
                }}
              >
                {portalGate
                  ? "Portal clients can read this article once published."
                  : "Staff-only: the portal gate needs the client-facing flags on BOTH the article and its category."}
              </span>
            </span>
          </label>
          {error !== null ? (
            <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {error}
            </p>
          ) : null}
          <KbArticleEditor
            article={article as KbArticle & { content_jsonb?: unknown }}
            onSaved={() => setError(null)}
            onRejection={(friendly) =>
              setError(friendly ?? "The content save was rejected.")
            }
          />
          <DialogActions>
            <Button variant="ghost" onClick={() => void previewFts()} disabled={previewing || title.trim() === ""}>
              {previewing ? <Spinner /> : null} Preview search
            </Button>
            <Button variant="ghost" onClick={onDone}>
              Close
            </Button>
          </DialogActions>
          {preview !== null ? (
            <div className="grid gap-1">
              {preview.length === 0 ? (
                <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                  No indexed hits yet — drafts index on save; new titles surface after the next save.
                </p>
              ) : (
                preview.map((r) => (
                  <p key={r.entity_id} className="flex items-center gap-2 text-[12px]">
                    <Tag>{r.entity_kind}</Tag> {r.title}
                    {r.entity_kind === "kb_article" && r.entity_id === (article?.id ?? "") ? (
                      <span className="font-medium">(this article)</span>
                    ) : null}
                  </p>
                ))
              )}
            </div>
          ) : null}
        </>
      )}
    </div>
  );
}
