// /documents — staff document library (docs-platform v1; UX-excellence
// pass, Wave 17 lane 2).
//
// Purpose: the org's documentation library across ALL scopes —
// org-global and per-client documents — laid out like a real library:
// a stat strip (total / org-global / client-scoped / typed), a toolbar
// (title search + scope + type filters), and a dense table with type
// chips, scope links, versions, and hover micro-feedback. Typed
// layout management (create/edit the custom schemas) stays here. Motion
// rides transform/opacity ≤120ms with staggered entrances and
// prefers-reduced-motion collapse.
//
// Non-purpose: version history, diff, restore, attachments, and
// relationship links live on /documents/[id].
"use client";

import { rejectionText } from "../../lib/error-copy";
import {
  Button,
  ColumnChooser,
  DataToolbar,
  DensitySwitch,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  ExportControl,
  FieldHint,
  FilterBar,
  Input,
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
import {
  BlushNote,
  HEAD_CELL_CLASS,
  KeyHint,
  LedgerFooter,
  PageFrame,
  PageHeader,
  PlusIcon,
  SkeletonRows,
  StatPill,
  StatusPill,
  TableCard,
  downloadCsv,
} from "../../components/knowledge/ledger";
import { FieldsEditor, parseFieldInputs } from "./fields-editor";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";
import { runEnvelopeMutation } from "../../lib/envelope-write";

type Document = components["schemas"]["Document"];
type DocumentLayout = components["schemas"]["DocumentLayout"];
type LayoutField = components["schemas"]["LayoutField"];
type Client = components["schemas"]["Client"];
type LayoutMutationResponse = components["schemas"]["LayoutMutationResponse"];

type Scope = { kind: "all" } | { kind: "global" } | { kind: "client"; id: string };

type DocumentColumnKey = "title" | "type" | "scope" | "version" | "updated";

interface DocumentColumn {
  key: DocumentColumnKey;
  label: string;
  visible: boolean;
  required?: boolean;
}

/** The pack's ledger columns; `title` is the identity and stays first. */
const DOCUMENT_COLUMNS: DocumentColumn[] = [
  { key: "title", label: "Title", visible: true, required: true },
  { key: "type", label: "Type", visible: true },
  { key: "scope", label: "Scope", visible: true },
  { key: "version", label: "Version", visible: true },
  { key: "updated", label: "Updated", visible: true },
];

export default function DocumentsPage() {
  const auth = useRequireAuth();
  const [docs, setDocs] = useState<Document[] | null>(null);
  const [layouts, setLayouts] = useState<DocumentLayout[] | null>(null);
  const [clients, setClients] = useState<Client[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [scope, setScope] = useState<Scope>({ kind: "all" });
  const [q, setQ] = useState("");
  const [layoutFilter, setLayoutFilter] = useState<string>("all");

  const [createOpen, setCreateOpen] = useState(false);
  const [layoutsOpen, setLayoutsOpen] = useState(false);
  const [clauses, setClauses] = useState<{ id: string; field: string; op: never; value?: string }[]>(
    [],
  );
  const [columns, setColumns] = useState<DocumentColumn[]>(DOCUMENT_COLUMNS);
  const density = useDensity();

  const layoutById = useMemo(
    () => new Map((layouts ?? []).map((l) => [l.id, l])),
    [layouts],
  );
  const clientById = useMemo(
    () => new Map((clients ?? []).map((c) => [c.id, c])),
    [clients],
  );

  const refresh = useCallback(async () => {
    const [d, l, c] = await Promise.all([
      api.api.GET("/api/v1/documents"),
      api.api.GET("/api/v1/documents/layouts"),
      api.api.GET("/api/v1/clients"),
    ]);
    if (d.error !== undefined || l.error !== undefined || c.error !== undefined) {
      setLoadError("Failed to load documents.");
      return;
    }
    setDocs(d.data ?? []);
    setLayouts(l.data ?? []);
    setClients(c.data ?? []);
    setLoadError(null);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  const filtered = useMemo(() => {
    if (docs === null) return null;
    const needle = q.trim().toLowerCase();
    return docs.filter((d) => {
      if (scope.kind === "global" && d.client_id !== null) return false;
      if (scope.kind === "client" && d.client_id !== scope.id) return false;
      // "typed"/"untyped" are views, not a type id, so they are matched here
      // rather than against layout_id.
      if (layoutFilter === "typed" && d.layout_id == null) return false;
      if (layoutFilter === "untyped" && d.layout_id != null) return false;
      if (layoutFilter !== "all" && layoutFilter !== "typed" && layoutFilter !== "untyped" && d.layout_id !== layoutFilter) {
        return false;
      }
      if (needle !== "" && !d.title.toLowerCase().includes(needle)) return false;
      for (const clause of clauses) {
        const value = (clause.value ?? "").toLowerCase();
        if (clause.field === "title" && !d.title.toLowerCase().includes(value)) return false;
        if (clause.field === "type") {
          const name = d.layout_id != null ? (layoutById.get(d.layout_id)?.name ?? "") : "";
          if (!name.toLowerCase().includes(value)) return false;
        }
        if (clause.field === "scope") {
          const name = d.client_id != null ? (clientById.get(d.client_id)?.name ?? "") : "org-global";
          if (!name.toLowerCase().includes(value)) return false;
        }
      }
      return true;
    });
  }, [docs, scope, q, layoutFilter, clauses, layoutById, clientById]);

  const shownColumns = columns.filter((c) => c.visible);

  const stats = useMemo(() => {
    if (docs === null) return null;
    const layoutIds = new Set((layouts ?? []).map((l) => l.id));
    const global = docs.filter((d) => d.client_id == null).length;
    const typed = docs.filter((d) => d.layout_id != null && layoutIds.has(d.layout_id)).length;
    return { total: docs.length, global, scoped: docs.length - global, typed };
  }, [docs, layouts]);

  if (auth.status !== "authed") return null;

  return (
    <PageFrame>
      <PageHeader
        title="Documents"
        subtitle="Org-global or per client · versioned"
        attention={
          <>
            <StatPill tone="neutral" label="Org-global" count={stats?.global ?? 0} onClick={() => setScope((s) => (s.kind === "global" ? { kind: "all" } : { kind: "global" }))} pressed={scope.kind === "global"} />
            <StatPill tone="neutral" label="Client-scoped" count={stats?.scoped ?? 0} onClick={() => setScope((s) => (s.kind === "all" ? { kind: "global" } : { kind: "all" }))} pressed={scope.kind === "all"} />
            <StatPill tone="gold" label="Typed" count={stats?.typed ?? 0} />
          </>
        }
        actions={
          <>
            <Button variant="secondary" onClick={() => setLayoutsOpen(true)} data-testid="documents-manage-types">
              Manage types
            </Button>
            <Dialog open={createOpen} onOpenChange={setCreateOpen}>
              <DialogTrigger asChild>
                <Button data-testid="documents-new">
                  <PlusIcon size={16} /> New document
                </Button>
              </DialogTrigger>
              <DialogContent
                title="New document"
                description="Pick a scope and, optionally, a typed layout. Custom fields are validated against the layout's schema."
              >
                <CreateDocumentForm
                  layouts={layouts ?? []}
                  clients={clients ?? []}
                  onDone={() => {
                    setCreateOpen(false);
                    void refresh();
                  }}
                />
              </DialogContent>
            </Dialog>
          </>
        }
      />

      <div className="flex min-h-0 flex-1 flex-col">
        <DataToolbar
          label="Document controls"
          views={
            <ViewSwitch
              label="Document views"
              views={[
                { id: "all", label: "All", hint: String(filtered?.length ?? 0) },
                { id: "global", label: "Org-global" },
                { id: "typed", label: "Typed" },
                { id: "untyped", label: "Untyped" },
              ]}
              activeId={layoutFilter === "all" ? "all" : layoutFilter === "untyped" ? "untyped" : scope.kind === "global" ? "global" : "typed"}
              onChange={(id) => {
                if (id === "all") {
                  setLayoutFilter("all");
                  setScope({ kind: "all" });
                }
                if (id === "global") {
                  setScope({ kind: "global" });
                  setLayoutFilter("all");
                }
                if (id === "typed") setLayoutFilter("typed");
                if (id === "untyped") setLayoutFilter("untyped");
              }}
            />
          }
          filters={
            <FilterBar
              label="Document filters"
              emptyHint="No filters"
              fields={[
                { key: "title", label: "Title", operators: ["contains"] },
                { key: "type", label: "Type", operators: ["contains"] },
                { key: "scope", label: "Scope", operators: ["contains"] },
              ]}
              clauses={clauses}
              onChange={(next) => setClauses(next.map((c) => ({ ...c, op: c.op as never })))}
            />
          }
          columns={
            <ColumnChooser
              label="Document columns"
              columns={columns.map((c) => ({ key: c.key, label: c.label, visible: c.visible, required: c.required }))}
              onChange={(next) =>
                setColumns(
                  next.map((c) => ({
                    key: c.key as DocumentColumnKey,
                    label: c.label,
                    visible: c.visible,
                    required: c.required,
                  })),
                )
              }
            />
          }
          export={
            <ExportControl
              label="Export"
              formats={["csv"]}
              columns={shownColumns.map((c) => c.label)}
              rowCount={filtered?.length ?? 0}
              filters={clauses}
              onExport={() => {
                downloadCsv(
                  "documents.csv",
                  shownColumns.map((c) => c.label),
                  (filtered ?? []).map((d) =>
                    shownColumns.map((c) => {
                      if (c.key === "title") return d.title;
                      if (c.key === "type")
                        return d.layout_id != null ? (layoutById.get(d.layout_id)?.name ?? "") : "";
                      if (c.key === "scope")
                        return d.client_id != null ? (clientById.get(d.client_id)?.name ?? "") : "Org-global";
                      if (c.key === "version") return `v${d.current_version}`;
                      return d.updated_at;
                    }),
                  ),
                );
              }}
            />
          }
        />

        <TableCard label="Documents">
          {filtered === null ? (
            loadError !== null ? (
              <div className="p-4">
                <BlushNote title="The list did not load.">{loadError}</BlushNote>
              </div>
            ) : (
              <SkeletonRows sentence="Loading documents." columns={4} />
            )
          ) : filtered.length === 0 ? (
            <EmptyState
              message="No documents match. Documents can be org-global or client-scoped; pick a type to give them structured fields."
              action={
                <Button onClick={() => setCreateOpen(true)}>New document</Button>
              }
            />
          ) : (
            <Table data-testid="documents-table">
              <thead>
                <tr>
                  {shownColumns.map((c) => (
                    <Th key={c.key} className={c.key === "version" ? `${HEAD_CELL_CLASS} text-right` : HEAD_CELL_CLASS}>
                      {c.label}
                    </Th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {filtered.map((d) => (
                  <Tr key={d.id} style={{ height: "var(--m-density-row)" }}>
                    {shownColumns.map((c) => (
                      <Td key={c.key} className={c.key === "version" ? "text-right tabular-nums" : undefined}>
                        {c.key === "title" ? (
                          <Link
                            href={`/documents/${d.id}`}
                            className="font-semibold underline decoration-transparent underline-offset-2 transition-colors duration-100 hover:decoration-[var(--m-border-strong)]"
                            style={{ color: "var(--m-text)" }}
                          >
                            {d.title}
                          </Link>
                        ) : null}
                        {c.key === "type" ? (
                          d.layout_id != null && layoutById.get(d.layout_id) ? (
                            <StatusPill tone="outline">{layoutById.get(d.layout_id)!.name}</StatusPill>
                          ) : (
                            <span style={{ color: "var(--m-text-muted)" }}>—</span>
                          )
                        ) : null}
                        {c.key === "scope" ? (
                          d.client_id != null && clientById.get(d.client_id) ? (
                            <Link
                              href={`/clients/${d.client_id}`}
                              className="underline decoration-[var(--m-border)] underline-offset-2"
                            >
                              {clientById.get(d.client_id)!.name}
                            </Link>
                          ) : (
                            <span style={{ color: "var(--m-text-muted)" }}>Org-global</span>
                          )
                        ) : null}
                        {c.key === "version" ? `v${d.current_version}` : null}
                        {c.key === "updated" ? (
                          <span className="tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                            {new Date(d.updated_at).toLocaleDateString()}
                          </span>
                        ) : null}
                      </Td>
                    ))}
                  </Tr>
                ))}
              </tbody>
            </Table>
          )}
        </TableCard>

        <LedgerFooter
          shown={filtered?.length ?? 0}
          total={stats?.total ?? 0}
          noun="documents"
          density={<DensitySwitch value={density} onChange={applyDensity} />}
          hints={
            <>
              <KeyHint keys="J K" label="move" />
              <KeyHint keys="↵" label="open" />
              <KeyHint keys="F" label="filter" />
            </>
          }
        />
      </div>

      <Dialog open={layoutsOpen} onOpenChange={setLayoutsOpen}>
        <DialogContent
          title="Document types"
          description="The standard MSP set ships with every org and can be edited (not deleted). Custom types are fully yours."
        >
          <LayoutManager
            layouts={layouts ?? []}
            onDone={() => {
              void refresh();
            }}
          />
        </DialogContent>
      </Dialog>
    </PageFrame>
  );
}

function CreateDocumentForm({
  layouts,
  clients,
  onDone,
}: {
  layouts: DocumentLayout[];
  clients: Client[];
  onDone: () => void;
}) {
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [clientId, setClientId] = useState<string>("global");
  const [layoutId, setLayoutId] = useState<string>("untyped");
  const [fields, setFields] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const layout = layouts.find((l) => l.id === layoutId) ?? null;

  const submit = async () => {
    setBusy(true);
    setError(null);
    const parsed = layout ? parseFieldInputs(layout.fields, fields) : { values: {} };
    if ("error" in parsed) {
      setError(parsed.error);
      setBusy(false);
      return;
    }
    const result = await runEnvelopeMutation<
      components["schemas"]["CreateDocumentRequest"],
      components["schemas"]["DocumentMutationResponse"]
    >({
      describe: "Create document",
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        title,
        body,
        client_id: clientId === "global" ? null : clientId,
        layout_id: layoutId === "untyped" ? null : layoutId,
        fields: parsed.values,
        change_summary: "Initial version",
      }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body) => client.POST("/api/v1/documents", { body }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setBusy(false);
    if (!result.ok) {
      setError(rejectionText(result, "The document was rejected."));
      return;
    }
    onDone();
  };

  return (
    <div className="grid gap-3 p-4">
      <div>
        <Label htmlFor="doc-title">Title</Label>
        <Input id="doc-title" value={title} onChange={(e) => setTitle(e.target.value)} />
      </div>
      <div className="grid grid-cols-2 gap-3">
        <div>
          <Label htmlFor="doc-scope">Scope</Label>
          <Select value={clientId} onValueChange={setClientId}>
            <SelectTrigger id="doc-scope">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="global">Org-global</SelectItem>
              {clients.map((c) => (
                <SelectItem key={c.id} value={c.id}>
                  {c.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div>
          <Label htmlFor="doc-layout">Type</Label>
          <Select value={layoutId} onValueChange={setLayoutId}>
            <SelectTrigger id="doc-layout">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="untyped">Untyped</SelectItem>
              {layouts.map((l) => (
                <SelectItem key={l.id} value={l.id}>
                  {l.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>
      {layout !== null ? <FieldsEditor layout={layout} fields={fields} onChange={setFields} /> : null}
      <div>
        <Label htmlFor="doc-body">Body</Label>
        <textarea
          id="doc-body"
          value={body}
          onChange={(e) => setBody(e.target.value)}
          rows={5}
          className="w-full rounded-[var(--m-radius-container)] border bg-transparent p-2 text-[13px]"
          style={{ borderColor: "var(--m-border)" }}
        />
      </div>
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
          {busy ? <Spinner /> : null} Create
        </Button>
      </DialogActions>
    </div>
  );
}

function LayoutManager({ layouts, onDone }: { layouts: DocumentLayout[]; onDone: () => void }) {
  const [editing, setEditing] = useState<DocumentLayout | null>(null);
  const [creating, setCreating] = useState(false);

  if (editing !== null || creating) {
    return (
      <LayoutEditForm
        layout={editing}
        onDone={() => {
          setEditing(null);
          setCreating(false);
          onDone();
        }}
        onCancel={() => {
          setEditing(null);
          setCreating(false);
        }}
      />
    );
  }

  return (
    <div className="grid max-h-[60vh] gap-2 overflow-auto p-4">
      {layouts.map((l) => (
        <div
          key={l.id}
          className="m-entity-row flex items-center justify-between gap-3 rounded-[0.5rem] border p-3"
          style={{ borderColor: "var(--m-border)" }}
        >
          <div className="min-w-0">
            <p className="truncate text-[13px] font-medium">{l.name}</p>
            <p className="truncate text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              {l.description} · {l.fields.length} field{l.fields.length === 1 ? "" : "s"}
              {l.is_system ? " · standard" : ""}
            </p>
          </div>
          <Button variant="ghost" onClick={() => setEditing(l)}>
            Edit
          </Button>
        </div>
      ))}
      <Button
        variant="secondary"
        className="mt-2"
        onClick={() => setCreating(true)}
      >
        New custom type
      </Button>
    </div>
  );
}

function LayoutEditForm({
  layout,
  onDone,
  onCancel,
}: {
  layout: DocumentLayout | null;
  onDone: () => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState(layout?.name ?? "");
  const [description, setDescription] = useState(layout?.description ?? "");
  const [schemaText, setSchemaText] = useState(
    JSON.stringify(layout?.fields ?? [], null, 2),
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Parsed on every keystroke, not on save: the invalid-JSON state is the one
  // this editor has to be honest about, and a reader should meet it while
  // they are still holding the mistake, not after a round trip.
  const schemaState = useMemo(() => {
    if (schemaText.trim() === "") {
      return { ok: false as const, message: "The schema is empty. An array of field objects goes here." };
    }
    try {
      JSON.parse(schemaText);
      return { ok: true as const, message: null };
    } catch (e) {
      const detail = e instanceof Error ? e.message : "the JSON does not parse";
      return {
        ok: false as const,
        message: `That is not valid JSON: ${detail}. Fix the braces and quotes, then save — nothing is sent until it parses.`,
      };
    }
  }, [schemaText]);

  const submit = async () => {
    if (!schemaState.ok) return;
    setBusy(true);
    setError(null);
    let fields: unknown;
    try {
      fields = JSON.parse(schemaText);
    } catch {
      setError("Field schema is not valid JSON.");
      setBusy(false);
      return;
    }
    const result =
      layout === null
        ? await runEnvelopeMutation<
            components["schemas"]["CreateLayoutRequest"],
            LayoutMutationResponse
          >({
            describe: "Create document type",
            buildBody: (base) => ({
              idempotency_key: base.idempotency_key,
              name,
              description,
              fields: fields as LayoutField[],
            }),
            optimistic: () => {},
            revert: () => {},
            request: (client, body) => client.POST("/api/v1/documents/layouts", { body }),
            versions: (response) => [[response.id, response.mutation.version]],
          })
        : await runEnvelopeMutation<
            components["schemas"]["UpdateLayoutRequest"],
            LayoutMutationResponse
          >({
            describe: "Update document type",
            entityId: layout.id,
            buildBody: (base) => ({
              idempotency_key: base.idempotency_key,
              name,
              description,
              fields: fields as LayoutField[],
            }),
            optimistic: () => {},
            revert: () => {},
            request: (client, body) =>
              client.PATCH("/api/v1/documents/layouts/{id}", {
                params: { path: { id: layout.id } },
                body,
              }),
            versions: (response) => [[response.id, response.mutation.version]],
          });
    setBusy(false);
    if (!result.ok) {
      setError(rejectionText(result, "The type was rejected."));
      return;
    }
    onDone();
  };

  return (
    <div className="grid gap-3 p-4">
      <div>
        <Label htmlFor="layout-name">Name</Label>
        <Input id="layout-name" value={name} onChange={(e) => setName(e.target.value)} />
      </div>
      <div>
        <Label htmlFor="layout-desc">Description</Label>
        <Input
          id="layout-desc"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
        />
      </div>
      <div>
        <Label htmlFor="layout-fields">Field schema</Label>
        <FieldHint>
          JSON array of {"{ key, label, type, required?, position?, options? }"}; type is one of
          text, number, date, bool, select, url, email.
        </FieldHint>
        <textarea
          id="layout-fields"
          value={schemaText}
          onChange={(e) => setSchemaText(e.target.value)}
          rows={10}
          aria-invalid={!schemaState.ok}
          aria-describedby={!schemaState.ok ? "layout-schema-problem" : undefined}
          className="mt-1 w-full rounded-[var(--m-radius-container)] border bg-transparent p-2 font-mono text-[12px]"
          style={{ borderColor: "var(--m-border)" }}
        />
        {schemaState.ok ? null : (
          <div id="layout-schema-problem" className="mt-2" data-testid="documents-invalid-schema">
            <BlushNote title="The schema does not parse.">{schemaState.message}</BlushNote>
          </div>
        )}
      </div>
      {error !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-due-fg)" }}>
          {error}
        </p>
      ) : null}
      <DialogActions>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button
          onClick={() => void submit()}
          disabled={busy || name.trim() === "" || !schemaState.ok}
          data-testid="documents-layout-save"
        >
          {busy ? <Spinner /> : null} {layout === null ? "Create type" : "Save changes"}
        </Button>
      </DialogActions>
    </div>
  );
}
