"use client";

// CatalogCard — admin-curated model catalog (ADR 0008): every enabled
// entry is an admin-allowlist (provider, model) pair for the completion
// + decision edges; capabilities carry the edge tags
// (completion / tools / decision). Entries are sealed-credential-preferent
// (a pair with no org credential answers its typed "no credential"
// refusals on the edge instead — the UI shows the honest pairing state).

import { useEffect, useState } from "react";
import {
  Button,
  Card,
  Checkbox,
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
import { api } from "../../lib/api";
import { resultErrorText } from "../../lib/error-copy";

type AiCatalogDto = components["schemas"]["AiCatalogDto"];

const EDGE_TAGS = ["completion", "tools", "decision"] as const;

export function CatalogCard({ onChanged }: { onChanged: () => void }) {
  const [catalog, setCatalog] = useState<AiCatalogDto[] | null>(null);
  const [provider, setProvider] = useState<string>("openai");
  const [model, setModel] = useState<string>("");
  const [label, setLabel] = useState<string>("");
  const [services, setServices] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    (async () => {
      const { data } = await api.api.GET("/api/v1/settings/ai/matrix");
      if (!alive) return;
      setCatalog(data?.catalog ?? []);
    })();
    return () => {
      alive = false;
    };
  }, []);

  const add = async () => {
    setBusy(true);
    setError(null);
    const { error: e } = await api.api.PUT("/api/v1/settings/ai/catalog", {
      body: {
        provider,
        model,
        label: label === "" ? `${provider}/${model}` : label,
        capabilities: [...services],
        enabled: true,
        idempotency_key: `catalog:${provider}:${model}:${Date.now()}`,
      } as never,
    });
    setBusy(false);
    if (e !== undefined) {
      setError(resultErrorText(e, "The catalog entry was rejected."));
      return;
    }
    setServices(new Set());
    setModel("");
    setLabel("");
    onChanged();
    const { data } = await api.api.GET("/api/v1/settings/ai/matrix");
    setCatalog(data?.catalog ?? []);
  };

  const toggle = async (entry: AiCatalogDto, enabled: boolean) => {
    setCatalog((prev) =>
      prev?.map((e) => (e.id === entry.id ? { ...e, enabled } : e)) ?? null,
    );
    const { error: e } = await api.api.PUT("/api/v1/settings/ai/catalog", {
      body: {
        provider: entry.provider,
        model: entry.model,
        label: entry.label,
        capabilities: entry.capabilities,
        enabled,
        idempotency_key: `catalog:${entry.id}:${Date.now()}`,
      } as never,
    });
    if (e !== undefined) {
      setCatalog((prev) =>
        prev?.map((x) => (x.id === entry.id ? { ...x, enabled: !enabled } : x)) ?? null,
      );
      setError(resultErrorText(e, "The change was rejected."));
    }
  };

  const removeEntry = async (entryId: string) => {
    await api.api.DELETE("/api/v1/settings/ai/catalog/{entry_id}", {
      params: { path: { entry_id: entryId } },
    });
    setCatalog((prev) => prev?.filter((e) => e.id !== entryId) ?? null);
    onChanged();
  };

  return (
    <Card className="mb-4">
      <div className="p-4">
        <h2 className="text-sm font-semibold">Model catalog</h2>
        <p className="mt-1 max-w-prose text-[13px]" style={{ color: "var(--m-text-muted)" }}>
          Curate which (provider, model) pairs exist in the org. Enabled entries ARE the admin
          allowlist every resolution falls inside; the edge tags gate what a pair may serve.
        </p>
        <div className="mt-3 flex flex-wrap items-end gap-3" data-testid="catalog-add-row">
          <div>
            <Label htmlFor="cat-provider">Provider</Label>
            <Input
              id="cat-provider"
              className="mt-1 w-40 text-[12px]"
              value={provider}
              onChange={(e) => setProvider(e.target.value)}
            />
          </div>
          <div>
            <Label htmlFor="cat-model">Model id</Label>
            <Input
              id="cat-model"
              className="mt-1 w-56 text-[12px]"
              value={model}
              onChange={(e) => setModel(e.target.value)}
              placeholder="e.g. jev-1.13.0 (pinned, never a floating tag)"
            />
          </div>
          <div>
            <Label htmlFor="cat-label">Label</Label>
            <Input
              id="cat-label"
              className="mt-1 w-56 text-[12px]"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
            />
          </div>
          <div className="flex gap-3 pb-1">
            {EDGE_TAGS.map((tag) => (
              <label key={tag} className="flex items-center gap-1 text-[12px]">
                <Checkbox
                  checked={services.has(tag)}
                  onCheckedChange={(v) =>
                    setServices((prev) => {
                      const next = new Set(prev);
                      if (v) next.add(tag);
                      else next.delete(tag);
                      return next;
                    })
                  }
                  aria-label={`${tag} capability`}
                />
                {tag}
              </label>
            ))}
          </div>
          <Button
            size="sm"
            disabled={busy || model.trim() === "" || provider.trim() === ""}
            onClick={() => void add()}
          >
            {busy ? <Spinner /> : "Add to catalog"}
          </Button>
        </div>
        {error != null && (
          <p className="mt-2 text-[13px]" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
            {error}
          </p>
        )}
        <Table className="mt-3">
          <thead>
            <tr>
              <Th>Model</Th>
              <Th>Edges</Th>
              <Th>Enabled</Th>
              <Th />
            </tr>
          </thead>
          <tbody data-testid="catalog-rows">
            {(catalog ?? []).map((e) => (
              <Tr key={e.id}>
                <Td>
                  <div className="flex flex-col">
                    <span className="text-[13px] font-medium">{e.label}</span>
                    <span className="font-mono text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                      {e.provider}/{e.model} · weight {e.cost_weight}
                    </span>
                  </div>
                </Td>
                <Td>
                  <span className="flex flex-wrap gap-1">
                    {((e.capabilities ?? []) as string[]).sort().map((tag: string) => (
                      <Tag key={tag}>{tag}</Tag>
                    ))}
                  </span>
                </Td>
                <Td>
                  <input
                    type="checkbox"
                    checked={e.enabled}
                    onChange={(ev) => void toggle(e, ev.target.checked)}
                    className="mr-2"
                    style={{ accentColor: "var(--m-primary)" }}
                    aria-label={`${e.model} enabled`}
                  />
                </Td>
                <Td>
                  <Button size="sm" variant="ghost" onClick={() => void removeEntry(e.id)}>
                    Remove
                  </Button>
                </Td>
              </Tr>
            ))}
          </tbody>
        </Table>
      </div>
    </Card>
  );
}

interface CapEditorProps {
  caps: components["schemas"]["AiCapDto"][];
  users: { id: string; name: string }[];
  onChanged: () => void;
}

/** Org-wide (scope=org) + per-user soft/hard token caps. */
export function CapsCard({ caps, users, onChanged }: CapEditorProps) {
  const [scope, setScope] = useState<"org" | "user">("org");
  const [userId, setUserId] = useState<string>("");
  const [soft, setSoft] = useState<string>("");
  const [hard, setHard] = useState<string>("");
  const [period, setPeriod] = useState<string>("month");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const save = async () => {
    const softNum = soft === "" ? null : Number(soft);
    const hardNum = hard === "" ? null : Number(hard);
    if (scope === "user" && userId === "") {
      setError("Pick a user for a per-user cap.");
      return;
    }
    setBusy(true);
    setError(null);
    const { error: e } = await api.api.PUT("/api/v1/settings/ai/caps", {
      body: {
        scope,
        user_id: scope === "user" ? userId : undefined,
        period,
        soft_token_limit: softNum,
        hard_token_limit: hardNum,
        idempotency_key: `cap:${scope}:${userId}:${period}:${Date.now()}`,
      } as never,
    });
    setBusy(false);
    if (e !== undefined) {
      setError(resultErrorText(e, "The cap was rejected."));
      return;
    }
    setSoft("");
    setHard("");
    onChanged();
  };

  const removeCap = async (id: string) => {
    await api.api.DELETE("/api/v1/settings/ai/caps/{cap_id}", {
      params: { path: { cap_id: id } },
    });
    onChanged();
  };

  return (
    <Card className="mb-4">
      <div className="p-4">
        <h2 className="text-sm font-semibold">Caps</h2>
        <p className="mt-1 max-w-prose text-[13px]" style={{ color: "var(--m-text-muted)" }}>
          Every configured period binds at admission over its own window (week/month/quarter/year):
          hard caps refuse typed before dispatch — degrade, never a 5xx; soft caps warn first.
        </p>
        <div className="mt-3 flex flex-wrap items-end gap-3" data-testid="cap-add-row">
          <div>
            <Label>Scope</Label>
            <Select value={scope} onValueChange={(v) => setScope(v as "org" | "user")}>
              <SelectTrigger className="mt-1 w-28 text-[12px]" style={{ borderRadius: "var(--m-radius-control)" }}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="org">Org-wide</SelectItem>
                <SelectItem value="user">Per-user</SelectItem>
              </SelectContent>
            </Select>
          </div>
          {scope === "user" && (
            <div>
              <Label>User</Label>
              <select
                className="mt-1 h-8 rounded-[var(--m-radius-control)] border px-2 text-[12px]"
                style={{ borderColor: "var(--m-border)" }}
                value={userId}
                onChange={(e) => setUserId(e.target.value)}
              >
                <option value="">Pick a user…</option>
                {users.map((u) => (
                  <option key={u.id} value={u.id}>
                    {u.name}
                  </option>
                ))}
              </select>
            </div>
          )}
          <div>
            <Label>Period</Label>
            <select
              className="mt-1 h-8 rounded-[var(--m-radius-control)] border px-2 text-[12px]"
              style={{ borderColor: "var(--m-border)" }}
              value={period}
              onChange={(e) => setPeriod(e.target.value)}
            >
              {["day", "week", "month", "quarter", "year"].map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              ))}
            </select>
          </div>
          <div>
            <Label htmlFor="cap-soft">Soft limit (tokens)</Label>
            <Input
              id="cap-soft"
              type="number"
              className="mt-1 w-36 text-[12px]"
              value={soft}
              onChange={(e) => setSoft(e.target.value)}
              placeholder="warn at"
            />
          </div>
          <div>
            <Label htmlFor="cap-hard">Hard limit (tokens)</Label>
            <Input
              id="cap-hard"
              type="number"
              className="mt-1 w-36 text-[12px]"
              value={hard}
              onChange={(e) => setHard(e.target.value)}
              placeholder="refuse at"
            />
          </div>
          <Button size="sm" disabled={busy} onClick={() => void save()}>
            {busy ? <Spinner /> : "Set cap"}
          </Button>
        </div>
        {error != null && (
          <p className="mt-2 text-[13px]" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
            {error}
          </p>
        )}
        {caps.length > 0 && (
          <Table className="mt-3">
            <thead>
              <tr><Th>Scope</Th><Th>Period</Th><Th>Soft</Th><Th>Hard</Th><Th /></tr>
            </thead>
            <tbody data-testid="cap-rows">
              {caps.map((c) => (
                <Tr key={c.id}>
                  <Td className="text-[12px]">
                    {c.user_id === null
                      ? "Org-wide"
                      : users.find((u) => u.id === c.user_id)?.name ?? "user"}
                  </Td>
                  <Td className="text-[12px]">{c.period}</Td>
                  <Td className="text-[12px]">{c.soft_token_limit}</Td>
                  <Td className="text-[12px]">{c.hard_token_limit}</Td>
                  <Td>
                    <Button size="sm" variant="ghost" onClick={() => void removeCap(c.id)}>
                      Remove
                    </Button>
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </div>
    </Card>
  );
}
