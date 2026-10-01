// /purchasing/vendors — vendor records for purchasing (Wave 32 lane 4).
//
// Purpose: one screen for the supplier book: search and filter vendors,
// create and edit a vendor, and expand a vendor in place to manage its
// contacts (replace-on-write) and its portal/tracking/invoice links.
//
// Non-purpose: no purchase orders here (the browser and the order detail own
// those) and no nested vendor routes — the editor stays on this screen.
"use client";

import {
  Button,
  Card,
  Checkbox,
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
  StatusTag,
  Table,
  Tag,
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { LedgerSurface, type LedgerColumn } from "../../../components/assets-stock/ledger-surface";
import { LedgerRow } from "../../../components/assets-stock/ledger-row";
import { csvFileName, downloadCsv } from "../../../components/assets-stock/csv";
import { NeutralMark, ReturnedMark } from "../../../components/assets-stock/ledger-marks";
import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { rejectionText } from "../../../lib/error-copy";
import type { FilterClause } from "@marula-core/ui";

const VENDOR_VIEWS = [
  { id: "vendors", label: "Vendors" },
  { id: "orders", label: "Orders" },
];

const VENDOR_COLUMNS: LedgerColumn[] = [
  { key: "vendor", label: "Vendor", required: true },
  { key: "code", label: "Code" },
  { key: "currency", label: "Currency" },
  { key: "contact", label: "Contact" },
  { key: "state", label: "State" },
  { key: "actions", label: "Actions", align: "right" },
];

const VENDOR_FILTERS = [
  { key: "vendor", label: "Vendor" },
  { key: "code", label: "Code" },
  { key: "contact", label: "Contact" },
  { key: "state", label: "State" },
];

type Vendor = components["schemas"]["Vendor"];
type VendorDetail = components["schemas"]["VendorDetail"];
type VendorContact = components["schemas"]["VendorContact"];
type VendorContactInput = components["schemas"]["VendorContactInput"];
type ExternalLink = components["schemas"]["ExternalLink"];
type ExternalLinkKind = components["schemas"]["ExternalLinkKind"];

const LINK_KINDS: ExternalLinkKind[] = ["order_portal", "tracking", "invoice", "other"];

const LINK_KIND_LABELS: Record<ExternalLinkKind, string> = {
  order_portal: "Order portal",
  tracking: "Tracking",
  invoice: "Invoice",
  other: "Other",
};

interface VendorForm {
  name: string;
  code: string;
  currency: string;
  accountNumber: string;
  website: string;
  email: string;
  phone: string;
  notes: string;
  active: boolean;
}

interface ContactRow {
  key: string;
  name: string;
  email: string;
  phone: string;
  role: string;
  isPrimary: boolean;
}

interface LinkForm {
  url: string;
  label: string;
  kind: ExternalLinkKind;
  carrier: string;
}

const EMPTY_FORM: VendorForm = {
  name: "",
  code: "",
  currency: "USD",
  accountNumber: "",
  website: "",
  email: "",
  phone: "",
  notes: "",
  active: true,
};

function emptyLinkForm(): LinkForm {
  return { url: "", label: "", kind: "order_portal", carrier: "" };
}

function contactRow(contact: VendorContact): ContactRow {
  return {
    key: contact.id,
    name: contact.name,
    email: contact.email ?? "",
    phone: contact.phone ?? "",
    role: contact.role ?? "",
    isPrimary: contact.is_primary,
  };
}

export default function VendorsPage(): ReactNode {
  const auth = useRequireAuth();
  const [vendors, setVendors] = useState<Vendor[] | null>(null);
  const [clauses, setClauses] = useState<FilterClause[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const router = useRouter();
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [showInactive, setShowInactive] = useState(false);

  const [editOpen, setEditOpen] = useState(false);
  const [editing, setEditing] = useState<Vendor | null>(null);
  const [form, setForm] = useState<VendorForm>(EMPTY_FORM);
  const [formError, setFormError] = useState<string | null>(null);
  const [formBusy, setFormBusy] = useState(false);

  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [detail, setDetail] = useState<VendorDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState<string | null>(null);

  const [contactRows, setContactRows] = useState<ContactRow[]>([]);
  const [contactsBusy, setContactsBusy] = useState(false);
  const [contactsError, setContactsError] = useState<string | null>(null);

  const [linkForm, setLinkForm] = useState<LinkForm>(emptyLinkForm);
  const [linkBusy, setLinkBusy] = useState(false);
  const [linkError, setLinkError] = useState<string | null>(null);
  const [linkBusyId, setLinkBusyId] = useState<string | null>(null);

  async function refresh(): Promise<void> {
    const { data } = await api.api.GET("/api/v1/vendors", {
      params: { query: { q: query.trim() === "" ? null : query.trim(), include_inactive: showInactive } },
    });
    if (data === undefined) {
      setLoadError("Could not load vendors — try reloading.");
      return;
    }
    setVendors(data);
  }

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refresh();
  }, [auth.status, showInactive]);

  async function loadDetail(vendorId: string): Promise<void> {
    setDetailLoading(true);
    setDetailError(null);
    const { data } = await api.api.GET("/api/v1/vendors/{id}", { params: { path: { id: vendorId } } });
    setDetailLoading(false);
    if (data === undefined) {
      setDetailError("Could not load this vendor — try reloading.");
      return;
    }
    setDetail(data);
    setContactRows(data.contacts.map(contactRow));
    setLinkForm(emptyLinkForm());
    setContactsError(null);
    setLinkError(null);
  }

  async function toggleDetail(vendorId: string): Promise<void> {
    if (expandedId === vendorId) {
      setExpandedId(null);
      setDetail(null);
      return;
    }
    setExpandedId(vendorId);
    setDetail(null);
    await loadDetail(vendorId);
  }

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (vendors ?? []).filter(
      (vendor) =>
        q === "" ||
        vendor.name.toLowerCase().includes(q) ||
        (vendor.code ?? "").toLowerCase().includes(q) ||
        (vendor.email ?? "").toLowerCase().includes(q),
    );
  }, [vendors, query]);

  function openCreate(): void {
    setEditing(null);
    setForm(EMPTY_FORM);
    setFormError(null);
    setEditOpen(true);
  }

  function openEdit(vendor: Vendor): void {
    setEditing(vendor);
    setForm({
      name: vendor.name,
      code: vendor.code ?? "",
      currency: vendor.default_currency,
      accountNumber: vendor.account_number ?? "",
      website: vendor.website ?? "",
      email: vendor.email ?? "",
      phone: vendor.phone ?? "",
      notes: vendor.notes ?? "",
      active: vendor.active,
    });
    setFormError(null);
    setEditOpen(true);
  }

  function vendorFields(): Omit<components["schemas"]["CreateVendorRequest"], "idempotency_key" | "version"> {
    return {
      name: form.name.trim(),
      code: form.code.trim() === "" ? null : form.code.trim(),
      default_currency: form.currency.trim() === "" ? null : form.currency.trim().toUpperCase(),
      account_number: form.accountNumber.trim() === "" ? null : form.accountNumber.trim(),
      website: form.website.trim() === "" ? null : form.website.trim(),
      email: form.email.trim() === "" ? null : form.email.trim(),
      phone: form.phone.trim() === "" ? null : form.phone.trim(),
      notes: form.notes.trim() === "" ? null : form.notes.trim(),
    };
  }

  async function save(): Promise<void> {
    setFormError(null);
    if (form.name.trim() === "") {
      setFormError("Name is required.");
      return;
    }
    setFormBusy(true);
    if (editing === null) {
      const created: { id: string | null } = { id: null };
      const result = await runEnvelopeMutation({
        describe: "Create vendor",
        buildBody: (base) => ({ ...vendorFields(), ...base }),
        optimistic: () => {},
        revert: () => {},
        request: async (client, requestBody) => {
          const response = await client.POST("/api/v1/vendors", { body: requestBody });
          if (response.data !== undefined) created.id = response.data.id;
          return response;
        },
        versions: (response) => [[response.id, response.mutation.version]],
      });
      setFormBusy(false);
      if (result.ok === false) {
        setFormError(rejectionText(result, "Could not create the vendor."));
        return;
      }
      setEditOpen(false);
      await refresh();
      if (created.id !== null) {
        setExpandedId(created.id);
        await loadDetail(created.id);
        setNotice("Vendor created — add its contacts and portal link below.");
      }
      return;
    }

    const target = editing;
    const result = await runEnvelopeMutation({
      describe: "Update vendor",
      entityId: target.id,
      buildBody: (base) => ({ ...vendorFields(), active: form.active, ...base }),
      optimistic: () =>
        setVendors((cur) =>
          (cur ?? []).map((vendor) =>
            vendor.id === target.id
              ? {
                  ...vendor,
                  name: form.name.trim(),
                  code: vendorFields().code,
                  default_currency:
                    form.currency.trim() === "" ? vendor.default_currency : form.currency.trim().toUpperCase(),
                  account_number: form.accountNumber.trim() === "" ? null : form.accountNumber.trim(),
                  website: form.website.trim() === "" ? null : form.website.trim(),
                  email: form.email.trim() === "" ? null : form.email.trim(),
                  phone: form.phone.trim() === "" ? null : form.phone.trim(),
                  notes: form.notes.trim() === "" ? null : form.notes.trim(),
                  active: form.active,
                }
              : vendor,
          ),
        ),
      revert: () =>
        setVendors((cur) => (cur ?? []).map((vendor) => (vendor.id === target.id ? target : vendor))),
      request: (client, requestBody) =>
        client.PATCH("/api/v1/vendors/{id}", {
          params: { path: { id: target.id } },
          body: requestBody,
        }),
      versions: (response) => [[response.id, response.mutation.version]],
    });
    setFormBusy(false);
    if (result.ok === false) {
      setFormError(rejectionText(result, "Could not save the vendor."));
      return;
    }
    setEditOpen(false);
    setNotice("Vendor saved.");
    if (expandedId === target.id) await loadDetail(target.id);
  }

  async function saveContacts(): Promise<void> {
    if (detail === null) return;
    setContactsError(null);
    const contacts: VendorContactInput[] = [];
    for (const [index, row] of contactRows.entries()) {
      const name = row.name.trim();
      if (name === "" && row.email.trim() === "" && row.phone.trim() === "") continue;
      if (name === "") {
        setContactsError(`Contact ${index + 1} needs a name.`);
        return;
      }
      contacts.push({
        name,
        email: row.email.trim() === "" ? null : row.email.trim(),
        phone: row.phone.trim() === "" ? null : row.phone.trim(),
        role: row.role.trim() === "" ? null : row.role.trim(),
        is_primary: row.isPrimary,
      });
    }
    const vendorId = detail.vendor.id;
    setContactsBusy(true);
    const box: { contacts: VendorContact[] | null } = { contacts: null };
    const result = await runEnvelopeMutation({
      describe: "Replace vendor contacts",
      entityId: `${vendorId}:contacts`,
      buildBody: (base) => ({ contacts, ...base }),
      optimistic: () => {},
      revert: () => {},
      request: async (client, requestBody) => {
        const response = await client.POST("/api/v1/vendors/{id}/contacts", {
          params: { path: { id: vendorId } },
          body: requestBody,
        });
        if (response.data !== undefined) box.contacts = response.data.contacts;
        return response;
      },
      versions: (response) => [[`${vendorId}:contacts`, response.mutation.version]],
    });
    setContactsBusy(false);
    if (result.ok === false) {
      setContactsError(rejectionText(result, "Could not save the contacts."));
      return;
    }
    const saved = box.contacts ?? [];
    setDetail({ ...detail, contacts: saved });
    setContactRows(saved.map(contactRow));
    setNotice("Contacts saved.");
  }

  async function addLink(): Promise<void> {
    if (detail === null) return;
    setLinkError(null);
    if (linkForm.url.trim() === "") {
      setLinkError("A URL is required.");
      return;
    }
    const vendorId = detail.vendor.id;
    setLinkBusy(true);
    const box: { link: ExternalLink | null } = { link: null };
    const result = await runEnvelopeMutation({
      describe: "Add vendor link",
      buildBody: (base) => ({
        kind: linkForm.kind,
        url: linkForm.url.trim(),
        label: linkForm.label.trim() === "" ? null : linkForm.label.trim(),
        carrier: linkForm.carrier.trim() === "" ? null : linkForm.carrier.trim(),
        ...base,
      }),
      optimistic: () => {},
      revert: () => {},
      request: async (client, requestBody) => {
        const response = await client.POST("/api/v1/vendors/{id}/links", {
          params: { path: { id: vendorId } },
          body: requestBody,
        });
        if (response.data !== undefined) box.link = response.data.link;
        return response;
      },
      versions: (response) => [[response.link.id, response.mutation.version]],
    });
    setLinkBusy(false);
    if (result.ok === false) {
      setLinkError(rejectionText(result, "Could not add the link — the URL must be https."));
      return;
    }
    const link = box.link;
    if (link !== null) setDetail({ ...detail, links: [...detail.links, link] });
    setLinkForm(emptyLinkForm());
    setNotice("Link added.");
  }

  async function removeLink(link: ExternalLink): Promise<void> {
    if (detail === null) return;
    setLinkError(null);
    setActionError(null);
    setLinkBusyId(link.id);
    const result = await runEnvelopeMutation({
      describe: "Remove vendor link",
      buildBody: () => ({}),
      optimistic: () => setDetail((cur) => (cur === null ? cur : { ...cur, links: cur.links.filter((l) => l.id !== link.id) })),
      revert: () =>
        setDetail((cur) => (cur === null ? cur : { ...cur, links: [...cur.links, link] })),
      request: (client) =>
        client.DELETE("/api/v1/external-links/{id}", { params: { path: { id: link.id } } }),
    });
    setLinkBusyId(null);
    if (result.ok === false) {
      setActionError(rejectionText(result, "Could not remove the link."));
    }
  }

  const rows = filtered.filter((vendor) => {
    for (const clause of clauses) {
      const value = (
        {
          vendor: vendor.name,
          code: vendor.code ?? "",
          contact: vendor.email ?? vendor.phone ?? "",
          state: vendor.active ? "active" : "inactive",
        }[clause.field] ?? ""
      ).toLowerCase();
      const want = (clause.value ?? "").toLowerCase();
      if (clause.op === "is_set" && value === "") return false;
      if (clause.op === "is_not_set" && value !== "") return false;
      if (clause.op === "contains" && !value.includes(want)) return false;
      if (clause.op === "eq" && value !== want) return false;
      if (clause.op === "ne" && value === want) return false;
    }
    return true;
  });

  if (vendors === null && loadError === null) return <Spinner />;
  if (loadError !== null) return <EmptyState message={loadError} />;

  return (
    <>
    <LedgerSurface
      surface="purchasing-vendors"
      testId="vendors-ledger"
      title="Vendors"
      subtitle="Suppliers, their contacts, and the portal links a buyer opens mid-order."
      headerAction={
        <Button onClick={openCreate} data-testid="purchasing-vendors-new">
          New vendor
        </Button>
      }
      views={VENDOR_VIEWS}
      activeView="vendors"
      onViewChange={(id) => {
        if (id === "orders") router.push("/purchasing");
      }}
      columns={VENDOR_COLUMNS}
      filterFields={VENDOR_FILTERS}
      filterClauses={clauses}
      onFilterChange={setClauses}
      rowCount={rows.length}
      rowsShown={rows.length}
      onExport={(request) => {
        const stamp = new Date().toISOString().slice(0, 10);
        downloadCsv(
          csvFileName("vendors", stamp),
          request.columns,
          rows.map((vendor) => ({
            vendor: vendor.name,
            code: vendor.code ?? "",
            currency: vendor.default_currency,
            contact: vendor.email ?? vendor.phone ?? "",
            state: vendor.active ? "active" : "inactive",
            website: vendor.website ?? "",
          })),
        );
      }}
      selectedIds={selected}
      onToggleRow={(id, isSelected) =>
        setSelected((cur) => (isSelected ? [...cur, id] : cur.filter((row) => row !== id)))
      }
      onToggleAll={(isSelected) => setSelected(isSelected ? rows.map((v) => v.id) : [])}
      allSelected={rows.length > 0 && selected.length === rows.length}
      someSelected={selected.length > 0 && selected.length < rows.length}
      bulkActions={[
        {
          key: "edit",
          label: "Edit",
          disabled: selected.length !== 1,
          title: "Edit the selected vendor",
          onClick: () => {
            const only = rows.find((vendor) => vendor.id === selected[0]);
            if (only != null) openEdit(only);
          },
        },
      ]}
      onClearSelection={() => setSelected([])}
      emptyMessage="No vendors match. Add the supplier you order from."
      emptyAction={
        <Button variant="secondary" onClick={openCreate} data-testid="purchasing-vendors-empty-new">
          New vendor
        </Button>
      }
      errorMessage={actionError}
      onRetry={() => void refresh()}
    >
      {(columns) =>
        rows.map((vendor) => (
          <LedgerRow
            key={vendor.id}
            id={vendor.id}
            selected={selected.includes(vendor.id)}
            selectable
            onToggle={(rowId, isSelected) =>
              setSelected((cur) => (isSelected ? [...cur, rowId] : cur.filter((id) => id !== rowId)))
            }
            columns={columns}
            selectLabel={`Select ${vendor.name}`}
            testId={`purchasing-vendor-row-${vendor.id}`}
            cells={{
              vendor: (
                <span className="flex items-center gap-1.5">
                  <span style={{ color: "var(--m-text)" }}>{vendor.name}</span>
                  {vendor.website != null && vendor.website !== "" ? <Tag tone="outline">web</Tag> : null}
                </span>
              ),
              code: <span className="text-xs">{vendor.code ?? "—"}</span>,
              currency: <span className="text-xs">{vendor.default_currency}</span>,
              contact: vendor.email ?? vendor.phone ?? "—",
              state: vendor.active ? (
                <ReturnedMark label="Active" testId={`purchasing-vendor-state-${vendor.id}`} />
              ) : (
                <NeutralMark label="Inactive" />
              ),
              actions: (
                <span className="flex justify-end gap-1">
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() => void toggleDetail(vendor.id)}
                    data-testid={`purchasing-vendor-open-${vendor.id}`}
                  >
                    {expandedId === vendor.id ? "Close" : "Contacts"}
                  </Button>
                  <Button size="sm" variant="ghost" onClick={() => openEdit(vendor)} data-testid={`purchasing-vendor-edit-${vendor.id}`}>
                    Edit
                  </Button>
                </span>
              ),
            }}
          />
        ))
      }
    </LedgerSurface>


      {expandedId !== null && (
        <Card className="m-rise grid gap-4 p-4" data-testid={`purchasing-vendor-panel-${expandedId}`}>
          {detailLoading && <Spinner />}
          {detailError !== null && <EmptyState message={detailError} />}
          {detail !== null && (
            <>
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div>
                  <h2 className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
                    {detail.vendor.name}
                  </h2>
                  <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                    {detail.contacts.length} contact{detail.contacts.length === 1 ? "" : "s"} · {detail.links.length} link
                    {detail.links.length === 1 ? "" : "s"}
                  </p>
                </div>
                <div className="flex flex-wrap gap-1">
                  {detail.vendor.website != null && detail.vendor.website !== "" && (
                    <Button asChild size="sm" variant="secondary">
                      <a
                        href={detail.vendor.website}
                        target="_blank"
                        rel="noreferrer"
                        data-testid="purchasing-vendor-website"
                      >
                        Website
                      </a>
                    </Button>
                  )}
                  <Button size="sm" variant="secondary" onClick={() => openEdit(detail.vendor)} data-testid="purchasing-vendor-panel-edit">
                    Edit vendor
                  </Button>
                </div>
              </div>

              <div className="grid gap-3">
                <div className="flex items-center justify-between">
                  <p className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
                    Contacts
                  </p>
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() =>
                      setContactRows((cur) => [
                        ...cur,
                        {
                          key: `new-contact-${Date.now()}`,
                          name: "",
                          email: "",
                          phone: "",
                          role: "",
                          isPrimary: cur.length === 0,
                        },
                      ])
                    }
                    data-testid="purchasing-contact-add"
                  >
                    Add contact
                  </Button>
                </div>
                <FieldHint>Contacts are replaced as a list — every save rewrites the set.</FieldHint>
                {contactRows.map((row, index) => (
                  <div
                    key={row.key}
                    className="grid gap-2 border-t pt-3"
                    style={{ borderColor: "var(--m-border)" }}
                    data-testid={`purchasing-contact-${index}`}
                  >
                    <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
                      <div className="grid gap-1">
                        <Label htmlFor={`purchasing-contact-${index}-name`}>Name</Label>
                        <Input
                          id={`purchasing-contact-${index}-name`}
                          value={row.name}
                          onChange={(e) =>
                            setContactRows((cur) =>
                              cur.map((r) => (r.key === row.key ? { ...r, name: e.target.value } : r)),
                            )
                          }
                          data-testid={`purchasing-contact-${index}-name`}
                        />
                      </div>
                      <div className="grid gap-1">
                        <Label htmlFor={`purchasing-contact-${index}-email`}>Email</Label>
                        <Input
                          id={`purchasing-contact-${index}-email`}
                          value={row.email}
                          onChange={(e) =>
                            setContactRows((cur) =>
                              cur.map((r) => (r.key === row.key ? { ...r, email: e.target.value } : r)),
                            )
                          }
                          data-testid={`purchasing-contact-${index}-email`}
                        />
                      </div>
                      <div className="grid gap-1">
                        <Label htmlFor={`purchasing-contact-${index}-phone`}>Phone</Label>
                        <Input
                          id={`purchasing-contact-${index}-phone`}
                          value={row.phone}
                          onChange={(e) =>
                            setContactRows((cur) =>
                              cur.map((r) => (r.key === row.key ? { ...r, phone: e.target.value } : r)),
                            )
                          }
                          data-testid={`purchasing-contact-${index}-phone`}
                        />
                      </div>
                      <div className="grid gap-1">
                        <Label htmlFor={`purchasing-contact-${index}-role`}>Role</Label>
                        <Input
                          id={`purchasing-contact-${index}-role`}
                          value={row.role}
                          onChange={(e) =>
                            setContactRows((cur) =>
                              cur.map((r) => (r.key === row.key ? { ...r, role: e.target.value } : r)),
                            )
                          }
                          data-testid={`purchasing-contact-${index}-role`}
                        />
                      </div>
                    </div>
                    <div className="flex items-center justify-between">
                      <label className="flex items-center gap-1.5 text-xs" style={{ color: "var(--m-text-secondary)" }}>
                        <Checkbox
                          checked={row.isPrimary}
                          onCheckedChange={(value) =>
                            setContactRows((cur) =>
                              cur.map((r) => (r.key === row.key ? { ...r, isPrimary: value === true } : r)),
                            )
                          }
                          data-testid={`purchasing-contact-${index}-primary`}
                        />
                        Primary contact
                      </label>
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={() => setContactRows((cur) => cur.filter((r) => r.key !== row.key))}
                        data-testid={`purchasing-contact-${index}-remove`}
                      >
                        Remove
                      </Button>
                    </div>
                  </div>
                ))}
                {contactsError !== null && (
                  <p className="text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }} data-testid="purchasing-contacts-error">
                    {contactsError}
                  </p>
                )}
                <div className="flex justify-end">
                  <Button disabled={contactsBusy} onClick={() => void saveContacts()} data-testid="purchasing-contacts-save">
                    Save contacts
                  </Button>
                </div>
              </div>

              <div className="grid gap-3 border-t pt-3" style={{ borderColor: "var(--m-border)" }}>
                <div>
                  <p className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
                    Portal links
                  </p>
                  <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                    These become one-click buttons on every order for this vendor.
                  </p>
                </div>
                {detail.links.length > 0 && (
                  <div className="flex flex-wrap items-center gap-1.5">
                    {detail.links.map((link) => (
                      <span key={link.id} className="flex items-center gap-1">
                        <Button asChild size="sm" variant="secondary">
                          <a
                            href={link.url}
                            target="_blank"
                            rel="noreferrer"
                            data-testid={`purchasing-vendor-link-${link.id}`}
                          >
                            {link.label ?? LINK_KIND_LABELS[link.kind]}
                          </a>
                        </Button>
                        <Button
                          size="sm"
                          variant="ghost"
                          disabled={linkBusyId === link.id}
                          onClick={() => void removeLink(link)}
                          data-testid={`purchasing-vendor-link-remove-${link.id}`}
                        >
                          Remove
                        </Button>
                      </span>
                    ))}
                  </div>
                )}
                <div className="grid gap-2 sm:grid-cols-5">
                  <div className="grid gap-1">
                    <Label htmlFor="purchasing-vendor-link-url">URL</Label>
                    <Input
                      id="purchasing-vendor-link-url"
                      value={linkForm.url}
                      onChange={(e) => setLinkForm({ ...linkForm, url: e.target.value })}
                      placeholder="https://"
                      data-testid="purchasing-vendor-link-url"
                    />
                  </div>
                  <div className="grid gap-1">
                    <Label htmlFor="purchasing-vendor-link-label">Label</Label>
                    <Input
                      id="purchasing-vendor-link-label"
                      value={linkForm.label}
                      onChange={(e) => setLinkForm({ ...linkForm, label: e.target.value })}
                      data-testid="purchasing-vendor-link-label"
                    />
                  </div>
                  <div className="grid gap-1">
                    <Label htmlFor="purchasing-vendor-link-kind">Kind</Label>
                    <Select
                      value={linkForm.kind}
                      onValueChange={(value) => setLinkForm({ ...linkForm, kind: value as ExternalLinkKind })}
                    >
                      <SelectTrigger id="purchasing-vendor-link-kind" data-testid="purchasing-vendor-link-kind">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {LINK_KINDS.map((kind) => (
                          <SelectItem key={kind} value={kind}>
                            {LINK_KIND_LABELS[kind]}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="grid gap-1">
                    <Label htmlFor="purchasing-vendor-link-carrier">Carrier</Label>
                    <Input
                      id="purchasing-vendor-link-carrier"
                      value={linkForm.carrier}
                      onChange={(e) => setLinkForm({ ...linkForm, carrier: e.target.value })}
                      data-testid="purchasing-vendor-link-carrier"
                    />
                  </div>
                  <div className="flex items-end">
                    <Button disabled={linkBusy} onClick={() => void addLink()} data-testid="purchasing-vendor-link-add">
                      Add link
                    </Button>
                  </div>
                </div>
                {linkError !== null && (
                  <p className="text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }} data-testid="purchasing-vendor-link-error">
                    {linkError}
                  </p>
                )}
              </div>
            </>
          )}
        </Card>
      )}

      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent title={editing === null ? "New vendor" : `Edit ${editing.name}`}>
          <div className="grid max-h-[80vh] gap-3 overflow-auto p-1">
            <div className="grid gap-2 sm:grid-cols-3">
              <div className="grid gap-1 sm:col-span-2">
                <Label htmlFor="purchasing-vendor-name">Name</Label>
                <Input
                  id="purchasing-vendor-name"
                  value={form.name}
                  onChange={(e) => setForm({ ...form, name: e.target.value })}
                  data-testid="purchasing-vendor-name"
                />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="purchasing-vendor-code">Code</Label>
                <Input
                  id="purchasing-vendor-code"
                  value={form.code}
                  onChange={(e) => setForm({ ...form, code: e.target.value })}
                  data-testid="purchasing-vendor-code"
                />
              </div>
            </div>
            <div className="grid gap-2 sm:grid-cols-3">
              <div className="grid gap-1">
                <Label htmlFor="purchasing-vendor-currency">Default currency</Label>
                <Input
                  id="purchasing-vendor-currency"
                  value={form.currency}
                  maxLength={3}
                  onChange={(e) => setForm({ ...form, currency: e.target.value.toUpperCase() })}
                  data-testid="purchasing-vendor-currency"
                />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="purchasing-vendor-account">Account number</Label>
                <Input
                  id="purchasing-vendor-account"
                  value={form.accountNumber}
                  onChange={(e) => setForm({ ...form, accountNumber: e.target.value })}
                  data-testid="purchasing-vendor-account"
                />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="purchasing-vendor-website">Website</Label>
                <Input
                  id="purchasing-vendor-website"
                  value={form.website}
                  onChange={(e) => setForm({ ...form, website: e.target.value })}
                  placeholder="https://"
                  data-testid="purchasing-vendor-website-input"
                />
              </div>
            </div>
            <div className="grid gap-2 sm:grid-cols-2">
              <div className="grid gap-1">
                <Label htmlFor="purchasing-vendor-email">Email</Label>
                <Input
                  id="purchasing-vendor-email"
                  value={form.email}
                  onChange={(e) => setForm({ ...form, email: e.target.value })}
                  data-testid="purchasing-vendor-email"
                />
              </div>
              <div className="grid gap-1">
                <Label htmlFor="purchasing-vendor-phone">Phone</Label>
                <Input
                  id="purchasing-vendor-phone"
                  value={form.phone}
                  onChange={(e) => setForm({ ...form, phone: e.target.value })}
                  data-testid="purchasing-vendor-phone"
                />
              </div>
            </div>
            <div className="grid gap-1">
              <Label htmlFor="purchasing-vendor-notes">Notes</Label>
              <Input
                id="purchasing-vendor-notes"
                value={form.notes}
                onChange={(e) => setForm({ ...form, notes: e.target.value })}
                data-testid="purchasing-vendor-notes"
              />
            </div>
            {editing !== null && (
              <label className="flex items-center gap-2 text-xs" style={{ color: "var(--m-text-secondary)" }}>
                <Checkbox
                  checked={form.active}
                  onCheckedChange={(value) => setForm({ ...form, active: value === true })}
                  data-testid="purchasing-vendor-active"
                />
                Active — inactive vendors stay out of new orders
              </label>
            )}
            {formError !== null && (
              <p className="text-xs" role="alert" style={{ color: "var(--m-status-danger-fg)" }} data-testid="purchasing-vendor-form-error">
                {formError}
              </p>
            )}
            <DialogActions>
              <Button variant="secondary" onClick={() => setEditOpen(false)}>
                Cancel
              </Button>
              <Button disabled={formBusy} onClick={() => void save()} data-testid="purchasing-vendor-save">
                {editing === null ? "Create vendor" : "Save vendor"}
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
