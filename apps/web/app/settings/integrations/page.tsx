// /settings/integrations — webhook manager + inbound source registry + API-key entry point.
//
// Purpose: signed OUTBOUND webhook subscriptions over /api/v1/webhooks (CRUD,
// registry-driven event-type multiselect, show-once secrets, test-fire, the
// keyset-paged DELIVERY LOG) and — Wave 43 lane 5 — the INBOUND side beside
// it: registered sources with their ingest URL, source_ref path segment,
// show-once signing secret, declared event type, and a per-source ingest log
// read off the existing ledgers (the audit trail for generic receivers, the
// alerts ledger for the alert profile).
//
// Non-purpose: API-keys management lives on its own page
// (/settings/api-keys); no delivery retry logic here (worker-owned); no
// connector installation consent flows. There is no source-delete endpoint
// on the API today, so this surface offers none — an absent control is the
// honest surface, not a dead button.
"use client";

import { friendlyMutationText, resultErrorText } from "../../../lib/error-copy";
import {
  Badge,
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
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useReducer, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import { SettingsHeader } from "../../../components/settings/settings-shell";
import Link from "next/link";
import {
  ALL_CATALOG_TYPES,
  EVENT_CATALOG,
  classifyDelivery,
  eventSelectionReducer,
  eventTypeOrigin,
  eventTypeOriginLabel,
  selectionToPayload,
  ungrantedTypesFromError,
} from "../../../lib/admin/webhook-events";
import {
  EVENT_TYPE_RE,
  SOURCE_REF_RE,
  alertIngestRows,
  eventTypeProblem,
  ingestUrl,
  m2mIngestRows,
  sourceProfile,
  sourceRefProblem,
} from "../../../lib/admin/webhook-sources";
import type { IngestLogRow } from "../../../lib/admin/webhook-sources";

type WebhookView = components["schemas"]["WebhookView"];
type DeliveryAttemptView = components["schemas"]["DeliveryAttemptView"];
type WebhookSecretResponse = components["schemas"]["WebhookSecretResponse"];
type AlertSourceView = components["schemas"]["AlertSourceView"];
type AlertView = components["schemas"]["AlertView"];
type AuditEvent = components["schemas"]["AuditEvent"];
type BoardNavEntry = components["schemas"]["BoardNavEntry"];

const PAGE_SIZE = 20;
/** Ledger page for the per-source ingest log (audit trail or alerts). */
const LOG_PAGE = 25;
/** How many audit pages one log open walks past another source's traffic. */
const M2M_SCAN_PAGES = 5;
/** The audit action the generic receiver writes per accepted envelope. */
const WEBHOOK_INGESTED_ACTION = "webhook.ingested";

function idempotencyKey(prefix: string): string {
  return typeof crypto !== "undefined" && crypto.randomUUID !== undefined
    ? crypto.randomUUID()
    : `${prefix}-${Date.now()}`;
}

/** Copy for the registry refusal: name every type the server turned away. */
function registryRefusalCopy(refused: string[]): string {
  const names = refused.map((t) => `“${t}”`).join(", ");
  return `The event registry refused ${names}. Those types are unknown or ungranted — subscribe to platform types, or ask an owner for a grant.`;
}

export default function IntegrationsSettingsPage() {
  const auth = useRequireAuth();
  const [webhooks, setWebhooks] = useState<WebhookView[]>([]);
  const [url, setUrl] = useState("");
  const [selection, dispatchSelection] = useReducer(eventSelectionReducer, { selected: new Set<string>() });
  const [customType, setCustomType] = useState("");
  const [busy, setBusy] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  /** Secret reveal after create or rotate — rendered once, never refetched. */
  const [shownSecret, setShownSecret] = useState<{ label: string; secret: string } | null>(null);

  // Per-webhook log state.
  const [logPages, setLogPages] = useState<Map<string, DeliveryAttemptView[]>>(new Map());
  const [logCursors, setLogCursors] = useState<Map<string, string | undefined>>(new Map());
  const [logLoading, setLogLoading] = useState<string | null>(null);

  // Rotate dialog state.
  const [rotatingId, setRotatingId] = useState<string | null>(null);
  const [rotateBusy, setRotateBusy] = useState(false);

  // Test-fire feedback keyed by webhook id → last fired envelope id.
  const [firedEventIds, setFiredEventIds] = useState<Map<string, string>>(new Map());

  // --- inbound sources (Wave 43 lane 5, spec item B) ---------------------------
  const [sources, setSources] = useState<AlertSourceView[] | null>(null);
  const [sourcesError, setSourcesError] = useState<string | null>(null);
  const [boards, setBoards] = useState<BoardNavEntry[]>([]);

  // Register dialog state.
  const [registerOpen, setRegisterOpen] = useState(false);
  const [srcName, setSrcName] = useState("");
  const [srcRef, setSrcRef] = useState("");
  const [srcEventType, setSrcEventType] = useState("");
  const [srcBoardId, setSrcBoardId] = useState("");
  const [srcWindow, setSrcWindow] = useState("900");
  const [registerBusy, setRegisterBusy] = useState(false);
  const [registerError, setRegisterError] = useState<string | null>(null);
  /** Source secret reveal — shown once, same contract as webhooks. */
  const [shownSourceSecret, setShownSourceSecret] = useState<{
    name: string;
    sourceRef: string;
    secret: string;
  } | null>(null);

  // Edit dialog state.
  const [editingSource, setEditingSource] = useState<AlertSourceView | null>(null);
  const [editName, setEditName] = useState("");
  const [editEventType, setEditEventType] = useState("");
  const [editBusy, setEditBusy] = useState(false);
  const [editError, setEditError] = useState<string | null>(null);

  // Per-source ingest log: raw ledger pages per source id.
  const [auditPages, setAuditPages] = useState<Map<string, AuditEvent[]>>(new Map());
  const [auditCursors, setAuditCursors] = useState<Map<string, string | undefined>>(new Map());
  const [alertPages, setAlertPages] = useState<Map<string, AlertView[]>>(new Map());
  /** Alert-log sources whose ledger came back short — no older page left. */
  const [alertLogDone, setAlertLogDone] = useState<Set<string>>(new Set());
  const [ingestLogBusy, setIngestLogBusy] = useState<string | null>(null);
  const [ingestLogError, setIngestLogError] = useState<string | null>(null);

  const origin = typeof window !== "undefined" ? window.location.origin : "";

  const refresh = useCallback(async () => {
    const { data, error } = await api.api.GET("/api/v1/webhooks");
    if (error !== undefined || data === undefined) {
      setActionError("Failed to load webhooks.");
      return;
    }
    setWebhooks(data.webhooks);
  }, []);

  const refreshSources = useCallback(async () => {
    setSourcesError(null);
    const { data, error } = await api.api.GET("/api/v1/alert-sources");
    if (error !== undefined || data === undefined) {
      setSourcesError("Failed to load inbound sources.");
      return;
    }
    setSources(data.sources);
  }, []);

  const refreshBoards = useCallback(async () => {
    const { data } = await api.api.GET("/api/v1/boards/nav");
    if (data !== undefined) setBoards(data);
  }, []);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refresh();
    void refreshSources();
    void refreshBoards();
  }, [auth.status, refresh, refreshSources, refreshBoards]);

  if (auth.status !== "authed") return null;

  const create = async () => {
    setBusy(true);
    setFormError(null);
    try {
      const res = await api.api.POST("/api/v1/webhooks", {
        body: { url, event_types: selectionToPayload(selection.selected) },
      });
      if (!res.data) {
        // The registry refusal names every ungranted type — say exactly what
        // was refused instead of the generic validation line.
        const envelope = res.error as { error?: { message?: string } } | undefined;
        const refused = ungrantedTypesFromError(envelope?.error?.message);
        setFormError(
          refused.length > 0
            ? registryRefusalCopy(refused)
            : resultErrorText(res, "The webhook could not be created."),
        );
        return;
      }
      // Show-once contract: this render is the ONLY time the secret exists.
      const payload = res.data as WebhookSecretResponse;
      setShownSecret({ label: payload.webhook.url, secret: payload.secret });
      setUrl("");
      dispatchSelection({ type: "clear" });
      await refresh();
    } catch (e) {
      setFormError(friendlyMutationText(e, "The webhook could not be created."));
    } finally {
      setBusy(false);
    }
  };

  const mutate = async (
    id: string,
    action: "toggle" | "delete" | "test",
    current?: WebhookView,
  ) => {
    setActionError(null);
    try {
      if (action === "toggle" && current) {
        const res = await api.api.PATCH("/api/v1/webhooks/{id}", {
          params: { path: { id } },
          body: { enabled: !current.enabled },
        });
        if (res.error) throw new Error("update failed");
      } else if (action === "delete") {
        const res = await api.api.DELETE("/api/v1/webhooks/{id}", {
          params: { path: { id } },
        });
        if (res.error) throw new Error("delete failed");
      } else {
        const res = await api.api.POST("/api/v1/webhooks/{id}/test", {
          params: { path: { id } },
        });
        if (res.error) throw new Error("test fire failed");
        setFiredEventIds((prev) => new Map(prev).set(id, res.data.event_id));
        // Give the pipeline a beat, then pull the fresh first page.
        setTimeout(() => void reloadLog(id), 1500);
      }
      await refresh();
    } catch (e) {
      setActionError(friendlyMutationText(e));
    }
  };

  /** Rotation honors the show-once secret lifecycle without a rotate endpoint:
   * create a replacement subscription with identical config, surface its
   * secret once, then retire the original. */
  const rotate = async () => {
    const target = webhooks.find((w) => w.id === rotatingId);
    if (!target) return;
    setRotateBusy(true);
    try {
      const res = await api.api.POST("/api/v1/webhooks", {
        body: { url: target.url, event_types: [...target.event_types] },
      });
      if (!res.data) throw new Error("replacement creation failed");
      const payload = res.data as WebhookSecretResponse;
      await api.api.DELETE("/api/v1/webhooks/{id}", { params: { path: { id: target.id } } });
      setShownSecret({ label: `${payload.webhook.url} (rotated)`, secret: payload.secret });
      setRotatingId(null);
      await refresh();
    } catch (e) {
      setActionError(friendlyMutationText(e));
    } finally {
      setRotateBusy(false);
    }
  };

  const reloadLog = async (id: string, cursor?: string) => {
    setLogLoading(id);
    try {
      const res = await api.api.GET("/api/v1/webhooks/{id}/deliveries", {
        params: {
          path: { id },
          query: cursor !== undefined ? { limit: PAGE_SIZE, cursor } : { limit: PAGE_SIZE },
        },
      });
      if (res.error || res.data === undefined) throw new Error("delivery log failed to load");
      const page = res.data;
      setLogPages((prev) => {
        const existing = prev.get(id) ?? [];
        const merged = cursor === undefined ? page.deliveries : [...existing, ...page.deliveries];
        return new Map(prev).set(id, merged);
      });
      setLogCursors((prev) => new Map(prev).set(id, page.next_cursor ?? undefined));
    } catch (e) {
      setActionError(friendlyMutationText(e));
    } finally {
      setLogLoading(null);
    }
  };

  const toggleLog = async (id: string) => {
    if (logPages.has(id)) {
      setLogPages((prev) => {
        const next = new Map(prev);
        next.delete(id);
        return next;
      });
      return;
    }
    await reloadLog(id);
  };

  const customAdd = () => {
    const value = customType.trim();
    if (value.length > 0 && !ALL_CATALOG_TYPES.includes(value)) {
      dispatchSelection({ type: "add-custom", raw: value });
    }
    setCustomType("");
  };

  // --- inbound source mutations -------------------------------------------------

  const registerSource = async () => {
    if (srcName.trim() === "" || srcBoardId === "") return;
    const refProblem = sourceRefProblem(srcRef);
    const typeProblem = eventTypeProblem(srcEventType);
    if (refProblem !== null || typeProblem !== null) {
      setRegisterError(refProblem ?? typeProblem);
      return;
    }
    setRegisterBusy(true);
    setRegisterError(null);
    const eventType = srcEventType.trim();
    const res = await api.api.POST("/api/v1/alert-sources", {
      body: {
        name: srcName.trim(),
        source_ref: srcRef.trim() === "" ? undefined : srcRef.trim(),
        board_id: srcBoardId,
        event_type: eventType === "" ? null : eventType,
        correlation_window_secs:
          eventType === "" ? Number.parseInt(srcWindow, 10) || 900 : undefined,
        idempotency_key: idempotencyKey("src"),
      },
    });
    setRegisterBusy(false);
    if (!res.data) {
      setRegisterError(resultErrorText(res, "The source could not be registered."));
      return;
    }
    if (res.data.secret !== undefined && res.data.secret !== null) {
      setShownSourceSecret({
        name: res.data.name,
        sourceRef: res.data.source_ref,
        secret: res.data.secret,
      });
    }
    setRegisterOpen(false);
    setSrcName("");
    setSrcRef("");
    setSrcEventType("");
    setSrcWindow("900");
    await refreshSources();
  };

  /** Optimistic enable flip → authoritative reconcile or rollback. */
  const toggleSource = async (source: AlertSourceView) => {
    setActionError(null);
    setSources((cur) =>
      (cur ?? []).map((s) => (s.id === source.id ? { ...s, enabled: !s.enabled } : s)),
    );
    const { error } = await api.api.PATCH("/api/v1/alert-sources/{id}", {
      params: { path: { id: source.id } },
      body: { enabled: !source.enabled, idempotency_key: idempotencyKey("src-patch") },
    });
    if (error !== undefined) {
      setSources((cur) =>
        (cur ?? []).map((s) => (s.id === source.id ? { ...s, enabled: source.enabled } : s)),
      );
      setActionError(
        friendlyMutationText(error as never, "The enable toggle was rejected."),
      );
      return;
    }
    await refreshSources();
  };

  const openEditSource = (source: AlertSourceView) => {
    setEditingSource(source);
    setEditName(source.name);
    setEditEventType(source.event_type ?? "");
    setEditError(null);
  };

  const saveSourceEdit = async () => {
    const target = editingSource;
    if (target === null) return;
    if (editName.trim() === "") {
      setEditError("Give the source a name.");
      return;
    }
    const typeProblem = eventTypeProblem(editEventType);
    if (typeProblem !== null) {
      setEditError(typeProblem);
      return;
    }
    setEditBusy(true);
    setEditError(null);
    const nextType = editEventType.trim();
    const currentType = target.event_type ?? "";
    const { error } = await api.api.PATCH("/api/v1/alert-sources/{id}", {
      params: { path: { id: target.id } },
      body: {
        name: editName.trim(),
        // Absent = no change; null = revert to the alert profile; a value
        // promotes the source to the generic receiver.
        event_type: nextType === currentType ? undefined : nextType === "" ? null : nextType,
        idempotency_key: idempotencyKey("src-edit"),
      },
    });
    setEditBusy(false);
    if (error !== undefined) {
      setEditError(friendlyMutationText(error as never, "The source could not be updated."));
      return;
    }
    setEditingSource(null);
    await refreshSources();
  };

  // --- per-source ingest logs -----------------------------------------------------

  /** Generic receivers log one audit event per accepted envelope. The ledger
   * is org-wide while this surface shows one source, so a page carrying none
   * of this source's rows keeps walking forward — bounded, so a long tail of
   * another source's traffic cannot spin forever. */
  const loadM2mLog = async (source: AlertSourceView, cursor?: string) => {
    setIngestLogBusy(source.id);
    setIngestLogError(null);
    try {
      let nextCursor = cursor;
      let first = cursor === undefined;
      for (let walked = 0; walked < M2M_SCAN_PAGES; walked += 1) {
        const res = await api.api.GET("/api/v1/audit-events", {
          params: {
            query: {
              action: WEBHOOK_INGESTED_ACTION,
              limit: LOG_PAGE,
              cursor: nextCursor ?? undefined,
            },
          },
        });
        if (res.error || res.data === undefined) throw new Error("ingest log failed to load");
        const page = res.data;
        setAuditPages((prev) => {
          const existing = prev.get(source.id) ?? [];
          const merged = first ? page.events : [...existing, ...page.events];
          return new Map(prev).set(source.id, merged);
        });
        setAuditCursors((prev) =>
          new Map(prev).set(source.id, page.next_cursor ?? undefined),
        );
        first = false;
        nextCursor = page.next_cursor ?? undefined;
        if (m2mIngestRows(page.events, source.source_ref).length > 0 || nextCursor === undefined) {
          break;
        }
      }
    } catch (e) {
      setIngestLogError(friendlyMutationText(e, "The ingest log could not be loaded."));
    } finally {
      setIngestLogBusy(null);
    }
  };

  /** Alert-profile sources log into the alerts ledger, filterable server-side
   * by source; offset-paged, so "older" walks the offset. */
  const loadAlertLog = async (source: AlertSourceView, offset = 0) => {
    setIngestLogBusy(source.id);
    setIngestLogError(null);
    try {
      const res = await api.api.GET("/api/v1/alerts", {
        params: {
          query: {
            source_id: source.id,
            limit: LOG_PAGE,
            offset,
          },
        },
      });
      if (res.error || res.data === undefined) throw new Error("ingest log failed to load");
      const page = res.data;
      setAlertPages((prev) => {
        const existing = prev.get(source.id) ?? [];
        const merged = offset === 0 ? page.alerts : [...existing, ...page.alerts];
        return new Map(prev).set(source.id, merged);
      });
      // A short page is the offset-paged ledger's end-of-log signal.
      setAlertLogDone((prev) => {
        const next = new Set(prev);
        if (page.alerts.length < LOG_PAGE) next.add(source.id);
        else next.delete(source.id);
        return next;
      });
    } catch (e) {
      setIngestLogError(friendlyMutationText(e, "The ingest log could not be loaded."));
    } finally {
      setIngestLogBusy(null);
    }
  };

  const toggleIngestLog = (source: AlertSourceView) => {
    const profile = sourceProfile(source);
    const open =
      profile.kind === "m2m" ? auditPages.has(source.id) : alertPages.has(source.id);
    if (open) {
      setAuditPages((prev) => {
        const next = new Map(prev);
        next.delete(source.id);
        return next;
      });
      setAlertPages((prev) => {
        const next = new Map(prev);
        next.delete(source.id);
        return next;
      });
      setIngestLogError(null);
      return;
    }
    if (profile.kind === "m2m") void loadM2mLog(source);
    else void loadAlertLog(source);
  };

  const ingestRowsFor = (source: AlertSourceView): IngestLogRow[] => {
    const profile = sourceProfile(source);
    if (profile.kind === "m2m") {
      return m2mIngestRows(auditPages.get(source.id) ?? [], source.source_ref);
    }
    return alertIngestRows(alertPages.get(source.id) ?? []);
  };

  return (
    <div className="grid gap-6 max-w-2xl">
      <SettingsHeader
        title="Integrations"
        description={
          <>
            Signed webhooks push every change you subscribe to. Deliveries are
            HMAC-signed and retried with backoff — every attempt lands in the
            log. Machine API keys live under{" "}
            <Link
              href="/settings/api-keys"
              style={{ color: "var(--m-accent)", textDecoration: "underline" }}
            >
              API keys
            </Link>
            .
          </>
        }
      />

      {shownSecret !== null ? (
        <Card>
          <div className="p-4 grid gap-2">
            <Label>Signing secret for {shownSecret.label}</Label>
            <code
              className="text-[12px] break-all rounded-[var(--m-radius-container)] px-2 py-1.5"
              style={{
                background: "var(--m-surface)",
                color: "var(--m-text)",
                border: "1px solid var(--m-border)",
              }}
            >
              {shownSecret.secret}
            </code>
            <FieldHint>Copy it now — it is never shown again.</FieldHint>
            <div>
              <Button variant="ghost" onClick={() => setShownSecret(null)}>
                Dismiss
              </Button>
            </div>
          </div>
        </Card>
      ) : null}

      {shownSourceSecret !== null ? (
        <Card data-testid="source-secret-panel">
          <div className="p-4 grid gap-2">
            <Label>Signing secret for {shownSourceSecret.name}</Label>
            <code
              className="text-[12px] break-all rounded-[var(--m-radius-container)] px-2 py-1.5"
              style={{
                background: "var(--m-surface)",
                color: "var(--m-text)",
                border: "1px solid var(--m-border)",
              }}
              data-testid="source-secret-value"
            >
              {shownSourceSecret.secret}
            </code>
            <FieldHint>
              Ingest path <code>/api/v1/webhooks/ingest/{shownSourceSecret.sourceRef}</code>{" "}
              (generic) or <code>/api/v1/alerts/ingest/{shownSourceSecret.sourceRef}</code>{" "}
              (alert profile) — which one depends on the declared event type. Copy
              the secret now — it is never shown again.
            </FieldHint>
            <div>
              <Button variant="ghost" onClick={() => setShownSourceSecret(null)}>
                Dismiss
              </Button>
            </div>
          </div>
        </Card>
      ) : null}

      <Card>
        <form
          className="p-4 grid gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            void create();
          }}
        >
          <div>
            <Label htmlFor="wh-url">Endpoint URL</Label>
            <Input
              id="wh-url"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="https://hooks.example.test/marula"
              required
            />
          </div>
          <fieldset className="grid gap-1">
            <legend className="text-[12px]" style={{ color: "var(--m-text)" }}>
              Event types
            </legend>
            <FieldHint>
              Nothing selected subscribes to all events — including types a plugin
              emits later. Catalog types below are platform events and need no
              grant; anything else must be granted to this org before the server
              accepts it.
            </FieldHint>
            {EVENT_CATALOG.map((group) => (
              <div key={group.group}>
                <p className="text-[11px] uppercase tracking-wide mt-1" style={{ color: "var(--m-text-muted)" }}>
                  {group.group}
                </p>
                <div className="flex flex-wrap gap-x-3">
                  {group.types.map((t) => (
                    <label key={t} className="inline-flex items-center gap-1.5 text-[12px] py-0.5">
                      <input
                        type="checkbox"
                        checked={selection.selected.has(t)}
                        onChange={() => dispatchSelection({ type: "toggle", eventType: t })}
                        aria-label={t}
                      />
                      <code>{t}</code>
                    </label>
                  ))}
                </div>
              </div>
            ))}
            <CustomTypesBar
              selected={selection.selected}
              onToggle={(t) => dispatchSelection({ type: "toggle", eventType: t })}
              customType={customType}
              setCustomType={setCustomType}
              onAdd={customAdd}
            />
          </fieldset>
          {formError !== null ? (
            <p className="text-[12px]" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
              {formError}
            </p>
          ) : null}
          <div>
            <Button type="submit" disabled={busy || !url}>
              Create webhook
            </Button>
          </div>
        </form>
      </Card>

      {actionError !== null && (
        <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {actionError}
        </p>
      )}

      <div className="grid gap-3">
        {webhooks.length === 0 && !busy && (
          <EmptyState message="No webhooks yet — add an endpoint above." />
        )}
        {webhooks.map((wh) => {
          const firedId = firedEventIds.get(wh.id);
          return (
            <Card key={wh.id}>
              <div className="p-4 grid gap-2">
                <div className="flex items-center justify-between gap-2">
                  <code className="text-[12px]" style={{ color: "var(--m-text)" }}>
                    {wh.url}
                  </code>
                  <span
                    className="text-[10px] uppercase tracking-wide px-1.5 py-0.5 rounded-[var(--m-radius-container)]"
                    style={{
                      border: "1px solid var(--m-border)",
                      color: wh.enabled
                        ? "var(--m-status-success-fg)"
                        : "var(--m-text-muted)",
                    }}
                  >
                    {wh.enabled ? "enabled" : "disabled"}
                  </span>
                </div>
                <p className="text-[12px] flex flex-wrap items-center gap-1.5" style={{ color: "var(--m-text-muted)" }}>
                  {wh.event_types.length === 0 ? (
                    <span>All events</span>
                  ) : (
                    wh.event_types.map((t) => (
                      <span key={t} className="inline-flex items-center gap-1">
                        <code>{t}</code>
                        {eventTypeOrigin(t) === "grant" ? (
                          <Badge tone="warning">needs grant</Badge>
                        ) : null}
                      </span>
                    ))
                  )}
                </p>
                <div className="flex flex-wrap gap-2">
                  <Button variant="ghost" onClick={() => void mutate(wh.id, "toggle", wh)}>
                    {wh.enabled ? "Disable" : "Enable"}
                  </Button>
                  <Button variant="ghost" onClick={() => void mutate(wh.id, "test")}>
                    Send test
                  </Button>
                  <Button variant="ghost" onClick={() => setRotatingId(wh.id)}>
                    Rotate
                  </Button>
                  <Button variant="ghost" onClick={() => void toggleLog(wh.id)}>
                    {logPages.has(wh.id) ? "Hide log" : "Delivery log"}
                  </Button>
                  <Button variant="ghost" onClick={() => void mutate(wh.id, "delete")}>
                    Delete
                  </Button>
                </div>
                {firedId !== undefined && (
                  <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                    Test envelope <code>{firedId}</code> dispatched — watch for it below.
                  </p>
                )}
                {logPages.has(wh.id) ? (
                  <div className="rounded-[var(--m-radius-container)] mt-1" style={{ border: "1px solid var(--m-border)" }}>
                    {(logPages.get(wh.id) ?? []).length === 0 && logLoading !== wh.id ? (
                      <p className="text-[12px] p-3" style={{ color: "var(--m-text-muted)" }}>
                        No attempts yet — send a test event.
                      </p>
                    ) : (
                      (logPages.get(wh.id) ?? []).map((d) => {
                        const verdict = classifyDelivery(d);
                        return (
                          <div
                            key={d.id}
                            className="grid gap-0.5 px-3 py-2 text-[12px]"
                            style={{ borderBottom: "1px solid var(--m-border)" }}
                          >
                            <div className="flex items-center justify-between gap-2 flex-wrap">
                              <span style={{ color: "var(--m-text-secondary)" }}>
                                {d.event_type} · attempt #{d.attempt}
                              </span>
                              <Badge tone={verdict.tone}>{verdict.label}</Badge>
                            </div>
                            <div className="flex items-center justify-between gap-2 tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                              <span title={d.created_at}>{new Date(d.created_at).toLocaleString()}</span>
                              <span>{d.last_error ?? ""}</span>
                            </div>
                          </div>
                        );
                      })
                    )}
                    <div className="p-2">
                      {logCursors.get(wh.id) != null ? (
                        <Button
                          variant="ghost"
                          disabled={logLoading === wh.id}
                          onClick={() => void reloadLog(wh.id, logCursors.get(wh.id))}
                        >
                          {logLoading === wh.id ? "Loading…" : "Load older"}
                        </Button>
                      ) : (logPages.get(wh.id) ?? []).length > 0 ? (
                        <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                          Beginning of log.
                        </span>
                      ) : null}
                    </div>
                  </div>
                ) : null}
              </div>
            </Card>
          );
        })}
      </div>

      {/* Rotate confirmation */}
      <Dialog open={rotatingId !== null} onOpenChange={(open) => !open && setRotatingId(null)}>
        <DialogContent
          title="Rotate signing secret?"
          description="Marula creates a replacement subscription with the same URL and events, shows you its new secret once, then deletes this webhook."
        >
          <DialogActions>
            <Button variant="ghost" onClick={() => setRotatingId(null)} disabled={rotateBusy}>
              Cancel
            </Button>
            <Button onClick={() => void rotate()} disabled={rotateBusy}>
              {rotateBusy ? "Rotating…" : "Rotate now"}
            </Button>
          </DialogActions>
        </DialogContent>
      </Dialog>

      {/* --- Inbound sources -------------------------------------------------- */}
      <section className="grid gap-3" aria-labelledby="inbound-sources-heading">
        <div className="grid gap-1">
          <h2 id="inbound-sources-heading" className="text-sm font-semibold" style={{ color: "var(--m-text)" }}>
            Inbound sources
          </h2>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            A provider POSTs a signed envelope to a source&apos;s ingest URL. A
            source with a declared event type is a generic receiver — each
            accepted envelope is emitted onto the domain-event stream. Without
            one it is the classic alert profile, which creates an alert row and
            correlates storms into an incident ticket.
          </p>
        </div>

        <div>
          <Button variant="secondary" onClick={() => setRegisterOpen(true)} data-testid="register-source">
            Register source
          </Button>
        </div>

        {sources === null ? (
          <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
            Loading sources…
          </p>
        ) : sourcesError !== null ? (
          <p className="text-[12px]" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
            {sourcesError}
          </p>
        ) : sources.length === 0 ? (
          <EmptyState message="No inbound sources yet — register one above." />
        ) : (
          <div className="grid gap-3">
            {sources.map((source) => {
              const profile = sourceProfile(source);
              const logOpen =
                profile.kind === "m2m"
                  ? auditPages.has(source.id)
                  : alertPages.has(source.id);
              const rows = ingestRowsFor(source);
              // Cursor-paged ledgers end on a null cursor; offset-paged ones
              // end when a short page comes back.
              const olderLabel =
                profile.kind === "m2m"
                  ? auditCursors.get(source.id) != null
                    ? "Load older"
                    : undefined
                  : alertLogDone.has(source.id)
                    ? undefined
                    : "Load older";
              return (
                <Card key={source.id}>
                  <div className="p-4 grid gap-2">
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
                        {source.name}
                      </span>
                      <span className="flex items-center gap-1.5">
                        <Badge tone={profile.kind === "m2m" ? "neutral" : "outline"}>
                          {profile.label}
                        </Badge>
                        <span
                          className="text-[10px] uppercase tracking-wide px-1.5 py-0.5 rounded-[var(--m-radius-container)]"
                          style={{
                            border: "1px solid var(--m-border)",
                            color: source.enabled
                              ? "var(--m-status-success-fg)"
                              : "var(--m-text-muted)",
                          }}
                        >
                          {source.enabled ? "enabled" : "disabled"}
                        </span>
                      </span>
                    </div>
                    <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                      {profile.effect}
                      {source.event_type != null ? (
                        <>
                          {" "}
                          Declared event type: <code>{source.event_type}</code>
                        </>
                      ) : null}
                    </p>
                    <div className="grid gap-1">
                      <span className="text-[11px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
                        Ingest URL
                      </span>
                      <code
                        className="text-[12px] break-all rounded-[var(--m-radius-container)] px-2 py-1.5"
                        style={{
                          background: "var(--m-surface)",
                          color: "var(--m-text)",
                          border: "1px solid var(--m-border)",
                        }}
                        data-testid={`ingest-url-${source.id}`}
                      >
                        {ingestUrl(origin, profile.ingestPath)}
                      </code>
                      <FieldHint>
                        Path segment <code>{source.source_ref}</code> · HMAC-SHA256
                        over the exact raw body · the secret was shown once at
                        registration.
                      </FieldHint>
                    </div>
                    <div className="flex flex-wrap gap-2">
                      <Button variant="ghost" onClick={() => openEditSource(source)}>
                        Edit
                      </Button>
                      <Button variant="ghost" onClick={() => void toggleSource(source)}>
                        {source.enabled ? "Disable" : "Enable"}
                      </Button>
                      <Button
                        variant="ghost"
                        onClick={() => toggleIngestLog(source)}
                        data-testid={`ingest-log-${source.id}`}
                      >
                        {logOpen ? "Hide ingest log" : "Ingest log"}
                      </Button>
                    </div>
                    {logOpen ? (
                      <IngestLogPanel
                        rows={rows}
                        busy={ingestLogBusy === source.id}
                        error={ingestLogError}
                        noun={profile.kind === "m2m" ? "accepted envelopes" : "alerts"}
                        entityNoun={profile.kind === "m2m" ? "entity" : "ticket"}
                        olderLabel={olderLabel}
                        onOlder={() => {
                          if (profile.kind === "m2m") {
                            void loadM2mLog(source, auditCursors.get(source.id));
                          } else {
                            void loadAlertLog(
                              source,
                              (alertPages.get(source.id) ?? []).length,
                            );
                          }
                        }}
                      />
                    ) : null}
                  </div>
                </Card>
              );
            })}
          </div>
        )}
      </section>

      {/* Register-source dialog */}
      <Dialog open={registerOpen} onOpenChange={(open) => !open && setRegisterOpen(false)}>
        <DialogContent
          title="Register an inbound source"
          description="The provider signs each POST with the secret shown once after registration."
        >
          <div className="grid gap-3 p-4">
            <div>
              <Label htmlFor="src-name">Name</Label>
              <Input
                id="src-name"
                value={srcName}
                onChange={(e) => setSrcName(e.target.value)}
                placeholder="RMM production"
              />
            </div>
            <div>
              <Label htmlFor="src-ref">Custom source ref (optional)</Label>
              <Input
                id="src-ref"
                value={srcRef}
                onChange={(e) => setSrcRef(e.target.value)}
                placeholder="generated when empty"
                aria-label="Custom source ref"
              />
              {srcRef.trim() !== "" && !SOURCE_REF_RE.test(srcRef.trim()) ? (
                <p className="text-[11px] mt-1" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
                  {sourceRefProblem(srcRef)}
                </p>
              ) : (
                <FieldHint>3–64 lowercase letters, digits or dashes; collisions are refused.</FieldHint>
              )}
            </div>
            <div>
              <Label htmlFor="src-event">Declared event type (optional)</Label>
              <Input
                id="src-event"
                value={srcEventType}
                onChange={(e) => setSrcEventType(e.target.value)}
                placeholder="empty = classic alert profile"
                aria-label="Declared event type"
              />
              {srcEventType.trim() !== "" && !EVENT_TYPE_RE.test(srcEventType.trim()) ? (
                <p className="text-[11px] mt-1" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
                  {eventTypeProblem(srcEventType)}
                </p>
              ) : (
                <FieldHint>
                  Set it and every accepted POST emits a domain event of this type.
                  Leave empty for the alert profile.
                </FieldHint>
              )}
            </div>
            <div>
              <Label htmlFor="src-board">Board</Label>
              <Select value={srcBoardId} onValueChange={(v) => setSrcBoardId(v ?? "")}>
                <SelectTrigger id="src-board" className="w-full">
                  <SelectValue placeholder="Pick a board" />
                </SelectTrigger>
                <SelectContent>
                  {boards.map((b) => (
                    <SelectItem key={b.id} value={b.id}>
                      {b.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <FieldHint>
                Alert-profile incident tickets land here; webhook envelopes carry
                it so board-scoped rules can fire on them.
              </FieldHint>
            </div>
            {srcEventType.trim() === "" ? (
              <div>
                <Label htmlFor="src-window">Correlation window (seconds)</Label>
                <Input
                  id="src-window"
                  value={srcWindow}
                  onChange={(e) => setSrcWindow(e.target.value)}
                  inputMode="numeric"
                />
                <FieldHint>Alert profile only — storms inside this window collapse into one incident.</FieldHint>
              </div>
            ) : null}
            {registerError !== null ? (
              <p className="text-[12px]" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
                {registerError}
              </p>
            ) : null}
          </div>
          <DialogActions>
            <Button variant="ghost" onClick={() => setRegisterOpen(false)} disabled={registerBusy}>
              Cancel
            </Button>
            <Button
              onClick={() => void registerSource()}
              disabled={registerBusy || srcName.trim() === "" || srcBoardId === ""}
            >
              {registerBusy ? "Registering…" : "Register"}
            </Button>
          </DialogActions>
        </DialogContent>
      </Dialog>

      {/* Edit-source dialog */}
      <Dialog
        open={editingSource !== null}
        onOpenChange={(open) => !open && setEditingSource(null)}
      >
        <DialogContent
          title="Edit source"
          description="Rename, switch profiles, or pause intake. The source ref and secret never change here."
        >
          <div className="grid gap-3 p-4">
            <div>
              <Label htmlFor="edit-src-name">Name</Label>
              <Input
                id="edit-src-name"
                value={editName}
                onChange={(e) => setEditName(e.target.value)}
              />
            </div>
            <div>
              <Label htmlFor="edit-src-event">Declared event type</Label>
              <Input
                id="edit-src-event"
                value={editEventType}
                onChange={(e) => setEditEventType(e.target.value)}
                aria-label="Edit declared event type"
              />
              <FieldHint>
                Empty reverts this source to the alert profile; a value promotes it
                to the generic receiver.
              </FieldHint>
            </div>
            {editError !== null ? (
              <p className="text-[12px]" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
                {editError}
              </p>
            ) : null}
          </div>
          <DialogActions>
            <Button variant="ghost" onClick={() => setEditingSource(null)} disabled={editBusy}>
              Cancel
            </Button>
            <Button onClick={() => void saveSourceEdit()} disabled={editBusy}>
              {editBusy ? "Saving…" : "Save"}
            </Button>
          </DialogActions>
        </DialogContent>
      </Dialog>

      {/* DMS sync connector — connector-side, operator-managed today */}
      <Card>
        <div className="p-4 grid gap-2">
          <h2 className="text-sm font-semibold">DMS sync connector</h2>
          <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
            The document-management sync connector (<code>plugins/connectors/dms-sync</code>, live
            Hudu pulls via the adapters line) runs operator-side today — it is a
            standalone crate pushing through the public feed-upsert seam, not an in-org
            configurable integration, so there is nothing to enable or disable here yet.
          </p>
          <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }} data-testid="dms-syncstate-note">
            State ledger: the connector keeps its identity ledger (<code>SyncState</code>) in
            process — a cold start re-adopts records once, which the identity keys make safe.
            A durable (file/DB-backed) ledger and a per-connector enable/disable surface are
            queued backend work, not a settings toggle here.
          </p>
          <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            Until a connector admin API exists, no connector controls render here — this note
            is the honest surface, not a placeholder UI.
          </p>
        </div>
      </Card>
    </div>
  );
}

function CustomTypesBar({
  selected,
  onToggle,
  customType,
  setCustomType,
  onAdd,
}: {
  selected: Set<string>;
  onToggle: (t: string) => void;
  customType: string;
  setCustomType: (v: string) => void;
  onAdd: () => void;
}) {
  const extras = [...selected].filter((t) => !ALL_CATALOG_TYPES.includes(t));
  return (
    <>
      {extras.length > 0 && (
        <div className="flex flex-wrap gap-x-3 pt-1">
          <span className="text-[11px] uppercase tracking-wide w-full" style={{ color: "var(--m-text-muted)" }}>
            Custom
          </span>
          {extras.map((t) => (
            <label key={t} className="inline-flex items-center gap-1.5 text-[12px] py-0.5">
              <input type="checkbox" checked onChange={() => onToggle(t)} aria-label={t} />
              <code>{t}</code>
              <Badge tone="warning">{eventTypeOriginLabel(t)}</Badge>
            </label>
          ))}
        </div>
      )}
      <div className="flex items-center gap-2 pt-1">
        <Input
          aria-label="Custom event type"
          placeholder="custom.event.type…"
          value={customType}
          onChange={(e) => setCustomType(e.target.value)}
          className="w-56"
        />
        <Button type="button" variant="secondary" onClick={onAdd} disabled={!/^[a-z0-9_.]+$/.test(customType.trim())}>
          Add type
        </Button>
      </div>
    </>
  );
}

/** Per-source ingest log — rows come from the audit trail (generic
 * receivers) or the alerts ledger (alert profile); both are paged by the
 * caller and flat-mapped into the one row shape here. */
function IngestLogPanel({
  rows,
  busy,
  error,
  noun,
  entityNoun,
  olderLabel,
  onOlder,
}: {
  rows: IngestLogRow[];
  busy: boolean;
  error: string | null;
  noun: string;
  /** What a row's entity column names: an incident ticket for the alert
   * profile, the envelope's own entity for a generic receiver. */
  entityNoun: string;
  olderLabel: string | undefined;
  onOlder: () => void;
}) {
  return (
    <div className="rounded-[var(--m-radius-container)] mt-1" style={{ border: "1px solid var(--m-border)" }}>
      {error !== null ? (
        <p className="text-[12px] p-3" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </p>
      ) : rows.length === 0 && !busy ? (
        <p className="text-[12px] p-3" style={{ color: "var(--m-text-muted)" }}>
          No {noun} yet — nothing has been accepted from this source.
        </p>
      ) : (
        rows.map((row) => (
          <div
            key={row.id}
            className="grid gap-0.5 px-3 py-2 text-[12px]"
            style={{ borderBottom: "1px solid var(--m-border)" }}
          >
            <div className="flex items-center justify-between gap-2 flex-wrap">
              <span className="tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                {new Date(row.at).toLocaleString()}
              </span>
              <Badge tone={row.tone}>{row.label}</Badge>
            </div>
            <div className="flex items-center justify-between gap-2 flex-wrap" style={{ color: "var(--m-text-muted)" }}>
              <span>
                dedup <code>{row.dedupKey ?? "—"}</code>
              </span>
              {row.entityId !== null ? (
                <span>
                  {entityNoun} <code>{row.entityId}</code>
                </span>
              ) : null}
            </div>
          </div>
        ))
      )}
      <div className="p-2">
        {busy ? (
          <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            Loading…
          </span>
        ) : olderLabel !== undefined ? (
          <Button variant="ghost" onClick={onOlder}>
            {olderLabel}
          </Button>
        ) : rows.length > 0 ? (
          <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            Beginning of log.
          </span>
        ) : null}
      </div>
    </div>
  );
}
