"use client";

// /settings/ai — the AI availability matrix close-out (this lane).
//
// Purpose: rebuild the old flat BYOK page onto the ADR 0008 matrix +
// the ADR 0010 §4 decision harness:
// - AI features master switch (org-wide; default OFF — old-school orgs
//   keep the exact pre-AI product; gates the completion edge too).
// - The availability matrix: surfaces (native + plugin + decision) ×
//   per-role/per-user grants (deny-by-default, audited), model catalog
//   curation, org + per-user caps, ORG credential slot UI (sealed,
//   never surfaces secret bytes), org usage progress.
// - personal keys for the current admin / user (same credential slot,
//   gated by `ai.personal_credentials_enabled`).
// - The decision-harness leg: per-surface enable/grant/config editors
//   riding the DecisionSurfaceEditor component (state_template JSON
//   editor + live preview + thresholds + fork provenance + shadow
//   fire + shadow→live).
// - BYOK provider keys remain here (the Ollama/custom completion
//   posture previously shipped), now the "Byok providers" section.
//
// Non-purpose: secret bytes never render back; no new backend work —
// everything rides the landed matrix/harness/decision-config seams.

import { useCallback, useEffect, useState } from "react";
import {
  Button,
  Card,
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
import type { components } from "@marula-core/sdk";
import { api, useRequireAuth } from "../../../lib/api";
import { SettingsHeader } from "../../../components/settings/settings-shell";
import { rejectionText, resultErrorText } from "../../../lib/error-copy";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { DecisionSurfaceEditor } from "../../../components/settings/decision-surface-editor";
import { CatalogCard, CapsCard } from "../../../components/settings/ai-catalog-caps";
import { ByokLegacyCard } from "../../../components/settings/byok-legacy-card";
import { MyUsageCard, OrgUsageCard } from "../../../components/settings/ai-usage-cards";

const AI_PROVIDERS = ["openai", "anthropic", "ollama", "custom"] as const;
const DECISION_PROVIDERS = ["typesafe"] as const;
const CONNECTORS = ["qbo", "xero"] as const;

type AiMatrixResponse = components["schemas"]["AiMatrixResponse"];
type DecisionOverview = components["schemas"]["DecisionOverviewResponse"];
type DecisionSurfaceOverview = components["schemas"]["DecisionSurfaceOverview"];
type PutDecisionConfigRequest = components["schemas"]["PutDecisionConfigRequest"];

export default function AiSettingsPage() {
  const auth = useRequireAuth();
  const [matrix, setMatrix] = useState<AiMatrixResponse | null>(null);
  const [decision, setDecision] = useState<DecisionOverview | null>(null);
  const [roles, setRoles] = useState<{ id: string; display_name?: string; name?: string }[]>([]);
  const [users, setUsers] = useState<{ id: string; name: string }[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (auth.status !== "authed") return;
    const [m, d, r, u] = await Promise.all([
      api.api.GET("/api/v1/settings/ai/matrix"),
      api.api.GET("/api/v1/settings/ai/decision"),
      api.api.GET("/api/v1/roles"),
      api.api.GET("/api/v1/org/users"),
    ]);
    if (m.error !== undefined) setLoadError("Failed to load the AI matrix.");
    else setMatrix(m.data ?? null);
    setDecision(d.error === undefined ? d.data ?? null : null);
    setRoles((r.data as unknown as { roles?: { id: string; display_name?: string; name?: string }[] })?.roles ?? []);
    const userRows =
      (u.data as unknown as { users?: { id: string; name: string }[] } | undefined)?.users ?? [];
    setUsers(userRows);
    if (loadError !== null && m.error === undefined) setLoadError(null);
  }, [auth.status, loadError]);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status]); // eslint-disable-line react-hooks/exhaustive-deps

  const scrollToSurface = (key: string) => {
    document
      .querySelector(`[data-surface-editor="${key}"]`)
      ?.scrollIntoView({ behavior: "smooth", block: "start" });
  };

  if (loadError !== null) {
    return (
      <Card>
        <p className="p-4 text-[13px]" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
          {loadError}
        </p>
      </Card>
    );
  }
  if ((matrix === null || decision === null) && auth.status === "authed") {
    return (
      <div className="flex items-center gap-2 p-4">
        <Spinner />
        <span className="text-[13px]">Loading the AI matrix…</span>
      </div>
    );
  }

  return (
    <div className="p-6" data-testid="settings-ai-page">
      <SettingsHeader
        title="AI"
        description="Availability, grants, caps, credentials and the decision harness — deny-by-default across the org."
      />

      <MasterSwitchCard
        featuresEnabled={decision?.features_enabled ?? false}
        onChanged={(v) =>
          setDecision((d) => (d === null ? d : { ...d, features_enabled: v }))
        }
      />

      <SurfacesMatrixCard
        matrix={matrix}
        decision={decision}
        roles={roles}
        users={users}
        onMatrixRefresh={() => void refresh()}
        onFocusSurface={scrollToSurface}
      />

      <CatalogCard onChanged={() => void refresh()} />

      <CapsCard caps={matrix?.caps ?? []} users={users} onChanged={() => void refresh()} />

      <DecisionHarnessCard
        surfaces={decision?.surfaces ?? []}
        onChanged={(key, next) =>
          setDecision((prev) =>
            prev === null
              ? prev
              : {
                  ...prev,
                  surfaces: prev.surfaces.map((s) => (s.surface_key === key ? next : s)),
                },
          )
        }
      />

      <TerminalCredentialCard matrix={matrix} users={users} />

      <OrgUsageCard />

      <MyUsageCard />

      <ByokLegacyCard />
    </div>
  );
}

// ---------------------------------------------------------------------------

function MasterSwitchCard({
  featuresEnabled,
  onChanged,
}: {
  featuresEnabled: boolean | undefined;
  onChanged: (v: boolean) => void;
}) {
  const [busy, setBusy] = useState(false);
  return (
    <Card className="mb-4">
      <div className="p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <h2 className="text-sm font-semibold">AI features</h2>
            <p className="mt-1 max-w-prose text-[13px]" style={{ color: "var(--m-text-muted)" }}>
              The org-wide master switch, default OFF. OFF keeps the exact pre-AI product — every
              AI surface (completion, tools and the decision harness) answers quiet with zero
              ledger rows; flipping ON lets the per-surface grants and configs below decide what is
              actually audible.
            </p>
          </div>
          <Button
            size="sm"
            variant={featuresEnabled ? "secondary" : "primary"}
            data-testid="ai-features-switch"
            disabled={busy || featuresEnabled === undefined}
            onClick={async () => {
              setBusy(true);
              const { error } = await api.api.PUT("/api/v1/settings/ai/decision/features", {
                body: { enabled: !featuresEnabled, idempotency_key: `features:${Date.now()}` } as never,
              });
              setBusy(false);
              if (error !== undefined) return;
              onChanged(!featuresEnabled);
            }}
          >
            {busy ? <Spinner /> : featuresEnabled ? "Turn AI features off" : "Turn AI features on"}
          </Button>
        </div>
      </div>
    </Card>
  );
}

// ---------------------------------------------------------------------------

type RoleRow = { id: string; display_name?: string; name?: string };
type UserRow = { id: string; name: string };

function SurfacesMatrixCard({
  matrix,
  decision,
  roles,
  users,
  onMatrixRefresh,
  onFocusSurface,
}: {
  matrix: AiMatrixResponse | null;
  decision: DecisionOverview | null;
  roles: { id: string; display_name?: string; name?: string }[];
  users: { id: string; name: string }[];
  onMatrixRefresh: () => void;
  onFocusSurface: (key: string) => void;
}) {
  if (matrix === null) return null;
  const surfaces = (matrix.surfaces ?? []).filter(
    (s) => s.kind === "native" || s.kind === "plugin",
  );
  const editables = decision?.surfaces ?? [];
  const editableByKey = new Map(editables.map((s) => [s.surface_key, s]));
  return (
    <Card className="mb-4">
      <div className="p-4">
        <h2 className="text-sm font-semibold">Surfaces</h2>
        <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-muted)" }}>
          Per-surface access, deny-by-default. Grant a surface to a role or a user; nothing listed
          means nobody can call it. Decision surfaces configure below in the harness section.
        </p>
        <Table className="mt-3">
          <thead>
            <tr>
              <Th>Surface</Th>
              <Th>Kind</Th>
              <Th>Grants</Th>
              <Th>Actions</Th>
            </tr>
          </thead>
          <tbody data-testid="surface-matrix-rows">
            {surfaces.map((s) => {
              const rowGrants = matrix.grants.filter((g) => g.surface_key === s.key);
              const dec = editableByKey.get(s.key);
              return (
                <Tr key={s.id}>
                  <Td>
                    <div className="flex flex-col">
                      <span className="text-[13px] font-medium">{s.label}</span>
                      <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                        {s.key}
                      </span>
                    </div>
                  </Td>
                  <Td>
                    <Tag>{s.kind}</Tag>
                  </Td>
                  <Td>
                    <GrantEditor
                      surfaceKey={s.key}
                      grants={rowGrants}
                      roles={roles as unknown as RoleRow[]}
                      users={users as unknown as UserRow[]}
                      onChanged={onMatrixRefresh}
                    />
                  </Td>
                  <Td>
                    <span className="flex items-center gap-2">
                      {dec != null && (
                        <span
                          className="inline-flex h-[18px] items-center rounded-full px-2 text-[9px] font-bold tracking-wide"
                          style={
                            dec.enabled
                              ? { background: "var(--m-status-success-bg)", color: "var(--m-status-success-fg)" }
                              : { background: "var(--m-status-neutral-bg)", color: "var(--m-status-neutral-fg)" }
                          }
                        >
                          {dec.enabled ? dec.mode.toUpperCase() : "OFF"}
                        </span>
                      )}
                      <Button size="sm" variant="ghost" onClick={() => onFocusSurface(s.key)}>
                        Configure
                      </Button>
                    </span>
                  </Td>
                </Tr>
              );
            })}
          </tbody>
        </Table>
      </div>
    </Card>
  );
}

function GrantEditor({
  surfaceKey,
  grants,
  roles,
  users,
  onChanged,
}: {
  surfaceKey: string;
  grants: components["schemas"]["AiGrantDto"][];
  roles: RoleRow[];
  users: UserRow[];
  onChanged: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [grantee, setGrantee] = useState<"role" | "user">("role");
  const [target, setTarget] = useState<string>("");
  const pick = async (kind: "role" | "user") => {
    setBusy(true);
    const { error } = await api.api.PUT("/api/v1/settings/ai/grants", {
      body: {
        surface_key: surfaceKey,
        grantee_kind: kind,
        role_id: kind === "role" ? (target || undefined) : undefined,
        user_id: kind === "user" ? (target || undefined) : undefined,
        idempotency_key: `grant:${surfaceKey}:${kind}:${target}:${Date.now()}`,
      } as never,
    });
    setBusy(false);
    if (error !== undefined) return;
    onChanged();
  };
  const revoke = async (grantId: string) => {
    setBusy(true);
    await api.api.DELETE("/api/v1/settings/ai/grants/{grant_id}", {
      params: { path: { grant_id: grantId } },
    });
    setBusy(false);
    onChanged();
  };
  return (
    <div className="flex flex-col gap-1">
      {grants.length === 0 ? (
        <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
          Not granted to anyone.
        </span>
      ) : (
        grants.map((g) => (
          <span key={g.id} className="flex items-center gap-1 text-[12px]">
            <Tag>
              {g.grantee_kind === "role"
                ? `role ${roles.find((r) => r.id === g.role_id)?.display_name ?? roles.find((r) => r.id === g.role_id)?.name ?? g.role_id}`
                : `user ${users.find((u) => u.id === g.user_id)?.name ?? g.user_id}`}
            </Tag>
            <Button
              size="sm"
              variant="ghost"
              disabled={busy}
              onClick={() => void revoke(g.id)}
            >
              Revoke
            </Button>
          </span>
        ))
      )}
      <span className="flex items-center gap-1" data-testid={`grant-editor-${surfaceKey}`}>
        <Select
          value={grantee}
          onValueChange={(v) => setGrant(v as "role" | "user")}
        >
          <SelectTrigger
            className="h-8 w-24 text-[12px]"
            style={{ borderRadius: "var(--m-radius-control)" }}
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="role">Role</SelectItem>
            <SelectItem value="user">User</SelectItem>
          </SelectContent>
        </Select>
        <select
          className="h-8 max-w-48 rounded-[var(--m-radius-control)] border px-1 text-[12px]"
          style={{ borderColor: "var(--m-border)" }}
          value={target}
          onChange={(e) => setTarget(e.target.value)}
          aria-label={grantee === "role" ? "Role" : "User"}
        >
          <option value="">{grantee === "role" ? "Pick a role…" : "Pick a user…"}</option>
          {(grantee === "role" ? (roles as RoleRow[]) : (users as UserRow[])).map((row) => {
            const id = row.id;
            const label =
              grantee === "role"
                ? (row as RoleRow).display_name ?? (row as RoleRow).name ?? id
                : (row as UserRow).name ?? id;
            return (
              <option key={id} value={id}>
                {label}
              </option>
            );
          })}
        </select>
        <Button size="sm" disabled={busy || target === ""} onClick={() => void pick(grantee)}>
          Grant
        </Button>
      </span>
    </div>
  );
}

function setGrant(next: "role" | "user") {
  return next;
}

function DecisionHarnessCard({
  surfaces,
  onChanged,
}: {
  surfaces: DecisionSurfaceOverview[];
  onChanged: (key: string, next: DecisionSurfaceOverview) => void;
}) {
  if (surfaces.length === 0) return null;
  return (
    <Card className="mb-4">
      <div className="p-4">
        <h2 className="text-sm font-semibold">Decision harness</h2>
        <p className="mt-1 max-w-prose text-[13px]" style={{ color: "var(--m-text-muted)" }}>
          Every decision surface lands SHADOW and quiet until deliberately configured — an
          unconfigured surface shows no cards and offers no dead buttons (ADR 0011 §3); what each
          activates is stated on the card.
        </p>
        <div className="mt-3 flex flex-col gap-4" data-testid="decision-surface-editors">
          {surfaces.map((s) => (
            <DecisionSurfaceEditor
              key={s.surface_key}
              surface={s}
              onSaved={(next) => onChanged(s.surface_key, next)}
            />
          ))}
        </div>
      </div>
    </Card>
  );
}

function TerminalCredentialCard({ matrix, users }: { matrix: AiMatrixResponse | null; users: { id: string; name: string }[] }) {
  const [kind, setKind] = useState<"api_key" | "oauth_token" | "subscription_account">("api_key");
  const [label, setLabel] = useState("");
  const [provider, setProvider] = useState("typesafe");
  const [secret, setSecret] = useState("");
  const [personal, setPersonal] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (matrix === null) return null;
  const personalEnabled = matrix.personal_credentials_enabled;
  const orgCreds = matrix.credentials.filter((c) => c.owner_user_id === null);
  const saves = async () => {
    if (secret === "") {
      setError("Enter the secret first.");
      return;
    }
    setBusy(true);
    setError(null);
    const result = await runEnvelopeMutation<
      { provider: string; kind: string; secret: string; label?: string; idempotency_key: string },
      unknown
    >({
      describe: `Save ${provider} credential`,
      entityId: `ai-cred:${provider}`,
      optimistic: () => {},
      revert: () => {},
      buildBody: (base) => ({
        provider,
        kind,
        secret,
        label: label === "" ? undefined : label,
        idempotency_key: base.idempotency_key,
      }),
      request: (client, body) =>
        personal
          ? client.PUT("/api/v1/ai/me/credentials", { body: body as never })
          : client.PUT("/api/v1/settings/ai/credentials", { body: body as never }),
    });
    setBusy(false);
    setSecret("");
    if (!result.ok) {
      setError(result.friendly ?? resultErrorText(result.reason, "The credential was rejected."));
    }
  };
  return (
    <Card className="mb-4">
      <div className="p-4">
        <h2 className="text-sm font-semibold">Credential slot</h2>
        <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-muted)" }}>
          Org credentials and (when allowed) personal keys — sealed at rest, never surfaced back.
          The decision model rides the same slot (provider `typesafe`).
        </p>
        <div className="mt-3 grid grid-cols-[repeat(2,minmax(0,1fr))] gap-3" data-testid="credential-slot">
          <div>
            <Label htmlFor="cred-provider">Provider</Label>
            <Input
              id="cred-provider"
              className="mt-1 text-[12px]"
              value={provider}
              onChange={(e) => setProvider(e.target.value)}
              list="cred-provider-list"
            />
            <datalist id="cred-provider-list">
              {[...DECISION_PROVIDERS, ...AI_PROVIDERS].map((p) => (
                <option key={p} value={p} />
              ))}
              {(matrix.catalog.map((c) => c.provider) as string[])
                .filter((p, i, arr) => arr.indexOf(p) === i)
                .map((p) => (
                  <option key={p} value={p} />
                ))}
            </datalist>
          </div>
          <div>
            <Label htmlFor="cred-kind">Kind</Label>
            <Select
              value={kind}
              onValueChange={(v) => setKind(v as "api_key" | "oauth_token" | "subscription_account")}
            >
              <SelectTrigger
                className="mt-1 text-[12px]"
                style={{ borderRadius: "var(--m-radius-control)" }}
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="api_key">API key</SelectItem>
                <SelectItem value="oauth_token">OAuth token</SelectItem>
                <SelectItem value="subscription_account">Subscription account</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div>
            <Label htmlFor="cred-label">Label</Label>
            <Input
              id="cred-label"
              className="mt-1 text-[12px]"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              placeholder="e.g. Jev production"
            />
          </div>
          <div>
            <Label htmlFor="cred-secret">Secret</Label>
            <Input
              id="cred-secret"
              type="password"
              className="mt-1 text-[12px]"
              value={secret}
              onChange={(e) => setSecret(e.target.value)}
              placeholder="Sealed at rest on save — never rendered back"
            />
          </div>
          <label className="flex items-center gap-2 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
            <input
              type="checkbox"
              checked={personal}
              disabled={!personalEnabled}
              onChange={(e) => setPersonal(e.target.checked)}
              style={{ accentColor: "var(--m-primary)" }}
            />
            Personal (my own key)
            {!personalEnabled && (
              <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                — admin policy off
              </span>
            )}
          </label>
          <span className="flex items-center gap-2">
            <Button size="sm" disabled={busy || secret === ""} onClick={() => void saves()}>
              {busy ? <Spinner /> : "Save credential"}
            </Button>
          </span>
        </div>
        {error != null && (
          <p className="mt-2 text-[13px]" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
            {error}
          </p>
        )}
        {(orgCreds.length > 0) && (
          <Table className="mt-3">
            <thead>
              <tr><Th>Provider</Th><Th>Kind</Th><Th>Label</Th><Th>Owner</Th><Th /></tr>
            </thead>
            <tbody data-testid="credential-rows">
              {orgCreds.map((c) => (
                <Tr key={c.id}>
                  <Td className="text-[12px]">{c.provider}</Td>
                  <Td className="text-[12px]">{c.kind}</Td>
                  <Td className="text-[12px]">{c.label == null ? "—" : c.label}</Td>
                  <Td className="text-[12px]">
                    {c.owner_user_id != null ? users.find((u) => u.id === c.owner_user_id)?.name ?? "—" : "Org"}
                  </Td>
                  <Td>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={async () => {
                        await api.api.DELETE("/api/v1/settings/ai/credentials/{credential_id}", {
                          params: { path: { credential_id: c.id } },
                        });
                      }}
                    >
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

