// /settings/integrations — webhook management (plugin runtime v1).
//
// Purpose: create signed OUTBOUND webhook subscriptions (secret shown
// exactly once at creation), toggle/delete them, fire test events, and
// inspect the per-webhook delivery log (every attempt, newest first).
//
// Non-purpose: API-keys management UI and connector installation consent
// flows belong to the admin-configuration wave; no delivery retry logic
// lives here (the worker owns attempts).

"use client";

import { Button, Card, FieldHint, Input, Label } from "@meridian/ui";
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";

interface Webhook {
  id: string;
  url: string;
  event_types: string[];
  enabled: boolean;
  created_at: string;
}

interface DeliveryAttempt {
  id: string;
  event_id: string;
  event_type: string;
  attempt: number;
  response_status: number | null;
  next_retry_at: string | null;
  delivered_at: string | null;
  last_error: string | null;
  created_at: string;
}

export default function IntegrationsSettingsPage() {
  const auth = useRequireAuth();
  const [webhooks, setWebhooks] = useState<Webhook[]>([]);
  const [url, setUrl] = useState("");
  const [eventTypes, setEventTypes] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** Secret from the most recent create — rendered once, never refetched. */
  const [shownSecret, setShownSecret] = useState<{
    url: string;
    secret: string;
  } | null>(null);
  const [openDeliveries, setOpenDeliveries] = useState<
    Map<string, DeliveryAttempt[]>
  >(new Map());

  const refresh = useCallback(async () => {
    const { data } = await api.api.GET("/api/webhooks");
    setWebhooks((data as unknown as { webhooks: Webhook[] })?.webhooks ?? []);
  }, []);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refresh();
  }, [auth.status, refresh]);

  if (auth.status !== "authed") return null;

  const create = async () => {
    setBusy(true);
    setError(null);
    try {
      const types = eventTypes
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean);
      const res = await api.api.POST("/api/webhooks", {
        body: { url, event_types: types },
      });
      if (!res.data) {
        // Only 201 is declared, so success is proven by `data` alone.
        throw new Error("webhook creation was rejected");
      }
      // Show-once contract: this render is the ONLY time the secret exists.
      setShownSecret({
        url: res.data.webhook.url,
        secret: (res.data as unknown as { secret: string }).secret,
      });
      setUrl("");
      setEventTypes("");
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const mutate = async (
    id: string,
    action: "toggle" | "delete" | "test",
    current?: Webhook,
  ) => {
    setError(null);
    if (action === "toggle" && current) {
      const res = await api.api.PATCH("/api/webhooks/{id}", {
        params: { path: { id } },
        body: { enabled: !current.enabled },
      });
      if (res.error) setError("update failed");
    } else if (action === "delete") {
      const res = await api.api.DELETE("/api/webhooks/{id}", {
        params: { path: { id } },
      });
      if (res.error) setError("delete failed");
    } else {
      const res = await api.api.POST("/api/webhooks/{id}/test", {
        params: { path: { id } },
      });
      if (res.error) setError("test fire failed");
    }
    await refresh();
  };

  const toggleDeliveries = async (id: string) => {
    const next = new Map(openDeliveries);
    if (next.has(id)) {
      next.delete(id);
      setOpenDeliveries(next);
      return;
    }
    const res = await api.api.GET("/api/webhooks/{id}/deliveries", {
      params: { path: { id }, query: { limit: 20 } },
    });
    const page = res.data as unknown as
      | { deliveries: DeliveryAttempt[] }
      | undefined;
    next.set(id, page?.deliveries ?? []);
    setOpenDeliveries(next);
  };

  return (
    <div className="grid gap-6 max-w-2xl">
      <header>
        <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
          Integrations
        </h1>
        <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Signed webhooks push every change you subscribe to. Deliveries are
          HMAC-signed and retried with backoff — every attempt lands in the log.
        </p>
      </header>

      {shownSecret ? (
        <Card>
          <div className="p-4 grid gap-2">
            <Label>Signing secret for {shownSecret.url}</Label>
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
            <FieldHint>
              Copy it now — it is never shown again. Lost secrets are rotated
              by recreating the webhook.
            </FieldHint>
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
          <div>
            <Label htmlFor="wh-types">Event types</Label>
            <Input
              id="wh-types"
              value={eventTypes}
              onChange={(e) => setEventTypes(e.target.value)}
              placeholder="empty = all events, or ticket.created, ticket.closed"
            />
            <FieldHint>Comma-separated; leave empty for everything.</FieldHint>
          </div>
          {error ? (
            <p className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {error}
            </p>
          ) : null}
          <div>
            <Button type="submit" disabled={busy || !url}>
              Create webhook
            </Button>
          </div>
        </form>
      </Card>

      <div className="grid gap-3">
        {webhooks.map((wh) => (
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
                {wh.event_types.length === 0
                  ? "All events"
                  : wh.event_types.join(", ")}
              </p>
              <div className="flex flex-wrap gap-2">
                <Button
                  variant="ghost"
                  onClick={() => void mutate(wh.id, "toggle", wh)}
                >
                  {wh.enabled ? "Disable" : "Enable"}
                </Button>
                <Button variant="ghost" onClick={() => void mutate(wh.id, "test")}>
                  Send test
                </Button>
                <Button
                  variant="ghost"
                  onClick={() => void toggleDeliveries(wh.id)}
                >
                  {openDeliveries.has(wh.id) ? "Hide log" : "Delivery log"}
                </Button>
                <Button
                  variant="ghost"
                  onClick={() => void mutate(wh.id, "delete")}
                >
                  Delete
                </Button>
              </div>
              {openDeliveries.has(wh.id) ? (
                <div
                  className="rounded-[2px] mt-1"
                  style={{ border: "1px solid var(--m-border)" }}
                >
                  {(openDeliveries.get(wh.id) ?? []).length === 0 ? (
                    <p
                      className="text-[12px] p-3"
                      style={{ color: "var(--m-text-muted)" }}
                    >
                      No attempts yet — send a test event.
                    </p>
                  ) : (
                    (openDeliveries.get(wh.id) ?? []).map((d) => (
                      <div
                        key={d.id}
                        className="flex items-center justify-between text-[12px] px-3 py-2"
                        style={{ borderBottom: "1px solid var(--m-border)" }}
                      >
                        <span style={{ color: "var(--m-text-secondary)" }}>
                          #{d.attempt} · {d.event_type}
                        </span>
                        <span
                          style={{
                            color:
                              d.delivered_at != null
                                ? "var(--m-status-success-fg)"
                                : "var(--m-status-warning-fg)",
                          }}
                        >
                          {d.delivered_at != null
                            ? `delivered · ${d.response_status}`
                            : (d.last_error ?? "pending retry")}
                        </span>
                      </div>
                    ))
                  )}
                </div>
              ) : null}
            </div>
          </Card>
        ))}
      </div>
    </div>
  );
}
