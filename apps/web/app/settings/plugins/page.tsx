// /settings/plugins — plugin + agent-skills management (Wave 16 lane 10
// over lane 8's close-out, ADR 0009 / migration 0079).
//
// Purpose: the whole Settings → Plugins surface: the Installed roster
// split into the artifact-governance tabs (WASM / HTTP connector, the
// pinned manifest's `artifact_kind` — lane-10 roster split) plus the
// Agent-skills tab, version/rollback re-consent diffs, terminal
// uninstall with survival copy, the Registry browse, and the
// staged-approvals queue — all wired to the distribution API.
//
// Non-purpose: no further lifecycle logic — the tabs group REAL installs
// only (an empty tab is a genuine empty state, never a placeholder).
"use client";

import type { components } from "@marula-core/sdk";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  Spinner,
} from "@marula-core/ui";
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import { SettingsHeader } from "../../../components/settings/settings-shell";
import { GithubImportDrawer, ZipImportDrawer } from "./import-drawers";
import { InstallDetail } from "./install-detail";
import { AgentSkillsTab } from "./agent-skills-tab";
import { RegistryTab } from "./registry-tab";
import { StagedApprovalsTab } from "./staged-approvals";
import { MarketplaceViewTab } from "./marketplace-v2";

type InstallView = components["schemas"]["InstallView"];
type StagedItemView = components["schemas"]["StagedItemView"];
type SkillVersionView = components["schemas"]["SkillVersionView"];
type CatalogEntryView = components["schemas"]["CatalogEntryView"];

const TABS = ["WASM", "HTTP connectors", "Agent skills", "Registry", "Approvals", "Marketplace"] as const;
type Tab = (typeof TABS)[number];

/** The install-discriminator: unknown/absent ≤ v1 shapes degrade to the
 * WASM baseline (the v1 manifest/permission baseline every shipped entry
 * was published as — backend dual of `PluginManifestV2::runtime_kind`). */
function kindOf(install: InstallView): "wasm" | "http_connector" {
  return install.artifact_kind === "http_connector" ? "http_connector" : "wasm";
}

export default function PluginsSettingsPage() {
  const auth = useRequireAuth();
  const [tab, setTab] = useState<Tab>("WASM");
  const [installs, setInstalls] = useState<InstallView[] | null>(null);
  const [staged, setStaged] = useState<StagedItemView[] | null>(null);
  const [skills, setSkills] = useState<SkillVersionView[] | null>(null);
  const [catalog, setCatalog] = useState<CatalogEntryView[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [githubOpen, setGithubOpen] = useState(false);
  const [zipOpen, setZipOpen] = useState(false);

  const canManage = (auth.me?.permissions ?? []).includes("special:manage_plugins");
  const pendingCount = (staged ?? []).filter((s) => s.status === "pending").length;

  const refresh = useCallback(async () => {
    const [installed, stagedList, skillList, catalogList] = await Promise.all([
      api.api.GET("/api/v1/plugins/installed"),
      api.api.GET("/api/v1/plugins/distribution/staged"),
      api.api.GET("/api/v1/plugins/distribution/agent-skills"),
      api.api.GET("/api/v1/plugins/catalog"),
    ]);
    if (
      installed.error !== undefined ||
      stagedList.error !== undefined ||
      skillList.error !== undefined ||
      catalogList.error !== undefined
    ) {
      setLoadError("Failed to load the plugin and skill surfaces.");
      return;
    }
    setInstalls(installed.data ?? []);
    setStaged(stagedList.data ?? []);
    setSkills(skillList.data ?? []);
    setCatalog(catalogList.data ?? []);
    setLoadError(null);
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void refresh();
  }, [auth.status, refresh]);

  if (auth.status !== "authed") return null;

  return (
    <div className="grid gap-6">
      <SettingsHeader
        title="Plugins & agent skills"
        description="Nothing executes that did not arrive digest-verified with provenance and pass admin consent. Installs start DISABLED; updates pin the old version until the consent diff is approved."
      />
        {canManage ? null : (
          <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
            Lifecycle controls (consent, enable, settings, removal) are owner/admin — you can
            still stage imports and follow your own queue rows.
          </p>
        )}

      {loadError !== null ? (
        <p role="alert" className="text-[13px]">{loadError}</p>
      ) : installs === null ? (
        <Spinner />
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-2">
            {TABS.map((name) => (
              <button
                key={name}
                type="button"
                onClick={() => setTab(name)}
                aria-current={tab === name ? "true" : undefined}
                className="rounded-[var(--m-radius-container)] px-3 py-1.5 text-[13px] tab-button"
                style={{
                  background: tab === name ? "var(--m-surface)" : "transparent",
                  color: tab === name ? "var(--m-text)" : "var(--m-text-secondary)",
                  fontWeight: tab === name ? 600 : 400,
                  border: `1px solid ${tab === name ? "var(--m-border)" : "transparent"}`,
                  transition: "background 120ms, color 120ms",
                }}
                data-testid={`tab-${name.toLowerCase().replace(" ", "-")}`}
              >
                {name}
                {name === "Approvals" && pendingCount > 0 ? (
                  <strong className="ml-1" style={{ color: "var(--m-accent-fg, var(--m-accent))" }}>
                    {pendingCount}
                  </strong>
                ) : null}
              </button>
            ))}
            <div className="flex gap-2" style={{ marginLeft: "auto" }}>
              <Button size="sm" variant="secondary" onClick={() => setGithubOpen(true)} data-testid="open-github-drawer">
                Import from GitHub…
              </Button>
              <Button size="sm" variant="secondary" onClick={() => setZipOpen(true)} data-testid="open-zip-drawer">
                Import zip…
              </Button>
            </div>
          </div>

          {actionError !== null ? (
            <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {actionError}
            </p>
          ) : null}

          {tab === "WASM" || tab === "HTTP connectors" ? (
    <ArtifactRoster
      tab={tab}
      installs={(installs ?? []).filter((i) => kindOf(i) === (tab === "WASM" ? "wasm" : "http_connector"))}
      canManage={canManage}
      expanded={expanded}
      setExpanded={setExpanded}
      setInstalls={setInstalls}
      refresh={refresh}
    />
  ) : tab === "Registry" ? (
    <RegistryTab catalog={catalog} installs={installs} canManage={canManage} onChanged={() => void refresh()} />
  ) : tab === "Approvals" ? (
    <StagedApprovalsTab staged={staged} canManage={canManage} onChanged={() => void refresh()} />
  ) : tab === "Agent skills" ? (
    <AgentSkillsTab skills={skills} canManage={canManage} onChanged={() => void refresh()} />
  ) : (
    <MarketplaceViewTab canManage={canManage} />
  )}
        </>
      )}

      <GithubImportDrawer open={githubOpen} onOpenChange={setGithubOpen} onStaged={() => { setTab("Approvals"); void refresh(); }} />
      <ZipImportDrawer open={zipOpen} onOpenChange={setZipOpen} onStaged={() => { setTab("Approvals"); void refresh(); }} />
    </div>
  );
}

/** One governance tab of the Installed roster (ADR 0009 §6): the EXISTING
 * install card/expanded-detail surface, filtered to the tab's
 * `artifact_kind`. A tab with no installs is a genuine empty state. */
function ArtifactRoster({
  tab,
  installs,
  canManage,
  expanded,
  setExpanded,
  setInstalls,
  refresh,
}: {
  tab: Tab;
  installs: InstallView[];
  canManage: boolean;
  expanded: string | null;
  setExpanded: React.Dispatch<React.SetStateAction<string | null>>;
  setInstalls: React.Dispatch<React.SetStateAction<InstallView[] | null>>;
  refresh: () => Promise<void>;
}) {
  return (
    <div className="grid gap-3">
      {installs.length === 0 ? (
        <EmptyState
          message={
            tab === "WASM"
              ? "No WASM plugins installed yet. Import from GitHub or a zip — nothing runs without consent."
              : "No HTTP connectors installed yet. Connectors ride the same consent lifecycle — none installed for this org."
          }
        />
      ) : (
        installs.map((install) => (
          <Card key={install.id}>
            <div className="flex flex-wrap items-center gap-2 p-4 pb-1">
              <span className="text-sm font-semibold">{install.name}</span>
              <span className="font-mono text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                {install.slug}@{install.version}
              </span>
              <Badge tone={install.status === "enabled" ? "success" : install.status === "disabled" ? "warning" : "danger"}>
                {install.status}
              </Badge>
              <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                {install.granted_capabilities.length} granted caps
              </span>
              <Button
                size="sm"
                variant="ghost"
                className="ml-auto"
                onClick={() => setExpanded(expanded === install.id ? null : install.id)}
                aria-expanded={expanded === install.id}
                data-testid={`expand-${install.slug}`}
              >
                {expanded === install.id ? "Collapse" : "Manage"}
              </Button>
            </div>
            {expanded === install.id ? (
              <div className="px-4 pb-4" data-testid={`install-detail-${install.slug}`}>
                <InstallDetail
                  install={install}
                  canManage={canManage}
                  onOptimisticStatus={(next) =>
                    setInstalls((prev) =>
                      (prev ?? []).map((row) =>
                        row.id === install.id ? { ...row, status: next } : row,
                      ),
                    )
                  }
                  onChanged={() => void refresh()}
                />
              </div>
            ) : null}
          </Card>
        ))
      )}
    </div>
  );
}
