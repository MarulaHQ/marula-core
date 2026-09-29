// /settings/agent-platform — MCP/agent platform admin (Wave 9 lane 4
// close-out).
//
// Purpose: the three admin surfaces over the agent platform: the
// capability registry (grant/revoke per org, deny-by-default — nothing
// listed means no agent tool is callable), the published agent skills
// (immutable append-only versions), and the proposal approval queue —
// approve/decline with a before/after diff; a declined proposal writes
// nothing and shows it.
//
// Non-purpose: no MCP server hosting UI and no proposal authoring —
// agents create proposals through the platform API.
"use client";

import {
  Button,
  Card,
  EmptyState,
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
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { resultErrorText } from "../../../lib/error-copy";
import { SettingsHeader } from "../../../components/settings/settings-shell";

type CapabilityGrantView = components["schemas"]["CapabilityGrantView"];
type CapabilityMode = components["schemas"]["CapabilityMode"];
type AgentSkillView = components["schemas"]["AgentSkillView"];
type AgentProposalView = components["schemas"]["AgentProposalView"];

/** Renders one JSON value as an indented single-viewport block. */
function JsonBlock({ label, value }: { label: string; value: unknown }) {
  return (
    <div>
      <p className="text-[12px] font-semibold" style={{ color: "var(--m-text-secondary)" }}>
        {label}
      </p>
      <pre
        className="mt-1 max-h-40 overflow-auto rounded-[var(--m-radius-container)] border p-2 text-[12px]"
        style={{ borderColor: "var(--m-border)", background: "var(--m-canvas)" }}
      >
        {value === null ? "(no record)" : JSON.stringify(value, null, 2)}
      </pre>
    </div>
  );
}

export default function AgentPlatformPage() {
  const auth = useRequireAuth();
  const [grants, setGrants] = useState<CapabilityGrantView[] | null>(null);
  const [skills, setSkills] = useState<AgentSkillView[] | null>(null);
  const [proposals, setProposals] = useState<AgentProposalView[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const [newCapability, setNewCapability] = useState("");
  const [newMode, setNewMode] = useState<CapabilityMode>("proposed");
  const [grantBusy, setGrantBusy] = useState(false);

  const [proposalFilter, setProposalFilter] = useState<string>("proposed");
  const [decidingId, setDecidingId] = useState<string | null>(null);
  const [decisionNote, setDecisionNote] = useState("");
  const [openDiffId, setOpenDiffId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const [cap, sk, pr] = await Promise.all([
      api.api.GET("/api/v1/mcp/capabilities"),
      api.api.GET("/api/v1/agent-skills"),
      api.api.GET("/api/v1/agent-proposals", {
        params: { query: { status: proposalFilter } },
      }),
    ]);
    if (cap.error !== undefined || sk.error !== undefined || pr.error !== undefined) {
      setLoadError("Failed to load the agent platform surfaces.");
      return;
    }
    setGrants(cap.data?.grants ?? []);
    setSkills(sk.data?.skills ?? []);
    setProposals(pr.data?.proposals ?? []);
    setLoadError(null);
  }, [proposalFilter]);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  const grantCapability = async () => {
    const capability = newCapability.trim();
    if (capability === "") return;
    setGrantBusy(true);
    setActionError(null);
    const previous = grants;
    setGrants((prev) => [
      { capability, mode: newMode, granted_by: "me", granted_at: new Date().toISOString(), updated_at: new Date().toISOString(), id: `pending:${capability}` },
      ...(prev ?? []),
    ]);
    const { error } = await api.api.PUT("/api/v1/mcp/capabilities", {
      body: { capability, mode: newMode },
    });
    setGrantBusy(false);
    if (error !== undefined) {
      setGrants(previous);
      setActionError("The grant was rejected.");
      return;
    }
    setNewCapability("");
    void refresh();
  };

  const revokeCapability = async (capability: string) => {
    setActionError(null);
    const previous = grants;
    setGrants((prev) => (prev ?? []).filter((g) => g.capability !== capability));
    const { error } = await api.api.DELETE("/api/v1/mcp/capabilities/{capability}", {
      params: { path: { capability } },
    });
    if (error !== undefined) {
      setGrants(previous);
      setActionError(`Revoke of ${capability} failed.`);
      return;
    }
    void refresh();
  };

  const decide = async (p: AgentProposalView, decision: "approve" | "decline") => {
    setActionError(null);
    setDecidingId(p.id);
    const previous = proposals;
    // Optimistic status flip; a declined proposal writes nothing.
    setProposals((prev) =>
      (prev ?? []).map((row) =>
        row.id === p.id ? { ...row, status: decision === "approve" ? "approved" : "declined" } : row,
      ),
    );
    const { error } =
      decision === "approve"
        ? await api.api.POST("/api/v1/agent-proposals/{id}/approve", {
            params: { path: { id: p.id } },
            body: { note: decisionNote.trim() === "" ? null : decisionNote.trim() },
          })
        : await api.api.POST("/api/v1/agent-proposals/{id}/decline", {
            params: { path: { id: p.id } },
            body: { note: decisionNote.trim() === "" ? null : decisionNote.trim() },
          });
    setDecidingId(null);
    if (error !== undefined) {
      setProposals(previous);
      setActionError(`The ${decision} was rejected.`);
      return;
    }
    setDecisionNote("");
    void refresh();
  };

  if (auth.status !== "authed") return null;

  return (
    <div className="grid gap-6">
      <SettingsHeader
        title="Agent platform"
        description="Capability grants are deny-by-default: nothing granted here means no agent can call the tool. Proposals apply nothing until a person approves them."
      />

      {actionError !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {actionError}
        </p>
      ) : null}

      {loadError !== null ? (
        <p role="alert" className="text-[13px]">{loadError}</p>
      ) : grants === null || skills === null || proposals === null ? (
        <Spinner />
      ) : (
        <>
          <Card>
            <h2 className="p-4 pb-0 text-sm font-semibold">Capability registry</h2>
            <div className="flex items-end gap-3 p-4 pb-2">
              <div className="flex-1">
                <Label htmlFor="cap-name">Capability</Label>
                <Input
                  id="cap-name"
                  placeholder="tickets:write"
                  value={newCapability}
                  onChange={(e) => setNewCapability(e.target.value)}
                />
              </div>
              <div>
                <Label htmlFor="cap-mode">Dial</Label>
                <Select value={newMode} onValueChange={(v) => setNewMode(v as CapabilityMode)}>
                  <SelectTrigger id="cap-mode" className="w-40" aria-label="Involvement dial">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="proposed">proposed (approval)</SelectItem>
                    <SelectItem value="autonomous">autonomous</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <Button onClick={() => void grantCapability()} disabled={grantBusy || newCapability.trim() === ""}>
                {grantBusy ? <Spinner /> : null} Grant
              </Button>
            </div>
            {grants.length === 0 ? (
              <div className="px-4 pb-4">
                <EmptyState message="No capabilities granted. Agents operate deny-by-default — grant a namespace read/write here to open it up." />
              </div>
            ) : (
              <Table>
                <thead>
                  <tr>
                    <Th>Capability</Th>
                    <Th>Mode</Th>
                    <Th>Updated</Th>
                    <Th className="text-right">Actions</Th>
                  </tr>
                </thead>
                <tbody>
                  {grants.map((g) => (
                    <Tr key={g.capability}>
                      <Td>
                        <Tag>{g.capability}</Tag>
                      </Td>
                      <Td>{g.mode}</Td>
                      <Td>{new Date(g.updated_at).toLocaleString()}</Td>
                      <Td className="text-right">
                        <Button variant="ghost" size="sm" onClick={() => void revokeCapability(g.capability)}>
                          Revoke
                        </Button>
                      </Td>
                    </Tr>
                  ))}
                </tbody>
              </Table>
            )}
          </Card>

          <Card>
            <h2 className="p-4 pb-0 text-sm font-semibold">Agent skills</h2>
            {skills.length === 0 ? (
              <div className="p-4">
                <EmptyState message="No skills published. Skills are immutable versions — publishing the same slug again appends a new version." />
              </div>
            ) : (
              <Table>
                <thead>
                  <tr>
                    <Th>Slug</Th>
                    <Th className="text-right">Version</Th>
                    <Th>Manifest capabilities</Th>
                    <Th>Published</Th>
                  </tr>
                </thead>
                <tbody>
                  {skills.map((s) => {
                    const capabilities = (s.manifest as { capabilities?: unknown[] }).capabilities;
                    return (
                      <Tr key={s.id}>
                        <Td>{s.slug}</Td>
                        <Td className="text-right tabular-nums">v{s.version}</Td>
                        <Td>
                          {Array.isArray(capabilities) && capabilities.length > 0
                            ? capabilities.join(", ")
                            : "—"}
                        </Td>
                        <Td>{new Date(s.created_at).toLocaleString()}</Td>
                      </Tr>
                    );
                  })}
                </tbody>
              </Table>
            )}
          </Card>

          <Card>
            <div className="flex items-center justify-between p-4 pb-0">
              <h2 className="text-sm font-semibold">Proposal queue</h2>
              <Select value={proposalFilter} onValueChange={setProposalFilter}>
                <SelectTrigger aria-label="Filter proposals" className="w-44">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {["proposed", "approved", "declined", "withdrawn", "applied", "all"].map((s) => (
                    <SelectItem key={s} value={s}>
                      {s === "all" ? "All statuses" : s}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            {proposals.length === 0 ? (
              <div className="p-4">
                <EmptyState message="No proposals in this view. Agents stage writes as proposals; you decide each one here." />
              </div>
            ) : (
              <div className="grid gap-4 p-4">
                {proposals.map((p) => (
                  <div
                    key={p.id}
                    className="rounded-[var(--m-radius-container)] border p-3"
                    style={{ borderColor: "var(--m-border)" }}
                  >
                    <div className="flex flex-wrap items-center gap-2">
                      <Tag>{p.status}</Tag>
                      <span className="text-[13px] font-semibold">
                        {p.entity} · {p.op}
                      </span>
                      <span className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                        by {p.agent_kind} ({p.agent_ref})
                        {p.skill_slug !== null ? ` · skill ${p.skill_slug} v${p.skill_version ?? "?"}` : ""}
                        {p.classification === "client_visible" ? " · client-visible" : ""}
                        {p.classification === "external_commit" ? " · external commit" : ""}
                      </span>
                    </div>
                    {p.rationale !== null ? (
                      <p className="mt-2 text-[13px]">{p.rationale}</p>
                    ) : null}
                    <div className="mt-2 flex items-center gap-2">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setOpenDiffId(openDiffId === p.id ? null : p.id)}
                        aria-expanded={openDiffId === p.id}
                      >
                        {openDiffId === p.id ? "Hide diff" : "Show before/after"}
                      </Button>
                      {p.status === "proposed" ? (
                        <>
                          <Button
                            size="sm"
                            onClick={() => void decide(p, "approve")}
                            disabled={decidingId === p.id}
                          >
                            {decidingId === p.id ? <Spinner /> : null} Approve
                          </Button>
                          <Button
                            variant="secondary"
                            size="sm"
                            onClick={() => void decide(p, "decline")}
                            disabled={decidingId === p.id}
                          >
                            Decline
                          </Button>
                        </>
                      ) : p.status === "declined" ? (
                        <Tag>writes nothing</Tag>
                      ) : null}
                    </div>
                    {openDiffId === p.id ? (
                      <div className="mt-3 grid grid-cols-2 gap-3">
                        <JsonBlock label="Before" value={p.before_value} />
                        <JsonBlock label="After (proposed)" value={p.proposed_value} />
                      </div>
                    ) : null}
                  </div>
                ))}
              </div>
            )}
          </Card>

          <ToolRoutingGateCard />
        </>
      )}
    </div>
  );
}

/** ADR 0011 §1 tool-routing gate: the runner's pre-flight Choice gate
 over the already-granted MCP tool inventory. The gate NARROWS, never
 widens, and — per ADR 0011 §3 — it SURFACES only when its decision
 model is live (master switch ON + a decision-capable catalog entry +
 a configured `decision_tool_routing` surface with a pinned model);
 while the model is unconfigured the harness stays silently quiet and
 this section says so instead of rendering a dead control. */
function ToolRoutingGateCard() {
  const [decision, setDecision] = useState<components["schemas"]["DecisionOverviewResponse"] | null>(null);
  const [grants, setGrants] = useState<components["schemas"]["CapabilityGrantView"][]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const refresh = useCallback(async () => {
    const [d, cap] = await Promise.all([
      api.api.GET("/api/v1/settings/ai/decision"),
      api.api.GET("/api/v1/mcp/capabilities"),
    ]);
    setDecision(d.error === undefined ? d.data ?? null : null);
    setGrants(cap.data?.grants ?? []);
  }, []);
  useEffect(() => {
    void refresh();
  }, [refresh]);

  const gate = decision?.surfaces.find((s) => s.surface_key === "decision_tool_routing") ?? null;
  const toolCap = new Set(grants.map((g) => g.capability));
  const inventoryCount = toolCap.size;
  const gateLive =
    (decision?.features_enabled ?? false) &&
    gate != null &&
    gate.enabled &&
    gate.model_id != null;

  const toggleGate = async (enabled: boolean) => {
    setBusy(true);
    setError(null);
    const result = await runEnvelopeMutation<
      { enabled: boolean; idempotency_key: string },
      components["schemas"]["DecisionConfigMutationResponse"]
    >({
      describe: "Toggle tool-routing gate",
      entityId: "decision:decision_tool_routing",
      optimistic: () => {},
      revert: () => {},
      buildBody: (base) => ({ enabled, idempotency_key: base.idempotency_key }),
      request: (client, body) =>
        client.PUT("/api/v1/settings/ai/decision/{surface_key}/config", {
          params: { path: { surface_key: "decision_tool_routing" } },
          body: body as never,
        }),
    });
    setBusy(false);
    if (!result.ok) {
      setError(result.friendly ?? resultErrorText(result.reason, "The change was refused."));
      return;
    }
    void refresh();
  };

  return (
    <Card>
      <div className="p-4" data-testid="tool-routing-gate">
        <h2 className="text-sm font-semibold">Tool-routing gate (decision_tool_routing)</h2>
        <p className="mt-1 max-w-prose text-[13px]" style={{ color: "var(--m-text-muted)" }}>
          The pre-flight Choice gate over the agent runner's tool wardrobe. The gate can only
          narrow the already-granted inventory — it never widens one, and writes still ride the
          IF8 proposal pipeline with approval-by-default regardless.
        </p>
        {!gateLive ? (
          <p className="mt-3 text-[13px]" data-testid="gate-unconfigured-note" style={{ color: "var(--m-status-warning-fg)" }}>
            <strong>Decision model not configured.</strong> While the decision model is not live
            (master switch + a decision-capable catalog entry + this surface configured with a
            pinned model) the gate stays silently quiet — the inventory below is already granted
            and the runner picks from it directly. Configure the surface under
            <strong> Settings → AI → Decision harness</strong> to activate.
          </p>
        ) : (
          <div className="mt-3 flex flex-wrap items-center gap-3" data-testid="gate-live">
            <Button
              size="sm"
              variant={busy ? "secondary" : "primary"}
              disabled={busy}
              onClick={() => void toggleGate(false)}
            >
              {busy ? <Spinner /> : "Disable gate"}
            </Button>
            <span className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
              Gate active, pinned at <code>{gate?.model_provider}/{gate?.model_id}</code> — narrowing{" "}
              {inventoryCount} granted capabilit{inventoryCount === 1 ? "y" : "ies"}; the runner's
              inventory itself is beat-for-beat what /mcp/capabilities returns.
            </span>
          </div>
        )}
        {error != null && (
          <p className="mt-2 text-[13px]" role="alert" style={{ color: "var(--m-status-danger-fg)" }}>
            {error}
          </p>
        )}
      </div>
    </Card>
  );
}
