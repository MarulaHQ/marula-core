// /documents — staff document browser (docs-platform v1, Wave 11 lane 7).
//
// Purpose: the org's documentation library across ALL scopes — org-global
// documents and per-client documents — with typed layouts (the standard
// MSP set plus org-custom ones), scope and layout filters, and instant-
// first document creation that validates custom fields against the chosen
// layout. Layout management (create/edit the custom schemas) lives here.
//
// Non-purpose: version history, diff, restore, and relationship links
// live on /documents/[id].
"use client";

import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTrigger,
  EmptyState,
  FieldHint,
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
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
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
      if (layoutFilter !== "all" && d.layout_id !== layoutFilter) return false;
      if (needle !== "" && !d.title.toLowerCase().includes(needle)) return false;
      return true;
    });
  }, [docs, scope, q, layoutFilter]);

  if (auth.status !== "authed") return null;

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-8 p-4 md:p-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold">Documents</h1>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Client and org-wide documentation: typed records, runbooks, and credentials-adjacent
            references with full version history.
          </p>
        </div>
        <div className="flex gap-2">
          <Button variant="secondary" onClick={() => setLayoutsOpen(true)}>
            Manage types
          </Button>
          <Dialog open={createOpen} onOpenChange={setCreateOpen}>
            <DialogTrigger asChild>
              <Button>New document</Button>
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
        </div>
      </div>

      <Card>
        <div className="flex flex-wrap items-center justify-between gap-3 p-4 pb-0">
          <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
            <Input
              aria-label="Search documents"
              placeholder="Search by title…"
              value={q}
              onChange={(e) => setQ(e.target.value)}
              className="w-56"
            />
            <Select
              value={scope.kind === "client" ? scope.id : scope.kind}
              onValueChange={(v) =>
                setScope(
                  v === "all"
                    ? { kind: "all" }
                    : v === "global"
                      ? { kind: "global" }
                      : { kind: "client", id: v },
                )
              }
            >
              <SelectTrigger aria-label="Filter by scope" className="w-52 whitespace-nowrap">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All scopes</SelectItem>
                <SelectItem value="global">Org-global</SelectItem>
                {(clients ?? []).map((c) => (
                  <SelectItem key={c.id} value={c.id}>
                    {c.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Select value={layoutFilter} onValueChange={setLayoutFilter}>
              <SelectTrigger aria-label="Filter by type" className="w-44 whitespace-nowrap">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All types</SelectItem>
                <SelectItem value="untyped">Untyped</SelectItem>
                {(layouts ?? []).map((l) => (
                  <SelectItem key={l.id} value={l.id}>
                    {l.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
        {filtered === null ? (
          <div className="p-4">
            {loadError !== null ? (
              <p role="alert" className="text-[13px]">
                {loadError}
              </p>
            ) : (
              <Spinner />
            )}
          </div>
        ) : filtered.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No documents match. Documents can be org-global or client-scoped; pick a type to give them structured fields." />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Title</Th>
                <Th>Type</Th>
                <Th>Scope</Th>
                <Th className="text-right">Version</Th>
                <Th>Updated</Th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((d) => (
                <Tr key={d.id}>
                  <Td>
                    <Link
                      href={`/documents/${d.id}`}
                      className="underline decoration-[var(--m-border)] underline-offset-2"
                    >
                      {d.title}
                    </Link>
                  </Td>
                  <Td>
                    {d.layout_id != null && layoutById.get(d.layout_id) ? (
                      <Tag>{layoutById.get(d.layout_id)!.name}</Tag>
                    ) : (
                      <span style={{ color: "var(--m-text-muted)" }}>—</span>
                    )}
                  </Td>
                  <Td>
                    {d.client_id != null && clientById.get(d.client_id) ? (
                      <Link
                        href={`/clients/${d.client_id}`}
                        className="underline decoration-[var(--m-border)] underline-offset-2"
                      >
                        {clientById.get(d.client_id)!.name}
                      </Link>
                    ) : (
                      <span style={{ color: "var(--m-text-muted)" }}>Org-global</span>
                    )}
                  </Td>
                  <Td className="text-right tabular-nums">v{d.current_version}</Td>
                  <Td>{new Date(d.updated_at).toLocaleString()}</Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </Card>

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
    </div>
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
      setError(result.message ?? "The document was rejected.");
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
          className="w-full rounded-[2px] border bg-transparent p-2 text-[13px]"
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
          className="flex items-center justify-between gap-3 rounded-[2px] border p-3"
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

  const submit = async () => {
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
      setError(result.message ?? "The type was rejected.");
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
          className="mt-1 w-full rounded-[2px] border bg-transparent p-2 font-mono text-[12px]"
          style={{ borderColor: "var(--m-border)" }}
        />
      </div>
      {error !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </p>
      ) : null}
      <DialogActions>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button onClick={() => void submit()} disabled={busy || name.trim() === ""}>
          {busy ? <Spinner /> : null} {layout === null ? "Create type" : "Save changes"}
        </Button>
      </DialogActions>
    </div>
  );
}
