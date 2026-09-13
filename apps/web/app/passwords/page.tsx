// /passwords — the staff password vault (docs-platform v1).
//
// Purpose: org- and client-scoped credential entries. Secrets are
// encrypted at rest; the ONLY plaintext path is the audited reveal
// endpoint (POST /passwords/{id}/reveal) — every reveal is recorded as
// provenance ("who looked, when") and the UI shows that inline so nobody
// copies a credential without a trail. Rotation archives the previous
// ciphertext to an append-only history.
//
// Non-purpose: no sharing links or rotation automation yet (TASKLIST
// follow-ups); no plaintext ever persists in local state beyond the
// open reveal dialog.
"use client";

import { rejectionText } from "../../lib/error-copy";
import {
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
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
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, useRequireAuth } from "../../lib/api";
import { runEnvelopeMutation } from "../../lib/envelope-write";

type PasswordEntry = components["schemas"]["PasswordEntry"];
type Client = components["schemas"]["Client"];
type PasswordHistoryEntry = components["schemas"]["PasswordHistoryEntry"];
type PasswordReveal = components["schemas"]["PasswordReveal"];

type Scope = { kind: "all" } | { kind: "global" } | { kind: "client"; id: string };

export default function PasswordsPage() {
  const auth = useRequireAuth();
  const [entries, setEntries] = useState<PasswordEntry[] | null>(null);
  const [clients, setClients] = useState<Client[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [scope, setScope] = useState<Scope>({ kind: "all" });
  const [q, setQ] = useState("");

  const [editOpen, setEditOpen] = useState(false);
  const [editing, setEditing] = useState<PasswordEntry | null>(null);

  const [reveal, setReveal] = useState<(PasswordEntry & { secret: string; notes?: string | null }) | null>(null);
  const [revealing, setRevealing] = useState<string | null>(null);
  const [historyFor, setHistoryFor] = useState<PasswordEntry | null>(null);
  const [history, setHistory] = useState<PasswordHistoryEntry[] | null>(null);

  const clientById = useMemo(() => new Map((clients ?? []).map((c) => [c.id, c])), [clients]);

  const refresh = useCallback(async () => {
    const [p, c] = await Promise.all([
      api.api.GET("/api/v1/passwords"),
      api.api.GET("/api/v1/clients"),
    ]);
    if (p.error !== undefined || c.error !== undefined) {
      setLoadError("Failed to load the vault.");
      return;
    }
    setEntries(p.data ?? []);
    setClients(c.data ?? []);
    setLoadError(null);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  const filtered = useMemo(() => {
    if (entries === null) return null;
    const needle = q.trim().toLowerCase();
    return entries.filter((e) => {
      if (scope.kind === "global" && e.client_id !== null) return false;
      if (scope.kind === "client" && e.client_id !== scope.id) return false;
      if (needle === "") return true;
      return (
        e.title.toLowerCase().includes(needle) ||
        (e.username ?? "").toLowerCase().includes(needle) ||
        (e.url ?? "").toLowerCase().includes(needle)
      );
    });
  }, [entries, scope, q]);

  const expired = (e: PasswordEntry) => e.expires_at != null && new Date(e.expires_at) < new Date();

  const doReveal = async (entry: PasswordEntry) => {
    setRevealing(entry.id);
    const { data, error } = await api.api.POST("/api/v1/passwords/{id}/reveal", {
      params: { path: { id: entry.id } },
    });
    setRevealing(null);
    if (error !== undefined || data === undefined) {
      setLoadError("Reveal was rejected — you need vault edit rights.");
      return;
    }
    setReveal({ ...entry, secret: data.secret, notes: data.notes ?? null });
  };

  const openHistory = async (entry: PasswordEntry) => {
    setHistoryFor(entry);
    setHistory(null);
    const { data } = await api.api.GET("/api/v1/passwords/{id}/history", {
      params: { path: { id: entry.id } },
    });
    if (data !== undefined) setHistory(data);
  };

  const remove = async (entry: PasswordEntry) => {
    if (!window.confirm(`Delete “${entry.title}” from the vault? Its change history goes with it.`)) {
      return;
    }
    const { error } = await api.api.DELETE("/api/v1/passwords/{id}", {
      params: { path: { id: entry.id } },
    });
    if (error === undefined) await refresh();
  };

  if (auth.status !== "authed") return null;

  return (
    <div className="mx-auto grid w-full max-w-[1400px] gap-8 p-4 md:p-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold">Password vault</h1>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Org and client credentials, encrypted at rest. Every reveal is audited.
          </p>
        </div>
        <Button
          onClick={() => {
            setEditing(null);
            setEditOpen(true);
          }}
        >
          New credential
        </Button>
      </div>

      <Card>
        <div className="flex flex-wrap items-center gap-2 p-4 pb-0">
          <Input
            aria-label="Search vault"
            placeholder="Search title, user, URL…"
            value={q}
            onChange={(e) => setQ(e.target.value)}
            className="w-64"
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
              <SelectItem value="global">Org-level</SelectItem>
              {(clients ?? []).map((c) => (
                <SelectItem key={c.id} value={c.id}>
                  {c.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
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
            <EmptyState message="No credentials match. Vault entries can be org-level or scoped to a client; secrets are encrypted at rest and revealed only through the audited path." />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Title</Th>
                <Th>Username</Th>
                <Th>Scope</Th>
                <Th>Expires</Th>
                <Th className="text-right">Actions</Th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((e) => (
                <Tr key={e.id}>
                  <Td>
                    <span className="font-medium">{e.title}</span>
                    {(e.url ?? "") !== "" ? (
                      <a
                        href={e.url}
                        target="_blank"
                        rel="noreferrer"
                        className="ml-2 underline decoration-[var(--m-border)] underline-offset-2"
                        style={{ color: "var(--m-text-secondary)" }}
                      >
                        open
                      </a>
                    ) : null}
                  </Td>
                  <Td>{(e.username ?? "") !== "" ? e.username : <span style={{ color: "var(--m-text-muted)" }}>—</span>}</Td>
                  <Td>
                    {e.client_id != null && clientById.get(e.client_id) ? (
                      <Link
                        href={`/clients/${e.client_id}`}
                        className="underline decoration-[var(--m-border)] underline-offset-2"
                      >
                        {clientById.get(e.client_id)!.name}
                      </Link>
                    ) : (
                      <span style={{ color: "var(--m-text-muted)" }}>Org-level</span>
                    )}
                  </Td>
                  <Td>
                    {e.expires_at == null ? (
                      <span style={{ color: "var(--m-text-muted)" }}>—</span>
                    ) : expired(e) ? (
                      <Tag tone="danger">expired</Tag>
                    ) : (
                      new Date(e.expires_at).toLocaleDateString()
                    )}
                  </Td>
                  <Td className="text-right">
                    <div className="flex justify-end gap-1">
                      <Button variant="secondary" onClick={() => void doReveal(e)} disabled={revealing === e.id}>
                        {revealing === e.id ? <Spinner /> : null} Reveal
                      </Button>
                      <Button
                        variant="ghost"
                        onClick={() => {
                          setEditing(e);
                          setEditOpen(true);
                        }}
                      >
                        Edit
                      </Button>
                      <Button variant="ghost" onClick={() => void openHistory(e)}>
                        History
                      </Button>
                      <Button variant="ghost" onClick={() => void remove(e)}>
                        Delete
                      </Button>
                    </div>
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </Card>

      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent
          title={editing === null ? "New credential" : "Edit credential"}
          description={
            editing === null
              ? "Secrets are encrypted at rest and only ever returned through the audited reveal action."
              : "Leave the secret blank to keep the current one. Any change archives the previous secret to history."
          }
        >
          <VaultForm
            editing={editing}
            clients={clients ?? []}
            onDone={() => {
              setEditOpen(false);
              void refresh();
            }}
          />
        </DialogContent>
      </Dialog>

      <Dialog open={reveal !== null} onOpenChange={(o) => !o && setReveal(null)}>
        <DialogContent
          title="Credential revealed"
          description="This reveal was recorded in the audit trail. Close the dialog when you've copied what you need — the plaintext is not stored anywhere else."
        >
          {reveal !== null ? (
            <div className="grid gap-3 p-4">
              <div>
                <Label htmlFor="reveal-secret">Secret</Label>
                <div className="flex gap-2">
                  <Input id="reveal-secret" readOnly value={reveal.secret} className="font-mono" />
                  <Button
                    variant="secondary"
                    onClick={() => void navigator.clipboard.writeText(reveal.secret)}
                  >
                    Copy
                  </Button>
                </div>
              </div>
              {(reveal.username ?? "") !== "" ? (
                <div className="text-[13px]">
                  <span style={{ color: "var(--m-text-secondary)" }}>Username: </span>
                  {reveal.username}
                  <Button
                    variant="ghost"
                    className="ml-2"
                    onClick={() => void navigator.clipboard.writeText(reveal.username ?? "")}
                  >
                    Copy
                  </Button>
                </div>
              ) : null}
              {reveal.notes != null && reveal.notes !== "" ? (
                <div>
                  <Label htmlFor="reveal-notes">Notes</Label>
                  <p id="reveal-notes" className="whitespace-pre-wrap text-[13px]">
                    {reveal.notes}
                  </p>
                </div>
              ) : null}
              <DialogActions>
                <Button onClick={() => setReveal(null)}>Done</Button>
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
                  <li key={h.id} className="rounded-[2px] border px-3 py-2" style={{ borderColor: "var(--m-border)" }}>
                    Secret changed · {new Date(h.created_at).toLocaleString()}
                  </li>
                ))}
              </ul>
            )}
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function VaultForm({
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
  const [clientId, setClientId] = useState(editing?.client_id ?? "global");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    setBusy(true);
    setError(null);
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
        }),
        optimistic: () => {},
        revert: () => {},
        request: (client, body) => client.POST("/api/v1/passwords", { body }),
        versions: (response) => [[response.id, response.mutation.version]],
      });
      setBusy(false);
      if (!result.ok) {
        setError(rejectionText(result, "The credential was rejected."));
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
        },
      });
      setBusy(false);
      if (e !== undefined) {
        setError("The update was rejected — retry.");
        return;
      }
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
          className="w-full rounded-[2px] border bg-transparent p-2 text-[13px]"
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
