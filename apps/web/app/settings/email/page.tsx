// Email settings (/settings/email) — the mail edge in one page.
//
// Purpose: inbound routing (org catch-all + per-board addresses and
// outbound identities), the outbound transport with sealed credentials, the
// delivery ledger for every outbound message, the test-send probe, and the
// notification-email policy (brand wrapper, org defaults, digest).
//
// Honest states (ADR 0010): a transport of `none` renders as a typed
// not-configured panel — messages stay queued and visible in the ledger,
// never a fake success. Credentials are write-only: the page shows whether
// a credential is sealed and when, never its value.
//
// Non-purpose: no ticket-level mail (the ticket page owns that) and no
// per-user digest controls (Notifications page owns those).
"use client";

import { friendlyMutationText } from "../../../lib/error-copy";
import {
  Button,
  Card,
  CardContent,
  CardHeader,
  Checkbox,
  FieldError,
  FieldHint,
  Input,
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
import { useCallback, useEffect, useMemo, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import {
  SaveBar,
  SettingsHeader,
} from "../../../components/settings/settings-shell";
import { kindLabel } from "../../../lib/use-notifications";

// Local view types: the page does not wait on SDK regeneration to be
// written, but every shape mirrors the pinned OpenAPI schemas.
interface BoardIdentity {
  from_address: string | null;
  from_name: string | null;
  reply_to: string | null;
  signature: string | null;
}

interface BoardRoute {
  board_id: string;
  name: string;
  inbound_address: string | null;
  identity: BoardIdentity;
}

interface CredentialView {
  configured: boolean;
  updated_at: string | null;
}

interface OutboundView {
  transport: string;
  from_address: string;
  from_name: string;
  reply_to: string;
  signature: string;
  brand_name: string;
  logo_url: string;
  smtp_host: string;
  smtp_port: number;
  smtp_username: string;
  postmark_base_url: string;
  ses_region: string;
  ready: boolean;
  not_ready_reason: string | null;
  // Each key is optional on purpose: see the `credential()` reader below.
  credentials: {
    postmark?: CredentialView;
    smtp?: CredentialView;
    ses?: CredentialView;
  };
}

interface MailSettings {
  inbound: {
    catchall_address: string | null;
    catchall_board_id: string | null;
    enabled: boolean;
  };
  outbound: OutboundView;
  boards: BoardRoute[];
  reply_by_email: {
    enabled: boolean;
    catchall_address: string | null;
    board_addresses: number;
  };
  digest_policy: string;
}

interface LedgerRow {
  message_id: string;
  subject: string | null;
  to_addresses: string[];
  from_address: string;
  status: string;
  attempts: number;
  max_attempts: number;
  last_error: string | null;
  created_at: string;
}

interface LedgerView {
  rows: LedgerRow[];
  counts: { queued: number; sending: number; sent: number; dead: number };
  next_cursor: string | null;
  transport_configured: boolean;
}

interface OrgDefaultRow {
  kind: string;
  in_app: boolean;
  email: boolean;
  quiet_from: string | null;
  quiet_until: string | null;
  timezone: string;
}

interface OrgDefaultsView {
  defaults: OrgDefaultRow[];
}

/** What to render for a credential the server did not report at all. */
const UNCONFIGURED_CREDENTIAL: CredentialView = { configured: false, updated_at: null };

const TRANSPORT_LABELS: Record<string, string> = {
  none: "None (keep messages queued)",
  postmark: "Postmark",
  smtp: "SMTP relay",
  ses: "Amazon SES",
};

function statusTone(status: string): "success" | "warning" | "danger" | "neutral" {
  switch (status) {
    case "sent":
      return "success";
    case "queued":
      return "warning";
    case "sending":
      return "neutral";
    case "dead":
      return "danger";
    default:
      return "neutral";
  }
}

function shortAddress(list: string[]): string {
  const first = list[0];
  if (first === undefined) return "—";
  if (list.length === 1) return first;
  return `${first} +${list.length - 1}`;
}

function when(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "—";
  return date.toLocaleString();
}

export default function EmailSettingsPage() {
  const auth = useRequireAuth();
  const [settings, setSettings] = useState<MailSettings | null>(null);
  const [ledger, setLedger] = useState<LedgerView | null>(null);
  const [orgDefaults, setOrgDefaults] = useState<OrgDefaultRow[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [ledgerStatus, setLedgerStatus] = useState<string>("all");
  const [refreshTick, setRefreshTick] = useState(0);

  const load = useCallback(async () => {
    const [settingsRes, ledgerRes, defaultsRes] = await Promise.all([
      api.api.GET("/api/v1/email/settings"),
      api.api.GET("/api/v1/email/delivery-ledger", {
        params: { query: { limit: 25 } },
      }),
      api.api.GET("/api/v1/notification-org-defaults"),
    ]);
    if (settingsRes.data) setSettings(settingsRes.data as unknown as MailSettings);
    if (ledgerRes.data) setLedger(ledgerRes.data as unknown as LedgerView);
    if (defaultsRes.data) {
      const view = defaultsRes.data as unknown as OrgDefaultsView;
      setOrgDefaults(view.defaults);
    }
    const firstError = settingsRes.error ?? ledgerRes.error;
    setLoadError(
      firstError ? "The mail settings could not be loaded." : null,
    );
  }, []);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void load();
  }, [auth.status, load, refreshTick]);

  const refresh = useCallback(() => setRefreshTick((n) => n + 1), []);

  if (auth.status !== "authed") return null;
  if (!settings || !ledger) {
    return (
      <div className="grid gap-4">
        <SettingsHeader title="Email" description="Loading the mail settings…" />
        {loadError ? (
          <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {loadError}
          </p>
        ) : (
          <Spinner />
        )}
      </div>
    );
  }

  return (
    <div className="grid gap-6">
      <SettingsHeader
        title="Email"
        description="Where inbound mail routes, who outbound mail comes from, how it leaves, and what happened to each message. Credentials are sealed at rest and never displayed again."
      >
        <Tag tone={settings.reply_by_email.enabled ? "success" : "neutral"}>
          {settings.reply_by_email.enabled ? "Reply by email on" : "Reply by email off"}
        </Tag>
      </SettingsHeader>

      <InboundCard
        settings={settings}
        onSaved={refresh}
      />
      <SendingCard
        settings={settings}
        defaultProbeTo={auth.me?.email ?? ""}
        onSaved={refresh}
      />
      <LedgerCard
        ledger={ledger}
        statusFilter={ledgerStatus}
        onStatusFilter={setLedgerStatus}
        onRefresh={refresh}
      />
      <NotificationEmailCard
        settings={settings}
        defaults={orgDefaults}
        onSaved={refresh}
      />
    </div>
  );
}

// --- inbound routing ---------------------------------------------------------

function InboundCard({
  settings,
  onSaved,
}: {
  settings: MailSettings;
  onSaved: () => void;
}) {
  const [address, setAddress] = useState(settings.inbound.catchall_address ?? "");
  const [boardId, setBoardId] = useState(settings.inbound.catchall_board_id ?? "");
  const [enabled, setEnabled] = useState(settings.inbound.enabled);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setAddress(settings.inbound.catchall_address ?? "");
    setBoardId(settings.inbound.catchall_board_id ?? "");
    setEnabled(settings.inbound.enabled);
  }, [settings]);

  const dirty =
    boardId !== "" &&
    (address !== (settings.inbound.catchall_address ?? "") ||
      boardId !== (settings.inbound.catchall_board_id ?? "") ||
      enabled !== settings.inbound.enabled);

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const res = await api.api.PUT("/api/v1/email/inbound-settings", {
        body: { catchall_address: address, catchall_board_id: boardId, enabled },
      });
      if (res.error) {
        throw new Error(
          (res.error as { error?: { message?: string } }).error?.message ??
            "The catch-all was rejected.",
        );
      }
      setSaved(true);
      onSaved();
    } catch (e) {
      setError(friendlyMutationText(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Card>
      <CardHeader>
        <div>
          <h2 className="text-[14px] font-semibold">Inbound routing</h2>
          <p className="mt-0.5 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            Mail sent to these addresses becomes tickets on the target board.
            Addresses must be unique across your workspace.
          </p>
        </div>
      </CardHeader>
      <CardContent>
            <div className="grid gap-4">
        <div className="flex flex-wrap items-end gap-3">
          <label className="grid gap-1">
            <span className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              Catch-all address
            </span>
            <Input
              value={address}
              onChange={(e) => {
                setAddress(e.target.value);
                setSaved(false);
              }}
              placeholder="support@yourdomain.com"
              className="w-[260px]"
            />
          </label>
          <label className="grid gap-1">
            <span className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              Target board
            </span>
            <Select value={boardId} onValueChange={setBoardId}>
              <SelectTrigger className="w-[220px]" aria-label="Catch-all target board">
                <SelectValue placeholder="Choose a board" />
              </SelectTrigger>
              <SelectContent>
                {settings.boards.map((board) => (
                  <SelectItem key={board.board_id} value={board.board_id}>
                    {board.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>
          <label className="flex items-center gap-2 pb-2">
            <Checkbox
              checked={enabled}
              onCheckedChange={(v) => {
                setEnabled(v === true);
                setSaved(false);
              }}
              aria-label="Catch-all enabled"
            />
            <span className="text-[13px]">Routing enabled</span>
          </label>
        </div>
        <SaveBar dirty={dirty} saving={saving} saved={saved} error={error} onSave={() => void save()} />
        <FieldHint>
          Turning routing off keeps existing tickets intact; new mail to the
          catch-all is refused at the webhook instead of silently dropped.
        </FieldHint>

        <div className="rounded-md border overflow-hidden" style={{ borderColor: "var(--m-border)" }}>
          <Table>
            <thead>
              <tr
                className="text-left text-[11px] uppercase tracking-wide"
                style={{ color: "var(--m-text-muted)", background: "var(--m-canvas)" }}
              >
                <Th className="font-medium">Board</Th>
                <Th className="font-medium">Inbound address</Th>
                <Th className="font-medium">Outbound from</Th>
                <Th className="font-medium">Identity</Th>
              </tr>
            </thead>
            <tbody>
              {settings.boards.map((board) => (
                <BoardRouteRow key={board.board_id} board={board} onSaved={onSaved} />
              ))}
            </tbody>
          </Table>
        </div>
      </div>
          </CardContent>
    </Card>
  );
}

function BoardRouteRow({
  board,
  onSaved,
}: {
  board: BoardRoute;
  onSaved: () => void;
}) {
  const [address, setAddress] = useState(board.inbound_address ?? "");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [identityOpen, setIdentityOpen] = useState(false);

  useEffect(() => {
    setAddress(board.inbound_address ?? "");
  }, [board.inbound_address]);

  const saveAddress = async () => {
    setSaving(true);
    setError(null);
    try {
      const res = await api.api.PUT("/api/v1/boards/{id}/inbound-address", {
        params: { path: { id: board.board_id } },
        body: { address: address.trim() === "" ? null : address },
      });
      if (res.error) {
        throw new Error(
          (res.error as { error?: { message?: string } }).error?.message ??
            "The address was rejected.",
        );
      }
      setSaved(true);
      onSaved();
    } catch (e) {
      setError(friendlyMutationText(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Tr className="border-t align-top" style={{ borderColor: "var(--m-border)" }}>
      <Td className="px-3 py-2.5">
        <div className="text-[13px]">{board.name}</div>
        {error ? <FieldError>{error}</FieldError> : null}
      </Td>
      <Td className="px-3 py-2.5">
        <div className="flex items-center gap-2">
          <Input
            value={address}
            onChange={(e) => {
              setAddress(e.target.value);
              setSaved(false);
            }}
            placeholder="projects@yourdomain.com"
            aria-label={`${board.name} inbound address`}
            className="w-[230px]"
          />
          <Button
            variant="secondary"
            disabled={saving || address === (board.inbound_address ?? "")}
            onClick={() => void saveAddress()}
          >
            {saving ? "Saving…" : saved ? "Saved" : "Save"}
          </Button>
        </div>
      </Td>
      <Td className="px-3 py-2.5 text-[13px]">
        {board.identity.from_address ? (
          <span style={{ fontFamily: "var(--m-font-mono)" }}>{board.identity.from_address}</span>
        ) : (
          <span style={{ color: "var(--m-text-muted)" }}>Org default</span>
        )}
      </Td>
      <Td className="px-3 py-2.5">
        <Button variant="secondary" onClick={() => setIdentityOpen(true)}>
          Edit identity
        </Button>
        {identityOpen ? (
          <BoardIdentityDialog
            board={board}
            onClose={() => setIdentityOpen(false)}
            onSaved={() => {
              setIdentityOpen(false);
              onSaved();
            }}
          />
        ) : null}
      </Td>
    </Tr>
  );
}

function BoardIdentityDialog({
  board,
  onClose,
  onSaved,
}: {
  board: BoardRoute;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [fromAddress, setFromAddress] = useState(board.identity.from_address ?? "");
  const [fromName, setFromName] = useState(board.identity.from_name ?? "");
  const [replyTo, setReplyTo] = useState(board.identity.reply_to ?? "");
  const [signature, setSignature] = useState(board.identity.signature ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const res = await api.api.PUT("/api/v1/boards/{id}/email-identity", {
        params: { path: { id: board.board_id } },
        body: {
          from_address: fromAddress.trim() === "" ? null : fromAddress,
          from_name: fromName.trim() === "" ? null : fromName,
          reply_to: replyTo.trim() === "" ? null : replyTo,
          signature: signature.trim() === "" ? null : signature,
        },
      });
      if (res.error) {
        throw new Error(
          (res.error as { error?: { message?: string } }).error?.message ??
            "The identity was rejected.",
        );
      }
      onSaved();
    } catch (e) {
      setError(friendlyMutationText(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div
      role="dialog"
      aria-label={`${board.name} email identity`}
      className="fixed inset-0 z-50 grid place-items-center p-4"
      style={{ background: "color-mix(in oklab, var(--m-text) 40%, transparent)" }}
    >
      <Card className="w-full max-w-lg">
        <CardHeader>
          <div>
            <h3 className="text-[14px] font-semibold">{board.name} email identity</h3>
            <p className="mt-0.5 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              Blank fields fall back to the org identity. The From address must be
              unique across boards so replies route back to the right inbox.
            </p>
          </div>
        </CardHeader>
        <CardContent>
            <div className="grid gap-3">
          <label className="grid gap-1">
            <span className="text-[12px]">From address</span>
            <Input
              value={fromAddress}
              onChange={(e) => setFromAddress(e.target.value)}
              placeholder="projects@yourdomain.com"
            />
          </label>
          <label className="grid gap-1">
            <span className="text-[12px]">From display name</span>
            <Input
              value={fromName}
              onChange={(e) => setFromName(e.target.value)}
              placeholder="Acme Projects"
            />
          </label>
          <label className="grid gap-1">
            <span className="text-[12px]">Reply-To</span>
            <Input
              value={replyTo}
              onChange={(e) => setReplyTo(e.target.value)}
              placeholder="projects@yourdomain.com"
            />
          </label>
          <label className="grid gap-1">
            <span className="text-[12px]">Signature</span>
            <textarea
              value={signature}
              onChange={(e) => setSignature(e.target.value)}
              rows={3}
              className="m-input"
              placeholder={"Acme Projects\nprojects@yourdomain.com"}
            />
          </label>
          {error ? <FieldError>{error}</FieldError> : null}
          <div className="flex justify-end gap-2">
            <Button variant="secondary" onClick={onClose} disabled={saving}>
              Cancel
            </Button>
            <Button variant="primary" onClick={() => void save()} disabled={saving}>
              {saving ? "Saving…" : "Save identity"}
            </Button>
          </div>
        </div>
          </CardContent>
      </Card>
    </div>
  );
}

// --- sending -----------------------------------------------------------------

function SendingCard({
  settings,
  defaultProbeTo,
  onSaved,
}: {
  settings: MailSettings;
  defaultProbeTo: string;
  onSaved: () => void;
}) {
  const out = settings.outbound;
  const [transport, setTransport] = useState(out.transport);
  const [fromAddress, setFromAddress] = useState(out.from_address);
  const [fromName, setFromName] = useState(out.from_name);
  const [replyTo, setReplyTo] = useState(out.reply_to);
  const [signature, setSignature] = useState(out.signature);
  const [smtpHost, setSmtpHost] = useState(out.smtp_host);
  const [smtpPort, setSmtpPort] = useState(String(out.smtp_port));
  const [smtpUsername, setSmtpUsername] = useState(out.smtp_username);
  const [postmarkBase, setPostmarkBase] = useState(out.postmark_base_url);
  const [sesRegion, setSesRegion] = useState(out.ses_region || "eu-west-1");
  // The SES key pair. Both halves are held in state only long enough to
  // POST them into the sealed slot, exactly like the Postmark token.
  const [sesKeyId, setSesKeyId] = useState("");
  const [sesSecret, setSesSecret] = useState("");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [postmarkToken, setPostmarkToken] = useState("");
  const [smtpPassword, setSmtpPassword] = useState("");
  const [credSaving, setCredSaving] = useState(false);
  const [credSaved, setCredSaved] = useState(false);
  const [credError, setCredError] = useState<string | null>(null);

  const [probeTo, setProbeTo] = useState("");
  const [probeBusy, setProbeBusy] = useState(false);
  const [probeMessage, setProbeMessage] = useState<string | null>(null);
  const [probeError, setProbeError] = useState<string | null>(null);

  useEffect(() => {
    setTransport(out.transport);
    setFromAddress(out.from_address);
    setFromName(out.from_name);
    setReplyTo(out.reply_to);
    setSignature(out.signature);
    setSmtpHost(out.smtp_host);
    setSmtpPort(String(out.smtp_port));
    setSmtpUsername(out.smtp_username);
    setPostmarkBase(out.postmark_base_url);
    setSesRegion(out.ses_region || "eu-west-1");
  }, [out]);

  const dirty =
    transport !== out.transport ||
    fromAddress !== out.from_address ||
    fromName !== out.from_name ||
    replyTo !== out.reply_to ||
    signature !== out.signature ||
    smtpHost !== out.smtp_host ||
    smtpPort !== String(out.smtp_port) ||
    smtpUsername !== out.smtp_username ||
    postmarkBase !== out.postmark_base_url || sesRegion !== (out.ses_region || "eu-west-1");

  const saveSettings = async () => {
    setSaving(true);
    setError(null);
    try {
      const port = Number.parseInt(smtpPort, 10);
      const res = await api.api.PUT("/api/v1/email/outbound-settings", {
        body: {
          transport,
          from_address: fromAddress,
          from_name: fromName,
          reply_to: replyTo,
          signature,
          smtp_host: smtpHost,
          smtp_port: Number.isFinite(port) ? port : out.smtp_port,
          smtp_username: smtpUsername,
          postmark_base_url: postmarkBase,
          ses_region: sesRegion,
        },
      });
      if (res.error) {
        throw new Error(
          (res.error as { error?: { message?: string } }).error?.message ??
            "The settings were rejected.",
        );
      }
      setSaved(true);
      onSaved();
    } catch (e) {
      setError(friendlyMutationText(e));
    } finally {
      setSaving(false);
    }
  };

  const saveCredential = async () => {
    setCredSaving(true);
    setCredError(null);
    try {
      // The body is built per transport rather than merged, so a request
      // for one provider can never carry another provider's field. The
      // backend rejects a half-filled SES pair outright, which is the
      // behaviour that makes an accidental half-save visible instead of
      // leaving the org believing SES is configured.
      const res = await api.api.PUT("/api/v1/email/transport-credentials", {
        body:
          transport === "postmark"
            ? { transport, postmark_server_token: postmarkToken }
            : transport === "ses"
              ? { transport, ses_access_key_id: sesKeyId, ses_secret_access_key: sesSecret }
              : { transport, smtp_password: smtpPassword },
      });
      if (res.error) {
        throw new Error(
          (res.error as { error?: { message?: string } }).error?.message ??
            "The credential was rejected.",
        );
      }
      setCredSaved(true);
      // Clear the plaintext out of component state the moment it is sealed.
      // The value is never readable again, so leaving it in a controlled
      // input would be the only copy of a credential anywhere in the app.
      setPostmarkToken("");
      setSmtpPassword("");
      setSesKeyId("");
      setSesSecret("");
      onSaved();
    } catch (e) {
      setCredError(friendlyMutationText(e));
    } finally {
      setCredSaving(false);
    }
  };

  const sendProbe = async () => {
    setProbeBusy(true);
    setProbeError(null);
    setProbeMessage(null);
    try {
      const res = await api.api.POST("/api/v1/email/test-send", {
        body: { to: probeTo.trim() === "" ? null : probeTo.trim() },
      });
      if (res.error || !res.data) {
        throw new Error(
          (res.error as { error?: { message?: string } } | undefined)?.error?.message ??
            "The test message was refused.",
        );
      }
      setProbeMessage(
        `Queued to ${res.data.to}. Watch the delivery log below for the outcome.`,
      );
      onSaved();
    } catch (e) {
      setProbeError(friendlyMutationText(e));
    } finally {
      setProbeBusy(false);
    }
  };

  // A transport this build does not know about, or one an older server has
  // not heard of, must render as "not configured" rather than crash the
  // page. Reading `credentials.ses.configured` directly took the whole
  // settings card down against a server that predates the SES kind, which is
  // a real deployment window during a rolling upgrade and not a hypothetical
  // one. `UNCONFIGURED` is the honest rendering of "this server has no such
  // credential to report".
  const credential = (kind: "postmark" | "smtp" | "ses"): CredentialView =>
    out.credentials[kind] ?? UNCONFIGURED_CREDENTIAL;

  const secrets = useMemo(
    () => [
      {
        label: "Postmark server token",
        configured: credential("postmark").configured,
        updatedAt: credential("postmark").updated_at,
      },
      {
        label: "SMTP password",
        configured: credential("smtp").configured,
        updatedAt: credential("smtp").updated_at,
      },
      {
        // One row for the PAIR, not two: the access key id is not a secret
        // but it is useless alone, so showing it as a separately-sealed
        // credential would imply you can configure half of it.
        label: "SES access key pair",
        configured: credential("ses").configured,
        updatedAt: credential("ses").updated_at,
      },
    ],
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [out.credentials],
  );

  return (
    <Card>
      <CardHeader>
        <div>
          <h2 className="text-[14px] font-semibold">Sending</h2>
          <p className="mt-0.5 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            One transport per workspace. With “None”, messages stay queued in the
            delivery log below — nothing is lost, nothing is sent.
          </p>
        </div>
      </CardHeader>
      <CardContent>
            <div className="grid gap-4">
        {!out.ready ? (
          <div
            role="status"
            data-testid="mail-not-configured"
            className="rounded-md border px-3 py-2 text-[13px]"
            style={{
              borderColor: "var(--m-border)",
              background: "var(--m-canvas)",
              color: "var(--m-text-secondary)",
            }}
          >
            {out.not_ready_reason ?? "Sending is not configured yet."}
          </div>
        ) : null}

        <div className="grid gap-1">
          <span className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            Transport
          </span>
          <Select
            value={transport}
            onValueChange={(v) => {
              setTransport(v);
              setSaved(false);
            }}
          >
            <SelectTrigger className="w-[280px]" aria-label="Outbound transport">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {Object.entries(TRANSPORT_LABELS).map(([value, label]) => (
                <SelectItem key={value} value={value}>
                  {label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        {transport === "postmark" ? (
          <div className="grid gap-3 md:grid-cols-2">
            <label className="grid gap-1">
              <span className="text-[12px]">Postmark API base URL</span>
              <Input
                value={postmarkBase}
                onChange={(e) => {
                  setPostmarkBase(e.target.value);
                  setSaved(false);
                }}
              />
            </label>
            <label className="grid gap-1">
              <span className="text-[12px]">Server token</span>
              <Input
                type="password"
                value={postmarkToken}
                onChange={(e) => setPostmarkToken(e.target.value)}
                placeholder={credential("postmark").configured ? "Replace sealed token" : "Paste server token"}
                autoComplete="off"
              />
            </label>
          </div>
        ) : null}

        {transport === "ses" ? (
          <div className="grid gap-3 md:grid-cols-3">
            <label className="grid gap-1">
              <span className="text-[12px]">AWS region</span>
              <Input
                value={sesRegion}
                onChange={(e) => {
                  setSesRegion(e.target.value);
                  setSaved(false);
                }}
                placeholder="eu-west-1"
              />
            </label>
            <label className="grid gap-1">
              <span className="text-[12px]">Access key ID</span>
              <Input
                value={sesKeyId}
                onChange={(e) => setSesKeyId(e.target.value)}
                placeholder={credential("ses").configured ? "Replace sealed key id" : "AKIA\u2026"}
                autoComplete="off"
              />
            </label>
            <label className="grid gap-1">
              <span className="text-[12px]">Secret access key</span>
              <Input
                type="password"
                value={sesSecret}
                onChange={(e) => setSesSecret(e.target.value)}
                placeholder={credential("ses").configured ? "Replace sealed secret" : "Paste secret"}
                autoComplete="off"
              />
            </label>
            <p className="text-[12px] md:col-span-3" style={{ color: "var(--m-text-secondary)" }}>
              Both halves of the key pair are sealed together and neither is
              readable again. The region is part of the SES endpoint, not a
              preference, so it decides which data centre your mail leaves from.
            </p>
          </div>
        ) : null}

        {transport === "smtp" ? (
          <div className="grid gap-3 md:grid-cols-3">
            <label className="grid gap-1">
              <span className="text-[12px]">SMTP host</span>
              <Input
                value={smtpHost}
                onChange={(e) => {
                  setSmtpHost(e.target.value);
                  setSaved(false);
                }}
                placeholder="smtp.yourdomain.com"
              />
            </label>
            <label className="grid gap-1">
              <span className="text-[12px]">Port</span>
              <Input
                value={smtpPort}
                onChange={(e) => {
                  setSmtpPort(e.target.value);
                  setSaved(false);
                }}
                inputMode="numeric"
              />
            </label>
            <label className="grid gap-1">
              <span className="text-[12px]">Username</span>
              <Input
                value={smtpUsername}
                onChange={(e) => {
                  setSmtpUsername(e.target.value);
                  setSaved(false);
                }}
                placeholder="Optional on a trusted relay"
              />
            </label>
            <label className="grid gap-1 md:col-span-3">
              <span className="text-[12px]">SMTP password</span>
              <Input
                type="password"
                value={smtpPassword}
                onChange={(e) => setSmtpPassword(e.target.value)}
                placeholder={credential("smtp").configured ? "Replace sealed password" : "Paste password"}
                autoComplete="off"
              />
            </label>
          </div>
        ) : null}

        <div className="grid gap-3 md:grid-cols-2">
          <label className="grid gap-1">
            <span className="text-[12px]">From address</span>
            <Input
              value={fromAddress}
              onChange={(e) => {
                setFromAddress(e.target.value);
                setSaved(false);
              }}
              placeholder="support@yourdomain.com"
            />
          </label>
          <label className="grid gap-1">
            <span className="text-[12px]">From display name</span>
            <Input
              value={fromName}
              onChange={(e) => {
                setFromName(e.target.value);
                setSaved(false);
              }}
              placeholder="Acme Support"
            />
          </label>
          <label className="grid gap-1">
            <span className="text-[12px]">Reply-To</span>
            <Input
              value={replyTo}
              onChange={(e) => {
                setReplyTo(e.target.value);
                setSaved(false);
              }}
              placeholder="support@yourdomain.com"
            />
          </label>
          <label className="grid gap-1">
            <span className="text-[12px]">Org signature</span>
            <textarea
              value={signature}
              onChange={(e) => {
                setSignature(e.target.value);
                setSaved(false);
              }}
              rows={3}
              className="m-input"
              placeholder={"Acme Support\nsupport@yourdomain.com"}
            />
          </label>
        </div>

        <SaveBar dirty={dirty} saving={saving} saved={saved} error={error} onSave={() => void saveSettings()} />

        {transport !== "none" ? (
          <div className="grid gap-2 rounded-md border p-3" style={{ borderColor: "var(--m-border)" }}>
            <div className="text-[13px] font-medium">Sealed credentials</div>
            {secrets.map((secret) => (
              <div key={secret.label} className="flex items-center gap-2 text-[12px]">
                <Tag tone={secret.configured ? "success" : "neutral"}>
                  {secret.configured ? "Sealed" : "Not set"}
                </Tag>
                <span style={{ color: "var(--m-text-secondary)" }}>
                  {secret.label}
                  {secret.updatedAt ? ` · updated ${when(secret.updatedAt)}` : ""}
                </span>
              </div>
            ))}
            <FieldHint>
              Credentials are sealed with the workspace key and never shown again.
              Saving a new value replaces the old one.
            </FieldHint>
            <div className="flex items-center gap-2">
              <Button
                variant="secondary"
                disabled={
                  credSaving ||
                  (transport === "postmark" ? postmarkToken.trim() === "" : smtpPassword.trim() === "")
                }
                onClick={() => void saveCredential()}
              >
                {credSaving ? "Sealing…" : "Save credential"}
              </Button>
              {credSaved && !credError ? (
                <span className="text-[12px]" style={{ color: "var(--m-status-success-fg)" }}>
                  Sealed.
                </span>
              ) : null}
              {credError ? <FieldError>{credError}</FieldError> : null}
            </div>
          </div>
        ) : null}

        <div className="grid gap-2 rounded-md border p-3" style={{ borderColor: "var(--m-border)" }}>
          <div className="text-[13px] font-medium">Send a test email</div>
          <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            Renders a real message through the configured transport. The delivery
            log below carries the outcome.
          </p>
          <div className="flex flex-wrap items-center gap-2">
            <Input
              value={probeTo}
              onChange={(e) => setProbeTo(e.target.value)}
              placeholder={defaultProbeTo || "you@yourdomain.com"}
              aria-label="Test email recipient"
              className="w-[260px]"
            />
            <Button
              variant="secondary"
              disabled={probeBusy || !out.ready}
              onClick={() => void sendProbe()}
            >
              {probeBusy ? "Queueing…" : "Send test email"}
            </Button>
            {probeMessage ? (
              <span className="text-[12px]" style={{ color: "var(--m-status-success-fg)" }}>
                {probeMessage}
              </span>
            ) : null}
            {probeError ? <FieldError>{probeError}</FieldError> : null}
          </div>
        </div>
      </div>
          </CardContent>
    </Card>
  );
}

// --- delivery ledger ---------------------------------------------------------

function LedgerCard({
  ledger,
  statusFilter,
  onStatusFilter,
  onRefresh,
}: {
  ledger: LedgerView;
  statusFilter: string;
  onStatusFilter: (status: string) => void;
  onRefresh: () => void;
}) {
  const rows = ledger.rows.filter(
    (row) => statusFilter === "all" || row.status === statusFilter,
  );
  const counts = ledger.counts;
  return (
    <Card>
      <CardHeader>
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div>
            <h2 className="text-[14px] font-semibold">Delivery log</h2>
            <p className="mt-0.5 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              Every outbound message carries its retry state here, including
              ticket replies, notification email, digests, test sends.
            </p>
          </div>
          <div className="flex items-center gap-2">
            <Select value={statusFilter} onValueChange={onStatusFilter}>
              <SelectTrigger className="w-[130px]" aria-label="Filter by status">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All states</SelectItem>
                <SelectItem value="queued">Queued</SelectItem>
                <SelectItem value="sending">Sending</SelectItem>
                <SelectItem value="sent">Sent</SelectItem>
                <SelectItem value="dead">Dead</SelectItem>
              </SelectContent>
            </Select>
            <Button variant="secondary" onClick={onRefresh}>
              Refresh
            </Button>
          </div>
        </div>
      </CardHeader>
      <CardContent>
            <div className="grid gap-3">
        <div className="flex flex-wrap gap-2 text-[12px]">
          {(["queued", "sending", "sent", "dead"] as const).map((status) => (
            <span key={status} className="flex items-center gap-1.5">
              <Tag tone={statusTone(status)}>{counts[status]}</Tag>
              <span style={{ color: "var(--m-text-secondary)" }}>{status}</span>
            </span>
          ))}
        </div>

        {!ledger.transport_configured && counts.queued > 0 ? (
          <div
            role="status"
            data-testid="ledger-not-configured"
            className="rounded-md border px-3 py-2 text-[13px]"
            style={{
              borderColor: "var(--m-border)",
              background: "var(--m-canvas)",
              color: "var(--m-text-secondary)",
            }}
          >
            No transport is configured, so nothing has sent yet. These messages
            stay queued and leave as soon as sending is set up above.
          </div>
        ) : null}

        {rows.length === 0 ? (
          <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
            {statusFilter === "all"
              ? "No outbound messages yet."
              : `No messages in ${statusFilter} state.`}
          </p>
        ) : (
          <div className="rounded-md border overflow-hidden" style={{ borderColor: "var(--m-border)" }}>
            <Table>
              <thead>
                <tr
                  className="text-left text-[11px] uppercase tracking-wide"
                  style={{ color: "var(--m-text-muted)", background: "var(--m-canvas)" }}
                >
                  <Th className="font-medium">When</Th>
                  <Th className="font-medium">To</Th>
                  <Th className="font-medium">Subject</Th>
                  <Th className="font-medium">State</Th>
                  <Th className="font-medium">Attempts</Th>
                  <Th className="font-medium">Detail</Th>
                </tr>
              </thead>
              <tbody>
                {rows.map((row) => (
                  <Tr
                    key={row.message_id}
                    className="border-t"
                    style={{ borderColor: "var(--m-border)" }}
                  >
                    <Td className="px-3 py-2 text-[12px]" style={{ whiteSpace: "nowrap" }}>
                      {when(row.created_at)}
                    </Td>
                    <Td className="px-3 py-2 text-[12px]" style={{ fontFamily: "var(--m-font-mono)" }}>
                      {shortAddress(row.to_addresses)}
                    </Td>
                    <Td className="px-3 py-2 text-[13px]">{row.subject ?? "—"}</Td>
                    <Td className="px-3 py-2">
                      <Tag tone={statusTone(row.status)}>{row.status}</Tag>
                    </Td>
                    <Td className="px-3 py-2 text-[12px]" style={{ fontFamily: "var(--m-font-mono)" }}>
                      {row.attempts}/{row.max_attempts}
                    </Td>
                    <Td className="px-3 py-2 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                      {row.last_error ? (
                        <span title={row.last_error}>
                          {row.last_error.length > 60
                            ? `${row.last_error.slice(0, 60)}…`
                            : row.last_error}
                        </span>
                      ) : (
                        "—"
                      )}
                    </Td>
                  </Tr>
                ))}
              </tbody>
            </Table>
          </div>
        )}
      </div>
          </CardContent>
    </Card>
  );
}

// --- notification email ------------------------------------------------------

function NotificationEmailCard({
  settings,
  defaults,
  onSaved,
}: {
  settings: MailSettings;
  defaults: OrgDefaultRow[];
  onSaved: () => void;
}) {
  const [brandName, setBrandName] = useState(settings.outbound.brand_name);
  const [logoUrl, setLogoUrl] = useState(settings.outbound.logo_url);
  const [digestPolicy, setDigestPolicy] = useState(settings.digest_policy);
  const [brandSaving, setBrandSaving] = useState(false);
  const [brandSaved, setBrandSaved] = useState(false);
  const [brandError, setBrandError] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, OrgDefaultRow>>({});
  const [defaultsSaving, setDefaultsSaving] = useState(false);
  const [defaultsSaved, setDefaultsSaved] = useState(false);
  const [defaultsError, setDefaultsError] = useState<string | null>(null);

  useEffect(() => {
    setBrandName(settings.outbound.brand_name);
    setLogoUrl(settings.outbound.logo_url);
    setDigestPolicy(settings.digest_policy);
  }, [settings]);

  useEffect(() => {
    setDrafts(Object.fromEntries(defaults.map((row) => [row.kind, { ...row }])));
  }, [defaults]);

  const kinds = useMemo(() => {
    const known = Object.keys(drafts);
    return known.sort((a, b) => kindLabel(a).localeCompare(kindLabel(b)));
  }, [drafts]);

  const dirtyDefaults = kinds.some((kind) => {
    const original = defaults.find((row) => row.kind === kind);
    const draft = drafts[kind];
    if (!original || !draft) return false;
    return (
      original.in_app !== draft.in_app ||
      original.email !== draft.email ||
      (original.quiet_from ?? "") !== (draft.quiet_from ?? "") ||
      (original.quiet_until ?? "") !== (draft.quiet_until ?? "")
    );
  });

  const saveBrand = async () => {
    setBrandSaving(true);
    setBrandError(null);
    try {
      const res = await api.api.PUT("/api/v1/email/outbound-settings", {
        body: { brand_name: brandName, logo_url: logoUrl, digest_policy: digestPolicy },
      });
      if (res.error) {
        throw new Error(
          (res.error as { error?: { message?: string } }).error?.message ??
            "The notification email settings were rejected.",
        );
      }
      setBrandSaved(true);
      onSaved();
    } catch (e) {
      setBrandError(friendlyMutationText(e));
    } finally {
      setBrandSaving(false);
    }
  };

  const saveDefaults = async () => {
    setDefaultsSaving(true);
    setDefaultsError(null);
    try {
      for (const kind of kinds) {
        const draft = drafts[kind];
        const original = defaults.find((row) => row.kind === kind);
        if (!draft || !original) continue;
        const changed =
          original.in_app !== draft.in_app ||
          original.email !== draft.email ||
          (original.quiet_from ?? "") !== (draft.quiet_from ?? "") ||
          (original.quiet_until ?? "") !== (draft.quiet_until ?? "");
        if (!changed) continue;
        const quietFrom = draft.quiet_from === "" ? null : draft.quiet_from;
        const quietUntil = draft.quiet_until === "" ? null : draft.quiet_until;
        const res = await api.api.PUT("/api/v1/notification-org-defaults", {
          body: {
            kind: kind as never,
            in_app: draft.in_app,
            email: draft.email,
            quiet_from: quietFrom,
            quiet_until: quietUntil,
            timezone: draft.timezone,
          },
        });
        if (res.error) {
          throw new Error(
            (res.error as { error?: { message?: string } }).error?.message ??
              `The default for ${kindLabel(kind)} was rejected.`,
          );
        }
      }
      setDefaultsSaved(true);
      onSaved();
    } catch (e) {
      setDefaultsError(friendlyMutationText(e));
    } finally {
      setDefaultsSaving(false);
    }
  };

  const patchDraft = (kind: string, update: Partial<OrgDefaultRow>) => {
    setDrafts((prev) => {
      const current = prev[kind];
      if (!current) return prev;
      return { ...prev, [kind]: { ...current, ...update } };
    });
    setDefaultsSaved(false);
  };

  return (
    <Card>
      <CardHeader>
        <div>
          <h2 className="text-[14px] font-semibold">Notification email</h2>
          <p className="mt-0.5 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            The wrapper around notification and digest mail, and the workspace
            defaults members inherit until they override them.
          </p>
        </div>
      </CardHeader>
      <CardContent>
            <div className="grid gap-4">
        <div className="grid gap-3 md:grid-cols-3">
          <label className="grid gap-1">
            <span className="text-[12px]">Brand name in the header</span>
            <Input
              value={brandName}
              onChange={(e) => {
                setBrandName(e.target.value);
                setBrandSaved(false);
              }}
              placeholder="Acme MSP"
            />
          </label>
          <label className="grid gap-1">
            <span className="text-[12px]">Logo URL (https, optional)</span>
            <Input
              value={logoUrl}
              onChange={(e) => {
                setLogoUrl(e.target.value);
                setBrandSaved(false);
              }}
              placeholder="https://cdn.yourdomain.com/logo.png"
            />
          </label>
          <label className="grid gap-1">
            <span className="text-[12px]">Daily digest policy</span>
            <Select
              value={digestPolicy}
              onValueChange={(v) => {
                setDigestPolicy(v);
                setBrandSaved(false);
              }}
            >
              <SelectTrigger aria-label="Daily digest policy">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="allowed">Members may opt in</SelectItem>
                <SelectItem value="disabled">Disabled for the workspace</SelectItem>
              </SelectContent>
            </Select>
          </label>
        </div>
        <FieldHint>
          Members choose their own digest opt-in and send hour on the
          Notifications page; the policy here wins when it says disabled.
        </FieldHint>
        <SaveBar
          dirty={
            brandName !== settings.outbound.brand_name ||
            logoUrl !== settings.outbound.logo_url ||
            digestPolicy !== settings.digest_policy
          }
          saving={brandSaving}
          saved={brandSaved}
          error={brandError}
          onSave={() => void saveBrand()}
        />

        <div className="grid gap-2">
          <div className="text-[13px] font-medium">Workspace defaults per event</div>
          <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            A member with their own override keeps it; everyone else follows these
            rows. Quiet hours hold non-critical email during the window.
          </p>
          <div className="rounded-md border overflow-hidden" style={{ borderColor: "var(--m-border)" }}>
            <Table>
              <thead>
                <tr
                  className="text-left text-[11px] uppercase tracking-wide"
                  style={{ color: "var(--m-text-muted)", background: "var(--m-canvas)" }}
                >
                  <Th className="font-medium">Event</Th>
                  <Th className="font-medium">In-app</Th>
                  <Th className="font-medium">Email</Th>
                  <Th className="font-medium">Quiet from</Th>
                  <Th className="font-medium">Until</Th>
                </tr>
              </thead>
              <tbody>
                {kinds.map((kind) => {
                  const draft = drafts[kind];
                  if (!draft) return null;
                  return (
                    <Tr
                      key={kind}
                      className="border-t"
                      style={{ borderColor: "var(--m-border)" }}
                    >
                      <Td className="px-3 py-2 text-[13px]">{kindLabel(kind)}</Td>
                      <Td className="px-3 py-2">
                        <Checkbox
                          checked={draft.in_app}
                          onCheckedChange={(v) => patchDraft(kind, { in_app: v === true })}
                          aria-label={`${kindLabel(kind)} org default in-app`}
                        />
                      </Td>
                      <Td className="px-3 py-2">
                        <Checkbox
                          checked={draft.email}
                          onCheckedChange={(v) => patchDraft(kind, { email: v === true })}
                          aria-label={`${kindLabel(kind)} org default email`}
                        />
                      </Td>
                      <Td className="px-3 py-2">
                        <Input
                          type="time"
                          value={draft.quiet_from ?? ""}
                          onChange={(e) =>
                            patchDraft(kind, { quiet_from: e.target.value === "" ? null : e.target.value })
                          }
                          aria-label={`${kindLabel(kind)} org default quiet from`}
                          className="w-[110px]"
                        />
                      </Td>
                      <Td className="px-3 py-2">
                        <Input
                          type="time"
                          value={draft.quiet_until ?? ""}
                          onChange={(e) =>
                            patchDraft(kind, { quiet_until: e.target.value === "" ? null : e.target.value })
                          }
                          aria-label={`${kindLabel(kind)} org default quiet until`}
                          className="w-[110px]"
                        />
                      </Td>
                    </Tr>
                  );
                })}
              </tbody>
            </Table>
          </div>
          <SaveBar
            dirty={dirtyDefaults}
            saving={defaultsSaving}
            saved={defaultsSaved}
            error={defaultsError}
            onSave={() => void saveDefaults()}
          />
        </div>
      </div>
          </CardContent>
    </Card>
  );
}
