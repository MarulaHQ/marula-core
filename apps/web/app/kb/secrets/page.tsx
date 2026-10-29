// /kb/secrets — the secret-category vault (ADR 0006 wiring, Wave 16 lane 6).
//
// Purpose: the KB → client → secret category tree browsing surface for the
// /api/v1/secrets/* backend (migration 0078). The org vault and each
// client's vault ride the same category-tree model behind the scope
// toggle (org OR exactly one client — the backend's exactly-one-of scope
// invariant). Seeded default folders are immutable ADR-0004 template
// rows: org-scope seeds offer "Fork" (fork-the-seed into a client's
// scope) rather than edit-in-place. Restricted category chains
// (`restricted` + deny-by-default `secret_category_grants`) are hidden
// entirely by the backend — nothing dead or disabled renders in their
// place; per-record 403s render a typed "not available" state instead.
//
// Non-purpose: no note-composer changes (ADR 0007 scope boundary) and no
// /passwords page changes (the shipped vault stays byte-compatible; this
// is the generalizing surface beside it). Plaintext exists ONLY inside
// the audited reveal dialog (`secret.revealed`) and is never persisted in
// local state beyond it. Record reads/mutations render optimistic
// (instant-first patterns) with typed-rejection rollback; the
// server-side mutation envelope for secrets is a pinned landing-record
// follow-up, so these writes ride fresh idempotency keys with typed
// rejection copy rather than the versioned envelope.
"use client";

import {
  Badge,
  Button,
  Card,
  ColumnChooser,
  ContextColumn,
  DataToolbar,
  DensitySwitch,
  Dialog,
  DialogActions,
  DialogContent,
  EmptyState,
  ExportControl,
  FieldHint,
  FilterChips,
  FilterTrigger,
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
  ViewSwitch,
  applyDensity,
  useDensity,
  SurfaceFooter,
} from "@marula-core/ui";
import { LocalTime } from "../../../components/local-time";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import {
  AlertIcon,
  HEAD_CELL_CLASS,
  LockIcon,
  NoMatches,
  PageFrame,
  PlusIcon,
  SkeletonRows,
  StatPill,
  StatusPill,
  TableCard,
  downloadCsv,
} from "../../../components/knowledge/ledger";
import { api, useAuth, useRequireAuth } from "../../../lib/api";
import { friendlyMutationError } from "../../../lib/error-copy";
import { newIdempotencyKey } from "../../../lib/envelope-write";

type SecretFolder = components["schemas"]["SecretFolder"];
type SecretRecord = components["schemas"]["SecretRecord"];
type SecretReveal = components["schemas"]["SecretReveal"];
type ResolvedLink = components["schemas"]["ResolvedLink"];
type SecretGrant = components["schemas"]["SecretCategoryGrant"];
type Client = components["schemas"]["Client"];

const FOLDER_KIND_OPTIONS = [
  { value: "passwords", label: "Passwords" },
  { value: "api_keys", label: "API keys" },
  { value: "certificates", label: "Certificates" },
  { value: "recovery_codes", label: "Recovery codes" },
  { value: "documents", label: "Documents" },
] as const;

const KIND_LABELS: Record<string, string> = {
  password: "password",
  api_key: "API key",
  certificate: "certificate",
  recovery_code: "recovery code",
  document: "document",
  passwords: "passwords",
  api_keys: "API keys",
  certificates: "certificates",
  recovery_codes: "recovery codes",
  documents: "documents",
};

type SecretColumnKey = "record" | "kind" | "fields" | "expires" | "updated";

/** The pack's ledger columns; `record` is the identity and cannot be hidden. */
interface SecretColumn {
  key: SecretColumnKey;
  label: string;
  visible: boolean;
  required?: boolean;
}

const SECRET_COLUMNS: SecretColumn[] = [
  { key: "record", label: "Record", visible: true, required: true },
  { key: "kind", label: "Kind", visible: true },
  { key: "fields", label: "Sealed fields", visible: true },
  { key: "expires", label: "Expires", visible: true },
  { key: "updated", label: "Last rotated", visible: false },
];

/**
 * A record's shape, never its contents: which fields are sealed. This is the
 * one row a ledger can show honestly — the values themselves only exist inside
 * the confirmed, audited reveal.
 */
function sealedFields(record: SecretRecord): string {
  const filled = (record.fields ?? []).filter((f) => f.has_value);
  if (filled.length === 0) return record.object_key !== null ? "encrypted file" : "—";
  const keys = filled.map((f) => f.field_key);
  return record.object_key !== null ? `${keys.join(", ")} + file` : keys.join(", ");
}

const FIELD_LABELS: Record<string, string> = {
  secret: "Secret",
  username: "Username",
  url: "URL",
  notes: "Notes",
};

const LINK_KIND_OPTIONS = [
  { value: "asset", label: "Config item / app" },
  { value: "kb_article", label: "KB article" },
  { value: "ticket", label: "Ticket" },
  { value: "password", label: "Legacy password" },
  { value: "document", label: "Document" },
  { value: "client", label: "Client" },
] as const;

type Scope = { kind: "org" } | { kind: "client"; id: string };

interface TreeNode {
  folder: SecretFolder;
  children: TreeNode[];
}

function buildTree(folders: SecretFolder[]): TreeNode[] {
  const byId = new Map<string, TreeNode>();
  for (const f of folders) byId.set(f.id, { folder: f, children: [] });
  const roots: TreeNode[] = [];
  for (const node of byId.values()) {
    const parent =
      node.folder.parent_id != null ? byId.get(node.folder.parent_id) : undefined;
    if (parent !== undefined && parent.folder.id !== node.folder.id) {
      parent.children.push(node);
    } else {
      roots.push(node);
    }
  }
  const sort = (nodes: TreeNode[]) => {
    nodes.sort((a, b) =>
      a.folder.label.localeCompare(b.folder.label, undefined, { sensitivity: "base" }),
    );
    for (const n of nodes) sort(n.children);
  };
  sort(roots);
  return roots;
}

function slugify(label: string): string {
  return label
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

interface ParsedErr {
  message: string;
  status?: number;
}

function errOf(error: unknown): ParsedErr {
  const out: ParsedErr = { message: "The server rejected the change — retry." };
  if (error !== null && typeof error === "object") {
    const e = error as Record<string, unknown>;
    const inner = (e.error ?? e) as Record<string, unknown>;
    if (typeof inner.message === "string") out.message = inner.message;
    if (typeof e.status === "number") out.status = e.status;
  }
  return out;
}

export default function SecretsVaultPage() {
  const auth = useRequireAuth();
  const { me } = useAuth();
  const canReveal = (me?.permissions ?? []).includes("secrets:edit") ?? false;
  const canManage = (me?.permissions ?? []).includes("special:manage_users") ?? false;

  const [clients, setClients] = useState<Client[] | null>(null);
  const [scope, setScope] = useState<Scope>({ kind: "org" });
  const [folders, setFolders] = useState<SecretFolder[] | null>(null);
  const [foldersError, setFoldersError] = useState<string | null>(null);
  const [selectedFolder, setSelectedFolder] = useState<string | null>(null);
  const [records, setRecords] = useState<SecretRecord[] | null>(null);
  const [openRecord, setOpenRecord] = useState<SecretRecord | null>(null);
  const [folderDialog, setFolderDialog] = useState<"new" | "edit" | "fork" | null>(null);
  const [folderForDialog, setFolderForDialog] = useState<SecretFolder | null>(null);
  const [grantsOpen, setGrantsOpen] = useState(false);
  const [view, setView] = useState("all");
  const [filterOpen, setFilterOpen] = useState(false);
  const [clauses, setClauses] = useState<{ id: string; field: string; op: never; value?: string }[]>(
    [],
  );
  const [columns, setColumns] = useState<SecretColumn[]>(SECRET_COLUMNS);
  const density = useDensity();

  const clientById = useMemo(
    () => new Map((clients ?? []).map((c) => [c.id as string, c])),
    [clients],
  );

  useEffect(() => {
    if (auth.status === "authed") {
      api.api.GET("/api/v1/clients").then(({ data }) => {
        if (data !== undefined) setClients([...data]);
      });
    }
  }, [auth.status]);

  useEffect(() => {
    if (auth.status !== "authed") return;
    setFoldersError(null);
    const { kind, client_id } = scopeOf(scope);
    api.api
      .GET("/api/v1/secrets/folders", { params: { query: client_id !== null ? { client_id } : undefined } })
      .then(({ data, error }) => {
        if (error !== undefined || data === undefined) {
          setFoldersError("Failed to load the secret category trees — retry.");
          setFolders([]);
          return;
        }
        setFolders([...data]);
      });
  }, [auth.status, scope]);

  useEffect(() => {
    if (auth.status !== "authed" || selectedFolder === null) return;
    setRecords(null);
    const { kind, client_id } = scopeOf(scope);
    api.api
      .GET("/api/v1/secrets/records", {
        params: {
          query: {
            folder_id: selectedFolder,
            ...(kind === "client" ? { client_id } : {}),
          },
        },
      })
      .then(({ data, error }) => {
        setRecords(error === undefined && data !== undefined ? [...data] : []);
      });
  }, [auth.status, selectedFolder, scope]);

  const selectedFolderRow = useMemo(
    () => (folders ?? []).find((f) => f.id === selectedFolder) ?? null,
    [folders, selectedFolder],
  );

  const reloadRecords = useCallback(() => {
    if (selectedFolder === null) return;
    const { kind, client_id } = scopeOf(scope);
    api.api
      .GET("/api/v1/secrets/records", {
        params: {
          query: {
            folder_id: selectedFolder,
            ...(kind === "client" ? { client_id } : {}),
          },
        },
      })
      .then(({ data }) => {
        if (data !== undefined) setRecords([...data]);
      });
  }, [selectedFolder, scope]);

  /* --- the ledger: a record surface, so it takes the five controls (D12) --- */

  const sortedRecords = useMemo(
    () =>
      (records ?? [])
        .slice()
        .sort((a, b) =>
          a.title.localeCompare(b.title, undefined, { sensitivity: "base" }),
        ),
    [records],
  );

  const expiringSoon = useMemo(() => {
    const soon = Date.now() + 30 * 24 * 60 * 60 * 1000;
    return sortedRecords.filter((r) => {
      if (r.expires_at == null) return false;
      const at = Date.parse(r.expires_at);
      return !Number.isNaN(at) && at >= Date.now() && at <= soon;
    }).length;
  }, [sortedRecords]);

  const expiredCount = useMemo(
    () =>
      sortedRecords.filter((r) => {
        if (r.expires_at == null) return false;
        const at = Date.parse(r.expires_at);
        return !Number.isNaN(at) && at < Date.now();
      }).length,
    [sortedRecords],
  );

  const visibleRecords = useMemo(() => {
    let rows = sortedRecords;
    if (view === "expiring") rows = rows.filter((r) => r.expires_at != null);
    if (view === "expired") {
      rows = rows.filter((r) => {
        if (r.expires_at == null) return false;
        const at = Date.parse(r.expires_at);
        return !Number.isNaN(at) && at < Date.now();
      });
    }
    if (view === "files") rows = rows.filter((r) => r.object_key !== null);
    for (const clause of clauses) {
      const value = (clause.value ?? "").toLowerCase();
      if (clause.field === "title") rows = rows.filter((r) => r.title.toLowerCase().includes(value));
      if (clause.field === "kind") rows = rows.filter((r) => r.kind === clause.value);
    }
    return rows;
  }, [sortedRecords, view, clauses]);

  const shownColumns = columns.filter((c) => c.visible);

  if (auth.status !== "authed") return null;

  const scopeLabel =
    scope.kind === "org" ? "Org-level vault" : `Vault — ${clientById.get(scope.id)?.name ?? "client"}`;

  return (
    <PageFrame>
      <SurfaceHeader
        title="Secret vault"
        description="Org-level or per client · audited reveals"
        meta={
          <>
            <StatPill
              tone="blush"
              label="Expiring in 30 days"
              count={expiringSoon}
              onClick={() => setView(view === "expiring" ? "all" : "expiring")}
              pressed={view === "expiring"}
            />
            <StatPill
              tone="red"
              label="Expired"
              count={expiredCount}
              onClick={() => setView(view === "expired" ? "all" : "expired")}
              pressed={view === "expired"}
            />
          </>
        }
        actions={
          <>
            <Select
              value={scope.kind === "org" ? "org" : scope.id}
              onValueChange={(v) => {
                setScope(v === "org" ? { kind: "org" } : { kind: "client", id: v });
                setSelectedFolder(null);
              }}
            >
              <SelectTrigger aria-label="Vault scope" className="w-60 whitespace-nowrap">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="org">Org-level</SelectItem>
                {(clients ?? []).map((c) => (
                  <SelectItem key={c.id} value={c.id}>
                    {c.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {canManage ? (
              <Button variant="secondary" onClick={() => setGrantsOpen(true)}>
                Category grants
              </Button>
            ) : null}
            <Button
              onClick={() => {
                setFolderForDialog(null);
                setFolderDialog("new");
              }}
              data-testid="kb-secrets-new-category"
            >
              <PlusIcon size={16} /> New category
            </Button>
          </>
        }
      />

      <div className="grid min-h-0 flex-1 gap-6 xl:grid-cols-[280px_minmax(0,1fr)]">
        <div className="min-h-0 overflow-auto">
        <ContextColumn variant="none" label={scopeLabel}>
          {folders === null ? (
            <Spinner />
          ) : foldersError !== null ? (
            <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {foldersError}
            </p>
          ) : (
            <TreeList
              folders={folders}
              selected={selectedFolder}
              onSelect={setSelectedFolder}
              onFork={(f) => {
                setFolderForDialog(f);
                setFolderDialog("fork");
              }}
              onEdit={(f) => {
                setFolderForDialog(f);
                setFolderDialog("edit");
              }}
            />
          )}
        </ContextColumn>
        </div>

        <div className="flex min-h-0 flex-col">
          <DataToolbar
            label="Secret record controls"
            views={
              <ViewSwitch
                label="Secret views"
                views={[
                  { id: "all", label: "All", hint: String(sortedRecords.length) },
                  { id: "expiring", label: "With an expiry" },
                  { id: "expired", label: "Expired" },
                  { id: "files", label: "Files" },
                ]}
                activeId={view}
                onChange={setView}
              />
            }
            chips={
              <FilterChips
                label="Secret filters"
                emptyHint="No filters"
                fields={[
                  { key: "title", label: "Record", operators: ["contains"] },
                  { key: "kind", label: "Kind", operators: ["eq"] },
                ]}
                clauses={clauses}
                onChange={(next) => setClauses(next.map((c) => ({ ...c, op: c.op as never })))}
                onAdd={() => setFilterOpen(true)}
              />
            }
            filter={
              <FilterTrigger
                label="Secret filters"
                fields={[
                  { key: "title", label: "Record", operators: ["contains"] },
                  { key: "kind", label: "Kind", operators: ["eq"] },
                ]}
                clauses={clauses}
                onChange={(next) => setClauses(next.map((c) => ({ ...c, op: c.op as never })))}
                open={filterOpen}
                onOpenChange={setFilterOpen}
              />
            }
            columns={
              <ColumnChooser
                label="Secret columns"
                columns={columns.map((c) => ({
                  key: c.key,
                  label: c.label,
                  visible: c.visible,
                  required: c.required,
                }))}
                onChange={(next) =>
                  setColumns(
                    next.map((c) => ({
                      key: c.key as SecretColumnKey,
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
                rowCount={visibleRecords.length}
                filters={clauses}
                onExport={() => {
                  // Metadata only, never a value: an export of this vault must
                  // not become the plaintext copy it exists to avoid.
                  downloadCsv(
                    "secret-records.csv",
                    shownColumns.map((c) => c.label),
                    visibleRecords.map((r) =>
                      shownColumns.map((c) => {
                        if (c.key === "record") return r.title;
                        if (c.key === "kind") return r.kind;
                        if (c.key === "fields")
                          return (r.fields ?? [])
                            .filter((f) => f.has_value)
                            .map((f) => f.field_key)
                            .join(" ");
                        if (c.key === "expires")
                          return r.expires_at == null ? "" : r.expires_at;
                        return r.updated_at;
                      }),
                    ),
                  );
                }}
              />
            }
          />

          <TableCard label="Secret records">
            {selectedFolderRow !== null ? (
              <div
                className="flex flex-wrap items-center gap-2 border-b px-3 py-2 text-[12px]"
                style={{ color: "var(--m-text-secondary)" }}
              >
                <span className="font-semibold" style={{ color: "var(--m-text)" }}>
                  {selectedFolderRow.label}
                </span>
                <span>{KIND_LABELS[selectedFolderRow.kind] ?? selectedFolderRow.kind}</span>
                {selectedFolderRow.restricted ? (
                  <Badge tone="warning">restricted · reveal needs an edit grant</Badge>
                ) : null}
                {selectedFolderRow.forked_from !== null ? <Badge tone="neutral">forked seed</Badge> : null}
              </div>
            ) : null}

            {selectedFolder === null ? (
              <EmptyState message="Pick a category in the tree to browse its records. Nothing is listed until you do — a vault has no flat 'everything' view on purpose." />
            ) : records === null ? (
              <SkeletonRows sentence="Loading the secret records." columns={4} />
            ) : visibleRecords.length === 0 ? (
              records.length === 0 ? (
                <EmptyState message="No records in this category yet." />
              ) : (
                <NoMatches
                  title="No records match these filters."
                  hint="Try removing one, or switch back to All. Every record in this category is still here."
                  noun="records"
                  onClear={() => {
                    setClauses([]);
                    setView("all");
                  }}
                />
              )
            ) : (
              <Table data-testid="kb-secrets-table">
                <thead>
                  <tr>
                    {shownColumns.map((c) => (
                      <Th key={c.key} className={HEAD_CELL_CLASS}>
                        {c.label}
                      </Th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {visibleRecords.map((r) => {
                    const expired =
                      r.expires_at != null && Date.parse(r.expires_at) < Date.now();
                    return (
                      <Tr key={r.id} style={{ height: "var(--m-density-row)" }}>
                        {shownColumns.map((c) => (
                          <Td key={c.key}>
                            {c.key === "record" ? (
                              <button
                                type="button"
                                className="flex min-w-0 items-center gap-2 text-left"
                                onClick={() => setOpenRecord(r)}
                              >
                                <span style={{ color: "var(--m-text-muted)" }}>
                                  <LockIcon size={16} />
                                </span>
                                <span
                                  className="truncate font-semibold"
                                  style={{ color: "var(--m-text)" }}
                                >
                                  {r.title}
                                </span>
                              </button>
                            ) : null}
                            {c.key === "kind" ? (
                              <StatusPill tone="neutral">{KIND_LABELS[r.kind] ?? r.kind}</StatusPill>
                            ) : null}
                            {c.key === "fields" ? (
                              <span
                                className="font-mono text-[12px]"
                                style={{ color: "var(--m-text-secondary)" }}
                              >
                                {sealedFields(r)}
                              </span>
                            ) : null}
                            {c.key === "expires" ? (
                              expired ? (
                                <StatusPill tone="danger">expired</StatusPill>
                              ) : r.expires_at == null ? (
                                <span style={{ color: "var(--m-text-muted)" }}>never</span>
                              ) : (
                                <span className="tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                                  <LocalTime value={r.expires_at} kind="date" />
                                </span>
                              )
                            ) : null}
                            {c.key === "updated" ? (
                              <span className="tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                                <LocalTime value={r.updated_at} kind="date" />
                              </span>
                            ) : null}
                          </Td>
                        ))}
                      </Tr>
                    );
                  })}
                </tbody>
              </Table>
            )}
          </TableCard>

          <SurfaceFooter
            shown={visibleRecords.length}
            total={records?.length ?? 0}
            noun="records"
            density={<DensitySwitch value={density} onChange={applyDensity} />}
          />
        </div>
      </div>

      <RecordDialog
        record={openRecord}
        folders={folders ?? []}
        scope={scope}
        clientById={clientById}
        canReveal={canReveal}
        onClose={() => setOpenRecord(null)}
        onChanged={() => reloadRecords()}
      />

      <FolderDialog
        mode={folderDialog}
        folder={folderForDialog}
        scope={scope}
        folders={folders ?? []}
        clients={clients ?? []}
        onSuccess={() => {
          setFolderDialog(null);
          api.api
            .GET("/api/v1/secrets/folders", {
              params:
                scope.kind === "client"
                  ? { query: { client_id: scope.id } }
                  : undefined,
            })
            .then(({ data }) => {
              if (data !== undefined) setFolders([...data]);
            });
        }}
        onClose={() => setFolderDialog(null)}
      />

      <GrantsDialog open={grantsOpen} onClose={() => setGrantsOpen(false)} folders={folders ?? []} />
    </PageFrame>
  );
}

function scopeOf(scope: Scope): { kind: "org"; client_id?: undefined } | { kind: "client"; client_id: string } {
  return scope.kind === "org" ? { kind: "org" } : { kind: "client", client_id: scope.id };
}

function TreeList({
  folders,
  selected,
  onSelect,
  onFork,
  onEdit,
}: {
  folders: SecretFolder[];
  selected: string | null;
  onSelect: (id: string) => void;
  onFork: (f: SecretFolder) => void;
  onEdit: (f: SecretFolder) => void;
}) {
  const rows: ReactNode[] = [];
  const push = (nodes: TreeNode[], depth: number) => {
    for (const node of nodes) {
      const f = node.folder;
      rows.push(
        <li
          key={f.id}
          className="group flex items-center justify-between gap-1"
          style={{ paddingLeft: depth * 14 }}
        >
          <button
            className={`min-w-0 flex-1 truncate rounded-[var(--m-radius-container)] px-2 py-1 text-left text-[13px] transition-opacity duration-[120ms] hover:opacity-90 ${selected === f.id ? "font-medium" : ""}`}
            style={{ background: selected === f.id ? "var(--m-bg-subtle, transparent)" : undefined }}
            onClick={() => onSelect(f.id)}
          >
            {f.label}
            {f.is_default_seed ? (
              <span className="ml-2 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                seed
              </span>
            ) : null}
            {f.restricted ? (
              <Badge tone="warning" className="ml-2">
                restricted
              </Badge>
            ) : null}
          </button>
          {f.is_default_seed ? (
            <button
              type="button"
              className="shrink-0 rounded-[8px] px-2 py-1 text-[12px] opacity-0 transition-opacity duration-100 focus-visible:opacity-100 group-hover:opacity-100"
              style={{ color: "var(--m-text-secondary)" }}
              aria-label={`Fork ${f.label} into a client scope`}
              onClick={() => onFork(f)}
            >
              Fork
            </button>
          ) : (
            <button
              type="button"
              className="shrink-0 rounded-[8px] px-2 py-1 text-[12px] opacity-0 transition-opacity duration-100 focus-visible:opacity-100 group-hover:opacity-100"
              style={{ color: "var(--m-text-secondary)" }}
              aria-label={`Edit ${f.label}`}
              onClick={() => onEdit(f)}
            >
              Edit
            </button>
          )}
        </li>,
      );
      push(node.children, depth + 1);
    }
  };
  push(buildTree(folders), 0);
  return (
    <ul className="grid gap-1 px-2 pb-4">
      {rows.length === 0 ? (
        <li className="px-2 py-2 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
          No categories visible — restricted chains you cannot access are hidden entirely, never
          rendered as dead entries.
        </li>
      ) : (
        rows
      )}
    </ul>
  );
}

/* ------------------------------------------------------------------ */
/* Record dialog — the /passwords-class surface, metadata-first.       */
/* ------------------------------------------------------------------ */

function RecordDialog({
  record,
  folders,
  scope,
  clientById,
  canReveal,
  onClose,
  onChanged,
}: {
  record: SecretRecord | null;
  folders: SecretFolder[];
  scope: Scope;
  clientById: Map<string, Client>;
  canReveal: boolean;
  onClose: () => void;
  onChanged: () => void;
}) {
  const [current, setCurrent] = useState<SecretRecord | null>(record);
  const [reveal, setReveal] = useState<SecretReveal | null>(null);
  const [revealError, setRevealError] = useState<string | null>(null);
  const [revealing, setRevealing] = useState(false);
  const [confirmingReveal, setConfirmingReveal] = useState(false);
  const [editing, setEditing] = useState(false);
  const [notAvailable, setNotAvailable] = useState(false);
  const [links, setLinks] = useState<ResolvedLink[] | null>(null);
  const [linksError, setLinksError] = useState<string | null>(null);

  useEffect(() => {
    setCurrent(record);
    setReveal(null);
    setRevealError(null);
    setEditing(false);
    setNotAvailable(false);
    setLinks(null);
    setLinksError(null);
    if (record === null) return;
    void (async () => {
      const { data, error, response } = await api.api.GET("/api/v1/secrets/records/{id}", {
        params: { path: { id: record.id } },
      });
      if (error !== undefined) {
        if (response?.status === 403) setNotAvailable(true);
        return;
      }
      if (data !== undefined) setCurrent(data);
    })();
    void (async () => {
      const { data, error, response } = await api.api.GET("/api/v1/secrets/records/{id}/links", {
        params: { path: { id: record.id } },
      });
      const linksStatus = response.status;
      if (error !== undefined || data === undefined) {
        setLinks([]);
        setLinksError(
          linksStatus === 403
            ? "This record's links are not available with your role — category read access is required."
            : "Failed to load links.",
        );
        return;
      }
      setLinks([...data]);
    })();
  }, [record]);

  const clientName =
    current?.client_id !== undefined && current !== null && current.client_id !== null
      ? (clientById.get(current.client_id)?.name ?? "client")
      : "org-level";

  const doReveal = async () => {
    if (current === null) return;
    setRevealing(true);
    const { data, error } = await api.api.POST("/api/v1/secrets/records/{id}/reveal", {
      params: { path: { id: current.id } },
    });
    setRevealing(false);
    if (error !== undefined || data === undefined) {
      setRevealError(
        "Reveal was rejected. Reveal needs secrets:edit — and, on restricted chains, a category edit grant. The attempt is audited.",
      );
      return;
    }
    setRevealError(null);
    setReveal(data);
  };

  return (
    <Dialog
      open={record !== null && !confirmingReveal}
      onOpenChange={(o) => !o && onClose()}
    >
      <DialogContent
        title={current?.title ?? record?.title ?? "Secret record"}
        description="Metadata-only view — plaintext exists only in the audited reveal below."
      >
        {current === null ? null : notAvailable ? (
          <div className="p-4">
            <EmptyState message="This secret record is not available for your role — restricted categories deny by default until an owner or admin grants your role read access." />
          </div>
        ) : (
          <div className="grid gap-4 p-4">
            <dl className="grid gap-1 text-[13px]">
              <MetaRow label="Category">
                {folders.find((f) => f.id === current.folder_id)?.label ?? "—"}
              </MetaRow>
              <MetaRow label="Kind">{KIND_LABELS[current.kind] ?? current.kind}</MetaRow>
              <MetaRow label="Scope">{clientName}</MetaRow>
              <MetaRow label="Material fields">
                {current.fields?.filter((f) => f.has_value).map((f) => f.field_key.replaceAll("_", " ")).join(", ") ||
                  "none"}
              </MetaRow>
              <MetaRow label="Expires">
                {current.expires_at == null
                  ? "—"
                  : new Date(current.expires_at) < new Date()
                    ? "expired"
                    : <LocalTime value={current.expires_at} kind="date" />}
              </MetaRow>
            </dl>

            {canReveal ? null : (
              <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                Reveal is not available with your role — it requires secrets:edit (plus a
                category edit grant on restricted chains).
              </p>
            )}
            {revealError !== null ? (
              <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {revealError}
              </p>
            ) : null}
            {canReveal ? (
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  variant="secondary"
                  onClick={() => setConfirmingReveal(true)}
                  disabled={revealing}
                  data-testid="kb-secret-reveal-open"
                >
                  <LockIcon size={16} /> Reveal (audited)
                </Button>
                <Button variant="ghost" onClick={() => setEditing(true)}>
                  Edit / rotate fields
                </Button>
              </div>
            ) : null}

            {reveal !== null ? <RevealPanel reveal={reveal} /> : null}

            {editing ? (
              <RecordMetaEditor
                record={current}
                onDone={() => setEditing(false)}
                onChanged={onChanged}
                onUpdated={(next) => setCurrent(next)}
              />
            ) : null}

            {links !== null && linksError === null ? (
              <LinksPanel recordId={current.id} links={links} onChanged={onChanged} />
            ) : linksError !== null ? (
              <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {linksError}
              </p>
            ) : (
              <div className="px-1">
                <Spinner />
              </div>
            )}
          </div>
        )}
      </DialogContent>

      {/* The reveal is a separate confirmation, not the same click. A vault
          hides values by default, and the one gesture that un-hides them has
          to be its own decision — with the consequence stated before the
          plaintext exists anywhere on the page. */}
      <Dialog open={confirmingReveal} onOpenChange={setConfirmingReveal}>
        <DialogContent
          title="Reveal this secret?"
          description="The plaintext is decrypted for this browser only, shown here, and written to the audit trail with your name. It is never cached and never exported."
        >
          <div className="grid gap-3 p-1">
            <dl className="grid gap-1 text-[13px]">
              <MetaRow label="Record">{current?.title ?? "—"}</MetaRow>
              <MetaRow label="Category">
                {current === null
                  ? "—"
                  : (folders.find((f) => f.id === current.folder_id)?.label ?? "—")}
              </MetaRow>
              <MetaRow label="Fields">{sealedFields(current ?? ({ fields: [], object_key: null } as unknown as SecretRecord))}</MetaRow>
              <MetaRow label="Scope">{clientName}</MetaRow>
            </dl>
            <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              Close the dialog once you have what you came for. The next reader of this
              record sees the same masks.
            </p>
            <DialogActions>
              <Button variant="secondary" onClick={() => setConfirmingReveal(false)}>
                Keep it sealed
              </Button>
              <Button
                onClick={() => {
                  setConfirmingReveal(false);
                  void doReveal();
                }}
                disabled={revealing}
                data-testid="kb-secret-reveal-confirm"
              >
                {revealing ? <Spinner /> : null} Reveal and record it
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>
    </Dialog>
  );
}

function MetaRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <dt className="shrink-0" style={{ color: "var(--m-text-secondary)" }}>
        {label}
      </dt>
      <dd className="text-right">{children}</dd>
    </div>
  );
}

function RevealPanel({ reveal }: { reveal: SecretReveal }) {
  return (
    <Card className="border-dashed">
      <h3 className="p-4 pb-1 text-sm font-semibold">Revealed — audited</h3>
      <p className="px-4 pb-2 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
        Recorded in the audit trail. Close the dialog once you've copied what you need — this
        plaintext is not stored anywhere else.
      </p>
      {reveal.fields.map((f) => (
        <div key={f.field_key} className="px-4 pb-2">
          <Label htmlFor={`reveal-${f.field_key}`}>
            {FIELD_LABELS[f.field_key] ?? f.field_key}
          </Label>
          <div className="flex gap-2">
            <Input
              id={`reveal-${f.field_key}`}
              readOnly
              value={f.value}
              type={f.field_key === "secret" ? "password" : "text"}
              className={f.field_key === "secret" ? "font-mono" : undefined}
            />
            <Button
              variant="secondary"
              onClick={() => void navigator.clipboard.writeText(f.value)}
            >
              Copy
            </Button>
          </div>
        </div>
      ))}
    </Card>
  );
}

/* Record metadata + sealed-field rotation/clear (`value: null` clears). */
function RecordMetaEditor({
  record,
  onDone,
  onChanged,
  onUpdated,
}: {
  record: SecretRecord;
  onDone: () => void;
  onChanged: () => void;
  onUpdated: (next: SecretRecord) => void;
}) {
  const [title, setTitle] = useState(record.title);
  const [expiresAt, setExpiresAt] = useState(
    record.expires_at != null ? record.expires_at.slice(0, 10) : "",
  );
  const [values, setValues] = useState<Record<string, string>>({});
  const [clearKeys, setClearKeys] = useState<Record<string, boolean>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const sealedKeys = record.fields?.filter((f) => f.field_key !== "notes") ?? [];

  const submit = async () => {
    setBusy(true);
    setError(null);
    const fields = [
      ...Object.entries(values)
        .filter(([k, v]) => v !== "" && !clearKeys[k] && FIELD_LABELS[k] !== undefined)
        .map(([field_key, value]) => ({ field_key, value })),
      ...Object.keys(clearKeys)
        .filter((k) => clearKeys[k])
        .map((field_key) => ({ field_key, value: null })),
    ];
    const { data, error: e } = await api.api.PATCH("/api/v1/secrets/records/{id}", {
      params: { path: { id: record.id } },
      body: {
        title,
        expires_at: expiresAt === "" ? null : new Date(expiresAt).toISOString(),
        ...(fields.length > 0 ? { fields } : {}),
      } as components["schemas"]["UpdateSecretRecordRequest"],
    });
    setBusy(false);
    if (e !== undefined || data === undefined) {
      const friendly = friendlyMutationError(e);
      setError(friendly.text);
      return;
    }
    setValues({});
    setClearKeys({});
    onUpdated(data);
    onChanged();
    onDone();
  };

  return (
    <Card className="border-dashed">
      <h3 className="p-4 pb-2 text-sm font-semibold">Edit metadata</h3>
      <div className="grid gap-3 px-4 pb-4">
        <div>
          <Label htmlFor="rec-title">Title</Label>
          <Input id="rec-title" value={title} onChange={(e) => setTitle(e.target.value)} />
        </div>
        <div>
          <Label htmlFor="rec-expires">Expires (optional)</Label>
          <Input
            id="rec-expires"
            type="date"
            value={expiresAt}
            onChange={(e) => setExpiresAt(e.target.value)}
          />
        </div>
        {sealedKeys.map((f) => (
          <div key={f.field_key}>
            <Label htmlFor={`rec-${f.field_key}`}>
              {FIELD_LABELS[f.field_key] ?? f.field_key}{" "}
              {f.has_value ? "(rotates the sealed value)" : "(sets a sealed value)"}
            </Label>
            <Input
              id={`rec-${f.field_key}`}
              type={f.field_key === "secret" ? "password" : "text"}
              value={clearKeys[f.field_key] ? "" : (values[f.field_key] ?? "")}
              disabled={clearKeys[f.field_key]}
              autoComplete="new-password"
              onChange={(e) =>
                setValues((prev) => ({ ...prev, [f.field_key]: e.target.value }))
              }
            />
            {f.has_value ? (
              <label className="mt-1 flex items-center gap-2 text-[12px]">
                <input
                  type="checkbox"
                  checked={clearKeys[f.field_key] ?? false}
                  onChange={(e) =>
                    setClearKeys((prev) => ({ ...prev, [f.field_key]: e.target.checked }))
                  }
                />
                Clear this sealed value
              </label>
            ) : null}
            <FieldHint>Sealed server-side before persistence; never returned except by reveal.</FieldHint>
          </div>
        ))}
        {error !== null ? (
          <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {error}
          </p>
        ) : null}
        <DialogActions>
          <Button variant="ghost" onClick={onDone}>
            Cancel
          </Button>
          <Button onClick={() => void submit()} disabled={busy || title.trim() === ""}>
            {busy ? <Spinner /> : null} Save
          </Button>
        </DialogActions>
      </div>
    </Card>
  );
}

/* Record links: resolved targets via the RecordLink seam. Broken targets
   render the "deleted target" semantic (never dropped silently). Shared
   target pickers ride the Wave 16 backlog lane — this panel accepts ids
   for now, exactly like the shipped documents/asset panels. */
function LinksPanel({
  recordId,
  links,
  onChanged,
}: {
  recordId: string;
  links: ResolvedLink[];
  onChanged: () => void;
}) {
  const [kind, setKind] = useState<string>("");
  const [target, setTarget] = useState("");
  const [linkError, setLinkError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const addLink = async () => {
    if (kind === "" || target.trim() === "") return;
    setLinkError(null);
    const uuid = parseUuid(target.trim());
    if (uuid === null) {
      setLinkError(
        "Paste the target's id (a UUID). Cross-entity target pickers ride the Wave 16 backlog lane; this panel accepts ids like the documents panel.",
      );
      return;
    }
    setBusy(true);
    const { error } = await api.api.POST("/api/v1/secrets/records/{id}/links", {
      params: { path: { id: recordId } },
      body: { to_kind: kind as components["schemas"]["LinkKind"], to_id: uuid },
    });
    setBusy(false);
    if (error !== undefined) {
      setLinkError(
        "The server rejected this link. The target must exist and be visible in your scope.",
      );
      return;
    }
    setKind("");
    setTarget("");
    onChanged();
  };

  const removeLink = async (linkId: string) => {
    const { error } = await api.api.DELETE("/api/v1/documents/links/{id}", {
      params: { path: { id: linkId } },
    });
    if (error === undefined) onChanged();
  };

  return (
    <Card>
      <h3 className="p-4 pb-2 text-sm font-semibold">Links</h3>
      {links.length === 0 ? (
        <p className="px-4 pb-2 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
          Link config items, tickets, KB articles, or documents to this secret.
        </p>
      ) : (
        <ul className="grid gap-1 px-4 pb-4">
          {links.map((l) => (
            <li
              key={l.id}
              className="flex items-center justify-between gap-2 rounded-[var(--m-radius-container)] border px-3 py-2 text-[13px]"
              style={{ borderColor: "var(--m-border)" }}
            >
              <span className="min-w-0">
                <Badge>{l.to_kind.replaceAll("_", " ")}</Badge>{" "}
                {l.target_deleted ? (
                  <span style={{ color: "var(--m-status-danger-fg)" }}>deleted target</span>
                ) : (
                  <span className="truncate">{l.target_label ?? "target"}</span>
                )}
              </span>
              <Button
                variant="ghost"
                aria-label="Remove link"
                onClick={() => void removeLink(l.id)}
              >
                ✕
              </Button>
            </li>
          ))}
        </ul>
      )}
      <div className="grid gap-2 px-4 pb-4">
        <Select value={kind} onValueChange={setKind}>
          <SelectTrigger aria-label="Link target kind">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {LINK_KIND_OPTIONS.map((k) => (
              <SelectItem key={k.value} value={k.value}>
                {k.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Input
          aria-label="Link target id"
          placeholder="Target record id…"
          value={target}
          onChange={(e) => setTarget(e.target.value)}
        />
        {linkError !== null ? (
          <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {linkError}
          </p>
        ) : null}
        <Button
          variant="secondary"
          onClick={() => void addLink()}
          disabled={busy || kind === "" || target.trim() === ""}
        >
          Add link
        </Button>
      </div>
    </Card>
  );
}

function parseUuid(s: string): string | null {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s) ? s : null;
}

/* ------------------------------------------------------------------ */
/* Folder management: create / edit / fork-the-seed + grants admin.    */
/* ------------------------------------------------------------------ */

function FolderDialog({
  mode,
  folder,
  scope,
  folders,
  clients,
  onSuccess,
  onClose,
}: {
  mode: "new" | "edit" | "fork" | null;
  folder: SecretFolder | null;
  scope: Scope;
  folders: SecretFolder[];
  clients: Client[];
  onSuccess: () => void;
  onClose: () => void;
}) {
  const [label, setLabel] = useState("");
  const [slug, setSlug] = useState("");
  const [kind, setKind] = useState<string>("passwords");
  const [parentId, setParentId] = useState<string>("");
  const [restricted, setRestricted] = useState<boolean>(false);
  const [targetClient, setTargetClient] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (mode !== null) {
      setLabel(folder?.label ?? "");
      setSlug(folder?.slug ?? "");
      setKind(folder?.kind ?? "passwords");
      setParentId(folder?.parent_id ?? "");
      setRestricted(folder?.restricted ?? false);
      setTargetClient(scope.kind === "client" ? scope.id : "");
      setError(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, folder]);

  const submit = async () => {
    setBusy(true);
    setError(null);
    if (mode === "new") {
      const client_id = scope.kind === "client" ? scope.id : null;
      const { error: e } = await api.api.POST("/api/v1/secrets/folders", {
        body: {
          slug: (slug || slugify(label)) as string,
          label,
          kind: kind as components["schemas"]["SecretFolderKind"],
          parent_id: parentId === "" ? null : parentId,
          restricted,
          client_id,
        },
      });
      if (e !== undefined) {
        setError("The server rejected this category — the slug must be unique among siblings of this scope, and the scope cannot mix with an org parent.");
        setBusy(false);
        return;
      }
      setBusy(false);
      onSuccess();
      return;
    }
    if (mode === "edit" && folder !== null) {
      const { error: e } = await api.api.PATCH("/api/v1/secrets/folders/{id}", {
        params: { path: { id: folder.id } },
        body: {
          label,
          kind: kind as components["schemas"]["SecretFolderKind"],
          parent_id: parentId === "" ? null : parentId,
          restricted,
        },
      });
      setBusy(false);
      if (e !== undefined) {
        setError("The category was rejected — seeds are immutable (fork instead) and the scope cannot mix.");
        return;
      }
      onSuccess();
      return;
    }
    if (mode === "fork" && folder !== null) {
      if (targetClient === "") {
        setError("Choose the client this fork should land in.");
        setBusy(false);
        return;
      }
      const { error: e } = await api.api.POST("/api/v1/secrets/folders/{id}/fork", {
        params: { path: { id: folder.id } },
        body: { client_id: targetClient },
      });
      setBusy(false);
      if (e !== undefined) {
        setError(
          e === undefined ? "" : "The fork was rejected — a same-slug fork may already exist for this client.",
        );
        return;
      }
      onSuccess();
      return;
    }
    setBusy(false);
  };

  if (mode === null) return null;

  const isSeed = folder !== null && folder.is_default_seed;
  const parents = folders.filter(
    (f) => f.id !== folder?.id && f.id !== folder?.parent_id,
  );

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent
        title={
          mode === "new" ? "New category" : mode === "fork" ? `Fork “${folder?.label ?? ""}”` : "Edit category"
        }
        description={
          mode === "fork"
            ? "Shipped seeds are immutable (ADR-0004): forking copies the subtree into a client's scope with customization provenance. Re-using the same slug for this client is a typed conflict."
            : "Categories order the tree; siblings share a slug namespace per scope. Restricted categories deny by default until a grant row allows a role."
        }
      >
        <div className="grid gap-3 p-4">
          {mode === "fork" ? (
            <>
              <div>
                <Label htmlFor="fork-client">Target client</Label>
                <Select value={targetClient} onValueChange={setTargetClient}>
                  <SelectTrigger id="fork-client">
                    <SelectValue placeholder="Choose a client…" />
                  </SelectTrigger>
                  <SelectContent>
                    {clients.map((c) => (
                      <SelectItem key={c.id} value={c.id}>
                        {c.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </>
          ) : (
            <>
              <div>
                <Label htmlFor="fold-label">Label</Label>
                <Input id="fold-label" value={label} onChange={(e) => setLabel(e.target.value)} />
              </div>
              {mode === "new" ? (
                <div>
                  <Label htmlFor="fold-slug">Slug</Label>
                  <Input
                    id="fold-slug"
                    value={slug}
                    placeholder="auto from label"
                    onChange={(e) => setSlug(e.target.value)}
                  />
                  <FieldHint>Sibling-unique within (scope, parent); lowercase spellings.</FieldHint>
                </div>
              ) : null}
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <Label htmlFor="fold-kind">Kind</Label>
                  <Select value={kind} onValueChange={setKind}>
                    <SelectTrigger id="fold-kind">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {FOLDER_KIND_OPTIONS.map((k) => (
                        <SelectItem key={k.value} value={k.value}>
                          {k.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div>
                  <Label htmlFor="fold-parent">Parent</Label>
                  <Select value={parentId} onValueChange={setParentId}>
                    <SelectTrigger id="fold-parent">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="">Root of this scope</SelectItem>
                      {parents.map((f) => (
                        <SelectItem key={f.id} value={f.id}>
                          {f.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <FieldHint>A parent must share the new folder's scope (org ↔ client mix is refused).</FieldHint>
                </div>
              </div>
              {mode === "new" ? (
                <label className="flex items-center gap-2 text-[13px]">
                  <input type="checkbox" checked={restricted} onChange={(e) => setRestricted(e.target.checked)} />
                  Restricted (deny-by-default category)
                </label>
              ) : null}
            </>
          )}
          {error !== null ? (
            <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {error}
            </p>
          ) : null}
          <DialogActions>
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button
              variant="secondary"
              onClick={() => void submit()}
              disabled={busy || (mode !== "fork" && label.trim() === "")}
            >
              {busy ? <Spinner /> : null}
              {mode === "fork" ? "Fork into client" : mode === "new" ? "Create category" : "Save"}
            </Button>
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/* Owner/admin-only grant matrix (backend gates the route; a 403 here
   renders the typed not-available line — never a dead button). */
function GrantsDialog({
  open,
  onClose,
  folders,
}: {
  open: boolean;
  onClose: () => void;
  folders: SecretFolder[];
}) {
  const [roles, setRoles] = useState<components["schemas"]["RoleView"][] | null>(null);
  const [grants, setGrants] = useState<SecretGrant[] | null>(null);
  const [blocked, setBlocked] = useState(false);
  const [roleId, setRoleId] = useState<string>("");
  const [folderId, setFolderId] = useState<string>("");
  const [canRead, setCanRead] = useState(true);
  const [canEdit, setCanEdit] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(() => {
    void (async () => {
      const [g, r] = await Promise.all([
        api.api.GET("/api/v1/secrets/grants"),
        api.api.GET("/api/v1/roles"),
      ]);
      if (g.error !== undefined) {
        setBlocked(g.response?.status === 403);
        setGrants(null);
      } else {
        setBlocked(false);
        setGrants([...(g.data ?? [])]);
      }
      if (r.data !== undefined) setRoles([...r.data.roles]);
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  useEffect(() => {
    if (open) load();
  }, [open, load]);

  const setGrant = async () => {
    if (roleId === "" || folderId === "") return;
    setBusy(true);
    setError(null);
    if (!canRead && !canEdit) {
      setError("A grant needs can_read or can_edit (edit implies read) — to deny, delete the grant row.");
      setBusy(false);
      return;
    }
    const { error: e } = await api.api.PUT("/api/v1/secrets/grants", {
      body: { role_id: roleId, folder_id: folderId, can_read: canRead, can_edit: canEdit },
    });
    setBusy(false);
    if (e !== undefined) {
      setError("The grant was rejected — owner/admin only, and the folder must exist.");
      return;
    }
    setNotice("Grant saved");
    load();
  };

  const revoke = async (grant: SecretGrant) => {
    const { error: e } = await api.api.DELETE("/api/v1/secrets/grants", {
      params: { query: { role_id: grant.role_id, folder_id: grant.folder_id } },
    });
    if (e === undefined) load();
  };

  if (!open) return null;

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent
        title="Category grants"
        description="Per (role × folder) grants open restricted category chains. Unrestricted categories ride the permission matrix; grants are extra keys, never a veto."
      >
        <div className="grid gap-4 p-4">
          {blocked ? (
            <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
              The grant matrix is not available with your role — grants are owner/admin policy
              data.
            </p>
          ) : (
            <>
              {grants === null ? (
                <Spinner />
              ) : (
                <ul className="grid gap-1">
                  {grants.map((g) => (
                    <li
                      key={g.id}
                      className="flex items-center justify-between gap-2 rounded-[var(--m-radius-container)] border px-3 py-2 text-[13px]"
                      style={{ borderColor: "var(--m-border)" }}
                    >
                      <span className="min-w-0 truncate">
                        <code>{g.role_id.slice(0, 8)}</code> ·{" "}
                        {(folders).find((f) => f.id === g.folder_id)?.label ?? "folder"} ·{" "}
                        {g.can_edit ? "read+edit" : "read"}
                      </span>
                      <Button variant="ghost" onClick={() => void revoke(g)} aria-label="Revoke grant">
                        ✕
                      </Button>
                    </li>
                  ))}
                </ul>
              )}
              <div className="grid grid-cols-2 gap-2">
                <Select value={roleId} onValueChange={setRoleId}>
                  <SelectTrigger aria-label="Role">
                    <SelectValue placeholder="Role…" />
                  </SelectTrigger>
                  <SelectContent>
                    {(roles ?? []).map((r) => (
                      <SelectItem key={r.id} value={r.id}>
                        {r.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Select value={folderId} onValueChange={setFolderId}>
                  <SelectTrigger aria-label="Category">
                    <SelectValue placeholder="Category…" />
                  </SelectTrigger>
                  <SelectContent>
                    {(folders).map((f: SecretFolder) => (
                      <SelectItem key={f.id} value={f.id}>
                        {f.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <label className="flex items-center gap-2 text-[13px]">
                <input type="checkbox" checked={canRead} onChange={(e) => setCanRead(e.target.checked)} />
                Read
              </label>
              <label className="flex items-center gap-2 text-[13px]">
                <input type="checkbox" checked={canEdit} onChange={(e) => setCanEdit(e.target.checked)} />
                Edit (implies read; enables reveal on restricted chains)
              </label>
              {error !== null ? (
                <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                  {error}
                </p>
              ) : null}
              {notice !== null ? (
                <p className="text-[12px]" style={{ color: "var(--m-status-success-fg)" }}>
                  {notice}
                </p>
              ) : null}
              <DialogActions>
                <Button variant="ghost" onClick={onClose}>
                  Close
                </Button>
                <Button variant="secondary" onClick={() => void setGrant()} disabled={busy || roleId === "" || folderId === ""}>
                  {busy ? <Spinner /> : null} Save grant
                </Button>
              </DialogActions>
            </>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

