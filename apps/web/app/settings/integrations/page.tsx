// /settings/integrations — webhook manager + API-key registry entry point.
//
// Purpose: signed OUTBOUND webhook subscriptions over the existing
// /api/webhooks endpoints: CRUD, event-type multiselect from the real
// event catalog, enabled toggle, show-once secrets at creation AND on the
// explicit rotate flow (create replacement → delete original), a
// test-fire button whose envelope id can be chased in the per-webhook
// keyset-paged DELIVERY LOG (status/time/attempt/error rows).
//
// Non-purpose: API-keys management lives on its own page
// (/settings/api-keys); no delivery retry logic here (worker-owned), and
// no connector installation consent flows.
"use client";

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
  Tag,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useCallback, useEffect, useReducer, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import Link from "next/link";
import {
  EVENT_CATALOG,
  ALL_CATALOG_TYPES,
  classifyDelivery,
  eventSelectionReducer,
  selectionToPayload,
} from "../../../lib/admin/webhook-events";

type WebhookView = components["schemas"]["WebhookView"];
type DeliveryAttemptView = components["schemas"]["DeliveryAttemptView"];
type WebhookSecretResponse = components["schemas"]["WebhookSecretResponse"];

const PAGE_SIZE = 20;

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

  const refresh = useCallback(async () => {
    const { data, error } = await api.api.GET("/api/webhooks");
    if (error !== undefined || data === undefined) {
      setActionError("Failed to load webhooks.");
      return;
    }
    setWebhooks(data.webhooks);
  }, []);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refresh();
  }, [auth.status, refresh]);

  if (auth.status !== "authed") return null;

  const create = async () => {
    setBusy(true);
    setFormError(null);
    try {
      const res = await api.api.POST("/api/webhooks", {
        body: { url, event_types: selectionToPayload(selection.selected) },
      });
      if (!res.data) throw new Error("webhook creation was rejected");
      // Show-once contract: this render is the ONLY time the secret exists.
      const payload = res.data as WebhookSecretResponse;
      setShownSecret({ label: payload.webhook.url, secret: payload.secret });
      setUrl("");
      dispatchSelection({ type: "clear" });
      await refresh();
    } catch (e) {
      setFormError(e instanceof Error ? e.message : String(e));
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
        const res = await api.api.PATCH("/api/webhooks/{id}", {
          params: { path: { id } },
          body: { enabled: !current.enabled },
        });
        if (res.error) throw new Error("update failed");
      } else if (action === "delete") {
        const res = await api.api.DELETE("/api/webhooks/{id}", {
          params: { path: { id } },
        });
        if (res.error) throw new Error("delete failed");
      } else {
        const res = await api.api.POST("/api/webhooks/{id}/test", {
          params: { path: { id } },
        });
        if (res.error) throw new Error("test fire failed");
        setFiredEventIds((prev) => new Map(prev).set(id, res.data.event_id));
        // Give the pipeline a beat, then pull the fresh first page.
        setTimeout(() => void reloadLog(id), 1500);
      }
      await refresh();
    } catch (e) {
      setActionError(e instanceof Error ? e.message : String(e));
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
      const res = await api.api.POST("/api/webhooks", {
        body: { url: target.url, event_types: [...target.event_types] },
      });
      if (!res.data) throw new Error("replacement creation failed");
      const payload = res.data as WebhookSecretResponse;
      await api.api.DELETE("/api/webhooks/{id}", { params: { path: { id: target.id } } });
      setShownSecret({ label: `${payload.webhook.url} (rotated)`, secret: payload.secret });
      setRotatingId(null);
      await refresh();
    } catch (e) {
      setActionError(e instanceof Error ? e.message : String(e));
    } finally {
      setRotateBusy(false);
    }
  };

  const reloadLog = async (id: string, cursor?: string) => {
    setLogLoading(id);
    try {
      const res = await api.api.GET("/api/webhooks/{id}/deliveries", {
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
      setActionError(e instanceof Error ? e.message : String(e));
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

  return (
    <div className="grid gap-6 max-w-2xl">
      <header>
        <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
          Integrations
        </h1>
        <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Signed webhooks push every change you subscribe to. Deliveries are HMAC-signed and retried
          with backoff — every attempt lands in the log. Machine API keys live under{" "}
          <Link
            href="/settings/api-keys"
            style={{ color: "var(--m-accent)", textDecoration: "underline" }}
          >
            API keys
          </Link>
          .
        </p>
      </header>

      {shownSecret ? (
        <Card>
          <div className="p-4 grid gap-2">
            <Label>Signing secret for {shownSecret.label}</Label>
            <code
              className="text-[12px] break-all rounded-[2px] px-2 py-1.5"
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
              placeholder="https://hooks.example.test/meridian"
              required
            />
          </div>
          <fieldset className="grid gap-1">
            <legend className="text-[12px]" style={{ color: "var(--m-text)" }}>
              Event types
            </legend>
            <FieldHint>Nothing selected subscribes to all events.</FieldHint>
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
          {formError ? (
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
                    className="text-[10px] uppercase tracking-wide px-1.5 py-0.5 rounded-[2px]"
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
                <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                  {wh.event_types.length === 0 ? "All events" : wh.event_types.join(", ")}
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
                  <div className="rounded-[2px] mt-1" style={{ border: "1px solid var(--m-border)" }}>
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
                              <Tag tone={verdict.tone}>{verdict.label}</Tag>
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
          description="Meridian creates a replacement subscription with the same URL and events, shows you its new secret once, then deletes this webhook."
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
