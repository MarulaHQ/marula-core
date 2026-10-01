// /passwords — the staff vault (ADR 0006 browsing surface; UX-excellence
// pass, Wave 17 lane 2).
//
// Purpose: the sanctum has two legs. (1) THE VAULT BROWSER — KB-class
// secret category trees per client (fork-the-seed + user folders),
// org/client scope toggle, category-ACL hidden branches (strict backend
// rendering: what your roles cannot see is NEVER rendered as a dead
// entry), metadata-only record views with role-gated audited reveal
// (secrets:edit + category grants, never bypassed client-side),
// secret-to-record links through the shared cross-entity picker,
// field-clear PATCH wiring (`value: null` clears, audited via the
// record-updated event), and document-kind records showing the
// pointer+hash object (object_key + file_sha256 + file_bytes) with the
// audited reveal-file download/upload riding the core-store seam. (2)
// THE LEGACY ENTRIES — the shipped PasswordEntry surface stays
// byte-compatible (never a destructive cutover): flat entries with the
// same audited reveal, change history, and envelope-backed writes.
//
// Non-purpose: no sharing links / one-time shares / rotation automation
// (Wave 19 `p19-secrets-product`); no plaintext ever persists beyond
// the open reveal dialog; no composer changes.
"use client";

import { rejectionText } from "../../lib/error-copy";
import { friendlyMutationError } from "../../lib/error-copy";
import {
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
  SurfaceHeader,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { api, useAuth, useRequireAuth } from "../../lib/api";
import { runEnvelopeMutation, newIdempotencyKey } from "../../lib/envelope-write";
import {
  AlertIcon,
  HEAD_CELL_CLASS,
  KeyHint,
  LedgerFooter,
  LockIcon,
  NoMatches,
  PageFrame,
  PlusIcon,
  SkeletonRows,
  StatPill,
  StatusPill,
  TableCard,
  downloadCsv,
} from "../../components/knowledge/ledger";
import { EntityPicker } from "../../components/entities/entity-picker";
import { PasswordLinksPanel, SecretSharePanel, SecretTargetPicker } from "../../components/secret-share";
import type { LinkTarget } from "../../lib/entity-links";

type SecretFolder = components["schemas"]["SecretFolder"];
type SecretRecord = components["schemas"]["SecretRecord"];
type SecretReveal = components["schemas"]["SecretReveal"];
type ResolvedLink = components["schemas"]["ResolvedLink"];
type SecretGrant = components["schemas"]["SecretCategoryGrant"];
type Client = components["schemas"]["Client"];
type PasswordEntry = components["schemas"]["PasswordEntry"];
type PasswordHistoryEntry = components["schemas"]["PasswordHistoryEntry"];
type PasswordReveal = components["schemas"]["PasswordReveal"];

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

const FIELD_LABELS: Record<string, string> = {
  secret: "Secret",
  notes: "Notes",
  username: "Username",
  url: "URL",
};

const LINK_KINDS: components["schemas"]["LinkKind"][] = [
  "asset",
  "kb_article",
  "ticket",
  "password",
  "document",
  "client",
  "secret",
];

type VaultColumnKey = "record" | "username" | "kind" | "scope" | "expires" | "fields";

interface VaultColumn {
  key: VaultColumnKey;
  label: string;
  visible: boolean;
  required?: boolean;
}

/**
 * Two ledgers, one column set: the category records and the flat credential
 * vault are the same records seen through two hierarchies, so they share the
 * columns and switch by view instead of growing their own.
 */
const VAULT_COLUMNS: VaultColumn[] = [
  { key: "record", label: "Record", visible: true, required: true },
  { key: "username", label: "Username", visible: true },
  { key: "kind", label: "Kind", visible: true },
  { key: "scope", label: "Scope", visible: false },
  { key: "expires", label: "Expires", visible: true },
  { key: "fields", label: "Sealed fields", visible: false },
];

/**
 * A record's shape, never its contents: which fields are sealed. The values
 * themselves exist only inside a confirmed, audited reveal.
 */
function sealedFieldList(record: SecretRecord): string {
  const filled = (record.fields ?? []).filter((f) => f.has_value);
  if (filled.length === 0) return record.object_key != null ? "encrypted file" : "—";
  const keys = filled.map((f) => f.field_key);
  return record.object_key != null ? `${keys.join(", ")} + file` : keys.join(", ");
}

type VaultScope = { kind: "org" } | { kind: "client"; id: string };

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

function parseUuid(s: string): string | null {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s) ? s : null;
}

function scopeOf(scope: VaultScope) {
  return scope.kind === "org" ? {} : { client_id: scope.id };
}

export default function VaultPage() {
  const auth = useRequireAuth();
  const { me } = useAuth();
  const canReveal = (me?.permissions ?? []).includes("secrets:edit") ?? false;
  const canManage = (me?.permissions ?? []).includes("special:manage_users") ?? false;

  // ---- vault-tree state (ADR 0006 browsing leg) ----
  const [clients, setClients] = useState<Client[] | null>(null);
  const [scope, setScope] = useState<VaultScope>({ kind: "org" });
  const [folders, setFolders] = useState<SecretFolder[] | null>(null);
  const [foldersError, setFoldersError] = useState<string | null>(null);
  const [selectedFolder, setSelectedFolder] = useState<string | null>(null);
  const [records, setRecords] = useState<SecretRecord[] | null>(null);
  const [openRecord, setOpenRecord] = useState<SecretRecord | null>(null);
  const [folderDialog, setFolderDialog] = useState<"new" | "edit" | "fork" | null>(null);
  const [folderForDialog, setFolderForDialog] = useState<SecretFolder | null>(null);
  const [grantsOpen, setGrantsOpen] = useState(false);

  // ---- legacy-entries state (byte-compatible PasswordEntry leg) ----
  const [entries, setEntries] = useState<PasswordEntry[] | null>(null);

  const [legacyEditOpen, setLegacyEditOpen] = useState(false);
  const [legacyEditing, setLegacyEditing] = useState<PasswordEntry | null>(null);
  const [legacyReveal, setLegacyReveal] = useState<(PasswordEntry & { secret: string; notes?: string | null }) | null>(null);
  const [legacyRevealing, setLegacyRevealing] = useState<string | null>(null);
  const [historyFor, setHistoryFor] = useState<PasswordEntry | null>(null);
  const [history, setHistory] = useState<PasswordHistoryEntry[] | null>(null);

  const [vaultQuery, setVaultQuery] = useState("");
  const [view, setView] = useState("categories");
  const [confirmingLegacy, setConfirmingLegacy] = useState<PasswordEntry | null>(null);
  const [clauses, setClauses] = useState<{ id: string; field: string; op: never; value?: string }[]>(
    [],
  );
  const [columns, setColumns] = useState<VaultColumn[]>(VAULT_COLUMNS);
  const density = useDensity();

  const clientById = useMemo(() => new Map((clients ?? []).map((c) => [c.id, c])), [clients]);

  const refresh = useCallback(async () => {
    const [p, c] = await Promise.all([
      api.api.GET("/api/v1/passwords"),
      api.api.GET("/api/v1/clients"),
    ]);
    if (p.error !== undefined || c.error !== undefined) {
      setFoldersError("Failed to load the vault.");
      setEntries([]);
      setClients([]);
      return;
    }
    setEntries(p.data ?? []);
    setClients(c.data ?? []);
    setFoldersError(null);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  useEffect(() => {
    if (auth.status !== "authed") return;
    setFoldersError(null);
    api.api
      .GET("/api/v1/secrets/folders", { params: { query: scopeOf(scope) } })
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
    api.api
      .GET("/api/v1/secrets/records", {
        params: { query: { folder_id: selectedFolder, ...scopeOf(scope) } },
      })
      .then(({ data, error }) => {
        setRecords(error === undefined && data !== undefined ? [...data] : []);
      });
  }, [auth.status, selectedFolder, scope]);

  const refreshFolders = useCallback(() => {
    api.api
      .GET("/api/v1/secrets/folders", { params: { query: scopeOf(scope) } })
      .then(({ data }) => {
        if (data !== undefined) setFolders([...data]);
      });
  }, [scope]);

  const reloadRecords = useCallback(() => {
    if (selectedFolder === null) return;
    api.api
      .GET("/api/v1/secrets/records", {
        params: { query: { folder_id: selectedFolder, ...scopeOf(scope) } },
      })
      .then(({ data }) => {
        if (data !== undefined) setRecords([...data]);
      });
  }, [selectedFolder, scope]);

  // ---- legacy handlers (unchanged behavior) ----
  const legacyExpired = (e: PasswordEntry) => e.expires_at != null && new Date(e.expires_at) < new Date();
  // Rotation reminder honesty (Wave 19 lane 3): due means past rotate_at
  // AND the rotation never stamped (the worker-side sweep marker lives
  // in the DB; the client reflects the columns the entities carry).
  const legacyRotationDue = (e: PasswordEntry) => e.rotate_at != null && new Date(e.rotate_at) < new Date() && e.last_rotated_at == null;

  const legacyRevealEntry = async (entry: PasswordEntry) => {
    setLegacyRevealing(entry.id);
    const { data, error } = await api.api.POST("/api/v1/passwords/{id}/reveal", {
      params: { path: { id: entry.id } },
    });
    setLegacyRevealing(null);
    if (error !== undefined || data === undefined) {
      setFoldersError("Reveal was rejected — you need vault edit rights.");
      return;
    }
    setLegacyReveal({ ...entry, secret: (data as PasswordReveal).secret, notes: (data as PasswordReveal).notes ?? null });
  };

  const openHistory = async (entry: PasswordEntry) => {
    setHistoryFor(entry);
    setHistory(null);
    const { data } = await api.api.GET("/api/v1/passwords/{id}/history", {
      params: { path: { id: entry.id } },
    });
    if (data !== undefined) setHistory(data);
  };

  const removeLegacy = async (entry: PasswordEntry) => {
    if (!window.confirm(`Delete “${entry.title}” from the vault? Its change history goes with it.`)) {
      return;
    }
    const { error } = await api.api.DELETE("/api/v1/passwords/{id}", {
      params: { path: { id: entry.id } },
    });
    if (error === undefined) await refresh();
  };

  const legacyFiltered = useMemo(() => {
    let rows = entries;
    const needle = vaultQuery.trim().toLowerCase();
    if (rows === null) return null;
    if (needle !== "") {
      rows = rows.filter(
        (e) =>
          e.title.toLowerCase().includes(needle) ||
          (e.username ?? "").toLowerCase().includes(needle) ||
          (e.url ?? "").toLowerCase().includes(needle),
      );
    }
    for (const clause of clauses) {
      const value = (clause.value ?? "").toLowerCase();
      if (clause.field === "record") rows = rows.filter((e) => e.title.toLowerCase().includes(value));
      if (clause.field === "username") rows = rows.filter((e) => (e.username ?? "").toLowerCase().includes(value));
      if (clause.field === "url") rows = rows.filter((e) => (e.url ?? "").toLowerCase().includes(value));
    }
    if (view === "expired") rows = rows.filter(legacyExpired);
    if (view === "rotate") rows = rows.filter(legacyRotationDue);
    return rows;
  }, [entries, vaultQuery, clauses, view]);

  const sortedRecords = useMemo(
    () =>
      (records ?? [])
        .slice()
        .sort((a, b) => a.title.localeCompare(b.title, undefined, { sensitivity: "base" })),
    [records],
  );

  const filteredRecords = useMemo(() => {
    let rows = sortedRecords;
    for (const clause of clauses) {
      const value = (clause.value ?? "").toLowerCase();
      if (clause.field === "record") rows = rows.filter((r) => r.title.toLowerCase().includes(value));
      if (clause.field === "kind") rows = rows.filter((r) => r.kind === clause.value);
    }
    if (view === "expired") {
      rows = rows.filter((r) => r.expires_at != null && Date.parse(r.expires_at) < Date.now());
    }
    return rows;
  }, [sortedRecords, clauses, view]);

  const selectedFolderRow = useMemo(
    () => (folders ?? []).find((f) => f.id === selectedFolder) ?? null,
    [folders, selectedFolder],
  );

  const expiredCount = useMemo(() => {
    const rows = view === "credentials" ? (legacyFiltered ?? []) : filteredRecords;
    return rows.filter((r) =>
      view === "credentials"
        ? legacyExpired(r as PasswordEntry)
        : r.expires_at != null && Date.parse(r.expires_at) < Date.now(),
    ).length;
  }, [view, legacyFiltered, filteredRecords]);

  const rotationDueCount = useMemo(
    () => (legacyFiltered ?? []).filter(legacyRotationDue).length,
    [legacyFiltered],
  );

  const visibleColumns = columns;
  const shownColumns = visibleColumns.filter((c) => c.visible);

  /** One flat shape for the export and the footer, whichever view is on screen. */
  const shownRows = useMemo(
    () =>
      view === "credentials"
        ? (legacyFiltered ?? []).map((e) => ({
            title: e.title,
            username: e.username ?? "",
            kind: "password",
            scope:
              e.client_id != null ? (clientById.get(e.client_id)?.name ?? "") : "Org-level",
            expires: e.expires_at ?? "",
            fields: "",
            updated: e.updated_at ?? "",
          }))
        : filteredRecords.map((r) => ({
            title: r.title,
            username:
              (r.fields ?? []).find((f) => f.field_key === "username" && f.has_value) !== undefined
                ? "set"
                : "",
            kind: r.kind,
            scope:
              r.client_id != null ? (clientById.get(r.client_id)?.name ?? "") : "Org-level",
            expires: r.expires_at ?? "",
            fields: sealedFieldList(r),
            updated: r.updated_at,
          })),
    [view, legacyFiltered, filteredRecords, clientById],
  );

  const openNewRecord = () => {
    if (selectedFolder === null) return;
    const folder = (folders ?? []).find((f) => f.id === selectedFolder);
    const kind =
      folder?.kind === "api_keys"
        ? "api_key"
        : folder?.kind === "certificates"
          ? "certificate"
          : folder?.kind === "recovery_codes"
            ? "recovery_code"
            : folder?.kind === "documents"
              ? "document"
              : "password";
    setOpenRecord({
      id: "",
      org_id: scope.kind === "org" ? "" : scope.id,
      folder_id: selectedFolder,
      title: "",
      kind,
      fields: [],
      client_id: scope.kind === "client" ? scope.id : null,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    } satisfies SecretRecord);
  };

  if (auth.status !== "authed") return null;

  const scopeLabel =
    scope.kind === "org" ? "Org-level vault" : `Vault — ${clientById.get(scope.id)?.name ?? "client"}`;

  return (
    <PageFrame>
      <SurfaceHeader
        title="Vault"
        description="Categories and credentials · audited reveals"
        meta={
          <>
            <StatPill
              tone="blush"
              label="Rotation due"
              count={rotationDueCount}
              onClick={() => setView(view === "rotate" ? "categories" : "rotate")}
              pressed={view === "rotate"}
            />
            <StatPill
              tone="red"
              label="Expired"
              count={expiredCount}
              onClick={() => setView(view === "expired" ? "categories" : "expired")}
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
              data-testid="vault-new-category"
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
            <VaultTree
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
            label="Vault controls"
            views={
              <ViewSwitch
                label="Vault views"
                views={[
                  { id: "categories", label: "Categories", hint: String(sortedRecords.length) },
                  { id: "credentials", label: "Credentials", hint: String(legacyFiltered?.length ?? 0) },
                  { id: "expired", label: "Expired" },
                  { id: "rotate", label: "Rotation due" },
                ]}
                activeId={view}
                onChange={setView}
              />
            }
            filters={
              <FilterBar
                label="Vault filters"
                emptyHint="No filters"
                fields={[
                  { key: "title", label: "Record", operators: ["contains"] },
                  { key: "username", label: "Username", operators: ["contains"] },
                  { key: "kind", label: "Kind", operators: ["eq"] },
                  { key: "url", label: "URL", operators: ["contains"] },
                ]}
                clauses={clauses}
                onChange={(next) => setClauses(next.map((c) => ({ ...c, op: c.op as never })))}
              />
            }
            columns={
              <ColumnChooser
                label="Vault columns"
                columns={visibleColumns.map((c) => ({
                  key: c.key,
                  label: c.label,
                  visible: c.visible,
                  required: c.required,
                }))}
                onChange={(next) =>
                  setColumns(
                    next.map((c) => ({
                      key: c.key as VaultColumnKey,
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
                rowCount={shownRows.length}
                filters={clauses}
                onExport={() => {
                  // Never a value: an export of the vault is a metadata ledger,
                  // so the plaintext stays behind the reveal.
                  downloadCsv(
                    "vault.csv",
                    shownColumns.map((c) => c.label),
                    shownRows.map((row) =>
                      shownColumns.map((c) => {
                        if (c.key === "record") return row.title;
                        if (c.key === "username") return row.username ?? "";
                        if (c.key === "kind") return row.kind;
                        if (c.key === "scope") return row.scope;
                        if (c.key === "expires") return row.expires ?? "";
                        if (c.key === "fields") return row.fields;
                        return row.updated;
                      }),
                    ),
                  );
                }}
              />
            }
          />

          <TableCard label={view === "credentials" ? "Credentials" : "Secret records"}>
            {view === "credentials" ? (
              legacyFiltered === null ? (
                <SkeletonRows sentence="Loading credentials." columns={4} />
              ) : legacyFiltered.length === 0 ? (
                (entries?.length ?? 0) === 0 ? (
                  <EmptyState message="No credentials in this vault yet. Entries can be org-level or scoped to a client." />
                ) : (
                  <NoMatches
                    title="No credentials match these filters."
                    hint="Try removing one. Secrets stay encrypted at rest either way."
                    noun="credentials"
                    onClear={() => {
                      setClauses([]);
                      setVaultQuery("");
                    }}
                  />
                )
              ) : (
                <Table data-testid="vault-credentials-table">
                  <thead>
                    <tr>
                      {shownColumns.map((c) => (
                        <Th key={c.key} className={HEAD_CELL_CLASS}>
                          {c.label}
                        </Th>
                      ))}
                      <Th className={`${HEAD_CELL_CLASS} text-right`}>Reveal</Th>
                    </tr>
                  </thead>
                  <tbody>
                    {legacyFiltered.map((e) => (
                      <Tr key={e.id} style={{ height: "var(--m-density-row)" }}>
                        {shownColumns.map((c) => (
                          <Td key={c.key}>
                            {c.key === "record" ? (
                              <span className="flex min-w-0 items-center gap-2">
                                <span style={{ color: "var(--m-text-muted)" }}>
                                  <LockIcon size={16} />
                                </span>
                                <span className="truncate font-semibold" style={{ color: "var(--m-text)" }}>
                                  {e.title}
                                </span>
                                {(e.url ?? "") !== "" ? (
                                  <a
                                    href={e.url}
                                    target="_blank"
                                    rel="noreferrer"
                                    className="underline decoration-[var(--m-border)] underline-offset-2"
                                    style={{ color: "var(--m-text-secondary)" }}
                                  >
                                    open
                                  </a>
                                ) : null}
                              </span>
                            ) : null}
                            {c.key === "username" ? (
                              (e.username ?? "") !== "" ? (
                                e.username
                              ) : (
                                <span style={{ color: "var(--m-text-muted)" }}>—</span>
                              )
                            ) : null}
                            {c.key === "scope" ? (
                              e.client_id != null && clientById.get(e.client_id) ? (
                                <Link
                                  href={`/clients/${e.client_id}`}
                                  className="underline decoration-[var(--m-border)] underline-offset-2"
                                >
                                  {clientById.get(e.client_id)!.name}
                                </Link>
                              ) : (
                                <span style={{ color: "var(--m-text-muted)" }}>Org-level</span>
                              )
                            ) : null}
                            {c.key === "expires" ? (
                              e.expires_at == null ? (
                                <span style={{ color: "var(--m-text-muted)" }}>—</span>
                              ) : legacyExpired(e) ? (
                                <StatusPill tone="danger">expired</StatusPill>
                              ) : legacyRotationDue(e) ? (
                                <StatusPill tone="warning">rotation due</StatusPill>
                              ) : (
                                <span className="tabular-nums">
                                  {new Date(e.expires_at).toLocaleDateString()}
                                </span>
                              )
                            ) : null}
                          </Td>
                        ))}
                        <Td className="text-right">
                          <Button
                            variant="secondary"
                            size="sm"
                            onClick={() => setConfirmingLegacy(e)}
                            disabled={legacyRevealing === e.id}
                            data-testid={`vault-reveal-${e.id}`}
                          >
                            <LockIcon size={14} /> Reveal
                          </Button>
                        </Td>
                      </Tr>
                    ))}
                  </tbody>
                </Table>
              )
            ) : (
              <>
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
                      <Tag tone="warning">restricted · reveal needs an edit grant</Tag>
                    ) : null}
                    {selectedFolderRow.is_default_seed ? <Tag tone="neutral">shipped seed</Tag> : null}
                    {selectedFolderRow.forked_from !== null ? <Tag tone="neutral">forked seed</Tag> : null}
                  </div>
                ) : null}

                {selectedFolder === null ? (
                  <EmptyState message="Pick a category in the tree to browse its records. Restricted categories you cannot access are hidden entirely — never dead entries." />
                ) : records === null ? (
                  <SkeletonRows sentence="Loading the category records." columns={4} />
                ) : sortedRecords.length === 0 ? (
                  <EmptyState
                    message="No records in this category yet."
                    action={
                      <Button size="sm" onClick={() => openNewRecord()}>
                        New record
                      </Button>
                    }
                  />
                ) : (
                  <Table data-testid="vault-records-table">
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
                      {sortedRecords.map((r) => {
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
                                    disabled={r.id === ""}
                                  >
                                    <span style={{ color: "var(--m-text-muted)" }}>
                                      <LockIcon size={16} />
                                    </span>
                                    <span className="truncate font-semibold" style={{ color: "var(--m-text)" }}>
                                      {r.title}
                                    </span>
                                  </button>
                                ) : null}
                                {c.key === "kind" ? (
                                  // The flat credential ledger has no kind, so
                                  // the column says so rather than rendering an
                                  // empty cell down the screen.
                                  KIND_LABELS[r.kind] === undefined ? (
                                    <span style={{ color: "var(--m-text-muted)" }}>—</span>
                                  ) : (
                                    <StatusPill tone="neutral">{KIND_LABELS[r.kind] ?? r.kind}</StatusPill>
                                  )
                                ) : null}
                                {c.key === "fields" ? (
                                  <span className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                                    {sealedFieldList(r)}
                                  </span>
                                ) : null}
                                {c.key === "scope" ? (
                                  r.client_id != null && clientById.get(r.client_id) ? (
                                    <Link
                                      href={`/clients/${r.client_id}`}
                                      className="underline decoration-[var(--m-border)] underline-offset-2"
                                    >
                                      {clientById.get(r.client_id)!.name}
                                    </Link>
                                  ) : (
                                    <span style={{ color: "var(--m-text-muted)" }}>Org-level</span>
                                  )
                                ) : null}
                                {c.key === "expires" ? (
                                  expired ? (
                                    <StatusPill tone="danger">expired</StatusPill>
                                  ) : r.expires_at == null ? (
                                    <span style={{ color: "var(--m-text-muted)" }}>never</span>
                                  ) : (
                                    <span className="tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                                      {new Date(r.expires_at).toLocaleDateString()}
                                    </span>
                                  )
                                ) : null}
                              </Td>
                            ))}
                          </Tr>
                        );
                      })}
                    </tbody>
                  </Table>
                )}
              </>
            )}
          </TableCard>

          <LedgerFooter
            shown={shownRows.length}
            total={view === "credentials" ? (entries?.length ?? 0) : (records?.length ?? 0)}
            noun={view === "credentials" ? "credentials" : "records"}
            density={<DensitySwitch value={density} onChange={applyDensity} />}
            trailing={
              selectedFolder !== null && view !== "credentials" ? (
                <Button
                  size="sm"
                  onClick={() => openNewRecord()}
                  data-testid="vault-new-record"
                >
                  <PlusIcon size={14} /> New record
                </Button>
              ) : null
            }
            hints={
              <>
                <KeyHint keys="J K" label="move" />
                <KeyHint keys="↵" label="open" />
                <KeyHint keys="F" label="filter" />
              </>
            }
          />
        </div>
      </div>

      <RecordDialog
        record={openRecord}
        folders={folders ?? []}
        scope={scope}
        clientById={clientById}
        canReveal={canReveal}
        isNew={openRecord?.id === ""}
        onClose={() => {
          setOpenRecord(null);
        }}
        onChanged={() => {
          reloadRecords();
          void refresh();
        }}
      />

      {/* The flat credential vault gets the same confirmation the category
          records get: one gesture to open the value, a second, deliberate one
          to decrypt it. */}
      <Dialog open={confirmingLegacy !== null} onOpenChange={(o) => !o && setConfirmingLegacy(null)}>
        <DialogContent
          title="Reveal this credential?"
          description="The secret is decrypted for this browser only, shown below, and written to the audit trail with your name. It is never cached and never exported."
        >
          {confirmingLegacy !== null ? (
            <div className="grid gap-3 p-1">
              <dl className="grid gap-1 text-[13px]">
                <MetaRow label="Credential">{confirmingLegacy.title}</MetaRow>
                <MetaRow label="Username">{confirmingLegacy.username ?? "—"}</MetaRow>
                <MetaRow label="Scope">
                  {confirmingLegacy.client_id != null
                    ? (clientById.get(confirmingLegacy.client_id)?.name ?? "client")
                    : "Org-level"}
                </MetaRow>
              </dl>
              <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                Close this once you have what you came for. The next reader sees the same mask.
              </p>
              <DialogActions>
                <Button variant="secondary" onClick={() => setConfirmingLegacy(null)}>
                  Keep it sealed
                </Button>
                <Button
                  onClick={() => {
                    const entry = confirmingLegacy;
                    setConfirmingLegacy(null);
                    if (entry !== null) void legacyRevealEntry(entry);
                  }}
                  disabled={confirmingLegacy !== null && legacyRevealing === confirmingLegacy.id}
                  data-testid="vault-reveal-confirm"
                >
                  Reveal and record it
                </Button>
              </DialogActions>
            </div>
          ) : null}
        </DialogContent>
      </Dialog>

      <Dialog open={legacyEditOpen} onOpenChange={setLegacyEditOpen}>
        <DialogContent
          title={legacyEditing === null ? "New credential" : "Edit credential"}
          description={
            legacyEditing === null
              ? "Secrets are encrypted at rest and only ever returned through the audited reveal action."
              : "Leave the secret blank to keep the current one. Any change archives the previous secret to history."
          }
        >
          <div className="grid gap-4">
            <LegacyVaultForm
              editing={legacyEditing}
              clients={clients ?? []}
              onDone={() => {
                setLegacyEditOpen(false);
                void refresh();
              }}
            />
            {legacyEditing !== null && canManage ? (
              <div className="grid gap-3 px-4">
                <PasswordLinksPanel
                  entryId={legacyEditing.id}
                  onChanged={() => void refresh()}
                />
                <SecretSharePanel
                  subjectKind="password"
                  subjectId={legacyEditing.id}
                  canManage={canManage}
                />
              </div>
            ) : null}
          </div>
        </DialogContent>
      </Dialog>

      <Dialog open={legacyReveal !== null} onOpenChange={(o) => !o && setLegacyReveal(null)}>
        <DialogContent
          title="Credential revealed"
          description="This reveal was recorded in the audit trail. Close the dialog when you've copied what you need — the plaintext is not stored anywhere else."
        >
          {legacyReveal !== null ? (
            <div className="grid gap-3 p-4">
              <div>
                <Label htmlFor="reveal-secret">Secret</Label>
                <div className="flex gap-2">
                  <Input id="reveal-secret" readOnly value={legacyReveal.secret} className="" />
                  <Button
                    variant="secondary"
                    onClick={() => void navigator.clipboard.writeText(legacyReveal.secret)}
                  >
                    Copy
                  </Button>
                </div>
              </div>
              {(legacyReveal.username ?? "") !== "" ? (
                <div className="text-[13px]">
                  <span style={{ color: "var(--m-text-secondary)" }}>Username: </span>
                  {legacyReveal.username}
                  <Button
                    variant="ghost"
                    className="ml-2"
                    onClick={() => void navigator.clipboard.writeText(legacyReveal.username ?? "")}
                  >
                    Copy
                  </Button>
                </div>
              ) : null}
              {legacyReveal.notes != null && legacyReveal.notes !== "" ? (
                <div>
                  <Label htmlFor="reveal-notes">Notes</Label>
                  <p id="reveal-notes" className="whitespace-pre-wrap text-[13px]">
                    {legacyReveal.notes}
                  </p>
                </div>
              ) : null}
              <DialogActions>
                <Button onClick={() => setLegacyReveal(null)}>Done</Button>
              </DialogActions>
            </div>
          ) : null}
        </DialogContent>
      </Dialog>

      <Dialog open={historyFor !== null} onOpenChange={(o) => !o && setHistoryFor(null)}>
        <DialogContent
          title={`Change history — ${historyFor?.title ?? ""}`}
          description="Append-only ledger of past secret ciphertexts. Ciphertexts are never exposed."
        >
          <div className="p-4">
            {history === null ? (
              <Spinner />
            ) : history.length === 0 ? (
              <EmptyState message="No changes recorded — the secret has never been rotated." />
            ) : (
              <ul className="grid gap-1 text-[13px]">
                {history.map((h) => (
                  <li key={h.id} className="rounded-[0.5rem] border px-3 py-2" style={{ borderColor: "var(--m-border)" }}>
                    Secret changed · {new Date(h.created_at).toLocaleString()}
                  </li>
                ))}
              </ul>
            )}
          </div>
        </DialogContent>
      </Dialog>

      <FolderDialog
        mode={folderDialog}
        folder={folderForDialog}
        scope={scope}
        folders={folders ?? []}
        clients={clients ?? []}
        onSuccess={() => {
          setFolderDialog(null);
          refreshFolders();
        }}
        onClose={() => setFolderDialog(null)}
      />

      <GrantsDialog open={grantsOpen} onClose={() => setGrantsOpen(false)} folders={folders ?? []} />
    </PageFrame>
  );
}

/* ------------------------------------------------------------------ */
/* Vault tree (extends the /kb/secrets pattern for the /passwords leg). */
/* ------------------------------------------------------------------ */

function VaultTree({
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
          className="flex items-center justify-between gap-1"
          style={{ paddingLeft: depth * 14 }}
        >
          <button
            type="button"
            className={`m-entity-row min-w-0 flex-1 truncate rounded-[0.5rem] px-2 py-1 text-left text-[13px] ${selected === f.id ? "font-medium" : ""}`}
            style={{
              background: selected === f.id ? "var(--m-accent-soft)" : undefined,
              color: "var(--m-text)",
            }}
            aria-current={selected === f.id ? "true" : undefined}
            onClick={() => onSelect(f.id)}
          >
            {f.label}
            {f.is_default_seed ? (
              <span className="ml-2 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                seed
              </span>
            ) : null}
            {f.restricted ? (
              <Tag tone="warning" className="ml-2">
                restricted
              </Tag>
            ) : null}
          </button>
          {f.is_default_seed ? (
            <Button
              variant="ghost"
              size="sm"
              aria-label={`Fork ${f.label} into a client scope`}
              onClick={() => onFork(f)}
            >
              Fork
            </Button>
          ) : (
            <Button
              variant="ghost"
              size="sm"
              aria-label={`Edit ${f.label}`}
              onClick={() => onEdit(f)}
            >
              Edit
            </Button>
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
/* Record dialog — metadata-first + audited reveal + file surface.     */
/* ------------------------------------------------------------------ */

function RecordDialog({
  record,
  folders,
  scope,
  clientById,
  canReveal,
  isNew,
  onClose,
  onChanged,
}: {
  record: SecretRecord | null;
  folders: SecretFolder[];
  scope: VaultScope;
  clientById: Map<string, Client>;
  canReveal: boolean;
  isNew: boolean;
  onClose: () => void;
  onChanged: () => void;
}) {
  const [current, setCurrent] = useState<SecretRecord | null>(record && record.id !== "" ? record : null);
  const [reveal, setReveal] = useState<SecretReveal | null>(null);
  const [revealError, setRevealError] = useState<string | null>(null);
  const [revealing, setRevealing] = useState(false);
  const [confirmingReveal, setConfirmingReveal] = useState(false);
  const [editing, setEditing] = useState(false);
  const [notAvailable, setNotAvailable] = useState(false);
  const [links, setLinks] = useState<ResolvedLink[] | null>(null);
  const [linksError, setLinksError] = useState<string | null>(null);

  useEffect(() => {
    setCurrent(record && record.id !== "" ? record : null);
    setReveal(null);
    setRevealError(null);
    setEditing(isNew);
    setNotAvailable(false);
    setLinks(null);
    setLinksError(null);
    if (record === null || record.id === "") return;
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
  }, [record, isNew]);

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

  if (record === null || record.id === "") return null;

  return (
    <Dialog open={!confirmingReveal} onOpenChange={(o) => !o && onClose()}>
      <DialogContent
        title={
          isNew ? "New secret record" : current?.title ?? record.title
        }
        description={
          isNew
            ? "Materials fields are sealed server-side; plaintext never returns outside the audited reveal."
            : "Metadata-only view — plaintext exists only in the audited reveal below."
        }
      >
        {isNew ? (
          <RecordCreator
            record={record}
            scope={scope}
            canReveal={canReveal}
            onDone={() => {
              onClose();
              onChanged();
            }}
            onCancel={onClose}
          />
        ) : current === null ? (
          <div className="p-4">
            {notAvailable ? (
              <EmptyState message="This secret record is not available for your role — restricted categories deny by default until an owner or admin grants your role read access." />
            ) : (
              <Spinner />
            )}
          </div>
        ) : (
          <div className="grid max-h-[70vh] gap-4 overflow-auto p-4">
            <dl className="grid gap-1 text-[13px]">
              <MetaRow label="Category">
                {folders.find((f) => f.id === current.folder_id)?.label ?? "—"}
              </MetaRow>
              <MetaRow label="Kind">{KIND_LABELS[current.kind] ?? current.kind}</MetaRow>
              <MetaRow label="Scope">
                {current.client_id != null
                  ? clientById.get(current.client_id)?.name ?? "client"
                  : "org-level"}
              </MetaRow>
              <MetaRow label="Material fields">
                {current.fields?.filter((f) => f.has_value).map((f) => f.field_key.replaceAll("_", " ")).join(", ") ||
                  "none"}
              </MetaRow>
              <MetaRow label="Expires">
                {current.expires_at == null
                  ? "—"
                  : new Date(current.expires_at) < new Date()
                    ? "expired"
                    : new Date(current.expires_at).toLocaleDateString()}
              </MetaRow>
              <MetaRow label="Rotation">
                {current.rotate_at == null
                  ? "—"
                  : new Date(current.rotate_at) < new Date()
                    ? "rotation due"
                    : `rotate by ${new Date(current.rotate_at).toLocaleDateString()}`}
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
                  data-testid="vault-record-reveal-open"
                >
                  <LockIcon size={16} /> Reveal (audited)
                </Button>
                <Button variant="ghost" onClick={() => setEditing((v) => !v)}>
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

            {current.kind === "document" ? (
              <DocumentFilePanel record={current} canReveal={canReveal} onChanged={onChanged} />
            ) : null}

            {links !== null && linksError === null ? (
              <RecordLinksPanel recordId={current.id} links={links} onChanged={onChanged} />
            ) : linksError !== null ? (
              <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {linksError}
              </p>
            ) : (
              <div className="px-1">
                <Spinner />
              </div>
            )}

            {canReveal ? (
              <SecretSharePanel
                subjectKind="secret_record"
                subjectId={current.id}
                canManage={canReveal}
              />
            ) : null}
          </div>
        )}
      </DialogContent>

      {/* The reveal is its own confirmation, never the same click: the record
          dialog stands down while this asks, so the screen never carries two
          dimmed modals at once. */}
      <Dialog open={confirmingReveal} onOpenChange={setConfirmingReveal}>
        <DialogContent
          title="Reveal this secret?"
          description="The plaintext is decrypted for this browser only, shown here, and written to the audit trail with your name. It is never cached and never exported."
        >
          <div className="grid gap-3 p-1">
            <dl className="grid gap-1 text-[13px]">
              <MetaRow label="Record">{current?.title ?? "—"}</MetaRow>
              <MetaRow label="Fields">{sealedFieldList(current ?? ({ fields: [], object_key: null } as unknown as SecretRecord))}</MetaRow>
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
                data-testid="vault-record-reveal-confirm"
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

/* Record metadata + sealed-field rotation/clear (`value: null` clears,
   audited via the record-updated event — Wave 16 lane 9 semantics). */
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
  const [rotateAt, setRotateAt] = useState(
    record.rotate_at != null ? record.rotate_at.slice(0, 10) : "",
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
        // Moving the schedule stamps last_rotated_at server-side — the
        // rotation ledger entry rides the same write (Wave 19 lane 3).
        rotate_at: rotateAt === "" ? null : new Date(rotateAt).toISOString(),
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
        <div className="grid grid-cols-2 gap-3">
          <div>
            <Label htmlFor="rec-expires">Expires (optional)</Label>
            <Input
              id="rec-expires"
              type="date"
              value={expiresAt}
              onChange={(e) => setExpiresAt(e.target.value)}
            />
          </div>
          <div>
            <Label htmlFor="rec-rotate">Rotate by (optional)</Label>
            <Input
              id="rec-rotate"
              type="date"
              value={rotateAt}
              onChange={(e) => setRotateAt(e.target.value)}
            />
            <FieldHint>When due, the rotation sweep reminds once — then the marker waits for you to move the date forward.</FieldHint>
          </div>
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
                Clear this sealed value (<code>value: null</code> — audited)
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

/* Document-kind records: the pointer+hash object surface. The GET is the
   audited reveal-file path (secrets:edit + category gate; emits
   secret.revealed in-commit). The PUT seals and stores the ciphertext,
   pinning object_key + file_sha256 + file_bytes in the same tx. */
function DocumentFilePanel({
  record,
  canReveal,
  onChanged,
}: {
  record: SecretRecord;
  canReveal: boolean;
  onChanged: () => void;
}) {
  const [downloading, setDownloading] = useState(false);
  const [fileError, setFileError] = useState<string | null>(null);
  const [uploading, setUploading] = useState(false);
  const [uploadNote, setUploadNote] = useState<string | null>(null);

  const download = async () => {
    setDownloading(true);
    setFileError(null);
    try {
      const { data, response } = await api.api.GET("/api/v1/secrets/records/{id}/file", {
        params: { path: { id: record.id } },
        parseAs: "blob",
      });
      if (response.status !== 200 || data === undefined) {
        setFileError(
          response.status === 403
            ? "The stored file is not available with your role — document files ride the same audited gate as reveals."
            : "Failed to download the stored file.",
        );
        return;
      }
      const blob = data as Blob;
      const url = URL.createObjectURL(blob);
      const a = window.document.createElement("a");
      a.href = url;
      a.download = record.title;
      a.click();
      URL.revokeObjectURL(url);
    } finally {
      setDownloading(false);
    }
  };

  const upload = async (file: globalThis.File) => {
    setUploading(true);
    setFileError(null);
    setUploadNote(null);
    try {
      const bytes = await file.arrayBuffer();
      const { data, response } = await api.api.PUT("/api/v1/secrets/records/{id}/file", {
        params: { path: { id: record.id } },
        body: bytes as unknown as string,
        headers: { "content-type": "application/octet-stream" },
      });
      if (response.status !== 200 || data === undefined) {
        setFileError(
          response.status === 403
            ? "Uploading the stored file requires secrets:edit (and a category edit grant on restricted chains)."
            : "The file upload was rejected.",
        );
        return;
      }
      setUploadNote(
        `Sealed + stored${data.file_sha256 != null ? ` · sha256 ${data.file_sha256.slice(0, 12)}…` : ""}${data.file_bytes != null ? ` · ${data.file_bytes} bytes` : ""}`,
      );
      onChanged();
    } finally {
      setUploading(false);
    }
  };

  return (
    <Card>
      <h3 className="p-4 pb-1 text-sm font-semibold">Stored file — document kind</h3>
      <p className="px-4 pb-2 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
        Documents ride object storage as ciphertext with pointer + hash columns — never material
        blobs. Reads and writes ride the same audited gate as field reveal.
      </p>
      <dl className="grid gap-1 px-4 pb-2 text-[13px]">
        <MetaRow label="Object key">
          <span className="">{record.object_key ?? "—"}</span>
        </MetaRow>
        <MetaRow label="SHA-256">
          <span className="break-all text-[11px]">{record.file_sha256 ?? "—"}</span>
        </MetaRow>
        <MetaRow label="Bytes">
          <span className="tabular-nums">{record.file_bytes != null ? record.file_bytes.toLocaleString() : "—"}</span>
        </MetaRow>
      </dl>
      {canReveal ? (
        <div className="flex flex-wrap items-center gap-2 px-4 pb-2">
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void download()}
            disabled={downloading || record.object_key === null}
          >
            {downloading ? <Spinner /> : null} Download (audited)
          </Button>
          <label className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            Upload / rotate
            <input
              type="file"
              className="sr-only"
              onChange={(e) => {
                const f = e.target.files?.[0];
                if (f !== undefined) void upload(f);
              }}
            />
          </label>
        </div>
      ) : (
        <p className="px-4 pb-3 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
          Download and upload need secrets:edit (audited), like every reveal-path read.
        </p>
      )}
      {uploadNote !== null ? (
        <p role="status" className="px-4 pb-2 text-[12px]" style={{ color: "var(--m-status-success-fg)" }}>
          {uploadNote}
        </p>
      ) : null}
      {fileError !== null ? (
        <p role="alert" className="px-4 pb-3 text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {fileError}
        </p>
      ) : null}
    </Card>
  );
}

/* Record creator: first material fields for a NEW record (client-side
   validation only where the wire demands it; the server seals). */
function RecordCreator({
  record,
  scope,
  canReveal,
  onDone,
  onCancel,
}: {
  record: SecretRecord;
  scope: VaultScope;
  canReveal: boolean;
  onDone: () => void;
  onCancel: () => void;
}) {
  const [title, setTitle] = useState("");
  const [expiresAt, setExpiresAt] = useState("");
  const [values, setValues] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const materialKeys = record.kind === "document" ? ["notes"] : ["secret", "username", "url", "notes"];

  const submit = async () => {
    setBusy(true);
    setError(null);
    const fields = Object.entries(values)
      .filter(([k, v]) => v !== "")
      .map(([field_key, value]) => ({ field_key, value }));
    const { data, error: e } = await api.api.POST("/api/v1/secrets/records", {
      body: {
        idempotency_key: newIdempotencyKey(),
        title,
        folder_id: record.folder_id,
        kind: record.kind,
        client_id: scope.kind === "client" ? scope.id : null,
        expires_at: expiresAt === "" ? null : new Date(expiresAt).toISOString(),
        fields,
      } as components["schemas"]["CreateSecretRecordRequest"],
    });
    setBusy(false);
    if (e !== undefined || data === undefined) {
      const friendly = friendlyMutationError(e);
      setError(friendly.text);
      return;
    }
    onDone();
  };

  return (
    <div className="grid gap-3 p-4">
      <div>
        <Label htmlFor="cr-title">Title</Label>
        <Input id="cr-title" value={title} onChange={(e) => setTitle(e.target.value)} />
      </div>
      {record.kind === "document" && canReveal ? <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>Open the record after saving to upload the sealed file.</p> : null}
      {materialKeys.map((k) => (
        <div key={k}>
          <Label htmlFor={`cr-${k}`}>{FIELD_LABELS[k] ?? k}</Label>
          <Input
            id={`cr-${k}`}
            type={k === "secret" ? "password" : "text"}
            value={values[k] ?? ""}
            autoComplete="new-password"
            onChange={(e) => setValues((prev) => ({ ...prev, [k]: e.target.value }))}
          />
        </div>
      ))}
      <div>
        <Label htmlFor="cr-expires">Expires (optional)</Label>
        <Input id="cr-expires" type="date" value={expiresAt} onChange={(e) => setExpiresAt(e.target.value)} />
      </div>
      {error !== null ? (
        <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </p>
      ) : null}
      <DialogActions>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button onClick={() => void submit()} disabled={busy || title.trim() === ""}>
          {busy ? <Spinner /> : null} Create record
        </Button>
      </DialogActions>
    </div>
  );
}

/* Record links via the RecordLink seam with the shared cross-entity
   picker. Broken targets render the "deleted target" semantic. */
function RecordLinksPanel({
  recordId,
  links,
  onChanged,
}: {
  recordId: string;
  links: ResolvedLink[];
  onChanged: () => void;
}) {
  const [kind, setKind] = useState<components["schemas"]["LinkKind"]>("asset");
  const [target, setTarget] = useState("");
  const [pickerOpen, setPickerOpen] = useState(false);
  const [linkError, setLinkError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Secret-record targets need the vault-tree picker (the 0078 records
  // are outside the cross-entity search seams) — closed by Wave 19
  // lane 3.
  const secretPicker = kind === "secret";

  const add = async (t: LinkTarget) => {
    setLinkError(null);
    setBusy(true);
    const { error } = await api.api.POST("/api/v1/secrets/records/{id}/links", {
      params: { path: { id: recordId } },
      body: { to_kind: t.kind, to_id: t.id },
    });
    setBusy(false);
    if (error !== undefined) {
      setLinkError("The server rejected this link. The target must exist and be visible in your scope.");
      return;
    }
    onChanged();
  };

  const addTyped = async () => {
    const uuid = parseUuid(target.trim());
    if (uuid === null) {
      setLinkError("Paste a target id (UUID) — or use the picker.");
      return;
    }
    void add({ kind, id: uuid, title: uuid, meta: "pasted id" });
    setTarget("");
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
              className="m-entity-row flex items-center justify-between gap-2 rounded-[0.5rem] border px-3 py-2 text-[13px]"
              style={{ borderColor: "var(--m-border)" }}
            >
              <span className="min-w-0">
                <Tag tone="outline">{l.to_kind.replaceAll("_", " ")}</Tag>{" "}
                {l.target_deleted ? (
                  <span style={{ color: "var(--m-status-danger-fg)" }}>deleted target</span>
                ) : (
                  <span className="truncate">{l.target_label ?? "target"}</span>
                )}
              </span>
              <Button
                variant="ghost"
                size="sm"
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
        <Button
          variant="secondary"
          size="sm"
          onClick={() => setPickerOpen(true)}
          disabled={busy}
        >
          Add link — pick a target
        </Button>
        <div className="flex gap-2">
          <Select value={kind} onValueChange={(v) => setKind(v as components["schemas"]["LinkKind"])}>
            <SelectTrigger aria-label="Link target kind" className="w-44 whitespace-nowrap">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {LINK_KINDS.map((k) => (
                <SelectItem key={k} value={k}>
                  {k.replaceAll("_", " ")}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Input
            aria-label="Link target id"
            placeholder="or paste a target id…"
            value={target}
            onChange={(e) => setTarget(e.target.value)}
          />
          <Button variant="ghost" size="sm" onClick={() => void addTyped()} disabled={target.trim() === "" || busy}>
            Paste
          </Button>
        </div>
        {linkError !== null ? (
          <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {linkError}
          </p>
        ) : null}
      </div>
      {secretPicker ? (
        <SecretTargetPicker
          onPick={(recordId) => {
            void add({ kind: "secret", id: recordId, title: recordId, meta: "vault-tree" });
          }}
          onClose={() => setPickerOpen(false)}
        />
      ) : (
        <EntityPicker
          open={pickerOpen}
          defaultKind={kind}
          onPick={(t) => void add(t)}
          onClose={() => setPickerOpen(false)}
        />
      )}
    </Card>
  );
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
  scope: VaultScope;
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
        setError("The fork was rejected — a same-slug fork may already exist for this client.");
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
                      className="flex items-center justify-between gap-2 rounded-[0.5rem] border px-3 py-2 text-[13px]"
                      style={{ borderColor: "var(--m-border)" }}
                    >
                      <span className="min-w-0 truncate">
                        <code>{g.role_id.slice(0, 8)}</code> ·{" "}
                        {(folders).find((f) => f.id === g.folder_id)?.label ?? "folder"} ·{" "}
                        {g.can_edit ? "read+edit" : "read"}
                      </span>
                      <Button variant="ghost" size="sm" onClick={() => void revoke(g)} aria-label="Revoke grant">
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

/* ------------------------------------------------------------------ */
/* Legacy PasswordEntry create/edit (byte-compatible). Legacy ids ride │
/* fresh idempotency keys; edits PATCH without the secret.              */
/* ------------------------------------------------------------------ */

function LegacyVaultForm({
  editing,
  clients,
  onDone,
}: {
  editing: PasswordEntry | null;
  clients: Client[];
  onDone: () => void;
}) {
  const [title, setTitle] = useState(editing?.title ?? "");
  const [username, setUsername] = useState(editing?.username ?? "");
  const [url, setUrl] = useState(editing?.url ?? "");
  const [secret, setSecret] = useState("");
  const [notes, setNotes] = useState("");
  const [notesTouched, setNotesTouched] = useState(false);
  const [expiresAt, setExpiresAt] = useState(
    editing?.expires_at != null ? editing.expires_at.slice(0, 10) : "",
  );
  const [rotateAt, setRotateAt] = useState(
    editing?.rotate_at != null ? editing.rotate_at.slice(0, 10) : "",
  );
  const [clientId, setClientId] = useState<string>(editing?.client_id ?? "global");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      if (editing === null) {
        const result = await runEnvelopeMutation<
          components["schemas"]["CreatePasswordRequest"],
          components["schemas"]["PasswordMutationResponse"]
        >({
          describe: "Create vault credential",
          buildBody: (base) => ({
            idempotency_key: base.idempotency_key,
            title,
            username,
            url,
            secret,
            notes: notes === "" ? null : notes,
            client_id: clientId === "global" ? null : clientId,
            expires_at: expiresAt === "" ? null : new Date(expiresAt).toISOString(),
            rotate_at: rotateAt === "" ? null : new Date(rotateAt).toISOString(),
          }),
          optimistic: () => {},
          revert: () => {},
          request: (client, body) => client.POST("/api/v1/passwords", { body }),
          versions: (response) => [[response.id, response.mutation.version]],
        });
        if (!result.ok) {
          setError(rejectionText(result, "The credential was rejected."));
          setBusy(false);
          return;
        }
      } else {
        const { error: e } = await api.api.PATCH("/api/v1/passwords/{id}", {
          params: { path: { id: editing.id } },
          body: {
            idempotency_key: crypto.randomUUID(),
            title,
            username,
            url,
            ...(secret === "" ? {} : { secret }),
            ...(notesTouched ? { notes: notes === "" ? null : notes } : {}),
            expires_at: expiresAt === "" ? null : new Date(expiresAt).toISOString(),
            rotate_at: rotateAt === "" ? null : new Date(rotateAt).toISOString(),
          },
        });
        if (e !== undefined) {
          setError("The update was rejected — retry.");
          setBusy(false);
          return;
        }
      }
    } finally {
      setBusy(false);
    }
    onDone();
  };

  return (
    <div className="grid gap-3 p-4">
      <div>
        <Label htmlFor="pw-title">Title</Label>
        <Input id="pw-title" value={title} onChange={(ev) => setTitle(ev.target.value)} />
      </div>
      <div className="grid grid-cols-2 gap-3">
        <div>
          <Label htmlFor="pw-username">Username</Label>
          <Input id="pw-username" value={username} onChange={(ev) => setUsername(ev.target.value)} />
        </div>
        <div>
          <Label htmlFor="pw-url">URL</Label>
          <Input id="pw-url" value={url} onChange={(ev) => setUrl(ev.target.value)} />
        </div>
      </div>
      <div>
        <Label htmlFor="pw-secret">{editing === null ? "Secret" : "New secret (blank = keep current)"}</Label>
        <Input
          id="pw-secret"
          type="password"
          value={secret}
          onChange={(ev) => setSecret(ev.target.value)}
          autoComplete="new-password"
        />
      </div>
      <div>
        <Label htmlFor="pw-notes">Notes (encrypted)</Label>
        <textarea
          id="pw-notes"
          value={notes}
          onChange={(ev) => {
            setNotes(ev.target.value);
            setNotesTouched(true);
          }}
          rows={2}
          className="w-full rounded-[0.5rem] border bg-transparent p-2 text-[13px]"
          style={{ borderColor: "var(--m-border)" }}
        />
        <FieldHint>Replaced wholesale when edited; cleared with an empty value.</FieldHint>
      </div>
      <div className="grid grid-cols-2 gap-3">
        <div>
          <Label htmlFor="pw-scope">Scope</Label>
          <Select value={clientId} onValueChange={setClientId}>
            <SelectTrigger id="pw-scope">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="global">Org-level</SelectItem>
              {clients.map((c) => (
                <SelectItem key={c.id} value={c.id}>
                  {c.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div>
          <Label htmlFor="pw-expires">Expires (optional)</Label>
          <Input id="pw-expires" type="date" value={expiresAt} onChange={(ev) => setExpiresAt(ev.target.value)} />
        </div>
      </div>
      <div className="grid grid-cols-2 gap-3">
        <div className="col-span-2">
          <Label htmlFor="pw-rotate">Rotate by (optional)</Label>
          <Input id="pw-rotate" type="date" value={rotateAt} onChange={(ev) => setRotateAt(ev.target.value)} />
          <FieldHint>When due, the rotation sweep reminds you once in-app — move the date forward after rotating to re-arm it.</FieldHint>
        </div>
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
        <Button
          onClick={() => void submit()}
          disabled={busy || title.trim() === "" || (editing === null && secret === "")}
        >
          {busy ? <Spinner /> : null} {editing === null ? "Add credential" : "Save changes"}
        </Button>
      </DialogActions>
    </div>
  );
}
