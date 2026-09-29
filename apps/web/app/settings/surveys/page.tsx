"use client";
/**
 * /settings/surveys — CSAT survey configuration (Wave 34 lane 2).
 *
 * Purpose: send-on-resolve toggle, per-client minimum interval, response
 * link validity, and the quiet-hours window surveys defer to — all riding
 * the typed org settings registry. Honest states (ADR 0010): when the mail
 * transport is `none` the page says plainly that no surveys will be sent;
 * it never implies a queue that does not exist.
 *
 * Non-purpose: response data and rollups (Scorecards page owns those) and
 * the transport selection itself (Email page owns that).
 */
import { useCallback, useEffect, useState } from "react";
import { Button, Card, FieldHint, Input, Label } from "@marula-core/ui";
import { api, useRequireAuth } from "../../../lib/api";
import { SettingsHeader } from "../../../components/settings/settings-shell";

const KEYS = {
  send: "csat.send_on_resolve",
  interval: "csat.min_interval_hours",
  ttl: "csat.token_ttl_days",
  quietFrom: "csat.quiet_from",
  quietUntil: "csat.quiet_until",
} as const;

type OrgValue = { key: string; value: unknown };

function textOf(values: OrgValue[], key: string, fallback: string): string {
  const row = values.find((v) => v.key === key);
  return typeof row?.value === "string" && row.value !== ""
    ? row.value
    : fallback;
}

function boolOf(values: OrgValue[], key: string, fallback: boolean): boolean {
  const row = values.find((v) => v.key === key);
  return typeof row?.value === "boolean" ? row.value : fallback;
}

export default function SurveysSettingsPage() {
  const auth = useRequireAuth();
  const authed = auth.status === "authed";
  const isAdmin = (auth.me?.permissions ?? []).includes("special:manage_plugins") ||
    (auth.me?.permissions ?? []).includes("settings:manage");

  const [send, setSend] = useState(false);
  const [interval, setInterval] = useState("168");
  const [ttl, setTtl] = useState("14");
  const [quietFrom, setQuietFrom] = useState("");
  const [quietUntil, setQuietUntil] = useState("");
  const [transport, setTransport] = useState("none");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);

  const load = useCallback(async () => {
    if (!authed) return;
    try {
      const [orgRes, transportRes] = await Promise.all([
        api.api.GET("/api/v1/settings/org"),
        api.api.GET("/api/v1/settings/registry"),
      ]);
      const values = (orgRes.data ?? []) as OrgValue[];
      setSend(boolOf(values, KEYS.send, false));
      setInterval(textOf(values, KEYS.interval, "168"));
      setTtl(textOf(values, KEYS.ttl, "14"));
      setQuietFrom(textOf(values, KEYS.quietFrom, ""));
      setQuietUntil(textOf(values, KEYS.quietUntil, ""));
      const transportRow = (
        transportRes.data as unknown as
          | Array<{ key: string; default?: unknown }>
          | undefined
      )?.find((d) => d.key === "email.transport");
      setTransport(
        (values.find((v) => v.key === "email.transport")?.value as string) ??
          (typeof transportRow?.default === "string" ? transportRow.default : "none"),
      );
      setDirty(false);
    } catch {
      // Values stay on defaults; wire errors surface on the next write.
    }
  }, [authed]);

  useEffect(() => {
    void load();
  }, [load]);

  async function save() {
    setBusy(true);
    setNotice(null);
    const updates: Array<[string, unknown]> = [
      [KEYS.send, send],
      [KEYS.interval, Number(interval)],
      [KEYS.ttl, Number(ttl)],
      [KEYS.quietFrom, quietFrom],
      [KEYS.quietUntil, quietUntil],
    ];
    try {
      for (const [key, value] of updates) {
        const res = await api.api.PUT("/api/v1/settings/org", {
          body: { key, value },
        });
        if (res.error) {
          const detail =
            (res.error as { error?: { message?: string } }).error?.message ??
            "rejected";
          setNotice(detail);
          return;
        }
      }
      setNotice("Saved");
      setDirty(false);
    } catch {
      setNotice("Save failed — try again");
    } finally {
      setBusy(false);
    }
  }

  const noTransport = transport === "none";
  const quietSet = quietFrom !== "" && quietUntil !== "";

  return (
    <div className="grid gap-6 max-w-2xl">
      <SettingsHeader
        title="Surveys"
        description="Send a satisfaction survey by email whenever a ticket resolves. Responses land on the scorecards for the client and the tech."
      />

      <Card className="p-5 grid gap-4">
        <div className="grid gap-1">
          <h2 className="text-sm font-semibold">Send on resolve</h2>
          <FieldHint>
            Queues one survey per resolved ticket. Links are single-use and
            expire; a client receives at most one survey per interval below.
          </FieldHint>
        </div>
        <label className="flex items-center gap-2 text-[13px]">
          <input
            type="checkbox"
            checked={send}
            onChange={(e) => {
              setSend(e.target.checked);
              setDirty(true);
            }}
            disabled={!isAdmin}
            data-testid="csat-send-on-resolve"
          />
          Send a satisfaction survey when a ticket resolves
        </label>

        {send && noTransport && (
          <div
            role="status"
            data-testid="csat-not-configured"
            className="rounded-[var(--m-radius-control)] border p-3 text-[13px]"
            style={{ borderColor: "var(--m-border)", color: "var(--m-text-muted)" }}
          >
            Mail transport is set to <strong>none</strong> (Email settings), so
            no surveys are sent and scorecards stay empty. Configure a
            transport to turn this on — nothing is queued in the meantime.
          </div>
        )}

        <div className="grid gap-4 sm:grid-cols-2">
          <div className="grid gap-1">
            <Label htmlFor="csat-interval">Minimum interval per client (hours)</Label>
            <Input
              id="csat-interval"
              type="number"
              min={0}
              max={8760}
              value={interval}
              onChange={(e) => {
                setInterval(e.target.value);
                setDirty(true);
              }}
              disabled={!isAdmin || !send}
              data-testid="csat-min-interval"
            />
            <FieldHint>168 = at most one survey per client per week.</FieldHint>
          </div>
          <div className="grid gap-1">
            <Label htmlFor="csat-ttl">Response link validity (days)</Label>
            <Input
              id="csat-ttl"
              type="number"
              min={1}
              max={90}
              value={ttl}
              onChange={(e) => {
                setTtl(e.target.value);
                setDirty(true);
              }}
              disabled={!isAdmin || !send}
              data-testid="csat-token-ttl"
            />
            <FieldHint>Links work once, within this window.</FieldHint>
          </div>
        </div>

        <div className="grid gap-4 sm:grid-cols-2">
          <div className="grid gap-1">
            <Label htmlFor="csat-quiet-from">Quiet hours start</Label>
            <Input
              id="csat-quiet-from"
              type="time"
              value={quietFrom}
              onChange={(e) => {
                setQuietFrom(e.target.value);
                setDirty(true);
              }}
              disabled={!isAdmin || !send}
              data-testid="csat-quiet-from"
            />
          </div>
          <div className="grid gap-1">
            <Label htmlFor="csat-quiet-until">Quiet hours end</Label>
            <Input
              id="csat-quiet-until"
              type="time"
              value={quietUntil}
              onChange={(e) => {
                setQuietUntil(e.target.value);
                setDirty(true);
              }}
              disabled={!isAdmin || !send}
              data-testid="csat-quiet-until"
            />
          </div>
        </div>
        <FieldHint>
          {quietSet
            ? `Surveys arriving inside the window wait until ${quietUntil} (org timezone), then send.`
            : "No quiet window set — surveys send as soon as they are queued."}
        </FieldHint>

        {notice && (
          <p role="status" className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
            {notice}
          </p>
        )}
        {isAdmin && (
          <div className="flex items-center gap-3">
            <Button onClick={save} disabled={busy || !dirty} data-testid="csat-save">
              {busy ? "Saving…" : "Save changes"}
            </Button>
          </div>
        )}
      </Card>
    </div>
  );
}
