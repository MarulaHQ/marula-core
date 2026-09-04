// /clients/[id] — the client record (Workspace-core UI, T3).
//
// Purpose: one client's full surface: identity header with the
// per-client visibility hint, and tabs for Sites, Contacts, Assets, and
// linked Portal users. Child-entity creates call the existing plain
// POST endpoints (they carry no envelope); the portal-user create uses
// the admin-provisioned password flow (delivered out-of-band) and
// disable/enable rides PATCH /portal-users/{id}.
// Non-purpose: no board-exposure editing here (portal admin surface,
// TASKLIST Ops backlog); no asset lifecycle fields yet.
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

type Client = components["schemas"]["Client"];
type Site = components["schemas"]["Site"];
type Contact = components["schemas"]["Contact"];
type DeviceAsset = components["schemas"]["DeviceAsset"];
type PortalUser = components["schemas"]["PortalUser"];
type PortalRole = components["schemas"]["PortalRole"];

type Tab = "sites" | "contacts" | "assets" | "portal";

const TABS: { key: Tab; label: string }[] = [
  { key: "sites", label: "Sites" },
  { key: "contacts", label: "Contacts" },
  { key: "assets", label: "Assets" },
  { key: "portal", label: "Portal users" },
];

function tabButtonStyle(active: boolean): React.CSSProperties {
  return {
    color: active ? "var(--m-text)" : "var(--m-text-secondary)",
    fontWeight: active ? 600 : 400,
    borderBottom: `2px solid ${active ? "var(--m-text)" : "transparent"}`,
    background: active ? "var(--m-accent-soft)" : "transparent",
  };
}

export default function ClientDetailPage() {
  const auth = useRequireAuth();
  const { me } = useAuth();
  const params = useParams<{ id: string }>();
  const clientId = params.id;

  const [client, setClient] = useState<Client | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("sites");

  const [sites, setSites] = useState<Site[] | null>(null);
  const [contacts, setContacts] = useState<Contact[] | null>(null);
  const [assets, setAssets] = useState<DeviceAsset[] | null>(null);
  const [portalUsers, setPortalUsers] = useState<PortalUser[] | null>(null);

  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const isAdmin = me?.permissions.includes("special:manage_users") ?? false;

  // Create-dialog state per tab.
  const [createOpen, setCreateOpen] = useState(false);
  const [siteName, setSiteName] = useState("");
  const [siteCity, setSiteCity] = useState("");
  const [siteAddress, setSiteAddress] = useState("");
  const [contactName, setContactName] = useState("");
  const [contactEmail, setContactEmail] = useState("");
  const [contactPhone, setContactPhone] = useState("");
  const [contactTitle, setContactTitle] = useState("");
  const [assetTag, setAssetTag] = useState("");
  const [assetName, setAssetName] = useState("");
  const [assetKind, setAssetKind] = useState("");
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

  const refreshTab = useCallback(async () => {
    setFormError(null);
    if (tab === "sites") {
      const { data } = await api.api.GET("/api/v1/clients/{id}/sites", {
        params: { path: { id: clientId } },
      });
      setSites(data ?? []);
    } else if (tab === "contacts") {
      const { data } = await api.api.GET("/api/v1/clients/{id}/contacts", {
        params: { path: { id: clientId } },
      });
      setContacts(data ?? []);
    } else if (tab === "assets") {
      const { data } = await api.api.GET("/api/v1/clients/{id}/assets", {
        params: { path: { id: clientId } },
      });
      setAssets(data ?? []);
    } else {
      const { data, error } = await api.api.GET("/api/v1/clients/{id}/portal-users", {
        params: { path: { id: clientId } },
      });
      if (error !== undefined) {
        setPortalUsers([]);
        setFormError("Portal-user administration is not available to your role.");
      } else {
        setPortalUsers(data ?? []);
      }
    }
  }, [tab, clientId]);

  useEffect(() => {
    if (auth.status === "authed") void refreshClient();
  }, [auth.status, refreshClient]);

  useEffect(() => {
    if (auth.status === "authed") void refreshTab();
  }, [auth.status, refreshTab]);

  const resetCreateForm = () => {
    setSiteName("");
    setSiteCity("");
    setSiteAddress("");
    setContactName("");
    setContactEmail("");
    setContactPhone("");
    setContactTitle("");
    setAssetTag("");
    setAssetName("");
    setAssetKind("");
    setPuEmail("");
    setPuName("");
    setPuPassword("");
    setPuRole("contact");
  };

  const createLabel =
    tab === "sites" ? "Add site"
    : tab === "contacts" ? "Add contact"
    : tab === "assets" ? "Add asset"
    : "Provision portal user";

  const createDisabled =
    saving ||
    (tab === "sites" && siteName.trim() === "") ||
    (tab === "contacts" && contactName.trim() === "") ||
    (tab === "assets" && (assetTag.trim() === "" || assetName.trim() === "")) ||
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
    } else if (tab === "assets") {
      result = await api.api.POST("/api/v1/clients/{id}/assets", {
        params: { path: { id: clientId } },
        body: {
          tag: assetTag.trim(),
          name: assetName.trim(),
          kind: assetKind.trim() === "" ? null : assetKind.trim(),
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
    void refreshTab();
    if (tab === "portal") void refreshClient();
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
    void refreshTab();
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

  return (
    <div className="grid gap-6 p-6">
      <div>
        <Link href="/clients" className="text-[13px] underline decoration-[var(--m-border)] underline-offset-2" style={{ color: "var(--m-text-secondary)" }}>
          ← Clients
        </Link>
        <div className="mt-2 flex items-center gap-3">
          <h1 className="text-lg font-semibold">{client.name}</h1>
          {restricted ? (
            <Tag title={isAdmin ? `Visible to ${client.restricted_to.length} listed users + admins` : "Restricted client"}>
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

      <div className="flex items-center gap-4 border-b pb-0" style={{ borderColor: "var(--m-border)" }}>
        {TABS.map((t) => (
          <button
            key={t.key}
            type="button"
            onClick={() => setTab(t.key)}
            aria-current={tab === t.key ? "true" : undefined}
            className="cursor-pointer border-none bg-transparent px-1 pb-2 text-[13px]"
            style={tabButtonStyle(tab === t.key)}
          >
            {t.label}
          </button>
        ))}
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
                {tab === "assets" ? (
                  <>
                    <div>
                      <Label htmlFor="asset-tag">Asset tag</Label>
                      <Input id="asset-tag" value={assetTag} onChange={(e) => setAssetTag(e.target.value)} placeholder="EXCH-01" />
                    </div>
                    <div>
                      <Label htmlFor="asset-name">Name</Label>
                      <Input id="asset-name" value={assetName} onChange={(e) => setAssetName(e.target.value)} />
                    </div>
                    <div>
                      <Label htmlFor="asset-kind">Kind</Label>
                      <Input id="asset-kind" value={assetKind} onChange={(e) => setAssetKind(e.target.value)} placeholder="server, laptop, network…" />
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
      </div>

      {formError !== null && tab !== "portal" ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {formError}
        </p>
      ) : null}

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
                  <Tr key={site.id}>
                    <Td>{site.name}</Td>
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
        <Card>
          <h2 className="p-4 pb-0 text-sm font-semibold">Contacts</h2>
          {contacts === null ? (
            <div className="p-4"><Spinner /></div>
          ) : contacts.length === 0 ? (
            <div className="p-4">
              <EmptyState message="No contacts yet. Contacts are the people at this client your team works with." />
            </div>
          ) : (
            <Table>
              <thead>
                <tr>
                  <Th>Name</Th>
                  <Th>Title</Th>
                  <Th>Email</Th>
                  <Th>Phone</Th>
                  <Th></Th>
                </tr>
              </thead>
              <tbody>
                {contacts.map((contact) => (
                  <Tr key={contact.id}>
                    <Td>
                      {contact.name}
                      {contact.is_primary ? (
                        <span className="ml-2">
                          <Tag>primary</Tag>
                        </span>
                      ) : null}
                    </Td>
                    <Td>{contact.title ?? "—"}</Td>
                    <Td>{contact.email ?? "—"}</Td>
                    <Td>{contact.phone ?? "—"}</Td>
                    <Td></Td>
                  </Tr>
                ))}
              </tbody>
            </Table>
          )}
        </Card>
      ) : null}

      {tab === "assets" ? (
        <Card>
          <h2 className="p-4 pb-0 text-sm font-semibold">Assets</h2>
          {assets === null ? (
            <div className="p-4"><Spinner /></div>
          ) : assets.length === 0 ? (
            <div className="p-4">
              <EmptyState message="No assets yet. Track the devices and infrastructure you manage for this client." />
            </div>
          ) : (
            <Table>
              <thead>
                <tr>
                  <Th>Tag</Th>
                  <Th>Name</Th>
                  <Th>Kind</Th>
                  <Th>Status</Th>
                </tr>
              </thead>
              <tbody>
                {assets.map((asset) => (
                  <Tr key={asset.id}>
                    <Td className="tabular-nums">{asset.tag}</Td>
                    <Td>{asset.name}</Td>
                    <Td>{asset.kind ?? "—"}</Td>
                    <Td>
                      <Tag>{asset.status}</Tag>
                    </Td>
                  </Tr>
                ))}
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
          {portalUsers === null ? (
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
                  <Tr key={user.id}>
                    <Td>{user.email}</Td>
                    <Td>{user.display_name}</Td>
                    <Td>{user.role}</Td>
                    <Td>
                      <Tag>{user.status}</Tag>
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
  );
}
