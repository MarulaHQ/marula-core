// Client + contact pod for ticket detail.
//
// Purpose: view the ticket's client and contact with click-through to the
// main records (/clients/[id]; contacts live on the client record's
// Contacts tab), and change them via a client picker + client-scoped
// searchable contact picker (current contact preselected) with an inline
// create-contact path. All reads/writes ride the typed SDK client; the
// PATCH rides the instant-first envelope (optimistic ticket patch +
// authoritative reconcile handled by the page's mutate).
// Non-purpose: no client/contact admin surfaces here (they live under
// /clients).

"use client";

import type { components } from "@meridian/sdk";
import { Button, Input, Label, Spinner } from "@meridian/ui";
import Link from "next/link";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { api } from "../../lib/api";

type Client = components["schemas"]["Client"];
type Contact = components["schemas"]["Contact"];

export interface ClientContactProps {
  clientId: string | null;
  contactId: string | null;
  /** Applies the change through the page's instant-first mutate. */
  onChange(patch: { client_id?: string | null; contact_id?: string | null }): Promise<void>;
  /** Refresh the detail model after a client change (contact list scope). */
  onClientChanged(): void;
}

type LoadState<T> = { status: "loading" } | { status: "error"; message: string } | { status: "ready"; data: T };

function useClient(clientId: string | null): LoadState<Client | null> {
  const [state, setState] = useState<LoadState<Client | null>>({ status: "loading" });
  useEffect(() => {
    let cancelled = false;
    setState({ status: "loading" });
    if (clientId === null) {
      setState({ status: "ready", data: null });
      return;
    }
    api.api
      .GET("/api/v1/clients/{id}", { params: { path: { id: clientId } } })
      .then(({ data, error }) => {
        if (cancelled) return;
        if (error !== undefined) {
          setState({ status: "error", message: "Could not load client" });
          return;
        }
        setState({ status: "ready", data: data ?? null });
      })
      .catch(() => !cancelled && setState({ status: "error", message: "Could not load client" }));
    return () => {
      cancelled = true;
    };
  }, [clientId]);
  return state;
}

function useContacts(clientId: string | null, version: number): LoadState<Contact[]> {
  const [state, setState] = useState<LoadState<Contact[]>>({ status: "loading" });
  useEffect(() => {
    let cancelled = false;
    setState({ status: "loading" });
    if (clientId === null) {
      setState({ status: "ready", data: [] });
      return;
    }
    api.api
      .GET("/api/v1/clients/{id}/contacts", { params: { path: { id: clientId } } })
      .then(({ data, error }) => {
        if (cancelled) return;
        if (error !== undefined) {
          setState({ status: "error", message: "Could not load contacts" });
          return;
        }
        setState({ status: "ready", data: data ?? [] });
      })
      .catch(() => !cancelled && setState({ status: "error", message: "Could not load contacts" }));
    return () => {
      cancelled = true;
    };
  }, [clientId, version]);
  return state;
}

export function ClientContactPod(props: ClientContactProps): ReactNode {
  const { clientId, contactId, onChange } = props;
  const clientState = useClient(clientId);
  const [contactsVersion, bumpContacts] = useState(0);
  const contactsState = useContacts(clientId, contactsVersion);
  const [pickingClient, setPickingClient] = useState(false);
  const [pickingContact, setPickingContact] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const client = clientState.status === "ready" ? clientState.data : null;
  const contacts = contactsState.status === "ready" ? contactsState.data : [];
  const contact = useMemo(
    () => contacts.find((c) => c.id === contactId) ?? null,
    [contacts, contactId],
  );

  async function apply(patch: { client_id?: string | null; contact_id?: string | null }): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      await onChange(patch);
    } catch {
      setError("Change rejected");
    } finally {
      setBusy(false);
    }
  }

  return (
    <section
      className="grid content-start gap-2 rounded-[2px] border p-3"
      style={{ borderColor: "var(--m-border)" }}
      data-testid="pod-client-contact"
      aria-label="Client and contact"
    >
      <div className="flex items-center justify-between">
        <h2 className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
          Client &amp; contact
        </h2>
      </div>
      {clientState.status === "loading" && <Spinner />}
      {clientState.status === "error" && (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }}>{clientState.message}</p>
      )}
      {clientState.status === "ready" && (
        <dl className="grid gap-1 text-xs">
          <div className="flex items-center gap-2">
            <dt style={{ color: "var(--m-text-muted)" }} className="w-14 shrink-0">Client</dt>
            <dd className="min-w-0 flex-1 truncate">
              {client !== null ? (
                <Link href={`/clients/${client.id}`} className="underline-offset-2 hover:underline">
                  {client.name}
                </Link>
              ) : (
                <span style={{ color: "var(--m-text-muted)" }}>None</span>
              )}
            </dd>
            <Button size="sm" variant="ghost" onClick={() => setPickingClient(true)} data-testid="change-client">
              Change
            </Button>
          </div>
          <div className="flex items-center gap-2">
            <dt style={{ color: "var(--m-text-muted)" }} className="w-14 shrink-0">Contact</dt>
            <dd className="min-w-0 flex-1 truncate">
              {contact !== null ? (
                <Link href={`/clients/${contact.client_id}?tab=contacts`} className="underline-offset-2 hover:underline" title={contact.email ?? undefined}>
                  {contact.name}
                </Link>
              ) : (
                <span style={{ color: "var(--m-text-muted)" }}>None</span>
              )}
            </dd>
            <Button
              size="sm"
              variant="ghost"
              disabled={client === null || busy}
              title={client === null ? "Pick a client first" : undefined}
              onClick={() => setPickingContact(true)}
              data-testid="change-contact"
            >
              Change
            </Button>
          </div>
        </dl>
      )}
      {error !== null && (
        <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">{error}</p>
      )}

      {pickingClient && (
        <ClientPicker
          currentId={clientId}
          busy={busy}
          onCancel={() => setPickingClient(false)}
          onPick={async (id) => {
            await apply({ client_id: id, contact_id: null });
            setPickingClient(false);
          }}
        />
      )}
      {pickingContact && client !== null && (
        <ContactPicker
          clientId={client.id}
          clientName={client.name}
          currentId={contactId}
          busy={busy}
          onCancel={() => setPickingContact(false)}
          onPick={async (id) => {
            await apply({ contact_id: id });
            setPickingContact(false);
            bumpContacts((v) => v + 1);
          }}
        />
      )}
    </section>
  );
}

function ClientPicker({
  currentId,
  busy,
  onPick,
  onCancel,
}: {
  currentId: string | null;
  busy: boolean;
  onPick(id: string): Promise<void>;
  onCancel(): void;
}): ReactNode {
  const [search, setSearch] = useState("");
  const [clients, setClients] = useState<Client[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    // The clients list endpoint takes no query params; search filters client-side.
    api.api
      .GET("/api/v1/clients")
      .then(({ data, error: err }) => {
        if (cancelled) return;
        if (err !== undefined) {
          setError("Could not load clients");
          setClients([]);
          return;
        }
        const rows = data ?? [];
        const q = search.trim().toLowerCase();
        setClients(q === "" ? rows : rows.filter((c) => c.name.toLowerCase().includes(q)));
      })
      .catch(() => {
        if (!cancelled) {
          setError("Could not load clients");
          setClients([]);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [search]);

  return (
    <div className="grid gap-2 rounded-[2px] border p-2" style={{ borderColor: "var(--m-border)" }} data-testid="client-picker">
      <Label htmlFor="client-picker-search">Client</Label>
      <Input
        id="client-picker-search"
        value={search}
        onChange={(e) => setSearch(e.target.value)}
        placeholder="Search clients…"
        autoFocus
      />
      {error !== null && <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }}>{error}</p>}
      {clients === null ? (
        <Spinner />
      ) : (
        <ul className="grid max-h-48 gap-0.5 overflow-y-auto">
          {clients.map((c) => (
            <li key={c.id}>
              <button
                type="button"
                disabled={busy}
                className="w-full truncate rounded-[2px] px-2 py-1 text-left text-xs hover:bg-[var(--m-accent-soft)] disabled:opacity-50"
                style={c.id === currentId ? { background: "var(--m-accent-soft)" } : undefined}
                data-testid={`client-option-${c.id}`}
                onClick={() => void onPick(c.id)}
              >
                {c.name}
                {c.id === currentId ? " (current)" : ""}
              </button>
            </li>
          ))}
          {clients.length === 0 && (
            <li className="px-2 py-1 text-xs" style={{ color: "var(--m-text-muted)" }}>No matches</li>
          )}
        </ul>
      )}
      <div className="flex justify-end">
        <Button size="sm" variant="ghost" onClick={onCancel}>Cancel</Button>
      </div>
    </div>
  );
}

function ContactPicker({
  clientId,
  clientName,
  currentId,
  busy,
  onPick,
  onCancel,
}: {
  clientId: string;
  clientName: string;
  currentId: string | null;
  busy: boolean;
  onPick(id: string): Promise<void>;
  onCancel(): void;
}): ReactNode {
  const contacts = useContacts(clientId, 0);
  const [search, setSearch] = useState("");
  const [creating, setCreating] = useState(false);
  const [createName, setCreateName] = useState("");
  const [createEmail, setCreateEmail] = useState("");
  const [createBusy, setCreateBusy] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);

  const filtered = useMemo(() => {
    const data = contacts.status === "ready" ? contacts.data : [];
    const q = search.trim().toLowerCase();
    if (q === "") return data;
    return data.filter(
      (c) => c.name.toLowerCase().includes(q) || (c.email ?? "").toLowerCase().includes(q),
    );
  }, [contacts, search]);

  async function createContact(): Promise<void> {
    if (createName.trim() === "") {
      setCreateError("Name is required");
      return;
    }
    setCreateBusy(true);
    setCreateError(null);
    const { data, error } = await api.api.POST("/api/v1/clients/{id}/contacts", {
      params: { path: { id: clientId } },
      body: { name: createName.trim(), email: createEmail.trim() === "" ? null : createEmail.trim() },
    });
    setCreateBusy(false);
    if (error !== undefined || data === undefined) {
      setCreateError("Could not create contact");
      return;
    }
    await onPick(data.id);
  }

  return (
    <div className="grid gap-2 rounded-[2px] border p-2" style={{ borderColor: "var(--m-border)" }} data-testid="contact-picker">
      <div className="flex items-baseline justify-between gap-2">
        <Label htmlFor="contact-picker-search">Contact at {clientName}</Label>
        <button
          type="button"
          className="border-none bg-transparent text-[11px] underline underline-offset-2"
          style={{ color: "var(--m-accent)" }}
          onClick={() => setCreating((v) => !v)}
          data-testid="toggle-create-contact"
        >
          {creating ? "Pick existing" : "New contact…"}
        </button>
      </div>
      {creating ? (
        <div className="grid gap-2">
          <Input value={createName} onChange={(e) => setCreateName(e.target.value)} placeholder="Name" autoFocus aria-label="New contact name" data-testid="new-contact-name" />
          <Input value={createEmail} onChange={(e) => setCreateEmail(e.target.value)} placeholder="Email (optional)" type="email" aria-label="New contact email" data-testid="new-contact-email" />
          {createError !== null && (
            <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">{createError}</p>
          )}
          <div className="flex justify-end gap-2">
            <Button size="sm" variant="ghost" onClick={onCancel}>Cancel</Button>
            <Button size="sm" variant="primary" disabled={createBusy} onClick={() => void createContact()} data-testid="create-contact-save">
              Create &amp; use
            </Button>
          </div>
        </div>
      ) : (
        <>
          <Input
            id="contact-picker-search"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search contacts…"
            autoFocus
          />
          {contacts.status === "loading" && <Spinner />}
          {contacts.status === "error" && (
            <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }}>{contacts.message}</p>
          )}
          {contacts.status === "ready" && (
            <ul className="grid max-h-48 gap-0.5 overflow-y-auto">
              {filtered.map((c) => (
                <li key={c.id}>
                  <button
                    type="button"
                    disabled={busy}
                    className="w-full truncate rounded-[2px] px-2 py-1 text-left text-xs hover:bg-[var(--m-accent-soft)] disabled:opacity-50"
                    style={c.id === currentId ? { background: "var(--m-accent-soft)" } : undefined}
                    data-testid={`contact-option-${c.id}`}
                    onClick={() => void onPick(c.id)}
                  >
                    {c.name}
                    {c.email !== null ? <span style={{ color: "var(--m-text-muted)" }}> · {c.email}</span> : null}
                    {c.id === currentId ? " (current)" : ""}
                  </button>
                </li>
              ))}
              {filtered.length === 0 && (
                <li className="px-2 py-1 text-xs" style={{ color: "var(--m-text-muted)" }}>No matches</li>
              )}
            </ul>
          )}
          <div className="flex justify-end">
            <Button size="sm" variant="ghost" onClick={onCancel}>Cancel</Button>
          </div>
        </>
      )}
    </div>
  );
}
