// /settings/api-keys — machine credential management (TASKLIST 2.6).
//
// Purpose: create scoped API keys over the existing /api/v1/api-keys
// endpoints (name + capability-scope picker from the server's known
// scope list), surface the raw bearer material exactly once at creation,
// show the non-secret prefix, created/last-used/revoked columns, and
// revoke with confirmation. The documented rotation story (create a
// replacement → revoke the old key) is explained inline.
//
// Non-purpose: no key editing (revocation is immediate and final by
// design), no per-key usage analytics beyond last_used_at.
"use client";

import { friendlyMutationText } from "../../../lib/error-copy";
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
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useEffect, useMemo, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import { SaveBar, SettingsHeader } from "../../../components/settings/settings-shell";

type ApiKeyView = components["schemas"]["ApiKeyView"];

/** Server-known scopes; grouping is presentation-only. */
const SCOPE_GROUPS: { group: string; scopes: { id: string; hint: string }[] }[] = [
  {
    group: "Tickets",
    scopes: [
      { id: "tickets:read", hint: "list & read tickets" },
      { id: "tickets:write", hint: "create & update tickets" },
    ],
  },
  {
    group: "Clients",
    scopes: [
      { id: "clients:read", hint: "list & read clients" },
      { id: "clients:write", hint: "create & update clients" },
    ],
  },
  {
    group: "Timers",
    scopes: [
      { id: "timers:read", hint: "view running timers & entries" },
      { id: "timers:write", hint: "start/stop timers, add entries" },
    ],
  },
];

export default function ApiKeysSettingsPage() {
  const auth = useRequireAuth();
  const [keys, setKeys] = useState<ApiKeyView[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [name, setName] = useState("");
  const [scopes, setScopes] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  /** Raw material from the most recent create — shown once. */
  const [shownRawKey, setShownRawKey] = useState<{ name: string; raw: string } | null>(null);
  const [confirmingRevokeId, setConfirmingRevokeId] = useState<string | null>(null);
  const [revealedIds, setRevealedIds] = useState<Set<string>>(new Set());

  const refresh = async () => {
    const res = await api.api.GET("/api/v1/api-keys");
    if (res.error !== undefined || res.data === undefined) {
      setLoadError("Failed to load API keys.");
      setLoading(false);
      return;
    }
    // Most recent first for a management view.
    setKeys([...res.data.keys].sort((a, b) => b.created_at.localeCompare(a.created_at)));
    setLoading(false);
  };

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refresh();
  }, [auth.status]);

  const canSubmit = useMemo(
    () => name.trim().length > 0 && scopes.size > 0 && !busy,
    [name, scopes, busy],
  );

  if (auth.status !== "authed") return null;

  const create = async () => {
    setBusy(true);
    setFormError(null);
    try {
      const res = await api.api.POST("/api/v1/api-keys", {
        body: { name: name.trim(), scopes: [...scopes].sort() },
      });
      if (!res.data) throw new Error("key creation was rejected");
      setShownRawKey({ name: res.data.key.name, raw: res.data.raw_key });
      setName("");
      setScopes(new Set());
      await refresh();
    } catch (e) {
      setFormError(friendlyMutationText(e));
    } finally {
      setBusy(false);
    }
  };

  const revoke = async () => {
    const id = confirmingRevokeId;
    if (id === null) return;
    try {
      const res = await api.api.DELETE("/api/v1/api-keys/{id}", { params: { path: { id } } });
      if (res.error) throw new Error("revocation failed");
      setConfirmingRevokeId(null);
      await refresh();
    } catch (e) {
      setActionError(friendlyMutationText(e));
    }
  };

  return (
    <div className="grid gap-6 max-w-3xl">
      <SettingsHeader
        title="API keys"
        description="Bearer credentials for machine integrations. Secrets are shown once at creation; to rotate, create a replacement key then revoke this one."
      />

      <Card>
        <form
          className="p-4 grid gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            void create();
          }}
        >
          <div className="max-w-sm">
            <Label htmlFor="ak-name">Key name</Label>
            <Input
              id="ak-name"
              value={name}
              maxLength={64}
              placeholder="e.g. NinjaOne sync"
              onChange={(e) => setName(e.target.value)}
              required
            />
          </div>
          <fieldset className="grid gap-2">
            <legend className="text-[12px]" style={{ color: "var(--m-text)" }}>
              Scopes
            </legend>
            <FieldHint>Pick the narrowest set the integration needs.</FieldHint>
            {SCOPE_GROUPS.map((group) => (
              <div key={group.group} className="flex flex-wrap gap-x-4">
                <span className="text-[11px] uppercase tracking-wide w-full" style={{ color: "var(--m-text-muted)" }}>
                  {group.group}
                </span>
                {group.scopes.map((scope) => (
                  <label key={scope.id} className="inline-flex items-center gap-1.5 text-[12px] py-0.5">
                    <input
                      type="checkbox"
                      checked={scopes.has(scope.id)}
                      onChange={() =>
                        setScopes((prev) => {
                          const next = new Set(prev);
                          if (next.has(scope.id)) next.delete(scope.id);
                          else next.add(scope.id);
                          return next;
                        })
                      }
                      aria-label={scope.id}
                    />
                    <code>{scope.id}</code>
                    <span style={{ color: "var(--m-text-muted)" }}>{scope.hint}</span>
                  </label>
                ))}
              </div>
            ))}
          </fieldset>
          <div>
            <SaveBar
              dirty={canSubmit}
              saving={busy}
              error={formError}
              onSave={() => void create()}
              saveTestId="create-key"
              saveLabel="Create key"
            />
            {scopes.size === 0 && name.trim().length > 0 && (
              <FieldHint>Select at least one scope.</FieldHint>
            )}
          </div>
        </form>
      </Card>

      {shownRawKey !== null ? (
        <Card>
          <div className="p-4 grid gap-2">
            <Label>Raw key for “{shownRawKey.name}”</Label>
            <code
              className="text-[12px] break-all rounded-[var(--m-radius-container)] px-2 py-1.5"
              style={{
                background: "var(--m-surface)",
                color: "var(--m-text)",
                border: "1px solid var(--m-border)",
              }}
            >
              {shownRawKey.raw}
            </code>
            <FieldHint>Copy it now — it cannot be retrieved again.</FieldHint>
            <div>
              <Button variant="ghost" onClick={() => setShownRawKey(null)}>
                Dismiss
              </Button>
            </div>
          </div>
        </Card>
      ) : null}

      {loadError !== null && (
        <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {loadError}
        </p>
      )}

      <Card className="overflow-x-auto">
        {loading ? (
          <p className="text-[13px] p-4" style={{ color: "var(--m-text-muted)" }}>
            Loading…
          </p>
        ) : keys.length === 0 ? (
          <div className="p-4">
            <EmptyState message="No API keys yet — create one above." />
          </div>
        ) : (
          <table className="text-[12px]" style={{ borderCollapse: "collapse", width: "100%" }}>
            <thead>
              <tr style={{ borderBottom: "1px solid var(--m-border)" }}>
                <th scope="col" className="text-left p-2 font-medium" style={{ color: "var(--m-text-secondary)" }}>
                  Name
                </th>
                <th scope="col" className="text-left p-2 font-medium" style={{ color: "var(--m-text-secondary)" }}>
                  Prefix
                </th>
                <th scope="col" className="text-left p-2 font-medium" style={{ color: "var(--m-text-secondary)" }}>
                  Scopes
                </th>
                <th scope="col" className="text-left p-2 font-medium tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                  Created
                </th>
                <th scope="col" className="text-left p-2 font-medium tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                  Last used
                </th>
                <th scope="col" className="text-right p-2 font-medium" style={{ color: "var(--m-text-secondary)" }}>
                  Status
                </th>
                <th scope="col" aria-label="Actions"></th>
              </tr>
            </thead>
            <tbody>
              {keys.map((k) => (
                <tr key={k.id} style={{ borderBottom: "1px solid var(--m-border)" }}>
                  <td className="p-2" style={{ color: "var(--m-text)" }}>
                    {k.name}
                  </td>
                  <td className="p-2">
                    <button
                      type="button"
                      title="Toggle full prefix visibility"
                      onClick={() =>
                        setRevealedIds((prev) => {
                          const next = new Set(prev);
                          if (next.has(k.id)) next.delete(k.id);
                          else next.add(k.id);
                          return next;
                        })
                      }
                      style={{ background: "none", border: "none", cursor: "pointer", padding: 0 }}
                    >
                      <code style={{ color: "var(--m-text-secondary)" }}>
                        {revealedIds.has(k.id) ? k.prefix : `${k.prefix.slice(0, 7)}…`}
                      </code>
                    </button>
                  </td>
                  <td className="p-2">
                    <span className="flex flex-wrap gap-1">
                      {k.scopes.map((s) => (
                        <Tag key={s} tone="outline">
                          {s}
                        </Tag>
                      ))}
                    </span>
                  </td>
                  <td className="p-2 tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                    {new Date(k.created_at).toLocaleDateString()}
                  </td>
                  <td className="p-2 tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                    {k.last_used_at != null ? new Date(k.last_used_at).toLocaleString() : "never"}
                  </td>
                  <td className="p-2 text-right">
                    {k.revoked_at != null ? (
                      <Tag tone="danger">revoked</Tag>
                    ) : (
                      <Tag tone="success">active</Tag>
                    )}
                  </td>
                  <td className="p-2 text-right">
                    {k.revoked_at == null && (
                      <Button variant="ghost" onClick={() => setConfirmingRevokeId(k.id)}>
                        Revoke
                      </Button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>

      {actionError !== null && (
        <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {actionError}
        </p>
      )}

      <Dialog open={confirmingRevokeId !== null} onOpenChange={(open) => !open && setConfirmingRevokeId(null)}>
        <DialogContent
          title="Revoke this key?"
          description="Revocation is immediate — every request using this key starts failing at once."
        >
          <DialogActions>
            <Button variant="ghost" disabled={false} onClick={() => setConfirmingRevokeId(null)}>
              Cancel
            </Button>
            <Button onClick={() => void revoke()}>Revoke now</Button>
          </DialogActions>
        </DialogContent>
      </Dialog>
    </div>
  );
}
