// /settings/plugins — plugin + agent-skills management (Wave 16 lane 8
// close-out over ADR 0009 / migration 0079).
//
// Purpose: the whole Settings → Plugins surface: the Installed tab
// (enable/disable, per-install manifest-JSON-Schema settings, update/
// rollback re-consent diffs, terminal uninstall with survival copy,
// provenance chain), the Registry browse, the staged-approvals queue,
// and the Agent-skills tab — all wired to lane 7's distribution API.
//
// Non-purpose: no WASM-vs-HTTP-connector split beyond a roster filter —
// InstallView carries no artifact-kind discriminator; the grouping rides
// either the catalog manifest or a later pin change (recorded UNCHECKED).
"use client";

import type { components } from "@meridian/sdk";
import { Button, Card, EmptyState, Input, Spinner, Tag } from "@meridian/ui";
import { useCallback, useEffect, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import { GithubImportDrawer, ZipImportDrawer } from "./import-drawers";
import { InstallDetail } from "./install-detail";
import { AgentSkillsTab } from "./agent-skills-tab";
import { RegistryTab } from "./registry-tab";
import { StagedApprovalsTab } from "./staged-approvals";

type InstallView = components["schemas"]["InstallView"];
type StagedItemView = components["schemas"]["StagedItemView"];
type SkillVersionView = components["schemas"]["SkillVersionView"];
type CatalogEntryView = components["schemas"]["CatalogEntryView"];

const TABS = ["Installed", "Registry", "Approvals", "Agent skills"] as const;
type Tab = (typeof TABS)[number];

export default function PluginsSettingsPage() {
  const auth = useRequireAuth();
  const [tab, setTab] = useState<Tab>("Installed");
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
      <div>
        <h1 className="text-lg font-semibold">Plugins &amp; agent skills</h1>
        <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Nothing executes that did not arrive digest-verified with provenance and pass
          admin consent. Installs start DISABLED; updates pin the old version until the
          consent diff is approved.
        </p>
        {canManage ? null : (
          <p className="mt-1 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
            Lifecycle controls (consent, enable, settings, removal) are owner/admin — you can
            still stage imports and follow your own queue rows.
          </p>
        )}
      </div>

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
                className="rounded-[2px] px-3 py-1.5 text-[13px] tab-button"
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

          {tab === "Installed" ? (
            <div className="grid gap-3">
              {installs.length === 0 ? (
                <EmptyState message="No plugins installed yet. Import from GitHub or a zip, or stage a registry document — nothing runs without consent." />
              ) : (
                installs.map((install) => (
                  <Card key={install.id}>
                    <div className="flex flex-wrap items-center gap-2 p-4 pb-1">
                      <span className="text-sm font-semibold">{install.name}</span>
                      <span className="font-mono text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                        {install.slug}@{install.version}
                      </span>
                      <Tag tone={install.status === "enabled" ? "success" : install.status === "disabled" ? "warning" : "danger"}>
                        {install.status}
                      </Tag>
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
          ) : tab === "Registry" ? (
            <RegistryTab catalog={catalog} installs={installs} canManage={canManage} onChanged={() => void refresh()} />
          ) : tab === "Approvals" ? (
            <StagedApprovalsTab staged={staged} canManage={canManage} onChanged={() => void refresh()} />
          ) : (
            <AgentSkillsTab skills={skills} canManage={canManage} onChanged={() => void refresh()} />
          )}
        </>
      )}

      <GithubImportDrawer open={githubOpen} onOpenChange={setGithubOpen} onStaged={() => { setTab("Approvals"); void refresh(); }} />
      <ZipImportDrawer open={zipOpen} onOpenChange={setZipOpen} onStaged={() => { setTab("Approvals"); void refresh(); }} />
    </div>
  );
}
