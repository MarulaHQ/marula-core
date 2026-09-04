// /clients — staff CRM: the client list (Workspace-core UI, T3).
//
// Purpose: every client in the org with client-side search, per-client
// visibility hints (restricted_to), and the gated client-create dialog
// riding the IF1–IF3 envelope. Links into /clients/[id] for sites,
// contacts, assets, and portal users.
// Non-purpose: no editing of the visibility rule here (detail surface
// shows the hint; rule editing follows the visibility-rules backend UI).
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
import { api, useAuth, useRequireAuth } from "../../lib/api";
import { runEnvelopeMutation } from "../../lib/envelope-write";

type Client = components["schemas"]["Client"];
type ClientMutationResponse = components["schemas"]["ClientMutationResponse"];

export default function ClientsPage() {
  const auth = useRequireAuth();
  const { me } = useAuth();
  const [clients, setClients] = useState<Client[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [query, setQuery] = useState("");

  const [createOpen, setCreateOpen] = useState(false);
  const [name, setName] = useState("");
  const [notes, setNotes] = useState("");
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);

  const isAdmin = me?.permissions.includes("special:manage_users") ?? false;

  const refresh = useCallback(async () => {
    const { data, error } = await api.api.GET("/api/v1/clients");
    if (error !== undefined || data === undefined) {
      setLoadError("Failed to load clients.");
      setClients([]);
      return;
    }
    setClients(data);
    setLoadError(null);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  const filtered = useMemo(() => {
    if (clients === null) return null;
    const q = query.trim().toLowerCase();
    if (q === "") return clients;
    return clients.filter((c) => c.name.toLowerCase().includes(q));
  }, [clients, query]);

  const createClient = async () => {
    if (name.trim() === "") return;
    setCreating(true);
    setCreateError(null);
    const tempId = `pending-client:${name}`;
    const optimistic: Client = {
      id: tempId,
      org_id: "",
      name: name.trim(),
      notes: notes.trim() === "" ? null : notes.trim(),
      restricted_to: [],
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };
    setClients((prev) => [optimistic, ...(prev ?? [])]);
    const result = await runEnvelopeMutation<
      components["schemas"]["CreateClientRequest"],
      ClientMutationResponse
    >({
      describe: "Create client",
      entityId: tempId,
      optimistic: () => {},
      revert: () => setClients((prev) => (prev ?? []).filter((c) => c.id !== tempId)),
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        name: name.trim(),
        notes: notes.trim() === "" ? null : notes.trim(),
      }),
      request: (client, body) => client.POST("/api/v1/clients", { body }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setCreating(false);
    if (!result.ok) {
      setCreateError(result.message ?? "The client was rejected. Nothing was saved.");
      return;
    }
    setCreateOpen(false);
    setName("");
    setNotes("");
    void refresh();
  };

  if (auth.status !== "authed") return null;

  return (
    <div className="grid gap-8">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-lg font-semibold">Clients</h1>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Organizations you serve: sites, contacts, assets, and portal access.
          </p>
        </div>
        <Dialog
          open={createOpen}
          onOpenChange={(open) => {
            setCreateOpen(open);
            if (!open) setCreateError(null);
          }}
        >
          <DialogTrigger asChild>
            <Button>New client</Button>
          </DialogTrigger>
          <DialogContent title="New client" description="Create the client record, then add sites, contacts, and assets on its page.">
            <div className="grid gap-3 p-4">
              <div>
                <Label htmlFor="client-name">Name</Label>
                <Input
                  id="client-name"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="Acme Manufacturing"
                />
              </div>
              <div>
                <Label htmlFor="client-notes">Notes</Label>
                <Input
                  id="client-notes"
                  value={notes}
                  onChange={(e) => setNotes(e.target.value)}
                  placeholder="Optional"
                />
                <FieldHint>Internal notes; never shown to portal users.</FieldHint>
              </div>
              {createError !== null ? (
                <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                  {createError}
                </p>
              ) : null}
              <DialogActions>
                <Button variant="ghost" onClick={() => setCreateOpen(false)}>
                  Cancel
                </Button>
                <Button onClick={() => void createClient()} disabled={creating || name.trim() === ""}>
                  {creating ? <Spinner /> : null} Create client
                </Button>
              </DialogActions>
            </div>
          </DialogContent>
        </Dialog>
      </div>

      <Card>
        <div className="flex items-center justify-between p-4 pb-0">
          <h2 className="text-sm font-semibold">All clients</h2>
          <Input
            aria-label="Search clients"
            placeholder="Search by name…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="w-56"
          />
        </div>
        {filtered === null ? (
          <div className="p-4">
            {loadError !== null ? (
              <p role="alert" className="text-[13px]">{loadError}</p>
            ) : (
              <Spinner />
            )}
          </div>
        ) : filtered.length === 0 ? (
          <div className="p-4">
            <EmptyState
              message={
                query.trim() === ""
                  ? "No clients yet. Create the first client to attach sites, contacts, and contracts."
                  : "No clients match that search."
              }
            />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Name</Th>
                <Th>Visibility</Th>
                <Th>Notes</Th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((client) => (
                <Tr key={client.id}>
                  <Td>
                    <Link
                      href={`/clients/${client.id}`}
                      className="underline decoration-[var(--m-border)] underline-offset-2"
                    >
                      {client.name}
                    </Link>
                  </Td>
                  <Td>
                    {client.restricted_to === null || client.restricted_to.length === 0 ? (
                      <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                        All members
                      </span>
                    ) : (
                      <Tag title={isAdmin ? `Visible to ${client.restricted_to.length} listed users + admins` : "Restricted"}>
                        Restricted
                      </Tag>
                    )}
                  </Td>
                  <Td>
                    <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                      {client.notes ?? "—"}
                    </span>
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </Card>
    </div>
  );
}
