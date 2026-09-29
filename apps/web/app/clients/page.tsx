// /clients — staff CRM: the client list (Wave 17 UX excellence pass).
//
// Purpose: every client in the org, CRM-grade: a stat strip (total +
// restricted), name search, and a dense table with monogram identity,
// visibility pills, and hover micro-feedback riding the Tangerine
// tokens. Per-client documentation completeness lives on the detail
// page, where the tab data to score it is actually loaded. The
// client-create dialog rides the IF1–IF3 envelope; list motion is
// transform/opacity ≤120ms with staggered entrances, collapsed under
// prefers-reduced-motion.
// Non-purpose: no editing of the visibility rule here (detail surface
// shows the hint; rule editing follows the visibility-rules backend UI).
"use client";

import { rejectionText } from "../../lib/error-copy";
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
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, useAuth, useRequireAuth } from "../../lib/api";
import { runEnvelopeMutation } from "../../lib/envelope-write";

type Client = components["schemas"]["Client"];
type ClientMutationResponse = components["schemas"]["ClientMutationResponse"];

function Monogram({ name }: { name: string }) {
  const initials = name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]!.toUpperCase())
    .join("");
  return (
    <span
      aria-hidden="true"
      className="grid h-8 w-8 shrink-0 place-items-center rounded-[999px] text-[12px] font-semibold"
      style={{ background: "var(--m-accent-soft)", color: "var(--m-accent-fg)" }}
    >
      {initials}
    </span>
  );
}

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

  const isAdmin = (me?.permissions ?? []).includes("special:manage_users") ?? false;

  const refresh = useCallback(async () => {
    const { data, error } = await api.api.GET("/api/v1/clients");
    if (error !== undefined || data === undefined) {
      setLoadError("Failed to load clients.");
      setClients([]);
      return;
    }
    // Pre-guard (lane 10 sweep): a resolving read can carry a non-array —
    // an honest empty roster, never a crash.
    setClients(Array.isArray(data) ? data : []);
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

  const stats = useMemo(() => {
    if (clients === null) return null;
    const restricted = clients.filter((c) => (c.restricted_to ?? []).length > 0).length;
    return { total: clients.length, restricted };
  }, [clients]);

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
      setCreateError(rejectionText(result, "The client was rejected. Nothing was saved."));
      return;
    }
    setCreateOpen(false);
    setName("");
    setNotes("");
    void refresh();
  };

  if (auth.status !== "authed") return null;

  return (
    <div className="mx-auto grid w-full grid-cols-1 max-w-[1400px] gap-6 p-4 md:p-6">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold">Clients</h1>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Organizations you serve: sites, contacts, assets, documents, and portal access.
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
            <Button data-testid="new-record">New client</Button>
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

      {stats !== null ? (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          <div
            className="m-entity-rise rounded-[0.75rem] border p-4"
            style={{ borderColor: "var(--m-border)", "--stagger-i": 0 } as React.CSSProperties}
            data-testid="clients-stat-total"
          >
            <p className="text-[11px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Total clients
            </p>
            <p className="mt-1 text-2xl font-semibold tabular-nums">{stats.total}</p>
          </div>
          <div
            className="m-entity-rise rounded-[0.75rem] border p-4"
            style={{ borderColor: "var(--m-border)", "--stagger-i": 1 } as React.CSSProperties}
            data-testid="clients-stat-restricted"
          >
            <p className="text-[11px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Restricted visibility
            </p>
            <p className="mt-1 text-2xl font-semibold tabular-nums">{stats.restricted}</p>
          </div>
          <div
            className="m-entity-rise rounded-[0.75rem] border p-4"
            style={{ borderColor: "var(--m-border)", "--stagger-i": 2 } as React.CSSProperties}
            data-testid="clients-stat-visible"
          >
            <p className="text-[11px] font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Visible to all members
            </p>
            <p className="mt-1 text-2xl font-semibold tabular-nums">{stats.total - stats.restricted}</p>
          </div>
        </div>
      ) : null}

      <Card className="m-entity-rise" style={{ "--stagger-i": 3 } as React.CSSProperties}>
        <div className="flex flex-wrap items-center justify-between gap-3 p-4 pb-0">
          <h2 className="whitespace-nowrap text-sm font-semibold">All clients</h2>
          <Input
            aria-label="Search clients"
            placeholder="Search by name…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="min-w-48 flex-1 sm:max-w-72"
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
          <div className="overflow-x-auto">
            <Table>
            <thead>
              <tr>
                <Th>Client</Th>
                <Th>Visibility</Th>
                <Th>Notes</Th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((client, i) => (
                <Tr key={client.id} className="m-entity-row">
                  <Td>
                    <span className="flex items-center gap-3">
                      <Monogram name={client.name} />
                      <Link
                        href={`/clients/${client.id}`}
                        className="font-medium underline decoration-transparent underline-offset-2 transition-colors duration-[100ms] hover:decoration-[var(--m-border-strong)]"
                      >
                        {client.name}
                      </Link>
                    </span>
                  </Td>
                  <Td>
                    {!Array.isArray(client.restricted_to) || client.restricted_to.length === 0 ? (
                      <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                        All members
                      </span>
                    ) : (
                      <Tag tone="warning" title={isAdmin ? `Visible to ${client.restricted_to.length} listed users + admins` : "Restricted"}>
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
          </div>
        )}
      </Card>
    </div>
  );
}
