// /settings/taxonomy — ticket type & category tree editors (Wave 11 lane 3
// backend, migration 0048).
//
// Purpose: full org CRUD over the classification trees — create / rename /
// re-sort / reparent (subcategories between roots) / soft-retire — for both
// the flat type list and the two-level category tree. These trees are
// CLASSIFICATION-ONLY: nothing binds behavior to them (SLA, workflow,
// automations, billing and reports bind to the semantic status categories),
// and retirement is soft, so existing tickets keep their reference. The
// trees feed pickers and are covered by the U5 customization manifest.
// Mutations ride the instant-first envelope (optimistic patch + authoritative
// reconcile) and are owner/admin (`special:manage_boards`) actions; the list
// itself is readable by anyone with ticket view. Duplicate sibling names are
// typed 409s rendered inline.
// Non-purpose: no ticket assignment here (classification is set on the
// ticket) and no status/board editing (that lives on /settings/workflow).
"use client";

import {
  Button,
  Card,
  Checkbox,
  Dialog,
  DialogActions,
  DialogContent,
  FieldHint,
  Input,
  Label,
  Spinner,
  Tag,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { SaveBar, SettingsHeader } from "../../../components/settings/settings-shell";

type TaxonomyNodeView = components["schemas"]["TaxonomyNodeView"];
type TaxonomyMutationResponse = components["schemas"]["TaxonomyMutationResponse"];

const MANAGE_BOARDS = "special:manage_boards";

/** Stable sort for one taxonomy level: explicit order, then name. */
function sortNodes(a: TaxonomyNodeView, b: TaxonomyNodeView): number {
  return a.sort_order - b.sort_order || a.name.localeCompare(b.name);
}

function matches(a: TaxonomyNodeView, term: string): boolean {
  if (term.trim() === "") return true;
  const t = term.trim().toLowerCase();
  return (
    a.name.toLowerCase().includes(t) ||
    (a.description ?? "").toLowerCase().includes(t)
  );
}

/** Human copy for one typed rejection from the taxonomy surface. */
function rejectMessage(message: string | undefined): string {
  if (message === undefined) return "The change was rejected.";
  if (message.includes("already exists")) return "A node with that name already exists at this level.";
  return message;
}

interface NodeDraft {
  name: string;
  description: string;
  sortOrder: string;
}

const EMPTY_DRAFT: NodeDraft = { name: "", description: "", sortOrder: "0" };

export default function TaxonomySettingsPage() {
  const auth = useRequireAuth();
  const canManage = (auth.me?.permissions ?? []).includes(MANAGE_BOARDS) ?? false;

  const [types, setTypes] = useState<TaxonomyNodeView[] | null>(null);
  const [categories, setCategories] = useState<TaxonomyNodeView[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [showRetired, setShowRetired] = useState(true);
  const [search, setSearch] = useState("");

  const refresh = useCallback(async () => {
    const [t, c] = await Promise.all([
      api.api.GET("/api/v1/taxonomy/types", { params: { query: { include_retired: true } } }),
      api.api.GET("/api/v1/taxonomy/categories", { params: { query: { include_retired: true } } }),
    ]);
    if (t.error !== undefined || c.error !== undefined || t.data === undefined || c.data === undefined) {
      setLoadError("The classification trees could not be loaded.");
      return;
    }
    setLoadError(null);
    setTypes(t.data);
    setCategories(c.data);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  // --- form + confirmation state --------------------------------------------
  const [formOpen, setFormOpen] = useState(false);
  const [formKind, setFormKind] = useState<"type" | "category">("type");
  const [editing, setEditing] = useState<TaxonomyNodeView | null>(null);
  const [formParentId, setFormParentId] = useState<string | null>(null);
  const [draft, setDraft] = useState<NodeDraft>(EMPTY_DRAFT);
  const [formBusy, setFormBusy] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  const [reparenting, setReparenting] = useState<TaxonomyNodeView | null>(null);
  const [reparentTo, setReparentTo] = useState("");
  const [reparentBusy, setReparentBusy] = useState(false);
  const [reparentError, setReparentError] = useState<string | null>(null);

  const [retiring, setRetiring] = useState<{ kind: "type" | "category"; node: TaxonomyNodeView } | null>(
    null,
  );
  const [retireBusy, setRetireBusy] = useState(false);
  const [retireError, setRetireError] = useState<string | null>(null);

  /** Every root category, reused by the form's parent picker + reparenting. */
  const roots = useMemo(
    () => (categories ?? []).filter((c) => c.parent_id == null).sort(sortNodes),
    [categories],
  );

  const openCreate = (kind: "type" | "category", parentId: string | null) => {
    setFormKind(kind);
    setEditing(null);
    setFormParentId(parentId);
    setDraft(EMPTY_DRAFT);
    setFormError(null);
    setFormOpen(true);
  };

  const openEdit = (kind: "type" | "category", node: TaxonomyNodeView) => {
    setFormKind(kind);
    setEditing(node);
    setFormParentId(node.parent_id ?? null);
    setDraft({
      name: node.name,
      description: node.description ?? "",
      sortOrder: String(node.sort_order),
    });
    setFormError(null);
    setFormOpen(true);
  };

  const save = async () => {
    if (draft.name.trim() === "") return;
    setFormBusy(true);
    setFormError(null);
    const sortOrder = Number.isFinite(Number(draft.sortOrder)) ? Number(draft.sortOrder) : 0;
    const description = draft.description.trim() === "" ? null : draft.description.trim();
    const path = formKind === "type" ? "/api/v1/taxonomy/types" : "/api/v1/taxonomy/categories";

    if (editing === null) {
      const result = await runEnvelopeMutation<
        components["schemas"]["CreateTaxonomyNodeRequest"],
        TaxonomyMutationResponse
      >({
        describe: `Create ${formKind}`,
        optimistic: () => undefined,
        revert: () => undefined,
        buildBody: (base) => ({
          idempotency_key: base.idempotency_key,
          name: draft.name.trim(),
          description,
          sort_order: sortOrder,
          ...(formKind === "category" && formParentId !== null ? { parent_id: formParentId } : {}),
        }),
        request: (client, body) => client.POST(path as "/api/v1/taxonomy/types", { body }),
      });
      setFormBusy(false);
      if (!result.ok) {
        setFormError(rejectMessage(result.message));
        return;
      }
    } else {
      const node = editing;
      const before = formKind === "type" ? (types ?? []) : (categories ?? []);
      const apply = (list: TaxonomyNodeView[]): TaxonomyNodeView[] =>
        list.map((n) =>
          n.id === node.id
            ? { ...n, name: draft.name.trim(), description, sort_order: sortOrder }
            : n,
        );
      const restore = (list: TaxonomyNodeView[]): void => {
        if (formKind === "type") setTypes(list);
        else setCategories(list);
      };
      const result = await runEnvelopeMutation<
        components["schemas"]["UpdateTaxonomyNodeRequest"],
        TaxonomyMutationResponse
      >({
        describe: `Update ${formKind}`,
        entityId: node.id,
        optimistic: () => restore(apply(before)),
        revert: () => restore(before),
        buildBody: (base) => ({
          idempotency_key: base.idempotency_key,
          name: draft.name.trim(),
          description,
          sort_order: sortOrder,
        }),
        request: (client, body) =>
          formKind === "type"
            ? client.PATCH("/api/v1/taxonomy/types/{id}", { params: { path: { id: node.id } }, body })
            : client.PATCH("/api/v1/taxonomy/categories/{id}", {
                params: { path: { id: node.id } },
                body,
              }),
        versions: (response) => [[response.id, response.mutation.version]],
      });
      setFormBusy(false);
      if (!result.ok) {
        setFormError(rejectMessage(result.message));
        return;
      }
    }
    setFormOpen(false);
    void refresh();
  };

  const saveReparent = async () => {
    if (reparenting === null || reparentTo === "") return;
    const node = reparenting;
    const before = categories ?? [];
    setReparentBusy(true);
    setReparentError(null);
    const result = await runEnvelopeMutation<
      components["schemas"]["ReparentCategoryRequest"],
      TaxonomyMutationResponse
    >({
      describe: "Reparent category",
      entityId: node.id,
      optimistic: () =>
        setCategories((prev) => (prev ?? []).map((c) => (c.id === node.id ? { ...c, parent_id: reparentTo } : c))),
      revert: () => setCategories(before),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key, parent_id: reparentTo }),
      request: (client, body) =>
        client.POST("/api/v1/taxonomy/categories/{id}/reparent", {
          params: { path: { id: node.id } },
          body,
        }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setReparentBusy(false);
    if (!result.ok) {
      setReparentError(rejectMessage(result.message));
      return;
    }
    setNotice(`"${node.name}" moved under "${roots.find((r) => r.id === reparentTo)?.name ?? "the new root"}".`);
    setReparenting(null);
    void refresh();
  };

  const confirmRetire = async () => {
    if (retiring === null) return;
    const { kind, node } = retiring;
    const before = kind === "type" ? (types ?? []) : (categories ?? []);
    const setter = kind === "type" ? setTypes : setCategories;
    setRetireBusy(true);
    setRetireError(null);
    const result = await runEnvelopeMutation<
      components["schemas"]["RetireTaxonomyNodeRequest"],
      TaxonomyMutationResponse
    >({
      describe: `Retire ${kind}`,
      entityId: node.id,
      optimistic: () => setter((prev) => (prev ?? []).map((n) => (n.id === node.id ? { ...n, is_retired: true } : n))),
      revert: () => setter(before),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key }),
      request: (client, body) =>
        kind === "type"
          ? client.POST("/api/v1/taxonomy/types/{id}/retire", { params: { path: { id: node.id } }, body })
          : client.POST("/api/v1/taxonomy/categories/{id}/retire", {
              params: { path: { id: node.id } },
              body,
            }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setRetireBusy(false);
    if (!result.ok) {
      setRetireError(rejectMessage(result.message));
      return;
    }
    setNotice(`"${node.name}" retired — existing tickets keep their reference.`);
    setRetiring(null);
    void refresh();
  };

  const visible = useCallback(
    (list: TaxonomyNodeView[]): TaxonomyNodeView[] =>
      list.filter((n) => (showRetired || !n.is_retired) && matches(n, search)).sort(sortNodes),
    [showRetired, search],
  );

  if (auth.status !== "authed") return null;

  const typeRows = visible(types ?? []);
  const rootRows = visible(roots);
  const childrenOf = (rootId: string): TaxonomyNodeView[] =>
    visible((categories ?? []).filter((c) => c.parent_id === rootId));

  return (
    <div className="grid gap-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <SettingsHeader
          title="Taxonomy"
          description="Ticket types and the category / subcategory tree. Classification only — SLA, workflow, automations and billing stay bound to the four semantic status categories, so renaming or retiring a node never changes behavior."
        />
        {canManage ? (
          <div className="flex gap-2">
            <Button variant="secondary" data-testid="new-type" onClick={() => openCreate("type", null)}>
              New type
            </Button>
            <Button variant="secondary" data-testid="new-root-category" onClick={() => openCreate("category", null)}>
              New root category
            </Button>
          </div>
        ) : null}
      </div>

      <div className="flex flex-wrap items-center gap-4">
        <Input
          aria-label="Filter taxonomy"
          placeholder="Filter by name…"
          className="max-w-64"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <label className="flex items-center gap-2 text-[13px]">
          <Checkbox
            aria-label="Show retired nodes"
            checked={showRetired}
            onCheckedChange={(v) => setShowRetired(v === true)}
          />
          Show retired
        </label>
      </div>

      {loadError !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {loadError}
        </p>
      ) : types === null || categories === null ? (
        <Card>
          <div className="flex justify-center p-6">
            <Spinner />
          </div>
        </Card>
      ) : (
        <>
          {notice !== null ? (
            <div
              role="status"
              data-testid="taxonomy-notice"
              className="rounded-[var(--m-radius-container)] border px-3 py-2 text-[13px]"
              style={{ borderColor: "var(--m-border)", background: "var(--m-surface)", color: "var(--m-text-secondary)" }}
            >
              {notice}
            </div>
          ) : null}
          <section className="grid gap-2" aria-labelledby="taxonomy-types-heading">
            <h2 id="taxonomy-types-heading" className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Ticket types
            </h2>
            <Card>
              {typeRows.length === 0 ? (
                <p className="p-4 text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                  {types.length === 0 ? "No ticket types yet." : "No types match the filter."}
                </p>
              ) : (
                <ul className="divide-y" style={{ borderColor: "var(--m-border)" }}>
                  {typeRows.map((node) => (
                    <NodeRow
                      key={node.id}
                      node={node}
                      canManage={canManage}
                      onEdit={() => openEdit("type", node)}
                      onRetire={() => {
                        setRetiring({ kind: "type", node });
                        setRetireError(null);
                      }}
                    />
                  ))}
                </ul>
              )}
            </Card>
          </section>

          <section className="grid gap-2" aria-labelledby="taxonomy-categories-heading">
            <h2 id="taxonomy-categories-heading" className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Categories
            </h2>
            <Card>
              {rootRows.length === 0 ? (
                <p className="p-4 text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                  {roots.length === 0 ? "No categories yet." : "No categories match the filter."}
                </p>
              ) : (
                <ul className="grid">
                  {rootRows.map((root) => {
                    const children = childrenOf(root.id);
                    return (
                      <li key={root.id} className="border-b last:border-b-0" style={{ borderColor: "var(--m-border)" }}>
                        <NodeRow
                          node={root}
                          canManage={canManage}
                          strong
                          onCreateChild={
                            canManage
                              ? () => openCreate("category", root.id)
                              : undefined
                          }
                          onEdit={() => openEdit("category", root)}
                          onRetire={() => {
                            setRetiring({ kind: "category", node: root });
                            setRetireError(null);
                          }}
                        />
                        {children.length > 0 ? (
                          <ul className="ml-5 border-l" style={{ borderColor: "var(--m-border)" }}>
                            {children.map((child) => (
                              <li key={child.id} className="pl-3">
                                <NodeRow
                                  node={child}
                                  canManage={canManage}
                                  onEdit={() => openEdit("category", child)}
                                  onReparent={() => {
                                    setReparenting(child);
                                    setReparentTo(child.parent_id ?? "");
                                    setReparentError(null);
                                  }}
                                  onRetire={() => {
                                    setRetiring({ kind: "category", node: child });
                                    setRetireError(null);
                                  }}
                                />
                              </li>
                            ))}
                          </ul>
                        ) : null}
                      </li>
                    );
                  })}
                </ul>
              )}
            </Card>
          </section>

          {!canManage ? (
            <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              Editing the taxonomy is an owner/admin (boards) action; you are viewing it read-only.
            </p>
          ) : null}
        </>
      )}

      {/* create / rename dialog */}
      <Dialog
        open={formOpen}
        onOpenChange={(o) => {
          setFormOpen(o);
          if (!o) setFormError(null);
        }}
      >
        <DialogContent
          title={
            editing === null
              ? formKind === "type"
                ? "New ticket type"
                : formParentId === null
                  ? "New root category"
                  : "New subcategory"
              : formKind === "type"
                ? "Edit ticket type"
                : "Edit category"
          }
          description={
            editing === null && formKind === "category" && formParentId !== null
              ? `Added under ${roots.find((r) => r.id === formParentId)?.name ?? "the selected root"}.`
              : "The trees are exactly two levels deep; subcategories never have children."
          }
        >
          <div className="grid gap-3 p-4">
            <div>
              <Label htmlFor="tax-name">Name</Label>
              <Input
                id="tax-name"
                value={draft.name}
                maxLength={120}
                onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))}
              />
            </div>
            <div>
              <Label htmlFor="tax-description">Description</Label>
              <Input
                id="tax-description"
                value={draft.description}
                onChange={(e) => setDraft((d) => ({ ...d, description: e.target.value }))}
              />
              <FieldHint>Optional — shown alongside the name in pickers.</FieldHint>
            </div>
            <div>
              <Label htmlFor="tax-sort">Sort order</Label>
              <Input
                id="tax-sort"
                type="number"
                value={draft.sortOrder}
                onChange={(e) => setDraft((d) => ({ ...d, sortOrder: e.target.value }))}
              />
              <FieldHint>Lower numbers sort first within a level.</FieldHint>
            </div>
            <DialogActions>
              <Button variant="ghost" onClick={() => setFormOpen(false)}>
                Cancel
              </Button>
              <SaveBar
                dirty={draft.name.trim() !== ""}
                saving={formBusy}
                error={formError}
                onSave={() => void save()}
                saveTestId="save-taxonomy-node"
                saveLabel={editing === null ? "Create" : "Save"}
              />
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>

      {/* reparent dialog */}
      <Dialog
        open={reparenting !== null}
        onOpenChange={(o) => {
          if (!o) {
            setReparenting(null);
            setReparentError(null);
          }
        }}
      >
        <DialogContent
          title="Move subcategory"
          description="Subcategories live directly under a root category; roots are the top level and cannot be reparented."
        >
          <div className="grid gap-3 p-4">
            <div>
              <Label htmlFor="reparent-target">New root category</Label>
              <select
                id="reparent-target"
                aria-label="New root category"
                data-testid="reparent-target"
                value={reparentTo}
                onChange={(e) => setReparentTo(e.target.value)}
                className="w-full text-[13px] px-2 py-1.5"
                style={{
                  borderRadius: "var(--m-radius-control)",
                  border: "1px solid var(--m-border)",
                  background: "var(--m-surface)",
                  color: "var(--m-text)",
                }}
              >
                <option value="" disabled>
                  Choose a root category
                </option>
                {roots
                  .filter((r) => r.id !== reparenting?.parent_id)
                  .map((root) => (
                    <option key={root.id} value={root.id}>
                      {root.name}
                      {root.is_retired ? " (retired)" : ""}
                    </option>
                  ))}
              </select>
            </div>
            <DialogActions>
              <Button variant="ghost" onClick={() => setReparenting(null)}>
                Cancel
              </Button>
              <SaveBar
                dirty={reparentTo !== ""}
                saving={reparentBusy}
                error={reparentError}
                onSave={() => void saveReparent()}
                saveTestId="confirm-reparent"
                saveLabel="Move"
              />
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>

      {/* retire confirmation */}
      <Dialog
        open={retiring !== null}
        onOpenChange={(o) => {
          if (!o) {
            setRetiring(null);
            setRetireError(null);
          }
        }}
      >
        <DialogContent
          title="Retire node"
          description="Retiring is soft: pickers hide the node, but existing tickets keep their reference. There is no un-retire — the node is exported by the customization manifest as retired."
        >
          <div className="grid gap-3 p-4">
            <p className="text-[13px]">
              Retire{" "}
              <strong>{retiring?.node.name}</strong>
              {retiring?.kind === "category" && retiring.node.parent_id != null ? " (subcategory)" : ""}?
            </p>
            {retireError !== null ? (
              <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {retireError}
              </p>
            ) : null}
            <DialogActions>
              <Button variant="ghost" onClick={() => setRetiring(null)}>
                Cancel
              </Button>
              <Button
                variant="primary"
                data-testid="confirm-retire"
                onClick={() => void confirmRetire()}
                disabled={retireBusy}
              >
                {retireBusy ? <Spinner /> : null} Retire
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}

/** One row in the type list / category tree with its permitted actions. */
function NodeRow({
  node,
  canManage,
  strong,
  onCreateChild,
  onEdit,
  onReparent,
  onRetire,
}: {
  node: TaxonomyNodeView;
  canManage: boolean;
  strong?: boolean;
  onCreateChild?: () => void;
  onEdit(): void;
  onReparent?: () => void;
  onRetire(): void;
}): ReactNode {
  return (
    <div className="flex flex-wrap items-center gap-2 px-4 py-2">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className={`truncate text-[13px] ${strong === true ? "font-semibold" : ""}`}>{node.name}</span>
          {node.is_retired ? <Tag tone="neutral">Retired</Tag> : null}
        </div>
        {node.description != null && node.description !== "" ? (
          <p className="truncate text-[12px]" style={{ color: "var(--m-text-muted)" }}>
            {node.description}
          </p>
        ) : null}
      </div>
      {canManage ? (
        <div className="flex shrink-0 flex-wrap gap-1">
          {onCreateChild !== undefined ? (
            <Button variant="ghost" size="sm" onClick={onCreateChild}>
              Add subcategory
            </Button>
          ) : null}
          <Button variant="ghost" size="sm" onClick={onEdit}>
            Rename
          </Button>
          {onReparent !== undefined ? (
            <Button variant="ghost" size="sm" onClick={onReparent}>
              Move
            </Button>
          ) : null}
          {!node.is_retired ? (
            <Button variant="ghost" size="sm" onClick={onRetire}>
              Retire
            </Button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
