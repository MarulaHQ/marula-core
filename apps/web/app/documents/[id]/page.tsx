// /documents/[id] — document workspace (docs-platform v1; UX-excellence
// pass, Wave 17 lane 2).
//
// Purpose: the full record for one document laid out as content-first
// main + side rail on wide viewports: current content with
// instant-first editing (every save APPENDS an immutable version), typed
// fields + scope on the handle, the complete version history with
// side-by-side diff between any two versions, restore (which creates a
// NEW version — history is never rewritten), and the related-items panel
// driven by the shared cross-entity picker (never paste-a-UUID).
// Attachments-on-documents: the raw-byte seam (lib/api bypass) holds —
// rows render from the typed list endpoint, uploads POST raw bytes with
// the x-filename contract, and size-cap/permission rejections surface
// verbatim; never fake rows, never a dead button.
"use client";

import { rejectionText } from "../../../lib/error-copy";
import {
  Badge,
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
  SurfaceHeader,
  Table,
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  api,
  downloadDocumentAttachment,
  uploadDocumentAttachment,
  useRequireAuth,
} from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import {
  BlushNote,
  HEAD_CELL_CLASS,
  PageFrame,
} from "../../../components/knowledge/ledger";
import { FieldsEditor, parseFieldInputs } from "../fields-editor";
import { EntityPicker } from "../../../components/entities/entity-picker";
import type { LinkTarget } from "../../../lib/entity-links";

type Document = components["schemas"]["Document"];
type DocumentView = components["schemas"]["DocumentView"];
type DocumentVersion = components["schemas"]["DocumentVersion"];
type DocumentLayout = components["schemas"]["DocumentLayout"];
type Client = components["schemas"]["Client"];
type ResolvedLink = components["schemas"]["ResolvedLink"];
type DocumentAttachmentView = components["schemas"]["DocumentAttachmentView"];

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

/** Document custom fields as a flat record (wire type is a loose object). */
function fieldsOf(d: DocumentView): Record<string, unknown> {
  return (d.fields ?? {}) as Record<string, unknown>;
}

export default function DocumentDetailPage() {
  const auth = useRequireAuth();
  const params = useParams<{ id: string }>();
  const id = params?.id ?? "";

  const [doc, setDoc] = useState<DocumentView | null>(null);
  const [versions, setVersions] = useState<DocumentVersion[] | null>(null);
  const [layouts, setLayouts] = useState<DocumentLayout[] | null>(null);
  const [clients, setClients] = useState<Client[] | null>(null);
  const [links, setLinks] = useState<ResolvedLink[] | null>(null);
  const [attachments, setAttachments] = useState<DocumentAttachmentView[] | null>(null);
  const [attachmentsUnavailable, setAttachmentsUnavailable] = useState(false);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const [attachError, setAttachError] = useState<string | null>(null);
  const [attachBusy, setAttachBusy] = useState(false);
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

  // links (shared picker + legacy typed id fallback is NOT offered —
  // the picker resolves every kind over the real endpoints)
  const [linkPickerOpen, setLinkPickerOpen] = useState(false);
  const [linkError, setLinkError] = useState<string | null>(null);

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

  const loadAttachments = useCallback(async () => {
    const res = await api.api.GET("/api/v1/documents/{id}/attachments", {
      params: { path: { id } },
    });
    if (res.error !== undefined || res.data === undefined) {
      setAttachments(null);
      setAttachmentsUnavailable(true);
      return;
    }
    setAttachments(res.data.attachments);
    setAttachmentsUnavailable(false);
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
    void loadAttachments();
  }, [id, loadVersions, loadLinks, loadAttachments]);

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
      setError(rejectionText(result, "The save was rejected — the document may have changed. Reload and retry."));
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
      setError(rejectionText(result, "The update was rejected."));
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
      setError(rejectionText(result, "The restore was rejected."));
      return;
    }
    setRestoreTarget(null);
    await refresh();
  };

  const addLink = async (target: LinkTarget) => {
    setLinkError(null);
    const { error: e } = await api.api.POST("/api/v1/documents/{id}/links", {
      params: { path: { id } },
      body: { to_kind: target.kind, to_id: target.id },
    });
    if (e !== undefined) {
      setLinkError("Could not add the link — the target must exist and be visible in your scope.");
      return;
    }
    await loadLinks();
  };

  const removeLink = async (linkRowId: string) => {
    await api.api.DELETE("/api/v1/documents/links/{id}", { params: { path: { id: linkRowId } } });
    await loadLinks();
  };

  const uploadAttachment = async (file: File) => {
    setAttachBusy(true);
    setAttachError(null);
    try {
      await uploadDocumentAttachment(id, file);
      await loadAttachments();
    } catch (e) {
      setAttachError(e instanceof Error ? e.message : "The upload was rejected.");
    } finally {
      setAttachBusy(false);
    }
  };

  const deleteAttachment = async (attachmentId: string) => {
    setAttachError(null);
    const res = await api.api.DELETE(
      "/api/v1/documents/{id}/attachments/{attachment_id}",
      { params: { path: { id, attachment_id: attachmentId } } },
    );
    if (res.error !== undefined) {
      setAttachError(
        res.response.status === 404
          ? "That attachment is no longer on this document."
          : "Only the uploader or an org owner/admin can remove an attachment.",
      );
      return;
    }
    await loadAttachments();
  };

  const downloadAttachmentRow = async (attachment: DocumentAttachmentView) => {
    setAttachError(null);
    try {
      await downloadDocumentAttachment(id, attachment.id, attachment.filename);
    } catch (e) {
      setAttachError(e instanceof Error ? e.message : "Download failed.");
    }
  };

  if (auth.status !== "authed") return null;

  if (loadError !== null) {
    return (
      <PageFrame>
        <SurfaceHeader title="Document" />
        <div className="mb-2">
          <BlushNote title="This document did not load.">{loadError}</BlushNote>
        </div>
      </PageFrame>
    );
  }

  if (doc === null || body === null) {
    return (
      <PageFrame>
        <SurfaceHeader title="Document" />
        <Spinner />
      </PageFrame>
    );
  }

  const left = version(diffLeft);
  const right = version(diffRight);

  return (
    <PageFrame>
      <Link
        href="/documents"
        className="mb-1 inline-block text-[13px] underline decoration-[var(--m-border)] underline-offset-2"
        style={{ color: "var(--m-text-secondary)" }}
      >
        ← Documents
      </Link>
      <SurfaceHeader
        title={doc.title}
        description={
          <span className="flex flex-wrap items-center gap-2">
            {doc.client_id != null ? (
              <Link href={`/clients/${doc.client_id}`} className="underline decoration-[var(--m-border)] underline-offset-2">
                {clients?.find((c) => c.id === doc.client_id)?.name ?? "Client"}
              </Link>
            ) : (
              <span>Org-global</span>
            )}
            {layout !== null ? <Badge tone="neutral">{layout.name}</Badge> : null}
            <span className="tabular-nums">v{doc.current_version}</span>
            <span>
              ·{" "}
              {doc.content_updated_at === null || doc.content_updated_at === undefined
                ? "no content revision yet"
                : `updated ${new Date(doc.content_updated_at).toLocaleString()}`}
            </span>
            {doc.restored_from_version != null ? (
              <Badge tone="warning">restored from v{doc.restored_from_version}</Badge>
            ) : null}
          </span>
        }
        actions={
          <Button variant="secondary" onClick={() => setEditMeta((v) => !v)}>
            {editMeta ? "Close details" : "Edit details"}
          </Button>
        }
      />

      {error !== null ? (
        <div className="mb-2" role="alert">
          <BlushNote title="The last save did not land.">
            {error}{" "}
            <button className="underline underline-offset-2" onClick={() => void refresh()}>
              Reload
            </button>
          </BlushNote>
        </div>
      ) : null}

      <div className="grid grid-cols-1 gap-6 xl:grid-cols-[minmax(0,1fr)_340px]">
        {/* ---- main: typed fields, content, version history ---- */}
        <div className="grid content-start gap-6">
          {layout !== null && Object.keys(doc.fields ?? {}).length > 0 ? (
            <Card className="m-entity-rise" style={{ "--stagger-i": 0 } as React.CSSProperties}>
              <h2 className="p-4 pb-2 text-sm font-semibold">{layout.name} fields</h2>
              <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1 px-4 pb-4 text-[13px]">
                {layout.fields
                  .filter((f) => fieldsOf(doc)[f.key] !== undefined && fieldsOf(doc)[f.key] !== null)
                  .map((f, i) => (
                    <div key={f.key} className="contents">
                      <dt style={{ color: "var(--m-text-secondary)" }}>{f.label}</dt>
                      <dd className="">{String(fieldsOf(doc)[f.key])}</dd>
                    </div>
                  ))}
              </dl>
            </Card>
          ) : null}

          <Card className="m-entity-rise" style={{ "--stagger-i": 1 } as React.CSSProperties}>
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
                className="w-full rounded-[var(--m-radius-container)] border bg-transparent p-2 text-[12px]"
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

          <Card className="m-entity-rise" style={{ "--stagger-i": 2 } as React.CSSProperties}>
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
                <Table data-testid="document-versions">
                  <thead>
                    <tr>
                      <Th className={HEAD_CELL_CLASS}>Version</Th>
                      <Th className={HEAD_CELL_CLASS}>Change</Th>
                      <Th className={HEAD_CELL_CLASS}>Saved</Th>
                      <Th className={HEAD_CELL_CLASS}> </Th>
                    </tr>
                  </thead>
                  <tbody>
                    {versions.map((v) => {
                      const isCurrent = v.version === doc.current_version;
                      return (
                        <Tr key={v.version} style={{ height: "var(--m-density-row)" }}>
                          <Td className="tabular-nums font-semibold">v{v.version}</Td>
                          <Td style={{ color: "var(--m-text-secondary)" }}>
                            {v.change_summary !== "" ? v.change_summary : "no summary"}
                            {v.restored_from_version != null ? ` (restored from v${v.restored_from_version})` : ""}
                          </Td>
                          <Td className="tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                            {new Date(v.created_at).toLocaleString()}
                          </Td>
                          <Td className="text-right">
                            {isCurrent ? (
                              <Badge tone="success">current</Badge>
                            ) : (
                              <Button
                                size="sm"
                                variant="ghost"
                                onClick={() => setRestoreTarget(v)}
                              >
                                Restore
                              </Button>
                            )}
                          </Td>
                        </Tr>
                      );
                    })}
                  </tbody>
                </Table>
                {left !== null && right !== null ? <DiffView left={left} right={right} /> : null}
              </div>
            ) : (
              <div className="px-4 pb-4"><Spinner /></div>
            )}
          </Card>
        </div>

        {/* ---- side rail: details, attachments, related items ---- */}
        <div className="grid content-start gap-6" data-testid="document-rail">
          {editMeta ? (
            <Card>
              <div className="grid gap-3 p-4">
                <h2 className="text-sm font-semibold">Details</h2>
                <div>
                  <Label htmlFor="meta-title">Title</Label>
                  <Input id="meta-title" value={metaTitle} onChange={(e) => setMetaTitle(e.target.value)} />
                </div>
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

          <Card className="m-entity-rise" style={{ "--stagger-i": 3 } as React.CSSProperties}>
            <h2 className="p-4 pb-1 text-sm font-semibold">Attachments</h2>
            {attachError !== null ? (
              <p role="alert" className="px-4 pb-2 text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {attachError}
              </p>
            ) : null}
            {attachmentsUnavailable ? (
              <div className="p-4 pt-0">
                <EmptyState message="Attachments are not available for this document in your scope." />
              </div>
            ) : attachments === null ? (
              <div className="px-4 pb-4"><Spinner /></div>
            ) : (
              <div className="grid gap-2 px-4 pb-4">
                {attachments.length === 0 ? (
                  <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                    No files on this document yet. Uploads are content-addressed and integrity-checked.
                  </p>
                ) : (
                  <ul className="grid gap-2">
                    {attachments.map((a) => (
                      <li
                        key={a.id}
                        className="m-entity-row flex items-center justify-between gap-3 rounded-[0.5rem] border px-3 py-2 text-[13px]"
                        style={{ borderColor: "var(--m-border)" }}
                      >
                        <span className="min-w-0">
                          <span className="block truncate font-medium">{a.filename}</span>
                          <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                            {a.content_type} · {Number.isFinite(a.size_bytes) ? formatSize(a.size_bytes) : "—"}
                            {a.sha256 ? ` · ${a.sha256.slice(0, 12)}…` : ""}
                          </span>
                        </span>
                        <span className="flex shrink-0 items-center gap-1">
                          <Button
                            variant="ghost"
                            size="sm"
                            aria-label={`Download ${a.filename}`}
                            onClick={() => void downloadAttachmentRow(a)}
                          >
                            Download
                          </Button>
                          <Button
                            variant="ghost"
                            size="sm"
                            aria-label={`Remove ${a.filename}`}
                            data-testid={`document-attachment-delete-${a.id}`}
                            onClick={() => void deleteAttachment(a.id)}
                          >
                            ✕
                          </Button>
                        </span>
                      </li>
                    ))}
                  </ul>
                )}
                <div className="flex items-center gap-2">
                  <Button
                    variant="ghost"
                    onClick={() => fileInputRef.current?.click()}
                    disabled={attachBusy}
                  >
                    Add attachment
                  </Button>
                  <input
                    type="file"
                    ref={fileInputRef}
                    className="sr-only"
                    aria-label="Upload attachment"
                    data-testid="document-attach-upload"
                    disabled={attachBusy}
                    onChange={(e) => {
                      const f = e.target.files?.[0];
                      e.target.value = "";
                      if (f !== undefined) void uploadAttachment(f);
                    }}
                  />
                </div>
              </div>
            )}
          </Card>

          <Card className="m-entity-rise" style={{ "--stagger-i": 4 } as React.CSSProperties}>
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
                    className="m-entity-row flex items-center justify-between gap-2 rounded-[0.5rem] border px-3 py-2 text-[13px]"
                    style={{ borderColor: "var(--m-border)" }}
                  >
                    <span className="min-w-0">
                      <Badge tone="neutral">{l.to_kind.replace("_", " ")}</Badge>{" "}
                      {l.target_deleted ? (
                        <span style={{ color: "var(--m-status-danger-fg)" }}>deleted target</span>
                      ) : (
                        <span className="truncate">{l.target_label ?? "target"}</span>
                      )}
                    </span>
                    <Button variant="ghost" aria-label="Remove link" onClick={() => void removeLink(l.id)}>
                      ✕
                    </Button>
                  </div>
                ))
              )}
              <div className="mt-1 grid gap-2">
                <Button
                  variant="secondary"
                  onClick={() => setLinkPickerOpen(true)}
                  aria-label="Add related item"
                >
                  Add link
                </Button>
              </div>
              {linkError !== null ? (
                <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                  {linkError}
                </p>
              ) : null}
            </div>
          </Card>
        </div>
      </div>

      <EntityPicker
        open={linkPickerOpen}
        clientId={doc.client_id ?? undefined}
        onPick={(t) => void addLink(t)}
        onClose={() => setLinkPickerOpen(false)}
      />

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
    </PageFrame>
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
        <pre className="max-h-96 overflow-auto rounded-[0.5rem] border p-2 text-[11px]" style={{ borderColor: "var(--m-border)" }}>
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
        <pre className="max-h-96 overflow-auto rounded-[0.5rem] border p-2 text-[11px]" style={{ borderColor: "var(--m-border)" }}>
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
