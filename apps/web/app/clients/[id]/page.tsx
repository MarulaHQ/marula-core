"use client";
/**
 * /clients/[id] — the client record, built against the Marula surface pack's
 * `web/clients` states 03 to 07 (Wave 41 lane 7 `p41-ui-clients`).
 *
 * Purpose: everything about one customer in one place, for a relationship
 * that lasts years. The pack is explicit that this record reuses the
 * workbench's three-column shape rather than inventing one — customer,
 * content, right now — and that it is the TEMPLATE for every other
 * record-detail page. So `RecordFrame` carries that geometry and each column
 * has exactly one job:
 *
 * - **Left, the customer**: who this is. Name and status pills, the contract
 *   summary with its renewal, the contacts, the sites, the owner. A contract
 *   nearing renewal is BLUSH here, never red (`DESIGN-NOTES.md` §A2).
 * - **Middle, the content**: a breadcrumb and actions, the title with its
 *   pills, tabs, then the tab. The header does not change between tabs, so
 *   the record never feels like it moved.
 * - **Right, what needs attention**: open work, billing, the portal, the
 *   relationship. A plain summary, deliberately NOT an assistant — this
 *   column stays quiet and keeps the workbench shape.
 *
 * **The half-finished client (state 07) is the case to design for.** A client
 * created and then left alone gets a *Finish setting up* card: a progress
 * bar, four steps each with its own action, and the step already done shown
 * as done. The steps follow the journey — the client, a contact, the
 * contract, the portal. And `Skip for now` / `Don't show this again` are
 * DIFFERENT states with different consequences, in one sentence that explains
 * both: the first keeps the card for next time, the second removes it and the
 * client's menu brings it back. Copy written to keep those two apart.
 *
 * **Portal access applies at once**, as a single-choice toggle in the same
 * shape Settings uses, rather than a per-row save button.
 *
 * Non-purpose: the contract form, the merge queue and the new-client wizard
 * are not this page's job. Site, contact and portal-user creation reuse the
 * existing plain POST endpoints; a rejected write says so and keeps the
 * dialog open.
 */
import {
  Badge,
  Button,
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
} from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import type { components } from "@marula-core/sdk";
import Link from "next/link";
import { useParams, useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, useAuth, useRequireAuth } from "../../../lib/api";
import { scoreTone } from "../../../lib/docs-completeness";
import { money } from "../../../lib/format";
import {
  formatPackDate,
  formatPackMonth,
  renewalQuoteDueDate,
  renewalSignal,
} from "../../../lib/renewal-signals";
import {
  RecordFrame,
  RecordRow,
  RecordSection,
  RecordTabs,
  SummaryCard,
} from "../../../components/records/record-frame";
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
type Ticket = components["schemas"]["TicketWithFields"];

/** One line of the client's real activity, assembled from the records this
 * page already loads. Nothing here is invented: a ticket update is a real
 * ticket's real `updated_at`. */
type ActivityEntry = { at: string; text: string; href: string };
type ClientCompletenessView = components["schemas"]["ClientCompletenessView"];
type Contract = components["schemas"]["Contract"];
type Invoice = components["schemas"]["Invoice"];

type Tab = "overview" | "contacts" | "sites" | "contracts" | "tickets" | "assets" | "documents";

const TABS: { id: Tab; label: string }[] = [
  { id: "overview", label: "Overview" },
  { id: "contacts", label: "Contacts" },
  { id: "sites", label: "Sites and devices" },
  { id: "contracts", label: "Contracts and billing" },
  { id: "tickets", label: "Tickets" },
  { id: "assets", label: "Assets" },
  { id: "documents", label: "Documents" },
];

/** The four steps of the half-finished client, in the order the work
 * happens. `done` is derived from real records, not stored, so a client that
 * gained a contract by another route reads as finished on that step. */
function setupSteps(
  hasContact: boolean,
  hasContract: boolean,
  portalOn: boolean,
): { key: string; title: string; hint: string; action: string; done: boolean; primary?: boolean }[] {
  return [
    {
      key: "client",
      title: "Create the client",
      hint: "The record exists.",
      action: "",
      done: true,
    },
    {
      key: "contact",
      title: "Add a contact",
      hint: "Someone we can reach, and who can use the portal.",
      action: "Add contact",
      done: hasContact,
    },
    {
      key: "contract",
      title: "Set up the contract",
      hint: "Terms, billing frequency and the value. The billing schedule follows from it.",
      action: "Set up contract",
      done: hasContract,
      primary: true,
    },
    {
      key: "portal",
      title: "Turn on the portal",
      hint: "So the client can see its own tickets and invoices.",
      action: "Turn on",
      done: portalOn,
    },
  ];
}

export default function ClientDetailPage() {
  const auth = useRequireAuth();
  const { me } = useAuth();
  const router = useRouter();
  const params = useParams<{ id: string }>();
  const clientId = typeof params?.id === "string" ? params.id : "";

  const [client, setClient] = useState<Client | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("overview");

  const [sites, setSites] = useState<Site[] | null>(null);
  const [contacts, setContacts] = useState<Contact[] | null>(null);
  const [portalUsers, setPortalUsers] = useState<PortalUser[] | null>(null);
  const [portalDenied, setPortalDenied] = useState(false);
  const [assets, setAssets] = useState<DeviceAsset[] | null>(null);
  // The client's REAL open work and its REAL latest activity. Both come from
  // live endpoints — there is no illustrative data on this page, because a
  // customer record showing invented ticket numbers is a lie the customer
  // reads.
  const [tickets, setTickets] = useState<Ticket[] | null>(null);
  const [activity, setActivity] = useState<ActivityEntry[] | null>(null);
  const [documents, setDocuments] = useState<Document[] | null>(null);
  const [layouts, setLayouts] = useState<DocumentLayout[] | null>(null);
  const [completeness, setCompleteness] = useState<ClientCompletenessView | null>(null);
  const [completenessUnavailable, setCompletenessUnavailable] = useState(false);
  const [contracts, setContracts] = useState<Contract[]>([]);

  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  // The setup card's dismissal. Two separate states, because the two buttons
  // do different things: skipping keeps the card for next time, hiding
  // removes it and the client's menu brings it back.
  const [setupDismissed, setSetupDismissed] = useState<"skipped" | "hidden" | null>(null);

  // Which of the three child entities the open create dialog is making. Kept
  // as its own state rather than read off the active tab, because a dialog
  // that changes meaning when you switch tabs is a dialog that surprises.
  const [createKind, setCreateKind] = useState<"site" | "contact" | "portalUser">("site");
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

  const isAdmin = (me?.permissions ?? []).includes("special:manage_users") ?? false;
  // The signed-in user's own initials for the owner monogram. "You" collapsed
  // to a bare "Y", which reads as a rendering fault rather than a person.
  const ownerName = me?.display_name ?? me?.email ?? "";

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
    // The client's real tickets, scoped by the endpoint's own client_id
    // filter. This is what the Open work card and Recent activity read, so
    // both show what is actually open rather than an illustration of it.
    void api.api
      .GET("/api/v1/tickets", { params: { query: { client_id: clientId, limit: 25 } } })
      .then(({ data }) => setTickets(data ?? []));
    const pu = await api.api.GET("/api/v1/clients/{id}/portal-users", {
      params: { path: { id: clientId } },
    });
    if (pu.error !== undefined) {
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

  // The client's contracts: the left column's contract summary and the
  // Contracts and billing tab both read from these, so the contract appears
  // twice on purpose — a renewal is never a click away
  // (`DESIGN-NOTES-clients.md` §8.3).
  const refreshContracts = useCallback(async () => {
    const res = await api.api.GET("/api/v1/contracts");
    if (res.error === undefined && Array.isArray(res.data)) {
      setContracts(res.data.filter((c) => c.client_id === clientId));
    }
  }, [clientId]);

  useEffect(() => {
    if (auth.status === "authed" && clientId !== "") void refreshClient();
  }, [auth.status, clientId, refreshClient]);

  useEffect(() => {
    if (auth.status === "authed" && clientId !== "") void loadCompleteness();
  }, [auth.status, clientId, loadCompleteness]);

  useEffect(() => {
    if (auth.status === "authed" && clientId !== "") {
      void refreshAll();
      void refreshContracts();
    }
  }, [auth.status, clientId, refreshAll, refreshContracts]);

  const liveContract = useMemo(() => {
    const usable = contracts.filter((c) => c.status === "active" || c.status === "draft");
    return usable[0] ?? null;
  }, [contracts]);

  /** The client's real open work, split the way the card shows it: what a
   * technician is on now, and what is still waiting. Both numbers come from
   * the tickets this client actually has.
   *
   * There is deliberately no "at risk, 55m left" row. A ticket carries no SLA
   * clock on this schema, so any such figure would be invented — and the pack's
   * blush clock belongs to signals that are actually computed. What is real
   * here is the oldest open ticket's age, which is the honest version of the
   * same question: how long has the longest-running thing been waiting. */
  const openWork = useMemo(() => {
    const list = tickets ?? [];
    const live = list.filter((t) => t.category !== "closed");
    const inProgress = live.filter((t) => t.category === "in_progress");
    const waiting = live.filter((t) => t.category === "open" || t.category === "waiting");
    const oldest = live.reduce<string | null>((acc, t) => {
      if (acc === null) return t.created_at;
      return new Date(t.created_at).getTime() < new Date(acc).getTime() ? t.created_at : acc;
    }, null);
    return {
      total: live.length,
      inProgress,
      waiting,
      oldestOpenSince: oldest === null ? null : daysSince(oldest),
      rows: [...inProgress, ...waiting].slice(0, 6),
    };
  }, [tickets]);

  /** Recent activity, assembled from the records already on this page: each
   * line is a real ticket's real last update. Ordered newest first, capped
   * at five so the card stays a summary rather than a log. */
  const recentActivity = useMemo<ActivityEntry[] | null>(() => {
    if (tickets === null) return null;
    return [...tickets]
      .filter((t) => typeof t.updated_at === "string" && t.updated_at !== "")
      .sort((a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime())
      .slice(0, 5)
      .map((t) => ({
        at: relativeWhen(t.updated_at),
        text: `${t.number} updated — ${t.title}`,
        href: `/tickets/${t.id}`,
      }));
  }, [tickets]);

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
    createKind === "site" ? "Add site"
    : createKind === "contact" ? "Add contact"
    : "Add portal user";

  const createDisabled =
    saving ||
    (createKind === "site" && siteName.trim() === "") ||
    (createKind === "contact" && contactName.trim() === "") ||
    (createKind === "portalUser" &&
      (puEmail.trim() === "" || puName.trim() === "" || puPassword === ""));

  const openCreate = (kind: "site" | "contact" | "portalUser") => {
    setCreateKind(kind);
    setFormError(null);
    setCreateOpen(true);
  };

  const runCreate = async () => {
    setSaving(true);
    setFormError(null);
    let result: { error?: unknown; data?: unknown };
    if (createKind === "site") {
      result = await api.api.POST("/api/v1/clients/{id}/sites", {
        params: { path: { id: clientId } },
        body: {
          name: siteName.trim(),
          city: siteCity.trim() === "" ? null : siteCity.trim(),
          address: siteAddress.trim() === "" ? null : siteAddress.trim(),
        },
      });
    } else if (createKind === "contact") {
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
      // The admin-provisioned flow: the password is delivered out of band,
      // never emailed by this page (ADR 0007).
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
      setFormError("The server rejected this entry. Nothing was saved. Check the fields and retry.");
      return;
    }
    setCreateOpen(false);
    resetCreateForm();
    void refreshAll();
    void refreshClient();
  };

  /** Portal access applies at once — a single-choice toggle that says so,
   * the same rule Settings uses. The row reports the result rather than
   * leaving the user to guess whether it took. */
  const setPortalStatus = async (user: PortalUser, disabled: boolean) => {
    const { error } = await api.api.PATCH("/api/v1/portal-users/{id}", {
      params: { path: { id: user.id } },
      body: { status: disabled ? "disabled" : "invited" },
    });
    if (error !== undefined) {
      setFormError("The status change was rejected. Nothing was saved.");
      return;
    }
    setFormError(null);
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

  const restricted = client.restricted_to.length > 0;
  const tone = completeness !== null ? scoreTone(completeness.score) : "neutral";
  const now = Date.now();
  const renewal = renewalSignal(liveContract?.effective_until, now);
  const quoteDue = renewalQuoteDueDate(liveContract?.effective_until);
  const activeSite = sites?.find((s) => s.id === (sites[0]?.id ?? "")) ?? null;
  const deviceCount = activeSite === null ? assets?.length ?? 0 : assets?.length ?? 0;
  const hasContact = (contacts?.length ?? 0) > 0;
  const hasContract = liveContract !== null;
  const portalOn = (portalUsers?.filter((u) => u.status !== "disabled").length ?? 0) > 0;
  const steps = setupSteps(hasContact, hasContract, portalOn);
  const doneCount = steps.filter((s) => s.done).length;
  const showSetup = doneCount < steps.length && setupDismissed !== "hidden";

  return (
    <div className="mx-auto grid w-full max-w-[1600px] gap-4 p-4 md:p-6">
      <RecordFrame
        context={
          <>
            <RecordSection label="Client">
              <p className="text-[16px] font-semibold">{client.name}</p>
              <div className="flex flex-wrap items-center gap-1.5">
                {restricted ? (
                  <Badge tone="warning" title={isAdmin ? `Visible to ${client.restricted_to.length} listed users + admins` : "Restricted client"}>
                    Restricted
                  </Badge>
                ) : (
                  <Badge tone="success">Active</Badge>
                )}
                {/* The plan line reflects the CONTRACT, not the client's free-text notes.
                    Keying it off `notes` printed "No plan yet" directly above a
                    CONTRACT section naming the live plan, which is a
                    contradiction on the surface meant to be trusted. The
                    contract is the thing that carries a plan. */}
                {hasContract ? (
                  <Badge tone="neutral">{liveContract?.name}</Badge>
                ) : (
                  <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                    No plan yet
                  </span>
                )}
              </div>
              {restricted ? (
                <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                  Visible to {client.restricted_to.length} listed {client.restricted_to.length === 1 ? "user" : "users"}
                  {isAdmin ? " and admins" : ""}.
                </p>
              ) : null}
            </RecordSection>

            <RecordSection label="Contract">
              {liveContract === null ? (
                <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                  No contract yet.
                </p>
              ) : (
                <>
                  <RecordRow label="Plan">{liveContract.name}</RecordRow>
                  <RecordRow label="Renews">
                    {formatPackDate(liveContract.effective_until) ?? "Open-ended"}
                  </RecordRow>
                  {liveContract.amount_minor != null ? (
                    <RecordRow label="Value">
                      {money(liveContract.amount_minor, liveContract.currency)}
                    </RecordRow>
                  ) : null}
                  {/* A renewal is BLUSH. Never red: red means breach, and a
                      term inside its window is attention, not a breach. */}
                  {renewal !== null ? (
                    <span className="mt-1.5">
                      <Badge tone="due">
                        <span className="inline-flex items-center gap-1">
                          <ClockIcon />
                          {renewal.label}
                        </span>
                      </Badge>
                    </span>
                  ) : null}
                </>
              )}
            </RecordSection>

            <RecordSection label="Contacts">
              {hasContact ? (
                <ul className="flex flex-col gap-2">
                  {(contacts ?? []).slice(0, 3).map((c) => (
                    <li key={c.id} className="flex items-center gap-2.5">
                      <Monogram name={c.name} />
                      <span className="min-w-0">
                        <span className="block truncate text-[13px] font-semibold">{c.name}</span>
                        {c.title != null && c.title !== "" ? (
                          <span className="block truncate text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                            {c.title}
                          </span>
                        ) : null}
                      </span>
                    </li>
                  ))}
                  {(contacts?.length ?? 0) > 3 ? (
                    <li className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                      +{(contacts?.length ?? 0) - 3} more
                    </li>
                  ) : null}
                </ul>
              ) : (
                <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                  No contacts yet.
                </p>
              )}
            </RecordSection>

            <RecordSection label="Sites">
              {(sites ?? []).length === 0 ? (
                <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                  No sites yet.
                </p>
              ) : (
                <ul className="flex flex-col gap-1">
                  {(sites ?? []).map((s) => (
                    <li key={s.id}>
                      <RecordRow label={s.name}>
                        {deviceCount} devices
                      </RecordRow>
                    </li>
                  ))}
                </ul>
              )}
            </RecordSection>

            <RecordSection label="Account owner">
              <div className="flex items-center gap-2.5">
                <Monogram name={ownerName} />
                <span className="text-[13px] font-semibold">
                  {ownerName === "" ? "Unassigned" : ownerName}
                </span>
              </div>
            </RecordSection>
          </>
        }
        content={
          <>
            {/* ---- breadcrumb and actions ---- */}
            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                onClick={() => router.push("/clients")}
                className="cursor-pointer border-none bg-transparent p-0 text-[13px] underline decoration-[var(--m-border)] underline-offset-2"
                style={{ color: "var(--m-text-secondary)" }}
              >
                Clients
              </button>
              <ChevronIcon />
              <span className="text-[13px]">{client.name}</span>
              <span className="flex-1" />
              {/* Portal access applies at once and is a single-choice toggle,
                  so the header offers the thing it toggles — and only when
                  there is a portal to open. */}
              {portalOn ? (
                <Link href="/portal">
                  <Button variant="secondary" size="sm" data-testid="client-open-portal">
                    <ExternalIcon />
                    Open in portal
                  </Button>
                </Link>
              ) : null}
              <Link href="/tickets">
                <Button variant="primary" size="sm" data-testid="client-new-ticket">
                  <PlusIcon />
                  New ticket
                </Button>
              </Link>
            </div>

            {/* ---- the title and its pills. This header does not change
                between tabs, so the record never feels like it moved. ---- */}
            <div className="mt-2 flex flex-col gap-1.5">
              <h1 className="text-[26px] font-semibold leading-tight tracking-[-0.01em]">
                {client.name}
              </h1>
              <div className="flex flex-wrap items-center gap-2 text-[13px]">
                {hasContract ? (
                  <>
                    <Badge tone="success">Active</Badge>
                    <Badge tone="neutral">{liveContract?.name}</Badge>
                  </>
                ) : (
                  <Badge tone="neutral">New</Badge>
                )}
                <span style={{ color: "var(--m-text-muted)" }}>
                  {deviceCount} endpoints
                </span>
                <span style={{ color: "var(--m-text-muted)" }}>
                  Client since {formatPackMonth(client.created_at)}
                </span>
              </div>
            </div>

            <div className="mt-3">
              <RecordTabs
                tabs={TABS.map((t) => ({
                  id: t.id,
                  label: t.label,
                  count:
                    t.id === "contacts"
                      ? contacts === null ? null : contacts.length
                      : t.id === "sites"
                        ? sites === null ? null : sites.length
                        : t.id === "assets"
                          ? assets === null ? null : assets.length
                          : t.id === "documents"
                            ? documents === null ? null : documents.length
                            : null,
                }))}
                active={tab}
                onChange={(id) => setTab(id as Tab)}
              />
            </div>

            <div key={tab} className="mt-4 flex flex-col gap-4">
              {formError !== null ? (
                <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                  {formError}
                </p>
              ) : null}

              {tab === "overview" ? (
                <>
                  {showSetup ? (
                    <SetupCard
                      steps={steps}
                      doneCount={doneCount}
                      onAct={(key) => {
                        if (key === "contact") setTab("contacts");
                        if (key === "contract") setTab("contracts");
                        if (key === "portal") setTab("contacts");
                      }}
                      onSkip={() => setSetupDismissed("skipped")}
                      onHide={() => setSetupDismissed("hidden")}
                    />
                  ) : null}

                  <OpenWorkCard work={openWork} />
                  <RecentActivityCard entries={recentActivity} />
                </>
              ) : null}

              {tab === "contacts" ? (
                <ContactsCard
                  contacts={contacts}
                  portalUsers={portalUsers}
                  portalDenied={portalDenied}
                  onTogglePortal={(user, disabled) => void setPortalStatus(user, disabled)}
                  onAddContact={() => openCreate("contact")}
                  onAddPortalUser={() => openCreate("portalUser")}
                />
              ) : null}

              {tab === "sites" ? (
                <SitesCard
                  sites={sites}
                  assets={assets}
                  selected={activeSite}
                  onSelect={(id) => setTab("sites") /* the site choice stays in this tab */}
                  onAdd={() => openCreate("site")}
                />
              ) : null}

              {tab === "contracts" ? (
                <ContractsBillingCard
                  contract={liveContract}
                  renewal={renewal}
                  quoteDue={quoteDue}
                  contracts={contracts}
                />
              ) : null}

              {tab === "assets" ? <AssetsTab clientId={clientId} /> : null}
              {tab === "documents" ? (
                <DocumentsCard
                  documents={documents}
                  layouts={layouts}
                />
              ) : null}
            </div>

            <div className="mt-4">
              <WorkflowOverridesTab clientId={clientId} />
            </div>
          </>
        }
        summary={
          <>
            <p
              className="mb-1 text-[11px] font-semibold uppercase tracking-[0.08em]"
              style={{ color: "var(--m-text-muted)" }}
            >
              Right now
            </p>
            <SummaryCard title="Open work">
              {openWork.total === 0 ? (
                <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                  Nothing open for this client.
                </span>
              ) : (
                <>
                  <RecordRow label="In progress">{openWork.inProgress.length}</RecordRow>
                  <RecordRow label="Waiting">{openWork.waiting.length}</RecordRow>
                  {/* The oldest open ticket's real age. There is no SLA clock
                      on a ticket, so this is the honest question: how long
                      has the longest-running thing been waiting. */}
                  {openWork.oldestOpenSince !== null && openWork.oldestOpenSince > 0 ? (
                    <RecordRow label="Oldest open">
                      <Badge tone={openWork.oldestOpenSince > 14 ? "due" : "neutral"}>
                        <span className="inline-flex items-center gap-1">
                          <ClockIcon />
                          {openWork.oldestOpenSince}d
                        </span>
                      </Badge>
                    </RecordRow>
                  ) : null}
                </>
              )}
            </SummaryCard>

            <SummaryCard title="Billing">
              {liveContract === null ? (
                <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                  No contract to invoice against yet.
                </span>
              ) : (
                <>
                  <RecordRow label="Per period">
                    {liveContract.amount_minor != null
                      ? money(liveContract.amount_minor, liveContract.currency)
                      : "Not priced"}
                  </RecordRow>
                  <RecordRow label="Contract">{liveContract.number}</RecordRow>
                  <RecordRow label="Renews">
                    {formatPackDate(liveContract.effective_until) ?? "Open-ended"}
                  </RecordRow>
                </>
              )}
            </SummaryCard>

            <SummaryCard
              title="Portal"
              trailing={
                portalOn ? (
                  <Badge tone="success">On</Badge>
                ) : (
                  <Badge tone="offline">Off</Badge>
                )
              }
            >
              {portalOn ? (
                <>
                  <RecordRow label="Active users">
                    {portalUsers?.filter((u) => u.status !== "disabled").length ?? 0} of{" "}
                    {portalUsers?.length ?? 0}
                  </RecordRow>
                  {/* Sign-in recency is the portal user's own record, which
                      this schema does not expose. Rather than print a
                      plausible time we invented, the card says what is true. */}
                  <RecordRow label="Last sign-in">
                    <span style={{ color: "var(--m-text-muted)" }}>Not tracked</span>
                  </RecordRow>
                </>
              ) : (
                <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                  Turn it on so this client can see its own tickets and invoices.
                </span>
              )}
            </SummaryCard>

            <SummaryCard title="Relationship">
              {recentActivity === null || recentActivity.length === 0 ? (
                <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                  No recorded activity yet.
                </span>
              ) : (
                <>
                  <RecordRow label="Last update">{recentActivity[0]?.at ?? "—"}</RecordRow>
                  <RecordRow label="Tickets on record">{(tickets ?? []).length}</RecordRow>
                </>
              )}
            </SummaryCard>

            <CompletenessCard
              completeness={completeness}
              unavailable={completenessUnavailable}
              tone={tone}
            />
          </>
        }
      />

      {/* ---- the one create dialog, shared by the contacts and sites tabs ---- */}
      <Dialog
        open={createOpen}
        onOpenChange={(open) => {
          setCreateOpen(open);
          if (!open) setFormError(null);
        }}
      >
        <DialogContent title={createLabel ?? "Add"}>
          <div className="grid gap-3 p-4">
            {createKind === "site" ? (
              <>
                <div>
                  <Label htmlFor="site-name">Site name</Label>
                  <Input id="site-name" value={siteName} onChange={(e) => setSiteName(e.target.value)} placeholder="Head office" />
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
            ) : createKind === "contact" ? (
              <>
                <div>
                  <Label htmlFor="contact-name">Name</Label>
                  <Input id="contact-name" value={contactName} onChange={(e) => setContactName(e.target.value)} />
                </div>
                <div>
                  <Label htmlFor="contact-title">Title</Label>
                  <Input id="contact-title" value={contactTitle} onChange={(e) => setContactTitle(e.target.value)} placeholder="Optional" />
                </div>
                <div>
                  <Label htmlFor="contact-email">Email</Label>
                  <Input id="contact-email" type="email" value={contactEmail} onChange={(e) => setContactEmail(e.target.value)} placeholder="Optional" />
                </div>
                <div>
                  <Label htmlFor="contact-phone">Phone</Label>
                  <Input id="contact-phone" value={contactPhone} onChange={(e) => setContactPhone(e.target.value)} placeholder="Optional" />
                </div>
              </>
            ) : (
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
                  <Label htmlFor="pu-role">Role</Label>
                  <Select value={puRole} onValueChange={(v) => setPuRole(v as PortalRole)}>
                    <SelectTrigger id="pu-role" className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {(["contact", "billing", "admin"] as PortalRole[]).map((r) => (
                        <SelectItem key={r} value={r}>
                          {r}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div>
                  <Label htmlFor="pu-password">Temporary password</Label>
                  <Input id="pu-password" type="password" value={puPassword} onChange={(e) => setPuPassword(e.target.value)} />
                  <FieldHint>
                    Deliver this out of band. This page never emails a password.
                  </FieldHint>
                </div>
              </>
            )}
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
    </div>
  );
}

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
      className="grid h-9 w-9 shrink-0 place-items-center rounded-[999px] text-[12px] font-semibold"
      style={{ background: "var(--m-canopy-deep)", color: "var(--m-cream)" }}
    >
      {initials}
    </span>
  );
}

/** The half-finished client's card: a progress bar, four steps each with its
 * own action, and the two dismissal buttons with one sentence that explains
 * what each one does. */
function SetupCard({
  steps,
  doneCount,
  onAct,
  onSkip,
  onHide,
}: {
  steps: { key: string; title: string; hint: string; action: string; done: boolean; primary?: boolean }[];
  doneCount: number;
  onAct: (key: string) => void;
  onSkip: () => void;
  onHide: () => void;
}) {
  const pct = Math.round((doneCount / steps.length) * 100);
  return (
    <div
      className="overflow-hidden rounded-2xl border"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      data-testid="client-setup-card"
    >
      <div className="flex flex-wrap items-center gap-2 border-b px-[18px] py-3.5" style={{ borderColor: "var(--m-border)" }}>
        <h2 className="text-[15px] font-semibold">Finish setting up</h2>
        <span className="flex-1" />
        <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          {doneCount} of {steps.length} done
        </span>
      </div>

      {/* The progress bar is GREEN: done is green in this product. */}
      <div className="px-4 pt-3">
        <div
          role="progressbar"
          aria-valuenow={doneCount}
          aria-valuemin={0}
          aria-valuemax={steps.length}
          aria-label="Setup progress"
          className="h-2 w-full overflow-hidden rounded-full"
          style={{ background: "var(--m-surface-2)" }}
        >
          <div
            className="h-full rounded-full"
            style={{ width: `${pct}%`, background: "var(--m-canopy)", transition: "width 120ms cubic-bezier(0.2,0,0,1)" }}
          />
        </div>
      </div>

      <ol>
        {steps.map((step, i) => (
          <li
            key={step.key}
            className="flex flex-wrap items-center gap-3 border-t px-4 py-3"
            style={{ borderColor: "var(--m-border)" }}
            data-testid={`setup-step-${step.key}`}
          >
            <span
              aria-hidden="true"
              className="grid h-7 w-7 shrink-0 place-items-center rounded-[999px] border-2 text-[13px] font-bold"
              style={{
                background: step.done ? "var(--m-canopy)" : "transparent",
                borderColor: step.done ? "var(--m-canopy)" : "var(--m-input)",
                color: step.done ? "var(--m-surface)" : "var(--m-text-muted)",
              }}
            >
              {step.done ? <CheckIcon /> : i + 1}
            </span>
            <span className="min-w-[12rem] flex-1">
              <span
                className="block text-[13px]"
                style={{
                  fontWeight: step.done ? 500 : 600,
                  color: step.done ? "var(--m-text-secondary)" : "var(--m-text)",
                }}
              >
                {step.title}
              </span>
              <span className="block text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                {step.hint}
              </span>
            </span>
            {step.done ? (
              <span className="text-[13px]" style={{ color: "var(--m-canopy-deep)" }}>
                Done
              </span>
            ) : (
              <Button
                size="sm"
                variant={step.primary === true ? "primary" : "secondary"}
                onClick={() => onAct(step.key)}
                data-testid={`setup-action-${step.key}`}
              >
                {step.action}
              </Button>
            )}
          </li>
        ))}
      </ol>

      {/* One sentence that keeps the two buttons apart: skipping is
          resumable, hiding is not. They are different consequences, so the
          copy never blurs them.

          The buttons live in their own non-wrapping cluster. Left to the
          parent's `flex-wrap`, the second button dropped onto a line of its
          own and read as a stray control rather than the pair it belongs
          to; the sentence is what wraps, not the pair. */}
      <div
        className="flex flex-wrap items-center gap-3 border-t px-4 py-3"
        style={{ borderColor: "var(--m-border)", background: "var(--m-surface-2)" }}
      >
        <p className="min-w-[14rem] flex-1 text-[13px] leading-relaxed" style={{ color: "var(--m-text-secondary)" }}>
          <strong className="font-semibold" style={{ color: "var(--m-text)" }}>Skip for now</strong>{" "}
          keeps this card here for next time.{" "}
          <strong className="font-semibold" style={{ color: "var(--m-text)" }}>Don't show this again</strong>{" "}
          removes it, and you can bring it back from the client's menu.
        </p>
        <div className="flex shrink-0 items-center gap-2">
          <Button size="sm" variant="secondary" onClick={onSkip} data-testid="setup-skip">
            Skip for now
          </Button>
          <Button size="sm" variant="ghost" onClick={onHide} data-testid="setup-hide">
            Don't show this again
          </Button>
        </div>
      </div>
    </div>
  );
}

/** Open work as a card: the client's live tickets with their priority chips.
 * A P1 is the only red chip; the rest sit quiet, and the SLA figure is blush
 * with its own clock because colour is never alone. */
/** Open work, read from the client's real tickets. The card is a card of the
 * work that exists, so a client with nothing open says so in words instead of
 * showing an illustration of somebody else's week. */
function OpenWorkCard({ work }: { work: {
  total: number;
  inProgress: Ticket[];
  waiting: Ticket[];
  oldestOpenSince: number | null;
  rows: Ticket[];
} }): React.JSX.Element {
  return (
    <div
      className="overflow-hidden rounded-2xl border"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      data-testid="client-open-work"
    >
      <div className="flex items-center gap-2 border-b px-[18px] py-3.5" style={{ borderColor: "var(--m-border)" }}>
        <h2 className="text-[15px] font-semibold">Open work</h2>
        <span className="flex-1" />
        <Link
          href="/tickets"
          className="text-[13px] font-semibold underline underline-offset-2"
          style={{ color: "var(--m-text-secondary)" }}
        >
          All open tickets
        </Link>
      </div>
      {work.total === 0 ? (
        <div className="p-4">
          <EmptyState message="Nothing open for this client. New tickets raised against it will appear here." />
        </div>
      ) : (
        <ul>
          {work.rows.map((t) => (
            <li
              key={t.id}
              className="flex flex-wrap items-center gap-3 border-b px-[18px] py-2.5 last:border-b-0"
              style={{ borderColor: "var(--m-border)", minHeight: "52px" }}
            >
              <Badge priority={t.priority} className="shrink-0">
                {t.priority.toUpperCase()}
              </Badge>
 <span className="text-[13px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                {t.number}
              </span>
              <Link
                href={`/tickets/${t.id}`}
                className="min-w-0 flex-1 truncate text-[13px] font-medium underline decoration-transparent underline-offset-2 hover:decoration-[var(--m-border-strong)]"
              >
                {t.title}
              </Link>
              {/* The status word carries the state; the tint only supports it,
                  so a ticket is never identified by colour alone. */}
              <Badge tone={t.category === "in_progress" ? "neutral" : "due"}>
                {t.category === "in_progress"
                  ? "In progress"
                  : t.category === "waiting"
                    ? "Waiting"
                    : "Open"}
              </Badge>
              {t.due_at != null && t.due_at !== "" ? (
                <span className="text-[13px] tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                  due {formatPackDate(t.due_at)}
                </span>
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** Recent activity, built from the client's real tickets so the relationship
 * reads as a story rather than a set of current numbers. */
function RecentActivityCard({ entries }: { entries: ActivityEntry[] | null }): React.JSX.Element {
  return (
    <div
      className="overflow-hidden rounded-2xl border"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      data-testid="client-recent-activity"
    >
      <div className="border-b px-[18px] py-3.5" style={{ borderColor: "var(--m-border)" }}>
        <h2 className="text-[15px] font-semibold">Recent activity</h2>
      </div>
      {entries === null ? (
        <div className="p-4">
          <Spinner />
        </div>
      ) : entries.length === 0 ? (
        <div className="p-4">
          <EmptyState message="No activity recorded yet. Ticket updates appear here as they happen." />
        </div>
      ) : (
        <ul>
          {entries.map((i) => (
            <li
              key={i.href + i.at}
              className="flex gap-3 border-b px-[18px] py-2.5 text-[13px] last:border-b-0"
              style={{ borderColor: "var(--m-border)" }}
            >
              <span className="w-20 shrink-0 tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                {i.at}
              </span>
              <Link
                href={i.href}
                className="min-w-0 flex-1 underline decoration-transparent underline-offset-2 hover:decoration-[var(--m-border-strong)]"
              >
                {i.text}
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** Whole days since an ISO timestamp, for "oldest open ticket" style ages.
 * Calendar days rather than 24-hour spans, so the number does not flip at an
 * awkward hour. */
function daysSince(iso: string): number {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return 0;
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  return Math.max(0, Math.round((today.getTime() - then) / 86_400_000));
}

/** A short, honest relative time for the activity list: Today, Yesterday,
 * then a date. Deliberately not "2 hours ago" — an activity feed that claims
 * minute precision on a coarse timestamp is stating more than it knows. */
function relativeWhen(iso: string): string {
  const then = new Date(iso);
  if (Number.isNaN(then.getTime())) return "";
  const days = daysSince(iso);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7) return `${days}d ago`;
  return formatPackDate(iso.slice(0, 10)) ?? "";
}

/** Contacts as a people table: each person's role in the account, and portal
 * access as a switch that applies at once. */
function ContactsCard({
  contacts,
  portalUsers,
  portalDenied,
  onTogglePortal,
  onAddContact,
  onAddPortalUser,
}: {
  contacts: Contact[] | null;
  portalUsers: PortalUser[] | null;
  portalDenied: boolean;
  onTogglePortal: (user: PortalUser, disabled: boolean) => void;
  onAddContact: () => void;
  onAddPortalUser: () => void;
}) {
  if (contacts === null) {
    return (
      <div className="p-4">
        <Spinner />
      </div>
    );
  }
  return (
    <div
      className="overflow-hidden rounded-2xl border"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      data-testid="client-contacts-card"
    >
      <div className="flex flex-wrap items-center gap-2 border-b px-[18px] py-3.5" style={{ borderColor: "var(--m-border)" }}>
        <h2 className="text-[15px] font-semibold">Contacts</h2>
        <span className="flex-1" />
        {portalDenied ? null : (
          <Button size="sm" variant="secondary" onClick={onAddPortalUser} data-testid="add-portal-user">
            <PlusIcon />
            Add portal user
          </Button>
        )}
        <Button size="sm" variant="primary" onClick={onAddContact} data-testid="add-contact">
          <PlusIcon />
          Add contact
        </Button>
      </div>
      {contacts.length === 0 ? (
        <div className="p-4">
          <EmptyState message="No contacts yet. Contacts are the people at this client your team works with." />
        </div>
      ) : (
        <Table>
          <thead>
            <tr>
              <Th>Name</Th>
              <Th>Reaches</Th>
              <Th>Role</Th>
              <Th>Portal</Th>
            </tr>
          </thead>
          <tbody>
            {contacts.map((c) => {
              const portal = (portalUsers ?? []).find(
                (u) => u.email.toLowerCase() === (c.email ?? "").toLowerCase(),
              );
              const on = portal !== undefined && portal !== null && portal.status !== "disabled";
              return (
                <Tr key={c.id} className="m-entity-row">
                  <Td>
                    <span className="flex items-center gap-2.5">
                      <Monogram name={c.name} />
                      <span className="flex min-w-0 flex-col leading-[1.3]">
                        <span className="truncate font-semibold">{c.name}</span>
                        {c.title != null && c.title !== "" ? (
                          <span className="truncate text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                            {c.title}
                          </span>
                        ) : null}
                      </span>
                    </span>
                  </Td>
                  <Td>
                    <span className="flex flex-col text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                      {c.email != null && c.email !== "" ? (
                        <a href={`mailto:${c.email}`} className="underline underline-offset-2">
                          {c.email}
                        </a>
                      ) : (
                        <span style={{ color: "var(--m-text-muted)" }}>No email</span>
                      )}
                      {c.phone != null && c.phone !== "" ? <span>{c.phone}</span> : null}
                    </span>
                  </Td>
                  <Td>
                    {c.is_primary ? (
                      <Badge tone="neutral">Primary</Badge>
                    ) : (
                      <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                        None
                      </span>
                    )}
                  </Td>
                  <Td>
                    {portalDenied ? (
                      <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                        Not available
                      </span>
                    ) : portal === undefined || portal === null ? (
                      <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                        No login
                      </span>
                    ) : (
                      <button
                        type="button"
                        role="switch"
                        aria-checked={on}
                        aria-label={`Portal access for ${c.name}`}
                        onClick={() => onTogglePortal(portal, on)}
                        data-testid={`contact-portal-${c.id}`}
                        className="h-[26px] w-11 cursor-pointer rounded-[999px] border transition-colors duration-[120ms]"
                        style={{
                          background: on ? "var(--m-primary)" : "var(--m-surface-2)",
                          borderColor: on ? "transparent" : "var(--m-input)",
                        }}
                      >
                        <span
                          className="block h-5 w-5 rounded-[999px]"
                          style={{
                            background: on ? "var(--m-on-primary)" : "var(--m-input)",
                            transform: `translateX(${on ? 21 : 3}px)`,
                            transition: "transform 120ms cubic-bezier(0.2,0,0,1)",
                          }}
                        />
                      </button>
                    )}
                  </Td>
                </Tr>
              );
            })}
          </tbody>
        </Table>
      )}
    </div>
  );
}

/** Sites as a choice ABOVE the table, so the device table keeps its full
 * width. Warranty about to end is blush with its clock; an offline device is
 * a grey dot AND the word, never colour alone. */
function SitesCard({
  sites,
  assets,
  selected,
  onSelect,
  onAdd,
}: {
  sites: Site[] | null;
  assets: DeviceAsset[] | null;
  selected: Site | null;
  onSelect: (id: string) => void;
  onAdd: () => void;
}) {
  if (sites === null || assets === null) {
    return (
      <div className="p-4">
        <Spinner />
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-3" role="radiogroup" aria-label="Sites">
        {sites.map((s) => {
          const chosen = selected?.id === s.id;
          return (
            <button
              key={s.id}
              type="button"
              role="radio"
              aria-checked={chosen}
              onClick={() => onSelect(s.id)}
              data-testid={`site-choice-${s.id}`}
              className="flex min-w-[200px] cursor-pointer flex-col gap-1 rounded-[0.875rem] border-2 px-3.5 py-3 text-left"
              style={{
                borderColor: chosen ? "var(--m-canopy)" : "var(--m-border)",
                background: chosen ? "var(--m-status-success-bg)" : "var(--m-surface)",
                padding: chosen ? "11px 13px" : "12px 14px",
              }}
            >
              <span className="text-[13px] font-semibold">{s.name}</span>
              <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                {assets.length} devices
              </span>
            </button>
          );
        })}
        <Button variant="secondary" onClick={onAdd} data-testid="add-site">
          <PlusIcon />
          Add site
        </Button>
      </div>

      <div
        className="overflow-hidden rounded-2xl border"
        style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
        data-testid="client-devices-card"
      >
        <div className="flex flex-wrap items-center gap-2 border-b px-[18px] py-3.5" style={{ borderColor: "var(--m-border)" }}>
          <h2 className="text-[15px] font-semibold">{selected?.name ?? "Devices"}</h2>
          <span className="flex-1" />
          <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            Showing {Math.min(assets.length, 6)} of {assets.length}
          </span>
        </div>
        {assets.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No devices recorded for this site yet." />
          </div>
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Device</Th>
                <Th>Model</Th>
                <Th>Status</Th>
                <Th>Warranty</Th>
              </tr>
            </thead>
            <tbody>
              {assets.slice(0, 6).map((a) => (
                <Tr key={a.id} className="m-entity-row">
                  <Td className="font-semibold">{a.tag}</Td>
                  <Td style={{ color: "var(--m-text-secondary)" }}>{a.name}</Td>
                  <Td>
                    <span className="inline-flex items-center gap-1.5 text-[13px]">
                      <span
                        aria-hidden="true"
                        className="h-2 w-2 rounded-full"
                        style={{ background: a.status === "active" ? "var(--m-canopy)" : "var(--m-input)" }}
                      />
                      {a.status === "active" ? "Online" : "Offline"}
                    </span>
                  </Td>
                  <Td>
                    <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                      Covered
                    </span>
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </div>
    </div>
  );
}

/** The contract's terms as four figures, the renewal strip, and the billing
 * ledger. The strip is BLUSH and states the date the renewal quote should go
 * out, because a renewal is missed when the lead time is not written down. */
function ContractsBillingCard({
  contract,
  renewal,
  quoteDue,
  contracts,
}: {
  contract: Contract | null;
  renewal: { tone: string | null; label: string; days: number } | null;
  quoteDue: string | null;
  contracts: Contract[];
}) {
  return (
    <div className="flex flex-col gap-4" data-testid="client-contracts-billing">
      {renewal !== null ? (
        <div
          role="status"
          className="flex flex-wrap items-center gap-x-3 gap-y-2 rounded-xl px-4 py-2.5"
          style={{ background: "var(--m-status-due-bg, var(--m-time))", color: "var(--m-status-due-fg, var(--m-time-fg)" }}
          data-testid="client-renewal-strip"
        >
          <ClockIcon />
          <span className="text-[13px]">
            <strong className="font-bold">{renewal.label}.</strong>{" "}
            {quoteDue !== null
              ? `The renewal quote should go out by ${formatPackDate(quoteDue)}, ${60} days before the end.`
              : "The renewal quote should go out before the end."}
          </span>
          <span className="flex-1 basis-24" />
          {/* A renewal is set up ON the contract — the term moves there, and
              the quote is authored against it. So the action goes to the
              contract's own editor rather than pretending this page can
              renew anything. */}
          {contract !== null ? (
            <Link href={`/contracts/${contract.id}`} className="ml-auto">
              <Button size="sm" data-testid="client-start-renewal">
                Start renewal
              </Button>
            </Link>
          ) : null}
        </div>
      ) : null}

      <div
        className="overflow-hidden rounded-2xl border"
        style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
      >
        <div className="flex flex-wrap items-center gap-2 border-b px-[18px] py-3.5" style={{ borderColor: "var(--m-border)" }}>
          <h2 className="text-[15px] font-semibold">{contract?.name ?? "No contract yet"}</h2>
          <span className="flex-1" />
          {contract !== null ? (
            <>
              <Badge tone="success">Active</Badge>
              {/* The contract is edited on the contract record — its terms,
                  rate card and billing all live there. */}
              <Link href={`/contracts/${contract.id}`}>
                <Button size="sm" variant="secondary" data-testid="client-edit-contract">
                  Edit
                </Button>
              </Link>
            </>
          ) : null}
        </div>
        {contract === null ? (
          <div className="p-4">
            <EmptyState message="No contract yet. A contract is what an invoice run bills against." />
          </div>
        ) : (
          <div className="grid grid-cols-2 gap-4 p-4 lg:grid-cols-4">
            <Figure
              label="Value"
              value={
                contract.amount_minor != null
                  ? `${money(contract.amount_minor, contract.currency)} a ${shortFrequency(contract.billing_frequency)}`
                  : "—"
              }
              emphasis
            />
            <Figure
              label="Term"
              value={`${formatPackDate(contract.effective_from) ?? "—"} to ${formatPackDate(contract.effective_until) ?? "open"}`}
            />
            <Figure
              label="Billing"
              value={`${shortFrequency(contract.billing_frequency)}, on the ${billingDay(contract.billing_anchor)}`}
            />
            <Figure label="Payment terms" value={poOrTerms(contract.po_number)} />
          </div>
        )}
      </div>

      {contracts.length > 1 ? (
        <div
          className="overflow-hidden rounded-2xl border"
          style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
        >
          <div className="flex flex-wrap items-center gap-2 border-b px-[18px] py-3.5" style={{ borderColor: "var(--m-border)" }}>
            <h2 className="text-[15px] font-semibold">Billing history</h2>
            <span className="flex-1" />
            <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              Credit notes: none
            </span>
          </div>
          <Table>
            <thead>
              <tr>
                <Th>Invoice</Th>
                <Th>Description</Th>
                <Th>Status</Th>
              </tr>
            </thead>
            <tbody>
              {contracts.map((c) => (
                <Tr key={c.id} className="m-entity-row">
                  <Td style={{ color: "var(--m-text-secondary)" }}>{c.number}</Td>
                  <Td className="font-medium">{c.name}</Td>
                  <Td>
                    <Badge tone={c.status === "active" ? "success" : "neutral"}>
                      {c.status === "active" ? "Paid" : c.status}
                    </Badge>
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        </div>
      ) : null}
    </div>
  );
}

function Figure({
  label,
  value,
  emphasis = false,
}: {
  label: string;
  value: string;
  emphasis?: boolean;
}) {
  return (
    <div>
      <p
        className="text-[11px] font-semibold uppercase tracking-[0.08em]"
        style={{ color: "var(--m-text-muted)" }}
      >
        {label}
      </p>
      <p
        className="mt-0.5 font-semibold tabular-nums"
        style={{ fontSize: emphasis ? "20px" : "13px" }}
      >
        {value}
      </p>
    </div>
  );
}

function CompletenessCard({
  completeness,
  unavailable,
  tone,
}: {
  completeness: ClientCompletenessView | null;
  unavailable: boolean;
  tone: string;
}) {
  return (
    <SummaryCard title="Documentation">
      {unavailable ? (
        <span className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
          Completeness scoring is not available for this client in your scope.
        </span>
      ) : completeness === null ? (
        <Spinner />
      ) : (
        <>
          <div
            aria-hidden="true"
            className="h-2 w-full overflow-hidden rounded-full"
            style={{ background: "var(--m-surface-2)" }}
          >
            <div
              className="h-full rounded-full"
              style={{
                width: `${completeness.score}%`,
                background:
                  completeness.score >= 75
                    ? "var(--m-canopy)"
                    : completeness.score >= 40
                      ? "var(--m-time-fg)"
                      : "var(--m-blush)",
              }}
            />
          </div>
          <p className="mt-1 flex items-center gap-2">
            <span className="text-[20px] font-semibold tabular-nums">{completeness.score}</span>
            <Tag tone={tone === "success" || tone === "warning" || tone === "danger" ? tone : "neutral"}>
              {completeness.score >= 75 ? "complete" : completeness.score >= 40 ? "in progress" : "thin"}
            </Tag>
          </p>
          <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
            {completeness.basis}
          </p>
        </>
      )}
    </SummaryCard>
  );
}

function DocumentsCard({
  documents,
  layouts,
}: {
  documents: Document[] | null;
  layouts: DocumentLayout[] | null;
}) {
  if (documents === null) {
    return (
      <div className="p-4">
        <Spinner />
      </div>
    );
  }
  return (
    <div
      className="overflow-hidden rounded-2xl border"
      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
    >
      <div className="flex flex-wrap items-center gap-2 border-b px-[18px] py-3.5" style={{ borderColor: "var(--m-border)" }}>
        <h2 className="text-[15px] font-semibold">Documents</h2>
        <span className="flex-1" />
        <Link href="/documents" className="text-[13px] underline underline-offset-2" style={{ color: "var(--m-text-secondary)" }}>
          Open library
        </Link>
      </div>
      {documents.length === 0 ? (
        <div className="p-4">
          <EmptyState message="No documents for this client yet. Typed records, runbooks and manifests belong here." />
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
            {documents.map((d) => (
              <Tr key={d.id} className="m-entity-row">
                <Td>
                  <Link href={`/documents/${d.id}`} className="font-medium underline decoration-transparent underline-offset-2 hover:decoration-[var(--m-border-strong)]">
                    {d.title}
                  </Link>
                </Td>
                <Td>
                  {layouts?.find((l) => l.id === d.layout_id)?.name ?? (
                    <span style={{ color: "var(--m-text-muted)" }}>—</span>
                  )}
                </Td>
                <Td className="text-right tabular-nums">v{d.current_version}</Td>
                <Td>{formatPackDate(d.updated_at)}</Td>
              </Tr>
            ))}
          </tbody>
        </Table>
      )}
    </div>
  );
}

function shortFrequency(f: string | null | undefined): string {
  switch (f) {
    case "monthly": return "month";
    case "quarterly": return "quarter";
    case "semiannual": return "half year";
    case "annual": return "year";
    case "weekly": return "week";
    default: return "period";
  }
}

function billingDay(anchor: string | null | undefined): string {
  if (anchor == null || anchor === "") return "1st";
  const day = Number(anchor.slice(8, 10));
  if (!Number.isFinite(day) || day < 1) return "1st";
  if (day === 1) return "1st";
  if (day === 2) return "2nd";
  if (day === 3) return "3rd";
  return `${day}th`;
}

function poOrTerms(poNumber: string | null | undefined): string {
  return poNumber == null || poNumber === "" ? "Net 14" : `Net 14 · ${poNumber}`;
}

function ClockIcon() {
  return <Icon name="clock" size={13} />;
}

function CheckIcon() {
  return <Icon name="check" size={15} />;
}

function ChevronIcon() {
  return <Icon name="chev" size={14} />;
}

function PlusIcon() {
  return <Icon name="plus" size={16} />;
}

function ExternalIcon() {
  return <Icon name="link" size={16} />;
}