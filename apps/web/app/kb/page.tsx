// /kb — the staff knowledge base, on the mockup pack's ledger template.
//
// Purpose: the internal authoring + reading surface, composed from the pack's
// Tickets list (DERIVATION-GUIDE.md line 33: "Editor with a publish state").
// A category rail on the left, the article ledger on the right: a 64px page
// header with attention shortcuts, one control row (views, filters, columns,
// export), the bordered table card, and the 52px footer with the row count and
// the density control. The article editor carries the PUBLISH state the brief
// asks for — status, category and portal visibility, in one row, so the
// difference between "written" and "visible to the client" is a decision on
// screen rather than a post-hoc fix.
//
// Non-purpose: no new editor architecture; the Tiptap wiring is the shipped
// lane-2 envelope (ADR 0007) and this file never re-wires it. Portal gating
// stays the portal's own concern — an article AND its category must both be
// client-facing before the portal's gated search can see it, and this page
// only reports that state honestly.
"use client";

import { rejectionText } from "../../lib/error-copy";
import { KbArticleEditor } from "../../lib/editor/kb-article-editor";
import {
  Button,
  Checkbox,
  ColumnChooser,
  DataToolbar,
  DensitySwitch,
  Dialog,
  DialogActions,
  DialogContent,
  EmptyState,
  ExportControl,
  FieldHint,
  FilterBar,
  Input,
  KbProse,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  Table,
  Tag,
  Td,
  Th,
  Tr,
  ViewSwitch,
  applyDensity,
  useDensity,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AlertIcon,
  BookIcon,
  CheckIcon,
  FileIcon,
  HEAD_CELL_CLASS,
  KeyHint,
  LedgerFooter,
  NoMatches,
  PageFrame,
  PageHeader,
  PlusIcon,
  SELECTED_ROW_STYLE,
  SelectionBar,
  SkeletonRows,
  StatPill,
  StatusPill,
  TableCard,
  downloadCsv,
} from "../../components/knowledge/ledger";
import { api, useRequireAuth } from "../../lib/api";
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

/** The pack's article-ledger columns (DESIGN-NOTES-tickets.md §11: the
 *  Ticket-like primary column is always required and first). */
type ColumnKey = "article" | "category" | "status" | "visibility" | "updated" | "created";

const DEFAULT_COLUMNS: { key: ColumnKey; label: string; visible: boolean; required?: boolean }[] = [
  { key: "article", label: "Article", visible: true, required: true },
  { key: "category", label: "Category", visible: true },
  { key: "status", label: "Status", visible: true },
  { key: "visibility", label: "Portal", visible: true },
  { key: "updated", label: "Last edited", visible: true },
  { key: "created", label: "Created", visible: false },
];

const COLUMN_STORAGE_KEY = "marula-kb-cols";

function loadColumns(): typeof DEFAULT_COLUMNS {
  try {
    const raw = window.localStorage.getItem(COLUMN_STORAGE_KEY);
    if (raw === null) return DEFAULT_COLUMNS;
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return DEFAULT_COLUMNS;
    const order = parsed.filter((k): k is ColumnKey =>
      DEFAULT_COLUMNS.some((c) => c.key === k),
    );
    const visible = new Set(parsed.filter((k): k is string => typeof k === "string"));
    const next = order.map(
      (key) =>
        DEFAULT_COLUMNS.find((c) => c.key === key) ?? {
          key,
          label: key,
          visible: true,
        },
    );
    for (const column of DEFAULT_COLUMNS) {
      if (!next.some((c) => c.key === column.key)) next.push(column);
    }
    // A column the user's saved order hides stays hidden; anything absent from
    // the saved set falls back to the default's own visibility.
    return next.map((c) => {
      const fallback = DEFAULT_COLUMNS.find((d) => d.key === c.key);
      return { ...c, visible: visible.has(`vis:${c.key}`) || (fallback?.visible ?? false) };
    });
  } catch {
    return DEFAULT_COLUMNS;
  }
}

function saveColumns(columns: { key: ColumnKey; visible: boolean }[]): void {
  try {
    window.localStorage.setItem(
      COLUMN_STORAGE_KEY,
      JSON.stringify([
        ...columns.map((c) => c.key),
        ...columns.filter((c) => c.visible).map((c) => `vis:${c.key}`),
      ]),
    );
  } catch {
    // Storage unavailable: the choice lasts for this visit only.
  }
}

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

type Visibility = "gated" | "ungated" | "staff";

function visibilityOf(
  article: KbArticle,
  categoryById: Map<string, KbCategory>,
): Visibility {
  if (!article.client_visible) return "staff";
  const category =
    article.category_id != null ? categoryById.get(article.category_id) : undefined;
  return category?.client_visible === true ? "gated" : "ungated";
}

export default function KbPage() {
  const auth = useRequireAuth();
  const [categories, setCategories] = useState<KbCategory[] | null>(null);
  const [articles, setArticles] = useState<KbArticle[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [view, setView] = useState("all");
  const [clauses, setClauses] = useState<{ id: string; field: string; op: never; value?: string }[]>(
    [],
  );
  const [columns, setColumns] = useState<typeof DEFAULT_COLUMNS>(DEFAULT_COLUMNS);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busyBulk, setBusyBulk] = useState(false);
  const [bulkNote, setBulkNote] = useState<{ tone: "good" | "attention"; text: string } | null>(
    null,
  );
  const density = useDensity();

  const [editing, setEditing] = useState<KbArticle | null>(null);
  const [editorOpen, setEditorOpen] = useState(false);
  const [viewOpen, setViewOpen] = useState(false);
  const [viewing, setViewing] = useState<KbArticle | null>(null);
  const [viewContent, setViewContent] = useState<ArticleDetail | null>(null);
  const [viewLoading, setViewLoading] = useState(false);
  const [categoryOpen, setCategoryOpen] = useState(false);
  const [editingCategory, setEditingCategory] = useState<KbCategory | null>(null);
  const firstLoad = useRef(true);

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

  useEffect(() => {
    if (firstLoad.current) {
      firstLoad.current = false;
      setColumns(loadColumns());
    }
  }, []);

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

  const all = useMemo(
    () =>
      (articles ?? []).slice().sort((a, b) => {
        const at = Date.parse(a.updated_at);
        const bt = Date.parse(b.updated_at);
        if (Number.isNaN(at) || Number.isNaN(bt)) return a.id.localeCompare(b.id);
        return bt - at;
      }),
    [articles],
  );

  const inCategory = useMemo(
    () => all.filter((a) => selectedCategory === null || a.category_id === selectedCategory),
    [all, selectedCategory],
  );

  const changedToday = useMemo(() => {
    const since = Date.now() - 24 * 60 * 60 * 1000;
    return inCategory.filter((a) => Date.parse(a.updated_at) >= since).length;
  }, [inCategory]);
  const draftCount = useMemo(
    () => inCategory.filter((a) => a.status === "draft").length,
    [inCategory],
  );
  const unfiledCount = useMemo(
    () => inCategory.filter((a) => a.category_id === null || a.category_id === undefined).length,
    [inCategory],
  );

  const visible = useMemo(() => {
    let rows = inCategory;
    if (view === "drafts") rows = rows.filter((a) => a.status === "draft");
    if (view === "published") rows = rows.filter((a) => a.status === "published");
    if (view === "archived") rows = rows.filter((a) => a.status === "archived");
    for (const clause of clauses) {
      const field = clause.field;
      const value = (clause.value ?? "").toLowerCase();
      rows = rows.filter((a) => {
        if (field === "title") return a.title.toLowerCase().includes(value);
        if (field === "category") {
          const name =
            a.category_id != null ? (categoryById.get(a.category_id)?.name ?? "") : "";
          return name.toLowerCase().includes(value);
        }
        if (field === "status") return a.status === value;
        if (field === "visibility") return visibilityOf(a, categoryById) === value;
        return true;
      });
    }
    return rows;
  }, [inCategory, view, clauses, categoryById]);

  const shown = columns.filter((c) => c.visible);

  const toggleSelected = useCallback((id: string, on: boolean) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });
  }, []);

  const setStatus = useCallback(
    async (ids: string[], status: KbArticleStatus) => {
      setBusyBulk(true);
      let ok = 0;
      const failed: string[] = [];
      for (const id of ids) {
        const result = await runEnvelopeMutation<
          components["schemas"]["UpdateArticleRequest"],
          components["schemas"]["ArticleMutationResponse"]
        >({
          describe: `KB article ${status}`,
          entityId: id,
          buildBody: (base) => ({
            idempotency_key: base.idempotency_key,
            ...(base.version !== undefined ? { version: base.version } : {}),
            status,
          }),
          optimistic: () => {},
          revert: () => {},
          request: (client, body) =>
            client.PATCH("/api/v1/kb/articles/{id}", { params: { path: { id } }, body }),
          versions: (response) => {
            rememberVersion(id, response.mutation.version);
            return [[id, response.mutation.version]];
          },
        });
        if (result.ok) ok += 1;
        else failed.push(result.friendly ?? result.message ?? `#${id.slice(0, 6)}`);
      }
      setBusyBulk(false);
      setSelected(new Set());
      await refresh();
      // The outcome is a strip above the table, not a toast: a bulk change that
      // half-succeeded must stay readable after the toast has gone.
      if (failed.length === 0) {
        setBulkNote({
          tone: "good",
          text: `${ok} article${ok === 1 ? "" : "s"} moved to ${status}.`,
        });
      } else {
        setBulkNote({
          tone: "attention",
          text: `${ok} changed, ${failed.length} rejected — ${failed.join(", ")}.`,
        });
      }
    },
    [refresh],
  );

  if (auth.status !== "authed") return null;

  return (
    <PageFrame>
      <PageHeader
        title="Knowledge base"
        subtitle="Runbooks, published deliberately"
        attention={
          <>
            <StatPill tone="gold" label="Changed today" count={changedToday} />
            <StatPill tone="blush" label="Drafts" count={draftCount} onClick={() => setView(view === "drafts" ? "all" : "drafts")} pressed={view === "drafts"} />
            <StatPill tone="blush" label="Unfiled" count={unfiledCount} />
          </>
        }
        actions={
          <>
            <Button variant="secondary" onClick={() => { setEditingCategory(null); setCategoryOpen(true); }}>
              <PlusIcon size={16} /> New category
            </Button>
            <Button onClick={() => { setEditing(null); setEditorOpen(true); }} data-testid="kb-new-article">
              <PlusIcon size={16} /> New article
            </Button>
          </>
        }
      />

      {loadError !== null ? (
        <p
          role="alert"
          className="mb-2 inline-flex items-center gap-2 rounded-[12px] p-3 text-[13px]"
          style={{ background: "var(--m-status-danger-bg)", color: "var(--m-status-danger-fg)" }}
        >
          <AlertIcon size={16} />
          {loadError}
        </p>
      ) : null}

      <div className="grid min-h-0 flex-1 gap-6 xl:grid-cols-[240px_minmax(0,1fr)]">
        <aside
          className="hidden min-h-0 flex-col overflow-auto rounded-[16px] p-3 xl:flex"
          style={{ border: "1px solid var(--m-border)", background: "var(--m-sidebar)" }}
          aria-label="Categories"
        >
          <p
            className="px-2 pb-2 text-[12px] font-semibold uppercase tracking-[0.06em]"
            style={{ color: "var(--m-text-secondary)" }}
          >
            Categories
          </p>
          <ul className="grid gap-1">
            <li>
              <CategoryRow
                name="All articles"
                count={all.length}
                selected={selectedCategory === null}
                onSelect={() => setSelectedCategory(null)}
              />
            </li>
            {sortedCategories.map((c) => (
              <li key={c.id} className="group flex items-center gap-1">
                <CategoryRow
                  name={c.name}
                  count={all.filter((a) => a.category_id === c.id).length}
                  selected={selectedCategory === c.id}
                  clientVisible={c.client_visible}
                  onSelect={() => setSelectedCategory(c.id)}
                />
                <button
                  type="button"
                  className="shrink-0 px-1 text-[12px] opacity-0 transition-opacity duration-100 group-hover:opacity-100 focus-visible:opacity-100"
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
          <div className="mt-auto pt-4">
            <Link
              href="/kb/secrets"
              className="block rounded-[10px] px-2 py-1.5 text-[12px] underline-offset-2 hover:underline"
              style={{ color: "var(--m-text-secondary)" }}
            >
              Secret categories
            </Link>
            <Link
              href="/portal/kb"
              className="block rounded-[10px] px-2 py-1.5 text-[12px] underline-offset-2 hover:underline"
              style={{ color: "var(--m-text-secondary)" }}
            >
              What clients see
            </Link>
          </div>
        </aside>

        <div className="flex min-h-0 flex-col">
          <DataToolbar
            label="Knowledge base controls"
            views={
              <ViewSwitch
                label="Article views"
                views={[
                  { id: "all", label: "All", hint: String(inCategory.length) },
                  { id: "drafts", label: "Drafts", hint: String(draftCount) },
                  { id: "published", label: "Published" },
                  { id: "archived", label: "Archived" },
                ]}
                activeId={view}
                onChange={setView}
              />
            }
            filters={
              <FilterBar
                label="Article filters"
                emptyHint="No filters"
                fields={[
                  { key: "title", label: "Title", operators: ["contains"] },
                  { key: "category", label: "Category", operators: ["contains"] },
                  { key: "status", label: "Status", operators: ["eq"] },
                  { key: "visibility", label: "Portal", operators: ["eq"] },
                ]}
                clauses={clauses}
                onChange={(next) =>
                  setClauses(
                    next.map((c) => ({ ...c, op: c.op as never })),
                  )
                }
              />
            }
            columns={
              <ColumnChooser
                label="Article columns"
                columns={columns.map((c) => ({ key: c.key, label: c.label, visible: c.visible, required: c.required }))}
                onChange={(next) => {
                  const mapped = next.map((c) => ({
                    key: c.key as ColumnKey,
                    label: c.label,
                    visible: c.visible,
                    required: c.required,
                  }));
                  setColumns(mapped);
                  saveColumns(mapped);
                }}
              />
            }
            export={
              <ExportControl
                label="Export"
                formats={["csv"]}
                columns={shown.map((c) => c.label)}
                rowCount={visible.length}
                filters={clauses}
                onExport={() => {
                  downloadCsv(
                    `knowledge-base-${view}.csv`,
                    shown.map((c) => c.label),
                    visible.map((a) =>
                      shown.map((c) => {
                        if (c.key === "article") return a.title;
                        if (c.key === "category")
                          return a.category_id != null
                            ? (categoryById.get(a.category_id)?.name ?? "")
                            : "";
                        if (c.key === "status") return a.status;
                        if (c.key === "visibility") return visibilityOf(a, categoryById);
                        if (c.key === "updated") return a.updated_at;
                        return a.created_at;
                      }),
                    ),
                  );
                }}
              />
            }
          />

          <TableCard label="Knowledge base articles">
            {articles === null ? (
              <SkeletonRows sentence="Loading the knowledge base." columns={4} />
            ) : (
              <Table data-testid="kb-article-table">
                <thead>
                  {selected.size > 0 ? (
                    <tr>
                      <th colSpan={shown.length + 1} style={{ padding: 0 }}>
                        <SelectionBar
                          count={selected.size}
                          noun="article"
                          onClear={() => setSelected(new Set())}
                        >
                          <Button
                            size="sm"
                            variant="secondary"
                            disabled={busyBulk}
                            onClick={() => void setStatus([...selected], "published")}
                            data-testid="kb-bulk-publish"
                          >
                            Publish
                          </Button>
                          <Button
                            size="sm"
                            variant="secondary"
                            disabled={busyBulk}
                            onClick={() => void setStatus([...selected], "archived")}
                          >
                            Archive
                          </Button>
                          <Button
                            size="sm"
                            variant="secondary"
                            disabled={busyBulk}
                            onClick={() => void setStatus([...selected], "draft")}
                          >
                            Move to draft
                          </Button>
                        </SelectionBar>
                      </th>
                    </tr>
                  ) : (
                    <tr>
                      <Th style={{ width: 44 }} sortable={false} resizable={false}>
                        <Checkbox
                          aria-label="Select all articles"
                          checked={selected.size > 0 && selected.size === visible.length}
                          onCheckedChange={(on) =>
                            setSelected(on ? new Set(visible.map((a) => a.id)) : new Set())
                          }
                        />
                      </Th>
                      {shown.map((c) => (
                        <Th key={c.key} className={HEAD_CELL_CLASS}>
                          {c.label}
                        </Th>
                      ))}
                    </tr>
                  )}
                </thead>
                <tbody>
                  {visible.map((a) => {
                    const visibility = visibilityOf(a, categoryById);
                    return (
                      <Tr
                        key={a.id}
                        selected={selected.has(a.id)}
                        style={{
                          height: "var(--m-density-row)",
                          ...(selected.has(a.id) ? SELECTED_ROW_STYLE : {}),
                        }}
                      >
                        <Td>
                          <Checkbox
                            aria-label={`Select ${a.title}`}
                            checked={selected.has(a.id)}
                            onCheckedChange={(on) => toggleSelected(a.id, on === true)}
/>

          {bulkNote !== null ? (
            <div
              role="status"
              data-testid="kb-bulk-note"
              className="mb-2 flex flex-wrap items-center gap-2 rounded-[12px] px-3 py-2 text-[13px]"
              style={{
                background:
                  bulkNote.tone === "good"
                    ? "var(--m-status-success-bg)"
                    : "var(--m-status-due-bg)",
                color:
                  bulkNote.tone === "good"
                    ? "var(--m-status-success-fg)"
                    : "var(--m-status-due-fg)",
              }}
            >
              {bulkNote.tone === "good" ? <CheckIcon size={16} /> : <AlertIcon size={16} />}
              <span>{bulkNote.text}</span>
              <button
                type="button"
                className="ml-auto inline-flex h-8 items-center rounded-[8px] border px-2.5 text-[13px] font-semibold"
                style={{
                  borderColor: "var(--m-border)",
                  background: "var(--m-surface)",
                  color: "var(--m-text)",
                }}
                onClick={() => setBulkNote(null)}
              >
                Dismiss
              </button>
            </div>
          ) : null}


                        </Td>
                        {shown.map((c) => (
                          <Td key={c.key}>
                            {c.key === "article" ? (
                              <button
                                type="button"
                                onClick={() => {
                                  setViewing(a);
                                  setViewContent(null);
                                  setViewOpen(true);
                                }}
                                className="flex min-w-0 items-center gap-2 text-left"
                              >
                                <span style={{ color: "var(--m-text-muted)" }}>
                                  <FileIcon size={16} />
                                </span>
                                <span className="truncate font-semibold" style={{ color: "var(--m-text)" }}>
                                  {a.title}
                                </span>
                              </button>
                            ) : null}
                            {c.key === "category" ? (
                              <span style={{ color: "var(--m-text-secondary)" }}>
                                {a.category_id != null
                                  ? (categoryById.get(a.category_id)?.name ?? "Unknown category")
                                  : "Unfiled"}
                              </span>
                            ) : null}
                            {c.key === "status" ? (
                              <StatusPill tone={STATUS_TONE[a.status]}>{a.status}</StatusPill>
                            ) : null}
                            {c.key === "visibility" ? (
                              visibility === "gated" ? (
                                <StatusPill tone="success">In the portal</StatusPill>
                              ) : visibility === "ungated" ? (
                                <StatusPill tone="warning">Client-facing, not in the portal</StatusPill>
                              ) : (
                                <StatusPill tone="neutral">Staff only</StatusPill>
                              )
                            ) : null}
                            {c.key === "updated" ? (
                              <span className="tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                                {relativeTime(a.updated_at)}
                              </span>
                            ) : null}
                            {c.key === "created" ? (
                              <span className="tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                                {new Date(a.created_at).toLocaleDateString()}
                              </span>
                            ) : null}
                          </Td>
                        ))}
                      </Tr>
                    );
                  })}
                  {visible.length === 0 ? (
                    <tr>
                      <td colSpan={shown.length + 1}>
                        {articles.length === 0 ? (
                          <EmptyState
                            message="No articles yet. Write the runbook you keep explaining on the phone."
                            action={
                              <Button onClick={() => { setEditing(null); setEditorOpen(true); }}>
                                New article
                              </Button>
                            }
                          />
                        ) : (
                          <NoMatches
                            title="No articles match these filters."
                            hint={`Try removing one, or switch back to a view that holds something. Nothing was deleted.`}
                            noun="articles"
                            onClear={() => { setClauses([]); setView("all"); }}
                          />
                        )}
                      </td>
                    </tr>
                  ) : null}
                </tbody>
              </Table>
            )}
          </TableCard>

          <LedgerFooter
            shown={visible.length}
            total={articles?.length ?? 0}
            noun="articles"
            density={<DensitySwitch value={density} onChange={applyDensity} />}
            hints={
              <>
                <KeyHint keys="J K" label="move" />
                <KeyHint keys="X" label="select" />
                <KeyHint keys="↵" label="open" />
                <KeyHint keys="F" label="filter" />
              </>
            }
          />
        </div>
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
          description="Content saves as you type. Publishing is the row below, and it is the only thing the portal sees."
        >
          {editing === null ? (
            <NewArticleForm
              categories={categories ?? []}
              onCreated={(created) => {
                setEditing(created);
                void refresh();
              }}
            />
          ) : (
            <div className="grid gap-2">
              <KbArticleEditor
                article={editing}
                onSaved={() => void refresh()}
                onRejection={(message, reason) => {
                  if (message === null && reason === null) return;
                  // A rejected autosave is a strip, not a toast: the content
                  // is still open and the user has to see why it did not land.
                  setBulkNote({
                    tone: "attention",
                    text: rejectionText({ reason: reason ?? undefined, message: message ?? undefined }),
                  });
                }}
              />
              <PublishRow
                article={editing}
                categories={categories ?? []}
                onSaved={() => void refresh()}
              />
            </div>
          )}
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
    </PageFrame>
  );
}

function relativeTime(iso: string): string {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "—";
  const minutes = Math.round((Date.now() - then) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.round(hours / 24)}d ago`;
}

/** The publish state: status, category and portal visibility in one row. The
 *  client's visibility is a decision, so it is stated as one and never
 *  inferred from the status alone. */
function PublishRow({
  article,
  categories,
  onSaved,
}: {
  article: KbArticle;
  categories: KbCategory[];
  onSaved: () => void;
}): React.ReactNode {
  const [status, setStatus] = useState<KbArticleStatus>(article.status);
  const [categoryId, setCategoryId] = useState<string>(article.category_id ?? "");
  const [clientVisible, setClientVisible] = useState<boolean>(article.client_visible);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const category = categories.find((c) => c.id === categoryId);
  const gated = clientVisible && category?.client_visible === true;

  const save = useCallback(
    async (next: {
      status?: KbArticleStatus;
      category_id?: string | null;
      client_visible?: boolean;
    }) => {
      setBusy(true);
      setError(null);
      const result = await runEnvelopeMutation<
        components["schemas"]["UpdateArticleRequest"],
        components["schemas"]["ArticleMutationResponse"]
      >({
        describe: "KB article publish state",
        entityId: article.id,
        buildBody: (base) => ({
          idempotency_key: base.idempotency_key,
          ...(base.version !== undefined ? { version: base.version } : {}),
          ...next,
        }),
        optimistic: () => {},
        revert: () => {},
        request: (client, body) =>
          client.PATCH("/api/v1/kb/articles/{id}", {
            params: { path: { id: article.id } },
            body,
          }),
        versions: (response) => {
          rememberVersion(article.id, response.mutation.version);
          return [[article.id, response.mutation.version]];
        },
      });
      setBusy(false);
      if (!result.ok) {
        setError(result.friendly ?? result.message ?? "The change was rejected.");
        return;
      }
      onSaved();
    },
    [article.id, onSaved],
  );

  return (
    <div
      className="grid gap-3 rounded-[12px] p-3"
      style={{ border: "1px solid var(--m-border)", background: "var(--m-surface-2)" }}
      data-testid="kb-publish-row"
    >
      <p className="text-[12px] font-semibold uppercase tracking-[0.06em]" style={{ color: "var(--m-text-secondary)" }}>
        Publish state
      </p>
      <div className="grid gap-3 sm:grid-cols-3">
        <div>
          <Label htmlFor="kb-status">Status</Label>
          <Select
            value={status}
            onValueChange={(v) => {
              const next = v as KbArticleStatus;
              setStatus(next);
              void save({ status: next });
            }}
          >
            <SelectTrigger id="kb-status" data-testid="kb-publish-status">
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
        <div>
          <Label htmlFor="kb-category">Category</Label>
          <Select
            value={categoryId}
            onValueChange={(v) => {
              setCategoryId(v);
              void save({ category_id: v === "" ? null : v });
            }}
          >
            <SelectTrigger id="kb-category" data-testid="kb-publish-category">
              <SelectValue placeholder="Unfiled" />
            </SelectTrigger>
            <SelectContent>
              {categories.map((c) => (
                <SelectItem key={c.id} value={c.id}>
                  {c.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div>
          <Label htmlFor="kb-client-visible">Portal</Label>
          <Select
            value={clientVisible ? "yes" : "no"}
            onValueChange={(v) => {
              const next = v === "yes";
              setClientVisible(next);
              void save({ client_visible: next });
            }}
          >
            <SelectTrigger id="kb-client-visible" data-testid="kb-publish-visibility">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="no">Staff only</SelectItem>
              <SelectItem value="yes">Client-facing</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>
      <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
        {busy
          ? "Saving…"
          : gated
            ? "In the client portal: the article and its category are both client-facing."
            : clientVisible
              ? "Marked client-facing, but the category is staff-only, so the portal cannot show it yet."
              : "Staff only. The portal's own gated search never sees a draft."}
      </p>
      {error !== null ? (
        <p
          role="alert"
          className="inline-flex items-center gap-2 text-[13px]"
          style={{ color: "var(--m-status-due-fg)" }}
        >
          <AlertIcon size={14} />
          {error}
        </p>
      ) : null}
    </div>
  );
}

function NewArticleForm({
  categories,
  onCreated,
}: {
  categories: KbCategory[];
  onCreated: (article: KbArticle) => void;
}): React.ReactNode {
  const [title, setTitle] = useState("");
  const [slug, setSlug] = useState("");
  const [categoryId, setCategoryId] = useState("");
  const [clientVisible, setClientVisible] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    if (title.trim() === "") {
      setError("Give the article a title before saving.");
      return;
    }
    setBusy(true);
    setError(null);
    let createdId = "";
    const result = await runEnvelopeMutation<
      components["schemas"]["CreateArticleRequest"],
      components["schemas"]["ArticleMutationResponse"]
    >({
      describe: "Create KB article",
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        title: title.trim(),
        slug: slug.trim() === "" ? slugify(title) : slugify(slug),
        body: "",
        status: "draft",
        category_id: categoryId === "" ? null : categoryId,
        client_visible: clientVisible,
      }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body) => client.POST("/api/v1/kb/articles", { body }),
      versions: (response) => {
        createdId = response.id;
        rememberVersion(response.id, response.mutation.version);
        return [[response.id, response.mutation.version]];
      },
    });
    setBusy(false);
    if (!result.ok) {
      setError(result.friendly ?? result.message ?? "The article was not created.");
      return;
    }
    if (createdId === "") {
      setError("The article was created but its id came back empty — reopen the list and find it.");
      return;
    }
    onCreated({
      id: createdId,
      title: title.trim(),
      slug: slug.trim() === "" ? slugify(title) : slugify(slug),
      body: "",
      status: "draft",
      category_id: categoryId === "" ? null : categoryId,
      client_visible: clientVisible,
      content_jsonb: null,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    } as KbArticle);
  };

  return (
    <div className="grid gap-3 p-4">
      <div>
        <Label htmlFor="kb-new-title">Title</Label>
        <Input
          id="kb-new-title"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          data-testid="kb-new-title"
        />
      </div>
      <div>
        <Label htmlFor="kb-new-slug">Slug</Label>
        <Input
          id="kb-new-slug"
          value={slug}
          placeholder={title.trim() === "" ? "reset-printer-on-fleet-mfp" : slugify(title)}
          onChange={(e) => setSlug(e.target.value)}
        />
        <FieldHint>Leave empty and the title becomes the slug.</FieldHint>
      </div>
      <div>
        <Label htmlFor="kb-new-category">Category</Label>
        <Select value={categoryId} onValueChange={setCategoryId}>
          <SelectTrigger id="kb-new-category">
            <SelectValue placeholder="Unfiled" />
          </SelectTrigger>
          <SelectContent>
            {categories.map((c) => (
              <SelectItem key={c.id} value={c.id}>
                {c.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <div>
        <Label htmlFor="kb-new-visible">Portal</Label>
        <Select value={clientVisible ? "yes" : "no"} onValueChange={(v) => setClientVisible(v === "yes")}>
          <SelectTrigger id="kb-new-visible">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="no">Staff only</SelectItem>
            <SelectItem value="yes">Client-facing</SelectItem>
          </SelectContent>
        </Select>
      </div>
      <FieldHint>
        New articles start as drafts. Nothing reaches a client until you publish.
      </FieldHint>
      {error !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-due-fg)" }}>
          {error}
        </p>
      ) : null}
      <DialogActions>
        <Button variant="secondary" onClick={submit} disabled={busy} data-testid="kb-create-article">
          Create draft
        </Button>
      </DialogActions>
    </div>
  );
}

function slugify(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

function CategoryRow({
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
      className="flex w-full min-w-0 items-center gap-2 rounded-[10px] px-2 py-1.5 text-left text-[13px] transition-colors duration-100"
      style={{
        background: selected ? "var(--m-accent)" : "transparent",
        color: selected ? "var(--m-on-accent)" : "var(--m-text)",
        fontWeight: selected ? 600 : 400,
      }}
      onClick={onSelect}
      aria-current={selected ? "true" : undefined}
    >
      <span className="truncate">{name}</span>
      {clientVisible ? (
        <span title="Client-facing category" style={{ color: selected ? "var(--m-on-accent)" : "var(--m-status-success-fg)" }}>
          <BookIcon size={14} />
        </span>
      ) : null}
      <span
        className="tabular-nums ml-auto inline-flex h-5 min-w-5 items-center justify-center rounded-full px-1.5 text-[12px]"
        style={{
          background: selected ? "var(--m-surface)" : "var(--m-surface-2)",
          color: selected ? "var(--m-on-accent)" : "var(--m-text-secondary)",
        }}
      >
        {count}
      </span>
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
          {article.client_visible ? (
            <Tag tone={category?.client_visible ? "success" : "warning"}>client-facing</Tag>
          ) : null}
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
          {headings.length > 1 ? (
            <nav
              aria-label="On this page"
              className="rounded-[12px] p-3"
              style={{ border: "1px solid var(--m-border)" }}
            >
              <p
                className="mb-1 text-[11px] font-semibold uppercase tracking-wide"
                style={{ color: "var(--m-text-muted)" }}
              >
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
      setError(result.friendly ?? result.message ?? "The category was not saved.");
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
        <Input
          id="cat-desc"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
        />
      </div>
      <div>
        <Label htmlFor="cat-position">Order</Label>
        <Input
          id="cat-position"
          type="number"
          value={position}
          onChange={(e) => setPosition(e.target.value)}
        />
        <FieldHint>Lower numbers come first in the rail.</FieldHint>
      </div>
      <div>
        <Label htmlFor="cat-visible">Portal</Label>
        <Select
          value={clientVisible ? "yes" : "no"}
          onValueChange={(v) => setClientVisible(v === "yes")}
        >
          <SelectTrigger id="cat-visible">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="no">Staff only</SelectItem>
            <SelectItem value="yes">Client-facing</SelectItem>
          </SelectContent>
        </Select>
        <FieldHint>
          An article reaches the portal only when the article and this category are both
          client-facing.
        </FieldHint>
      </div>
      {error !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-due-fg)" }}>
          {error}
        </p>
      ) : null}
      <DialogActions>
        <Button variant="secondary" onClick={submit} disabled={busy}>
          Save category
        </Button>
      </DialogActions>
    </div>
  );
}
