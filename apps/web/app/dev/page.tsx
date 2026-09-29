// /dev — the dev ops page (TASKLIST DX3).
//
// Purpose: one ugly-but-clear QA surface for the env-gated dev endpoints:
// seed / reset-org / time-travel buttons, an outbox event-stream viewer,
// and build info (git SHA + migration head) from GET /api/dev/status.
//
// Gating contract: the page itself is reachable only through the header
// link, which renders ONLY when the status probe answers enabled:true —
// and that answer only exists when the API runs with MARULA_DEV_TOOLS=1.
// This page still probes independently (defense in depth): with the gate
// off it renders a disabled notice and no action buttons.
//
// Non-purpose: no design investment (Workbench tokens for form controls,
// raw <pre> output), no production operations (that is the settings hub),
// no auth of its own — the env gate on the API is the whole story.

"use client";

import { Button, Card, Input, Label, Spinner } from "@marula-core/ui";
import { useCallback, useEffect, useState } from "react";
import { apiBaseUrl, useRequireAuth } from "../../lib/api";
import { reasonCopy } from "../../lib/error-copy";

interface DevStatus {
  enabled?: boolean;
  git_sha?: string;
  migration_head?: number | null;
  demo_seeded?: boolean;
  demo_org_id?: string;
  demo_slug?: string;
}

function pretty(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

export default function DevPage() {
  const auth = useRequireAuth();
  const [probe, setProbe] = useState<"loading" | "disabled" | "enabled">("loading");
  const [status, setStatus] = useState<DevStatus | null>(null);

  const [busy, setBusy] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  /** The typed IF3 reason of the last failed call (e.g. already_seeded). */
  const [failReason, setFailReason] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);

  const [hours, setHours] = useState("24");

  const loadStatus = useCallback(async () => {
    const response = await fetch(`${apiBaseUrl()}/api/dev/status`).catch(() => null);
    if (response === null || !response.ok) {
      setProbe("disabled");
      return;
    }
    const body = (await response.json()) as DevStatus;
    if (body.enabled === true) {
      setStatus(body);
      setProbe("enabled");
    } else {
      setProbe("disabled");
    }
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void loadStatus();
  }, [auth.status, loadStatus]);

  /** One POST/GET helper: catches, renders the typed reason as friendly
   * copy (never the raw body as the failure line — the pretty JSON stays
   * in the result panel), pretty-prints the result. */
  const call = useCallback(
    async (label: string, path: string, init?: RequestInit) => {
      setBusy(label);
      setNotice(null);
      setFailReason(null);
      setResult(null);
      try {
        const response = await fetch(`${apiBaseUrl()}${path}`, init);
        const body = await response.json().catch(() => ({ note: "no body" }));
        if (!response.ok) {
          const reason =
            typeof (body as { error?: { reason?: unknown } })?.error?.reason === "string"
              ? (body as { error: { reason: string } }).error.reason
              : null;
          setFailReason(reason);
          setNotice(
            reason !== null
              ? `${label}: ${reasonCopy(reason, (body as { error?: Record<string, unknown> }).error)}`
              : `${label} failed (HTTP ${response.status})`,
          );
        }
        setResult(JSON.stringify(body, null, 2));
        if (path.startsWith("/api/dev/seed") || path === "/api/dev/reset-org") {
          // The status line may have changed (seeded flag).
          void loadStatus();
        }
      } catch (error) {
        setFailReason(null);
        setNotice(`${label} failed: could not reach the API — check that core-api is running.`);
        void error;
      } finally {
        setBusy(null);
      }
    },
    [loadStatus],
  );

  if (auth.status !== "authed") return null;

  if (probe === "loading") {
    return (
      <div className="grid gap-6 p-6">
        <h1 className="text-lg font-semibold">Dev ops</h1>
        <Spinner />
      </div>
    );
  }

  if (probe === "disabled") {
    return (
      <div className="grid gap-6 p-6">
        <h1 className="text-lg font-semibold">Dev ops</h1>
        <Card>
          <div className="p-4 text-[13px]">
            <p role="alert">
              Dev tools are not enabled on this API. The /api/dev surface only
              exists when the server runs with MARULA_DEV_TOOLS=1 — it is
              never on by default.
            </p>
          </div>
        </Card>
      </div>
    );
  }

  return (
    <div className="grid gap-6 p-6">
      <div>
        <h1 className="text-lg font-semibold">Dev ops</h1>
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          QA loop controls over the demo org. Dev-only surface; writes audit
          as the system actor.
        </p>
      </div>

      {notice !== null && (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-danger, #b00)" }}>
          {notice}
        </p>
      )}

      {failReason === "already_seeded" && (
        <p className="flex flex-wrap items-center gap-2 text-[13px]" role="status">
          <span style={{ color: "var(--m-text-secondary)" }}>
            The demo org already has data. Force-reseed wipes it and starts over.
          </span>
          <Button
            variant="secondary"
            disabled={busy !== null}
            onClick={() => void call("Force reseed", "/api/dev/seed?force=1", { method: "POST" })}
          >
            Wipe and reseed (force)
          </Button>
        </p>
      )}

      <Card>
        <div className="grid gap-2 p-4">
          <h2 className="text-sm font-semibold">Build info</h2>
          <pre className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
            {status === null ? "" : JSON.stringify(status, null, 2)}
          </pre>
        </div>
      </Card>

      <Card>
        <div className="flex flex-wrap items-end gap-3 p-4">
          <div>
            <Label>Demo org</Label>
            <div className="flex items-center gap-2 pt-1">
              <Button
                aria-label="Seed demo org"
                disabled={busy !== null}
                onClick={() => void call("Seed", "/api/dev/seed", { method: "POST" })}
              >
                Seed demo org
              </Button>
              <Button
                variant="secondary"
                disabled={busy !== null}
                onClick={() => {
                  if (!window.confirm("Wipe the demo org and reseed from scratch?")) return;
                  void call("Reset org", "/api/dev/reset-org", { method: "POST" });
                }}
              >
                Reset org (wipe + reseed)
              </Button>
            </div>
          </div>
          <div>
            <Label htmlFor="dev-hours">Shift SLA clocks back (hours)</Label>
            <div className="flex items-center gap-2 pt-1">
              <Input
                id="dev-hours"
                className="w-24"
                value={hours}
                inputMode="numeric"
                onChange={(event) => setHours(event.target.value)}
              />
              <Button
                variant="secondary"
                disabled={busy !== null}
                onClick={() => {
                  const parsed = Number.parseInt(hours, 10);
                  if (!Number.isFinite(parsed)) {
                    setNotice("Hours must be an integer between 1 and 720.");
                    return;
                  }
                  void call("Time travel", "/api/dev/time-travel", {
                    method: "POST",
                    headers: { "content-type": "application/json" },
                    body: JSON.stringify({ hours: parsed }),
                  });
                }}
              >
                Time-travel
              </Button>
            </div>
          </div>
        </div>
      </Card>

      <Card>
        <div className="grid gap-2 p-4">
          <h2 className="text-sm font-semibold">Local services</h2>
          <div className="flex flex-wrap gap-4 text-[13px]">
            <a href="http://localhost:8025" target="_blank" rel="noreferrer">Mailpit</a>
            <a href="http://localhost:9001" target="_blank" rel="noreferrer">MinIO console</a>
            <a href="http://localhost:3001" target="_blank" rel="noreferrer">Grafana</a>
          </div>
          <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
            Links assume the stage2 dev extras (DEV_STAGE2=1); a service that
            is down just fails to load in its tab.
          </p>
        </div>
      </Card>

      <Card>
        <div className="grid gap-2 p-4">
          <h2 className="text-sm font-semibold">Event stream (last 100 outbox envelopes)</h2>
          <div>
            <Button
              variant="secondary"
              disabled={busy !== null}
              onClick={() => void call("Events", "/api/dev/events?limit=100")}
            >
              Load events
            </Button>
          </div>
        </div>
      </Card>

      {busy !== null && <Spinner />}
      {result !== null && (
        <Card>
          <div className="p-4">
            <h2 className="text-sm font-semibold">Last result</h2>
            <pre className="overflow-x-auto text-xs">{result}</pre>
          </div>
        </Card>
      )}
    </div>
  );
}
