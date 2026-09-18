// /clients/[id] — the client record (Wave 17 UX excellence pass).
//
// Purpose: CRM-grade account detail: identity header with monogram +
// visibility hint, a two-pane layout (tab panes + an At-a-glance rail)
// carrying the documentation-completeness score with gap hints (scored
// server-side via GET /clients/{id}/completeness; the page renders the
// backend view verbatim and stays honest when scoring is out of scope), tabs for Sites, Contacts, Assets,
// Documents, Portal users, and Workflow, contact cards with mailto/tel
// affordances, and related-item quick links. Child-entity creates call
// the existing plain POST endpoints (they carry no envelope); the
// portal-user create uses the admin-provisioned password flow
// (delivered out-of-band) and disable/enable rides PATCH
// /portal-users/{id}. The Assets tab is the typed asset list (lane 5,
// migration 0054) — AssetsTab owns it and links rows to /assets/[id].
// Non-purpose: no board-exposure editing (portal admin surface); no
// note-composer changes (ADR 0007).
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
import Link from "next/link";
import { useParams } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { api, useAuth, useRequireAuth } from "../../../lib/api";
import { scoreTone } from "../../../lib/docs-completeness";
import AssetsTab from "./AssetsTab";
import WorkflowOverridesTab from "./WorkflowOverridesTab";

type Client = components["schemas"]["Client"];
type Site = components["schemas"]["Site"];
type Contact = components["schemas"]["Contact"];
type PortalUser = components["schemas"]["PortalUser"];
type PortalRole = components["schemas"]["PortalRole"];
type DeviceAsset = components["schemas"]["DeviceAsset"];
type Document = components["schemas"]["Document"];
type DocumentLayout = components["schemas"]["DocumentLayout"];
type ClientCompletenessView = components["schemas"]["ClientCompletenessView"];

type Tab = "sites" | "contacts" | "assets" | "documents" | "portal" | "workflow";

const TABS: { key: Tab; label: string }[] = [
  { key: "sites", label: "Sites" },
  { key: "contacts", label: "Contacts" },
  { key: "assets", label: "Assets" },
  { key: "documents", label: "Documents" },
  { key: "portal", label: "Portal users" },
  { key: "workflow", label: "Workflow" },
];

function tabButtonStyle(active: boolean): React.CSSProperties {
  return {
    color: active ? "var(--m-text)" : "var(--m-text-secondary)",
    fontWeight: active ? 600 : 400,
    borderBottom: `2px solid ${active ? "var(--m-text)" : "transparent"}`,
    background: active ? "var(--m-accent-soft)" : "transparent",
  };
}

function Monogram({ name, size = "md" }: { name: string; size?: "md" | "lg" }) {
  const initials = name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]!.toUpperCase())
    .join("");
  const cls = size === "lg" ? "h-14 w-14 text-xl" : "h-8 w-8 text-[12px]";
  return (
    <span
      aria-hidden="true"
      className={`grid shrink-0 place-items-center rounded-[999px] font-semibold ${cls}`}
      style={{ background: "var(--m-accent-soft)", color: "var(--m-accent-fg)" }}
    >
      {initials}
    </span>
  );
}

export default function ClientDetailPage() {
  const auth = useRequireAuth();
  const { me } = useAuth();
  const params = useParams<{ id: string }>();
  const clientId = params.id;

  const [client, setClient] = useState<Client | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("sites");

  // Tab metadata (all loaded once so the completeness rail is honest).
  const [sites, setSites] = useState<Site[] | null>(null);
  const [contacts, setContacts] = useState<Contact[] | null>(null);
  const [portalUsers, setPortalUsers] = useState<PortalUser[] | null>(null);
  const [portalDenied, setPortalDenied] = useState(false);
  const [assets, setAssets] = useState<DeviceAsset[] | null>(null);
  const [documents, setDocuments] = useState<Document[] | null>(null);
  const [layouts, setLayouts] = useState<DocumentLayout[] | null>(null);
  const [completeness, setCompleteness] = useState<ClientCompletenessView | null>(null);
  const [completenessUnavailable, setCompletenessUnavailable] = useState(false);

  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const isAdmin = (me?.permissions ?? []).includes("special:manage_users") ?? false;

  // Create-dialog state per tab.
  const [createOpen, setCreateOpen] = useState(false);
  const [siteName, setSiteName] = useState("");
  const [siteCity, setSiteCity] = useState("");
  const [siteAddress, setSiteAddress] = useState("");
  const [contactName, setContactName] = useState("");
  const [contactEmail, setContactEmail] = useState("");
  const [contactPhone, setContactPhone] = useState("");
  const [contactTitle, setContactTitle] = useState("");
  const [puEmail, setPuEmail] = useState("");
  const [puName, setPuName] = useState("");
  const [puPassword, setPuPassword] = useState("");
  const [puRole, setPuRole] = useState<PortalRole>("contact");

  const refreshClient = useCallback(async () => {
    const { data, error } = await api.api.GET("/api/v1/clients/{id}", {
      params: { path: { id: clientId } },
    });
    if (error !== undefined || data === undefined) {
      setLoadError("Failed to load this client. It may not exist or you may not have access.");
      return;
    }
    setClient(data);
    setLoadError(null);
  }, [clientId]);

  const refreshAll = useCallback(async () => {
    setFormError(null);
    void api.api.GET("/api/v1/clients/{id}/sites", { params: { path: { id: clientId } } }).then(({ data }) => setSites(data ?? []));
    void api.api.GET("/api/v1/clients/{id}/contacts", { params: { path: { id: clientId } } }).then(({ data }) => setContacts(data ?? []));
    void api.api.GET("/api/v1/clients/{id}/assets", { params: { path: { id: clientId } } }).then(({ data }) => setAssets(data ?? []));
    void api.api.GET("/api/v1/documents", { params: { query: { client_id: clientId } } }).then(({ data }) => setDocuments(data ?? []));
    void api.api.GET("/api/v1/documents/layouts").then(({ data }) => setLayouts(data ?? []));
    const pu = await api.api.GET("/api/v1/clients/{id}/portal-users", {
      params: { path: { id: clientId } },
    });
    if (pu.error !== undefined || pu.response.status !== 200) {
      setPortalUsers(null);
      setPortalDenied(true);
    } else {
      setPortalDenied(false);
      setPortalUsers(pu.data ?? []);
    }
  }, [clientId]);

  const loadCompleteness = useCallback(async () => {
    const res = await api.api.GET("/api/v1/clients/{id}/completeness", {
      params: { path: { id: clientId } },
    });
    if (res.error !== undefined || res.data === undefined) {
      setCompleteness(null);
      setCompletenessUnavailable(true);
      return;
    }
    setCompleteness(res.data);
    setCompletenessUnavailable(false);
  }, [clientId]);

  useEffect(() => {
    if (auth.status === "authed") void refreshClient();
  }, [auth.status, refreshClient]);

  useEffect(() => {
    if (auth.status === "authed" && clientId !== "") void loadCompleteness();
  }, [auth.status, clientId, loadCompleteness]);

  useEffect(() => {
    if (auth.status === "authed") void refreshAll();
  }, [auth.status, refreshAll]);

  const resetCreateForm = () => {
    setSiteName("");
    setSiteCity("");
    setSiteAddress("");
    setContactName("");
    setContactEmail("");
    setContactPhone("");
    setContactTitle("");
    setPuEmail("");
    setPuName("");
    setPuPassword("");
    setPuRole("contact");
  };

  const createLabel =
    tab === "sites" ? "Add site"
    : tab === "contacts" ? "Add contact"
    : tab === "portal" ? "Provision portal user"
    : null;

  const createDisabled =
    saving ||
    (tab === "sites" && siteName.trim() === "") ||
    (tab === "contacts" && contactName.trim() === "") ||
    (tab === "portal" &&
      (puEmail.trim() === "" || puName.trim() === "" || puPassword.trim() === ""));

  const runCreate = async () => {
    setSaving(true);
    setFormError(null);
    let result: { error?: unknown; data?: unknown };
    if (tab === "sites") {
      result = await api.api.POST("/api/v1/clients/{id}/sites", {
        params: { path: { id: clientId } },
        body: {
          name: siteName.trim(),
          city: siteCity.trim() === "" ? null : siteCity.trim(),
          address: siteAddress.trim() === "" ? null : siteAddress.trim(),
        },
      });
    } else if (tab === "contacts") {
      result = await api.api.POST("/api/v1/clients/{id}/contacts", {
        params: { path: { id: clientId } },
        body: {
          name: contactName.trim(),
          email: contactEmail.trim() === "" ? null : contactEmail.trim(),
          phone: contactPhone.trim() === "" ? null : contactPhone.trim(),
          title: contactTitle.trim() === "" ? null : contactTitle.trim(),
        },
      });
    } else {
      result = await api.api.POST("/api/v1/clients/{id}/portal-users", {
        params: { path: { id: clientId } },
        body: {
          email: puEmail.trim(),
          display_name: puName.trim(),
          password: puPassword,
          role: puRole,
        },
      });
    }
    setSaving(false);
    if (result.error !== undefined) {
      setFormError("The server rejected this entry. Check the fields and retry.");
      return;
    }
    setCreateOpen(false);
    resetCreateForm();
    void refreshAll();
    void refreshClient();
    void loadCompleteness();
  };

  const setPortalStatus = async (user: PortalUser, disabled: boolean) => {
    const { error } = await api.api.PATCH("/api/v1/portal-users/{id}", {
      params: { path: { id: user.id } },
      body: { status: disabled ? "disabled" : "invited" },
    });
    if (error !== undefined) {
      setFormError("The status change was rejected. Nothing was saved.");
      return;
    }
    void refreshAll();
  };

  if (auth.status !== "authed") return null;

  if (loadError !== null) {
    return (
      <div className="grid gap-4 p-6">
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {loadError}
        </p>
        <Link href="/clients" className="text-[13px] underline underline-offset-2">
          Back to clients
        </Link>
      </div>
    );
  }

  if (client === null) {
    return (
      <div className="p-6">
        <Spinner />
      </div>
    );
  }

  const restricted = client.restricted_to !== null && client.restricted_to.length > 0;
  const tone = completeness !== null ? scoreTone(completeness.score) : "neutral";

  return (
    <div className="mx-auto grid w-full grid-cols-1 max-w-[1400px] gap-6 p-4 md:p-6">
      {/* ---- identity header ---- */}
      <div className="m-entity-rise">
        <Link href="/clients" className="text-[13px] underline decoration-[var(--m-border)] underline-offset-2" style={{ color: "var(--m-text-secondary)" }}>
          ← Clients
        </Link>
        <div className="mt-2 flex flex-wrap items-center gap-4">
          <Monogram name={client.name} size="lg" />
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-3">
              <h1 className="text-lg font-semibold">{client.name}</h1>
              {restricted ? (
                <Tag tone="warning" title={isAdmin ? `Visible to ${client.restricted_to.length} listed users + admins` : "Restricted client"}>
                  Restricted
                </Tag>
              ) : null}
            </div>
            {client.notes !== null ? (
              <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                {client.notes}
              </p>
            ) : null}
          </div>
          <span className="ml-auto text-[12px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
            updated {new Date(client.updated_at).toLocaleDateString()}
          </span>
        </div>
      </div>

      <div className="grid grid-cols-1 gap-6 xl:grid-cols-[1fr_320px]">
        {/* ---- main pane: tabs ---- */}
        <div>
          <div className="m-entity-rise flex flex-wrap items-center gap-4 border-b pb-0" style={{ borderColor: "var(--m-border)", "--stagger-i": 1 } as React.CSSProperties}>
            {TABS.map((t) => (
              <button
                key={t.key}
                type="button"
                onClick={() => setTab(t.key)}
                aria-current={tab === t.key ? "true" : undefined}
                className="m-entity-row cursor-pointer rounded-t-[0.5rem] border-none bg-transparent px-2 pb-2 text-[13px]"
                style={tabButtonStyle(tab === t.key)}
              >
                {t.label}
                {t.key === "documents" && documents !== null ? (
                  <span className="ml-1 text-[11px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                    {documents.length}
                  </span>
                ) : null}
              </button>
            ))}
            {createLabel !== null ? (
              <span className="ml-auto pb-2">
                <Dialog
                  open={createOpen}
                  onOpenChange={(open) => {
                    setCreateOpen(open);
                    if (!open) setFormError(null);
                  }}
                >
                  <DialogTrigger asChild>
                    <Button size="sm">{createLabel}</Button>
                  </DialogTrigger>
                  <DialogContent title={createLabel}>
                    <div className="grid gap-3 p-4">
                      {tab === "sites" ? (
                        <>
                          <div>
                            <Label htmlFor="site-name">Site name</Label>
                            <Input id="site-name" value={siteName} onChange={(e) => setSiteName(e.target.value)} placeholder="Headquarters" />
                          </div>
                          <div>
                            <Label htmlFor="site-address">Address</Label>
                            <Input id="site-address" value={siteAddress} onChange={(e) => setSiteAddress(e.target.value)} placeholder="Optional" />
                          </div>
                          <div>
                            <Label htmlFor="site-city">City</Label>
                            <Input id="site-city" value={siteCity} onChange={(e) => setSiteCity(e.target.value)} placeholder="Optional" />
                          </div>
                        </>
                      ) : null}
                      {tab === "contacts" ? (
                        <>
                          <div>
                            <Label htmlFor="contact-name">Name</Label>
                            <Input id="contact-name" value={contactName} onChange={(e) => setContactName(e.target.value)} />
                          </div>
                          <div>
                            <Label htmlFor="contact-email">Email</Label>
                            <Input id="contact-email" type="email" value={contactEmail} onChange={(e) => setContactEmail(e.target.value)} placeholder="Optional" />
                          </div>
                          <div>
                            <Label htmlFor="contact-phone">Phone</Label>
                            <Input id="contact-phone" value={contactPhone} onChange={(e) => setContactPhone(e.target.value)} placeholder="Optional" />
                          </div>
                          <div>
                            <Label htmlFor="contact-title">Title</Label>
                            <Input id="contact-title" value={contactTitle} onChange={(e) => setContactTitle(e.target.value)} placeholder="Optional" />
                          </div>
                        </>
                      ) : null}
                      {tab === "portal" ? (
                        <>
                          <div>
                            <Label htmlFor="pu-email">Email</Label>
                            <Input id="pu-email" type="email" value={puEmail} onChange={(e) => setPuEmail(e.target.value)} />
                          </div>
                          <div>
                            <Label htmlFor="pu-name">Display name</Label>
                            <Input id="pu-name" value={puName} onChange={(e) => setPuName(e.target.value)} />
                          </div>
                          <div>
                            <Label htmlFor="pu-password">Initial password</Label>
                            <Input id="pu-password" type="password" value={puPassword} onChange={(e) => setPuPassword(e.target.value)} />
                            <FieldHint>Deliver out-of-band; the user is asked to change it on first login.</FieldHint>
                          </div>
                          <div>
                            <Label htmlFor="pu-role">Portal role</Label>
                            <Select value={puRole} onValueChange={(v) => setPuRole(v as PortalRole)}>
                              <SelectTrigger id="pu-role" className="w-44">
                                <SelectValue />
                              </SelectTrigger>
                              <SelectContent>
                                <SelectItem value="contact">contact</SelectItem>
                                <SelectItem value="approver">approver</SelectItem>
                                <SelectItem value="it_manager">it_manager</SelectItem>
                              </SelectContent>
                            </Select>
                          </div>
                        </>
                      ) : null}
                      {formError !== null ? (
                        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                          {formError}
                        </p>
                      ) : null}
                      <DialogActions>
                        <Button variant="ghost" onClick={() => setCreateOpen(false)}>
                          Cancel
                        </Button>
                        <Button onClick={() => void runCreate()} disabled={createDisabled}>
                          {saving ? <Spinner /> : null} Save
                        </Button>
                      </DialogActions>
                    </div>
                  </DialogContent>
                </Dialog>
              </span>
            ) : null}
          </div>

          {formError !== null && tab !== "portal" ? (
            <p role="alert" className="mt-2 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {formError}
            </p>
          ) : null}

          <div key={tab} className="mt-4 m-entity-rise" data-testid={`client-tab-${tab}`}>
            {tab === "sites" ? (
              <Card>
                <h2 className="p-4 pb-0 text-sm font-semibold">Sites</h2>
                {sites === null ? (
                  <div className="p-4"><Spinner /></div>
                ) : sites.length === 0 ? (
                  <div className="p-4">
                    <EmptyState message="No sites yet. A site is a physical location used for routing and travel estimates." />
                  </div>
                ) : (
                  <Table>
                    <thead>
                      <tr>
                        <Th>Name</Th>
                        <Th>Address</Th>
                        <Th>City</Th>
                        <Th>Region</Th>
                      </tr>
                    </thead>
                    <tbody>
                      {sites.map((site) => (
                        <Tr key={site.id} className="m-entity-row">
                          <Td className="font-medium">{site.name}</Td>
                          <Td>{site.address ?? "—"}</Td>
                          <Td>{site.city ?? "—"}</Td>
                          <Td>{site.region ?? "—"}</Td>
                        </Tr>
                      ))}
                    </tbody>
                  </Table>
                )}
              </Card>
            ) : null}

            {tab === "contacts" ? (
              <div className="grid gap-3 sm:grid-cols-2">
                {contacts === null ? (
                  <div className="p-4"><Spinner /></div>
                ) : contacts.length === 0 ? (
                  <Card>
                    <div className="p-4">
                      <EmptyState message="No contacts yet. Contacts are the people at this client your team works with." />
                    </div>
                  </Card>
                ) : (
                  contacts.map((contact, i) => (
                    <Card
                      key={contact.id}
                      className="m-entity-row m-entity-rise"
                      style={{ "--stagger-i": Math.min(i, 6) } as React.CSSProperties}
                    >
                      <div className="flex items-start gap-3 p-4">
                        <Monogram name={contact.name} />
                        <div className="min-w-0">
                          <p className="flex items-center gap-2 truncate text-[13px] font-semibold">
                            {contact.name}
                            {contact.is_primary ? <Tag tone="success">primary</Tag> : null}
                          </p>
                          {contact.title !== null ? (
                            <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                              {contact.title}
                            </p>
                          ) : null}
                          <p className="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-[12px]">
                            {contact.email != null ? (
                              <a href={`mailto:${contact.email}`} className="underline underline-offset-2" style={{ color: "var(--m-accent)" }}>
                                {contact.email}
                              </a>
                            ) : null}
                            {contact.phone != null ? (
                              <a href={`tel:${contact.phone.replace(/[^+\d]/g, "")}`} className="underline underline-offset-2" style={{ color: "var(--m-accent)" }}>
                                {contact.phone}
                              </a>
                            ) : null}
                            {contact.email == null && contact.phone == null ? (
                              <span style={{ color: "var(--m-text-muted)" }}>No reach channels yet</span>
                            ) : null}
                          </p>
                        </div>
                      </div>
                    </Card>
                  ))
                )}
              </div>
            ) : null}

            {tab === "assets" ? <AssetsTab clientId={clientId} /> : null}

            {tab === "workflow" ? <WorkflowOverridesTab clientId={clientId} /> : null}

            {tab === "documents" ? (
              <Card>
                <div className="flex items-center justify-between p-4 pb-0">
                  <h2 className="text-sm font-semibold">Documents</h2>
                  <Link href="/documents" className="text-[12px] underline underline-offset-2" style={{ color: "var(--m-accent)" }}>
                    Open library
                  </Link>
                </div>
                {documents === null ? (
                  <div className="p-4"><Spinner /></div>
                ) : documents.length === 0 ? (
                  <div className="p-4">
                    <EmptyState message="No documents for this client yet. Typed records, runbooks, and manifests belong here." />
                  </div>
                ) : (
                  <Table>
                    <thead>
                      <tr>
                        <Th>Title</Th>
                        <Th>Type</Th>
                        <Th className="text-right">Version</Th>
                        <Th>Updated</Th>
                      </tr>
                    </thead>
                    <tbody>
                      {documents.map((d) => {
                        const layoutName = layouts?.find((l) => l.id === d.layout_id)?.name;
                        return (
                          <Tr key={d.id} className="m-entity-row">
                            <Td>
                              <Link href={`/documents/${d.id}`} className="font-medium underline decoration-transparent underline-offset-2 transition-colors duration-[100ms] hover:decoration-[var(--m-border-strong)]">
                                {d.title}
                              </Link>
                            </Td>
                            <Td>
                              {layoutName != null ? <Tag tone="outline">{layoutName}</Tag> : <span style={{ color: "var(--m-text-muted)" }}>—</span>}
                            </Td>
                            <Td className="text-right tabular-nums">v{d.current_version}</Td>
                            <Td>{new Date(d.updated_at).toLocaleDateString()}</Td>
                          </Tr>
                        );
                      })}
                    </tbody>
                  </Table>
                )}
              </Card>
            ) : null}

            {tab === "portal" ? (
              <Card>
                <h2 className="p-4 pb-0 text-sm font-semibold">Portal users</h2>
                <p className="px-4 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                  Linked portal identities for this client. Portal users see only what the portal exposes.
                </p>
                {formError !== null ? (
                  <p role="alert" className="px-4 pt-2 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                    {formError}
                  </p>
                ) : null}
                {portalDenied ? (
                  <p role="alert" className="p-4 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                    Portal-user administration is not available to your role.
                  </p>
                ) : portalUsers === null ? (
                  <div className="p-4"><Spinner /></div>
                ) : portalUsers.length === 0 ? (
                  <div className="p-4">
                    <EmptyState message="No portal users yet. Provisioning one creates the client-facing login for this client's portal." />
                  </div>
                ) : (
                  <Table>
                    <thead>
                      <tr>
                        <Th>Email</Th>
                        <Th>Name</Th>
                        <Th>Role</Th>
                        <Th>Status</Th>
                        <Th className="text-right">Actions</Th>
                      </tr>
                    </thead>
                    <tbody>
                      {portalUsers.map((user) => (
                        <Tr key={user.id} className="m-entity-row">
                          <Td>{user.email}</Td>
                          <Td>{user.display_name}</Td>
                          <Td>{user.role}</Td>
                          <Td>
                            <Tag tone={user.status === "disabled" ? "danger" : "success"}>{user.status}</Tag>
                          </Td>
                          <Td className="text-right">
                            {user.status === "disabled" ? (
                              <Button variant="ghost" size="sm" onClick={() => void setPortalStatus(user, false)}>
                                Re-enable
                              </Button>
                            ) : (
                              <Button variant="ghost" size="sm" onClick={() => void setPortalStatus(user, true)}>
                                Disable
                              </Button>
                            )}
                          </Td>
                        </Tr>
                      ))}
                    </tbody>
                  </Table>
                )}
              </Card>
            ) : null}
          </div>
        </div>

        {/* ---- at-a-glance rail ---- */}
        <div className="grid content-start gap-4" data-testid="client-glance-rail">
          <Card className="m-entity-rise" style={{ "--stagger-i": 2 } as React.CSSProperties}>
            <h2 className="p-4 pb-0 text-sm font-semibold">Documentation completeness</h2>
            <p className="px-4 pb-2 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              Scored server-side over this client's records — the backend owns the weights.
            </p>
            {completenessUnavailable ? (
              <div className="p-4 pt-0">
                <EmptyState message="Completeness scoring is not available for this client in your scope." />
              </div>
            ) : completeness === null ? (
              <div className="px-4 pb-4"><Spinner /></div>
            ) : (
              <>
                <div className="px-4 pb-1">
                  <div
                    aria-hidden="true"
                    className="h-2 w-full overflow-hidden rounded-full"
                    style={{ background: "var(--m-bg-subtle, var(--m-border))" }}
                  >
                    <div
                      className="h-full rounded-full transition-[width] duration-[120ms]"
                      style={{
                        width: `${completeness.score}%`,
                        background: completeness.score >= 75
                          ? "var(--m-status-success-fg)"
                          : completeness.score >= 40
                            ? "var(--m-status-warning-fg)"
                            : "var(--m-status-danger-fg)",
                      }}
                    />
                  </div>
                  <p className="mt-2 flex items-center gap-2">
                    <span className="text-2xl font-semibold tabular-nums" data-testid="completeness-score">
                      {completeness.score}
                    </span>
                    <Tag tone={tone}>{completeness.score >= 75 ? "complete" : completeness.score >= 40 ? "in progress" : "thin"}</Tag>
                  </p>
                  <p className="mt-1 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                    {completeness.basis}
                  </p>
                </div>
                {completeness.gaps.length > 0 ? (
                  <ul className="grid gap-2 px-4 pb-4 pt-2">
                    {completeness.gaps.map((g, i) => (
                      <li
                        key={g.key}
                        className="m-entity-rise rounded-[0.5rem] border px-3 py-2"
                        style={{ borderColor: "var(--m-border)", "--stagger-i": i } as React.CSSProperties}
                        data-testid={`gap-${g.key}`}
                      >
                        <p className="text-[12px] font-semibold" style={{ color: "var(--m-text)" }}>
                          {g.label}
                        </p>
                        <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                          {g.hint}
                        </p>
                      </li>
                    ))}
                  </ul>
                ) : (
                  <p className="px-4 pb-4 pt-1 text-[12px]" style={{ color: "var(--m-status-success-fg)" }}>
                    No gaps in the visible records.
                  </p>
                )}
              </>
            )}
          </Card>
        </div>
      </div>
    </div>
  );
}
