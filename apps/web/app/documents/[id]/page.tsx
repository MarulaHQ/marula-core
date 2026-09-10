// /documents/[id] — document workspace (docs-platform v1).
//
// Purpose: the full record for one document: current content with
// instant-first editing (every save APPENDS an immutable version), typed
// fields + scope on the handle, the complete version history with
// side-by-side diff between any two versions, restore (which creates a
// NEW version — history is never rewritten), and the related-items panel
// (cross-entity links).
"use client";

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
import { useParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { FieldsEditor, parseFieldInputs } from "../fields-editor";

type Document = components["schemas"]["Document"];
type DocumentView = components["schemas"]["DocumentView"];
type DocumentVersion = components["schemas"]["DocumentVersion"];
type DocumentLayout = components["schemas"]["DocumentLayout"];
type Client = components["schemas"]["Client"];
type ResolvedLink = components["schemas"]["ResolvedLink"];

/** Document custom fields as a flat record (wire type is a loose object). */
function fieldsOf(d: DocumentView): Record<string, unknown> {
  return (d.fields ?? {}) as Record<string, unknown>;
}

const LINK_KINDS = [
  { value: "document", label: "Document" },
  { value: "password", label: "Password" },
  { value: "client", label: "Client" },
  { value: "ticket", label: "Ticket" },
  { value: "kb_article", label: "KB article" },
] as const;

export default function DocumentDetailPage() {
  const auth = useRequireAuth();
  const params = useParams<{ id: string }>();
  const id = params?.id ?? "";

  const [doc, setDoc] = useState<DocumentView | null>(null);
  const [versions, setVersions] = useState<DocumentVersion[] | null>(null);
  const [layouts, setLayouts] = useState<DocumentLayout[] | null>(null);
  const [clients, setClients] = useState<Client[] | null>(null);
  const [links, setLinks] = useState<ResolvedLink[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [body, setBody] = useState<string | null>(null);
  const [summary, setSummary] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedNote, setSavedNote] = useState<string | null>(null);

  // meta editing
  const [editMeta, setEditMeta] = useState(false);
  const [metaTitle, setMetaTitle] = useState("");
  const [metaClientId, setMetaClientId] = useState("keep");
  const [metaLayoutId, setMetaLayoutId] = useState("keep");
  const [metaFields, setMetaFields] = useState<Record<string, string>>({});

  // diff / restore
  const [diffLeft, setDiffLeft] = useState<number | null>(null);
  const [diffRight, setDiffRight] = useState<number | null>(null);
  const [restoreTarget, setRestoreTarget] = useState<DocumentVersion | null>(null);

  // links
  const [linkKind, setLinkKind] = useState<string>("ticket");
  const [linkId, setLinkId] = useState("");

  const loadVersions = useCallback(async () => {
    const { data, error } = await api.api.GET("/api/v1/documents/{id}/versions", {
      params: { path: { id } },
    });
    if (error === undefined && data !== undefined) setVersions(data);
  }, [id]);

  const loadLinks = useCallback(async () => {
    const { data } = await api.api.GET("/api/v1/documents/{id}/links", {
      params: { path: { id } },
    });
    if (data !== undefined) setLinks(data);
  }, [id]);

  const refresh = useCallback(async () => {
    const [d, l, c] = await Promise.all([
      api.api.GET("/api/v1/documents/{id}", { params: { path: { id } } }),
      api.api.GET("/api/v1/documents/layouts"),
      api.api.GET("/api/v1/clients"),
    ]);
    if (d.error !== undefined || d.data === undefined) {
      setLoadError("Document not found in your scope.");
      return;
    }
    setDoc(d.data);
    setBody(d.data.body);
    setMetaTitle(d.data.title);
    setMetaClientId(d.data.client_id ?? "global");
    setMetaLayoutId(d.data.layout_id ?? "untyped");
    setLoadError(null);
    if (l.data !== undefined) setLayouts(l.data);
    if (c.data !== undefined) setClients(c.data);
    void loadVersions();
    void loadLinks();
  }, [id, loadVersions, loadLinks]);

  useEffect(() => {
    if (auth.status === "authed" && id !== "") void refresh();
  }, [auth.status, id, refresh]);

  const layout = useMemo(
    () => layouts?.find((l) => l.id === doc?.layout_id) ?? null,
    [layouts, doc],
  );

  const version = useCallback(
    (n: number | null | undefined) => versions?.find((v) => v.version === n) ?? null,
    [versions],
  );

  const saveBody = async () => {
    if (doc === null || body === null) return;
    setBusy(true);
    setError(null);
    setSavedNote(null);
    const result = await runEnvelopeMutation<
      components["schemas"]["UpdateDocumentRequest"],
      components["schemas"]["DocumentMutationResponse"]
    >({
      describe: "Save document version",
      entityId: id,
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        version: base.version,
        body,
        change_summary: summary,
      }),
      optimistic: () => setSavedNote("Saving…"),
      revert: () => setSavedNote(null),
      request: (client, body_) => client.PATCH("/api/v1/documents/{id}", { params: { path: { id } }, body: body_ }),
      versions: (response) => [[id, response.mutation.version]],
    });
    setBusy(false);
    if (!result.ok) {
      setError(result.message ?? "The save was rejected — the document may have changed. Reload and retry.");
      return;
    }
    setSummary("");
    setSavedNote("Saved as a new version.");
    await refresh();
  };

  const saveMeta = async () => {
    if (doc === null) return;
    setBusy(true);
    setError(null);
    const parsed = layout ? parseFieldInputs(layout.fields, metaFields) : { values: {} };
    if ("error" in parsed) {
      setError(parsed.error);
      setBusy(false);
      return;
    }
    const result = await runEnvelopeMutation<
      components["schemas"]["UpdateDocumentRequest"],
      components["schemas"]["DocumentMutationResponse"]
    >({
      describe: "Update document details",
      entityId: id,
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        version: base.version,
        title: metaTitle,
        client_id: metaClientId === "global" ? null : metaClientId,
        layout_id: metaLayoutId === "untyped" ? null : metaLayoutId,
        fields: parsed.values,
      }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body_) => client.PATCH("/api/v1/documents/{id}", { params: { path: { id } }, body: body_ }),
      versions: (response) => [[id, response.mutation.version]],
    });
    setBusy(false);
    if (!result.ok) {
      setError(result.message ?? "The update was rejected.");
      return;
    }
    setEditMeta(false);
    await refresh();
  };

  const restore = async () => {
    if (doc === null || restoreTarget === null) return;
    setBusy(true);
    setError(null);
    const result = await runEnvelopeMutation<
      components["schemas"]["RestoreDocumentRequest"],
      components["schemas"]["DocumentMutationResponse"]
    >({
      describe: "Restore version",
      entityId: id,
      buildBody: (base) => ({
        // NOTE: RestoreDocumentRequest's outer `version` IS the restore
        // target; it doubles as the flattened instant version param, so
        // the if-match gate is omitted (allowed for restore).
        idempotency_key: base.idempotency_key,
        version: restoreTarget.version,
        change_summary: `Restored from v${restoreTarget.version}`,
      }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body_) =>
        client.POST("/api/v1/documents/{id}/restore", { params: { path: { id } }, body: body_ }),
      versions: (response) => [[id, response.mutation.version]],
    });
    setBusy(false);
    if (!result.ok) {
      setError(result.message ?? "The restore was rejected.");
      return;
    }
    setRestoreTarget(null);
    await refresh();
  };

  const addLink = async () => {
    setError(null);
    const { error: e } = await api.api.POST("/api/v1/documents/{id}/links", {
      params: { path: { id } },
      body: { to_kind: linkKind, to_id: linkId },
    });
    if (e !== undefined) {
      setError("Could not add the link — check the target id exists.");
      return;
    }
    setLinkId("");
    await loadLinks();
  };

  const removeLink = async (linkId: string) => {
    await api.api.DELETE("/api/v1/documents/links/{id}", { params: { path: { id: linkId } } });
    await loadLinks();
  };

  if (auth.status !== "authed") return null;

  if (loadError !== null) {
    return (
      <div className="mx-auto w-full max-w-[1400px] p-4 md:p-6">
        <Card>
          <div className="p-4">
            <EmptyState message={loadError} />
          </div>
        </Card>
      </div>
    );
  }

  if (doc === null || body === null) {
    return (
      <div className="mx-auto w-full max-w-[1400px] p-4 md:p-6">
        <Spinner />
      </div>
    );
  }

  const left = version(diffLeft);
  const right = version(diffRight);

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-6 p-4 md:p-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h1 className="text-lg font-semibold">{doc.title}</h1>
          <p className="mt-1 flex flex-wrap items-center gap-2 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            {doc.client_id != null ? (
              <Link href={`/clients/${doc.client_id}`} className="underline decoration-[var(--m-border)] underline-offset-2">
                {clients?.find((c) => c.id === doc.client_id)?.name ?? "Client"}
              </Link>
            ) : (
              <span>Org-global</span>
            )}
            {layout !== null ? <Tag>{layout.name}</Tag> : null}
            <span>v{doc.current_version}</span>
            <span>· updated {new Date(doc.content_updated_at).toLocaleString()}</span>
            {doc.restored_from_version != null ? (
              <Tag>restored from v{doc.restored_from_version}</Tag>
            ) : null}
          </p>
        </div>
        <Button variant="secondary" onClick={() => setEditMeta((v) => !v)}>
          {editMeta ? "Close details" : "Edit details"}
        </Button>
      </div>

      {error !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}{" "}
          <button className="underline" onClick={() => void refresh()}>
            Reload
          </button>
        </p>
      ) : null}

      {editMeta ? (
        <Card>
          <div className="grid gap-3 p-4">
            <div>
              <Label htmlFor="meta-title">Title</Label>
              <Input id="meta-title" value={metaTitle} onChange={(e) => setMetaTitle(e.target.value)} />
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <Label htmlFor="meta-scope">Scope</Label>
                <Select value={metaClientId} onValueChange={setMetaClientId}>
                  <SelectTrigger id="meta-scope">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="global">Org-global</SelectItem>
                    {(clients ?? []).map((c) => (
                      <SelectItem key={c.id} value={c.id}>
                        {c.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div>
                <Label htmlFor="meta-layout">Type</Label>
                <Select value={metaLayoutId} onValueChange={setMetaLayoutId}>
                  <SelectTrigger id="meta-layout">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
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
            {layout !== null ? (
              <TypedFields
                layoutId={layout.id}
                layouts={layouts ?? []}
                doc={doc}
                fields={metaFields}
                onChange={setMetaFields}
              />
            ) : null}
            <DialogActions>
              <Button variant="ghost" onClick={() => setEditMeta(false)}>
                Cancel
              </Button>
              <Button onClick={() => void saveMeta()} disabled={busy || metaTitle.trim() === ""}>
                {busy ? <Spinner /> : null} Save details
              </Button>
            </DialogActions>
          </div>
        </Card>
      ) : null}

      <div className="grid gap-6 lg:grid-cols-[1fr_320px]">
        <div className="grid gap-6">
          {layout !== null && Object.keys(doc.fields ?? {}).length > 0 ? (
            <Card>
              <h2 className="p-4 pb-2 text-sm font-semibold">{layout.name} fields</h2>
              <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1 px-4 pb-4 text-[13px]">
                {layout.fields
                  .filter((f) => fieldsOf(doc)[f.key] !== undefined && fieldsOf(doc)[f.key] !== null)
                  .map((f) => (
                    <div key={f.key} className="contents">
                      <dt style={{ color: "var(--m-text-secondary)" }}>{f.label}</dt>
                      <dd>{String(fieldsOf(doc)[f.key])}</dd>
                    </div>
                  ))}
              </dl>
            </Card>
          ) : null}

          <Card>
            <div className="flex items-center justify-between p-4 pb-2">
              <h2 className="text-sm font-semibold">Content</h2>
              {savedNote !== null ? (
                <span role="status" className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                  {savedNote}
                </span>
              ) : null}
            </div>
            <div className="px-4 pb-2">
              <textarea
                aria-label="Document body"
                value={body}
                onChange={(e) => setBody(e.target.value)}
                rows={16}
                className="w-full rounded-[2px] border bg-transparent p-2 font-mono text-[12px]"
                style={{ borderColor: "var(--m-border)" }}
              />
            </div>
            <div className="flex flex-wrap items-center gap-2 px-4 pb-4">
              <Input
                aria-label="Change summary"
                placeholder="What changed? (optional)"
                value={summary}
                onChange={(e) => setSummary(e.target.value)}
                className="w-72"
              />
              <Button onClick={() => void saveBody()} disabled={busy}>
                {busy ? <Spinner /> : null} Save as new version
              </Button>
            </div>
          </Card>

          <Card>
            <h2 className="p-4 pb-2 text-sm font-semibold">Version history</h2>
            <p className="px-4 pb-2 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              Immutable and append-only. Restoring never rewrites history — it appends a new version
              with the old content.
            </p>
            {versions !== null ? (
              <div className="px-4 pb-4">
                <div className="mb-2 flex flex-wrap items-center gap-2 text-[12px]">
                  <span style={{ color: "var(--m-text-secondary)" }}>Compare</span>
                  <Select value={diffLeft === null ? "" : String(diffLeft)} onValueChange={(v) => setDiffLeft(Number(v))}>
                    <SelectTrigger className="w-28" aria-label="Diff left version"><SelectValue placeholder="from" /></SelectTrigger>
                    <SelectContent>
                      {versions.map((v) => (
                        <SelectItem key={v.version} value={String(v.version)}>v{v.version}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <span style={{ color: "var(--m-text-secondary)" }}>→</span>
                  <Select value={diffRight === null ? "" : String(diffRight)} onValueChange={(v) => setDiffRight(Number(v))}>
                    <SelectTrigger className="w-28" aria-label="Diff right version"><SelectValue placeholder="to" /></SelectTrigger>
                    <SelectContent>
                      {versions.map((v) => (
                        <SelectItem key={v.version} value={String(v.version)}>v{v.version}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  {(left !== null || right !== null) && (
                    <Button
                      variant="ghost"
                      onClick={() => {
                        setDiffLeft(null);
                        setDiffRight(null);
                      }}
                    >
                      Clear
                    </Button>
                  )}
                </div>
                <ul className="grid gap-1">
                  {versions.map((v) => (
                    <li
                      key={v.version}
                      className="flex items-center justify-between gap-3 rounded-[2px] border px-3 py-2 text-[13px]"
                      style={{ borderColor: "var(--m-border)" }}
                    >
                      <div className="min-w-0">
                        <span className="font-medium">v{v.version}</span>
                        <span className="ml-2" style={{ color: "var(--m-text-secondary)" }}>
                          {v.change_summary !== "" ? v.change_summary : "no summary"}
                          {v.restored_from_version != null ? ` (restored from v${v.restored_from_version})` : ""}
                        </span>
                        <span className="ml-2" style={{ color: "var(--m-text-muted)" }}>
                          {new Date(v.created_at).toLocaleString()}
                        </span>
                      </div>
                      <Button
                        variant="ghost"
                        onClick={() => setRestoreTarget(v)}
                        disabled={v.version === doc.current_version}
                      >
                        {v.version === doc.current_version ? "current" : "Restore"}
                      </Button>
                    </li>
                  ))}
                </ul>
                {left !== null && right !== null ? <DiffView left={left} right={right} /> : null}
              </div>
            ) : (
              <div className="px-4 pb-4"><Spinner /></div>
            )}
          </Card>
        </div>

        <Card>
          <h2 className="p-4 pb-2 text-sm font-semibold">Related items</h2>
          <div className="grid gap-2 px-4 pb-4">
            {links === null ? (
              <Spinner />
            ) : links.length === 0 ? (
              <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                Link tickets, other documents, vault entries, clients, or KB articles here.
              </p>
            ) : (
              links.map((l) => (
                <div
                  key={l.id}
                  className="flex items-center justify-between gap-2 rounded-[2px] border px-3 py-2 text-[13px]"
                  style={{ borderColor: "var(--m-border)" }}
                >
                  <span className="min-w-0">
                    <Tag>{l.to_kind.replace("_", " ")}</Tag>{" "}
                    {l.target_deleted ? (
                      <span style={{ color: "var(--m-status-danger-fg)" }}>deleted target</span>
                    ) : (
                      <span className="truncate">{l.target_label ?? "target"}</span>
                    )}
                  </span>
                  <Button variant="ghost" onClick={() => void removeLink(l.id)}>
                    ✕
                  </Button>
                </div>
              ))
            )}
            <div className="mt-1 grid gap-2">
              <Select value={linkKind} onValueChange={setLinkKind}>
                <SelectTrigger aria-label="Link target kind"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {LINK_KINDS.map((k) => (
                    <SelectItem key={k.value} value={k.value}>
                      {k.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <Input
                aria-label="Link target id"
                placeholder="Target record id…"
                value={linkId}
                onChange={(e) => setLinkId(e.target.value)}
              />
              <Button variant="secondary" onClick={() => void addLink()} disabled={linkId.trim() === ""}>
                Add link
              </Button>
            </div>
          </div>
        </Card>
      </div>

      <Dialog open={restoreTarget !== null} onOpenChange={(o) => !o && setRestoreTarget(null)}>
        <DialogContent
          title={`Restore v${restoreTarget?.version ?? ""}?`}
          description="This appends a NEW version with this version's content. Nothing in the history is rewritten."
        >
          <div className="p-4"><DialogActions>
            <Button variant="ghost" onClick={() => setRestoreTarget(null)}>
              Cancel
            </Button>
            <Button onClick={() => void restore()} disabled={busy}>
              {busy ? <Spinner /> : null} Restore
            </Button>
          </DialogActions></div>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function TypedFields({
  layoutId,
  layouts,
  doc,
  fields,
  onChange,
}: {
  layoutId: string;
  layouts: DocumentLayout[];
  doc: DocumentView;
  fields: Record<string, string>;
  onChange: (next: Record<string, string>) => void;
}) {
  const target = layouts.find((l) => l.id === layoutId) ?? null;
  const [effective, setEffective] = useState<Record<string, string> | null>(null);
  useEffect(() => {
    if (target === null) {
      setEffective(null);
      return;
    }
    // Prefill from the current document fields when the editor is empty.
    const pre: Record<string, string> = { ...fields };
    for (const f of target.fields) {
      if (pre[f.key] === undefined) {
        const v = fieldsOf(doc)[f.key];
        pre[f.key] = v === undefined || v === null ? "" : String(v);
      }
    }
    setEffective(pre);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target?.id]);
  if (target === null || effective === null) return null;
  return <FieldsEditor layout={target} fields={effective} onChange={setEffective} />;
}

function DiffView({ left, right }: { left: DocumentVersion; right: DocumentVersion }) {
  const rows = useMemo(() => diffLines(left.body ?? "", right.body ?? ""), [left, right]);
  return (
    <div className="mt-3 grid grid-cols-2 gap-2">
      <div>
        <p className="mb-1 text-[12px] font-medium">v{left.version} — {left.title}</p>
        <pre className="max-h-96 overflow-auto rounded-[2px] border p-2 font-mono text-[11px]" style={{ borderColor: "var(--m-border)" }}>
          {rows.map((r) => (
            <div key={r.n} style={{ background: r.kind === "removed" ? "var(--m-status-danger-bg, transparent)" : undefined }}>
              {r.kind === "removed" ? "− " : "  "}
              {r.text}
            </div>
          ))}
        </pre>
      </div>
      <div>
        <p className="mb-1 text-[12px] font-medium">v{right.version} — {right.title}</p>
        <pre className="max-h-96 overflow-auto rounded-[2px] border p-2 font-mono text-[11px]" style={{ borderColor: "var(--m-border)" }}>
          {rows.map((r) => (
            <div key={r.n} style={{ background: r.kind === "added" ? "var(--m-status-success-bg, transparent)" : undefined }}>
              {r.kind === "added" ? "+ " : "  "}
              {r.text}
            </div>
          ))}
        </pre>
      </div>
    </div>
  );
}

type DiffRow = { n: number; kind: "same" | "added" | "removed"; text: string };

/** Line-based LCS diff — small bodies make this cheap client-side. */
function diffLines(a: string, b: string): DiffRow[] {
  const al = a.split("\n");
  const bl = b.split("\n");
  const n = al.length;
  const m = bl.length;
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i]![j] = al[i] === bl[j] ? dp[i + 1]![j + 1]! + 1 : Math.max(dp[i + 1]![j]!, dp[i]![j + 1]!);
    }
  }
  const rows: DiffRow[] = [];
  let i = 0;
  let j = 0;
  let k = 0;
  while (i < n && j < m) {
    const a = al[i]!;
    const b = bl[j]!;
    if (a === b) {
      rows.push({ n: k++, kind: "same", text: a });
      i++;
      j++;
    } else if (dp[i + 1]![j]! >= dp[i]![j + 1]!) {
      rows.push({ n: k++, kind: "removed", text: a });
      i++;
    } else {
      rows.push({ n: k++, kind: "added", text: b });
      j++;
    }
  }
  while (i < n) rows.push({ n: k++, kind: "removed", text: al[i++]! });
  while (j < m) rows.push({ n: k++, kind: "added", text: bl[j++]! });
  return rows;
}
