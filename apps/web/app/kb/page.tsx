// /kb — staff knowledge-base authoring (native KB v1 + docs-platform).
//
// Purpose: the internal authoring surface: category tree, article editor
// with lifecycle status (draft / published / archived) and the
// client-facing toggle (visible to portal clients only when the article
// AND its category both pass the gate), plus an FTS preview that runs the
// article's title+body through the unified search index the way a
// searcher would hit it.
"use client";

import { rejectionText } from "../../lib/error-copy";
import { KbArticleEditor } from "../../lib/editor/kb-article-editor";
import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  EmptyState,
  Input,
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
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";
import {
  rememberVersion,
  runEnvelopeMutation,
} from "../../lib/envelope-write";

type KbCategory = components["schemas"]["KbCategory"];
type KbArticle = components["schemas"]["KbArticle"];
type KbArticleStatus = components["schemas"]["KbArticleStatus"];

const STATUSES: KbArticleStatus[] = ["draft", "published", "archived"];

export default function KbPage() {
  const auth = useRequireAuth();
  const [categories, setCategories] = useState<KbCategory[] | null>(null);
  const [articles, setArticles] = useState<KbArticle[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [editing, setEditing] = useState<KbArticle | null>(null);
  const [editorOpen, setEditorOpen] = useState(false);
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

  const categoryById = useMemo(() => new Map((categories ?? []).map((c) => [c.id, c])), [categories]);

  const visible = useMemo(() => {
    if (articles === null) return null;
    return articles.filter((a) => selectedCategory === null || a.category_id === selectedCategory);
  }, [articles, selectedCategory]);

  if (auth.status !== "authed") return null;

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-8 p-4 md:p-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold">Knowledge base</h1>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Internal runbooks and how-tos. Articles flagged client-facing (in a client-facing
            category) appear in the portal.
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
              <button
                className={`w-full rounded-[2px] px-2 py-1 text-left text-[13px] ${selectedCategory === null ? "font-medium" : ""}`}
                style={{
                  background: selectedCategory === null ? "var(--m-bg-subtle, transparent)" : undefined,
                }}
                onClick={() => setSelectedCategory(null)}
              >
                All articles
              </button>
            </li>
            {(categories ?? [])
              .slice()
              .sort((a, b) => a.position - b.position)
              .map((c) => (
                <li key={c.id} className="flex items-center justify-between gap-1">
                  <button
                    className={`min-w-0 flex-1 truncate rounded-[2px] px-2 py-1 text-left text-[13px] ${selectedCategory === c.id ? "font-medium" : ""}`}
                    onClick={() => setSelectedCategory(c.id)}
                  >
                    {c.name}
                    {c.client_visible ? (
                      <Tag tone="success" className="ml-2">
                        client
                      </Tag>
                    ) : null}
                  </button>
                  <Button
                    variant="ghost"
                    onClick={() => {
                      setEditingCategory(c);
                      setCategoryOpen(true);
                    }}
                  >
                    Edit
                  </Button>
                </li>
              ))}
          </ul>
        </Card>

        <Card>
          <h2 className="p-4 pb-2 text-sm font-semibold">
            {selectedCategory !== null
              ? (categoryById.get(selectedCategory)?.name ?? "Articles")
              : "All articles"}
          </h2>
          {visible === null ? (
            <div className="p-4">
              <Spinner />
            </div>
          ) : visible.length === 0 ? (
            <div className="p-4">
              <EmptyState message="No articles here yet. Draft articles are staff-only — staff search (and the FTS preview) sees them; publishing makes an article (with the client-facing flags) portal-visible through the portal's own gated search." />
            </div>
          ) : (
            <ul className="grid gap-2 px-4 pb-4">
              {visible.map((a) => (
                <li
                  key={a.id}
                  className="flex flex-wrap items-center justify-between gap-2 rounded-[2px] border px-3 py-2"
                  style={{ borderColor: "var(--m-border)" }}
                >
                  <div className="min-w-0">
                    <p className="truncate text-[13px] font-medium">{a.title}</p>
                    <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                      /{a.slug} · updated {new Date(a.updated_at).toLocaleDateString()}
                    </p>
                  </div>
                  <div className="flex shrink-0 items-center gap-2">
                    <Tag
                      tone={
                        a.status === "published"
                          ? "success"
                          : a.status === "archived"
                            ? "danger"
                            : "neutral"
                      }
                    >
                      {a.status}
                    </Tag>
                    {a.client_visible ? <Tag tone="success">client-visible</Tag> : null}
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
                </li>
              ))}
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

function CategoryForm({ category, onDone }: { category: KbCategory | null; onDone: () => void }) {
  const [name, setName] = useState(category?.name ?? "");
  const [description, setDescription] = useState(category?.description ?? "");
  const [clientVisible, setClientVisible] = useState(category?.client_visible ?? false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    setBusy(true);
    setError(null);
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
            }),
            optimistic: () => {},
            revert: () => {},
            request: (client, body) =>
              client.PATCH("/api/v1/kb/categories/{id}", { params: { path: { id: category.id } }, body }),
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
      <label className="flex items-center gap-2 text-[13px]">
        <input
          type="checkbox"
          checked={clientVisible}
          onChange={(e) => setClientVisible(e.target.checked)}
        />
        Client-facing category
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
    async (id: string, patch: Partial<Record<"title" | "category_id" | "status" | "client_visible", unknown>>) => {
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

  useEffect(() => () => {
    if (timerRef.current !== null) clearTimeout(timerRef.current);
  }, []);

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

  const category = categories.find((c) => c.id === (categoryId || null));
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
          <label className="flex items-center gap-2 text-[13px]">
            <input
              type="checkbox"
              checked={clientVisible}
              onChange={(e) => {
                setClientVisible(e.target.checked);
                void patchMetadata(article.id, { client_visible: e.target.checked });
              }}
            />
            Client-facing article
          </label>
          <p
            className="text-[12px]"
            style={{ color: portalGate ? "var(--m-status-success-fg)" : "var(--m-text-secondary)" }}
          >
            {portalGate
              ? "Portal clients can read this article once published."
              : "Staff-only: the portal gate needs the client-facing flags on BOTH the article and its category."}
          </p>
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
                  <p key={r.entity_id} className="text-[12px]">
                    <Tag>{r.entity_kind}</Tag> {r.title}
                    {r.entity_kind === "kb_article" && r.entity_id === (article?.id ?? "") ? (
                      <span className="ml-2 font-medium">(this article)</span>
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
