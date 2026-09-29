// /settings/plugins — agent-skills tab (ADR 0009 skills half).
//
// Purpose: community-source pulls + manual uploads (both land PENDING in
// the same staged-approvals queue), the versioned skill list with
// per-version provenance inline, and the deny-by-default enable gate on
// the ACTIVE version row only (owner/admin explicit action).
//
// Non-purpose: no skill content editing here — the content is
// untrusted RAG-class data consumed by the rules layers, never executed;
// the legacy agent-platform page still owns publishing flows.
"use client";

import type { components } from "@marula-core/sdk";
import { Button, Card, EmptyState, Input, Label, Spinner, Tag } from "@marula-core/ui";
import { useState } from "react";
import { api } from "../../../lib/api";
import { envelopeOf, reasonCopy } from "../../../lib/error-copy";

type SkillVersionView = components["schemas"]["SkillVersionView"];

interface SkillGroup {
  slug: string;
  active: SkillVersionView;
  versions: SkillVersionView[];
}

function groupSkills(rows: SkillVersionView[]): SkillGroup[] {
  const bySlug = new Map<string, SkillVersionView[]>();
  for (const row of rows) {
    const list = bySlug.get(row.slug) ?? [];
    list.push(row);
    bySlug.set(row.slug, list);
  }
  const groups = [...bySlug.entries()].map(([slug, versions]) => {
    const sorted = versions.slice().sort((a, b) => b.version - a.version);
    let active = sorted[0];
    for (const row of sorted) if (row.version > (active as SkillVersionView).version) active = row;
    return { slug, active: active as SkillVersionView, versions: sorted };
  });
  groups.sort((a, b) => a.slug.localeCompare(b.slug));
  return groups;
}

function versionRows(group: SkillGroup): SkillVersionView[] {
  return group.versions;
}

export function AgentSkillsTab({
  skills,
  canManage,
  onChanged,
}: {
  skills: SkillVersionView[] | null;
  canManage: boolean;
  onChanged: () => void;
}) {
  const [communityUrl, setCommunityUrl] = useState("");
  const [communityRef, setCommunityRef] = useState("");
  const [communityDigest, setCommunityDigest] = useState("");
  const [manualManifest, setManualManifest] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [openSlug, setOpenSlug] = useState<string | null>(null);
  const [toggling, setToggling] = useState<string | null>(null);

  const digestProblem =
    communityDigest.trim() === ""
      ? "A sha256 pin is required — community pulls are digest-verified against the claim."
      : /^[0-9a-f]{64}$/i.test(communityDigest.trim())
        ? null
        : "The digest must be the full 64-hex sha256.";

  const stageCommunity = async () => {
    setBusy(true);
    setError(null);
    const { error } = await api.api.POST("/api/v1/plugins/distribution/staged/skill", {
      body: {
        source_kind: "skill_community",
        source_url: communityUrl.trim(),
        fetched_ref: communityRef.trim() === "" ? null : communityRef.trim(),
        artifact_digest: communityDigest.trim(),
      },
    });
    setBusy(false);
    if (error !== undefined) {
      const envelope = envelopeOf(error);
      setError(reasonCopy(envelope.reason ?? "validation_failed", envelope.details));
      return;
    }
    setCommunityUrl("");
    setCommunityRef("");
    setCommunityDigest("");
    setNotice("Community pull staged — it waits in the Approvals queue as PENDING.");
    onChanged();
  };

  const stageManual = async () => {
    let manifest: unknown;
    try {
      manifest = JSON.parse(manualManifest) as unknown;
    } catch {
      setError("The manifest must be valid JSON covering the skill manifest shape.");
      return;
    }
    if (manifest === null || typeof manifest !== "object" || Array.isArray(manifest)) {
      setError("The manifest must be a JSON object.");
      return;
    }
    setBusy(true);
    setError(null);
    const { error } = await api.api.POST("/api/v1/plugins/distribution/staged/skill", {
      body: {
        source_kind: "manual_upload",
        source_url: "upload://manual",
        manifest,
      },
    });
    setBusy(false);
    if (error !== undefined) {
      const envelope = envelopeOf(error);
      setError(reasonCopy(envelope.reason ?? "validation_failed", envelope.details));
      return;
    }
    setManualManifest("");
    setNotice("Upload staged — the server computes the digest. Waiting in the Approvals queue.");
    onChanged();
  };

  const toggleEnabled = async (skill: SkillVersionView, next: boolean) => {
    setToggling(skill.id);
    setError(null);
    const { error } = await api.api.POST("/api/v1/plugins/distribution/agent-skills/{id}/enabled", {
      params: { path: { id: skill.id } },
      body: { review_note: next ? "Enabled via Settings → Plugins → Agent skills." : "Disabled via Settings → Plugins → Agent skills." },
    });
    setToggling(null);
    if (error !== undefined) {
      const envelope = envelopeOf(error);
      setError(reasonCopy(envelope.reason ?? "forbidden", envelope.details));
      return;
    }
    onChanged();
  };

  const groups = skills === null ? null : groupSkills(skills);

  return (
    <div className="grid gap-3">
      <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
        Skills are data: versioned, roll-back-able, never executed as instructions.
        Imported skills (community sources or manual uploads) land PENDING in the
        staged-approvals queue; the active version of every skill is DISABLED until an
        owner/admin explicitly enables it.
      </p>

      {error !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }} data-testid="skills-error">
          {error}
        </p>
      ) : null}
      {notice !== null ? (
        <p role="status" className="text-[13px]" style={{ color: "var(--m-text-secondary)" }} data-testid="skills-notice">
          {notice}
        </p>
      ) : null}

      {groups === null ? (
        <Spinner />
      ) : groups.length === 0 ? (
        <EmptyState message="No agent skills recorded yet — import from a community source or upload a manifest below." />
      ) : (
        <div className="grid gap-3">
          {groups.map((group) => (
            <Card key={group.slug}>
              <div className="flex flex-wrap items-center gap-2 p-4 pb-1">
                <span className="text-sm font-semibold">{group.slug}</span>
                <Tag tone={group.active.enabled ? "success" : "neutral"}>
                  {group.active.enabled ? "enabled" : "disabled"}
                </Tag>
                <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                  active v{group.active.version} · {group.active.source_kind}
                </span>
                {canManage ? (
                  <Button
                    size="sm"
                    variant={group.active.enabled ? "secondary" : "primary"}
                    className="ml-auto"
                    disabled={toggling === group.active.id}
                    onClick={() => void toggleEnabled(group.active, !group.active.enabled)}
                    data-testid={`skill-toggle-${group.slug}`}
                  >
                    {toggling === group.active.id ? <Spinner /> : null}
                    {group.active.enabled ? "Disable" : "Enable"}
                  </Button>
                ) : (
                  <span className="ml-auto text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                    Deny-by-default — enable needs owner/admin.
                  </span>
                )}
              </div>
              <div className="px-4 pb-4">
                <div className="mt-1">
                  <Button size="sm" variant="ghost" onClick={() => setOpenSlug(openSlug === group.slug ? null : group.slug)} aria-expanded={openSlug === group.slug}>
                    {openSlug === group.slug ? "Hide versions" : `Version history (${group.versions.length})`}
                  </Button>
                </div>
                {openSlug === group.slug ? (
                  <ol className="mt-2 grid gap-2" data-testid={`skill-versions-${group.slug}`}>
                    {versionRows(group).map((row) => (
                      <li key={row.id} className="flex flex-wrap items-baseline gap-x-2 text-[12px]">
                        <Tag tone={row.version === group.active.version ? "outline" : "neutral"}>
                          v{row.version}
                        </Tag>
                        <span style={{ color: "var(--m-text-secondary)" }}>{row.source_kind}</span>
                        <span style={{ color: "var(--m-text-muted)" }}>{row.source_url}</span>
                        {row.fetched_ref !== null && row.fetched_ref !== undefined ? (
                          <span className="font-mono" style={{ color: "var(--m-text-muted)" }}>
                            {row.fetched_ref}
                          </span>
                        ) : null}
                        <span className="font-mono" style={{ color: "var(--m-text-muted)" }}>
                          {row.artifact_digest.slice(0, 12)}…
                        </span>
                        <span style={{ color: row.enabled ? "var(--m-status-success-fg)" : "var(--m-text-muted)" }}>
                          {row.enabled ? "enabled" : "disabled"}
                        </span>
                        <span style={{ color: "var(--m-text-muted)" }}>{new Date(row.created_at).toLocaleString()}</span>
                      </li>
                    ))}
                  </ol>
                ) : null}
              </div>
            </Card>
          ))}
        </div>
      )}

      <Card>
        <h3 className="p-4 pb-2 text-sm font-semibold">Import a community skill</h3>
        <div className="grid gap-3 p-4 pt-0" data-testid="community-skill-form">
          <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
            Free/open community marketplaces and repo APIs only. The sha256 claim is
            re-verified by refetching the manifest and checking the digest.
          </p>
          <div>
            <Label htmlFor="community-url">Source URL</Label>
            <Input id="community-url" placeholder="https://skills.example.dev/classifier/manifest.json" value={communityUrl} onChange={(e) => setCommunityUrl(e.target.value)} />
          </div>
          <div>
            <Label htmlFor="community-ref">Ref (tag / commit)</Label>
            <Input id="community-ref" placeholder="v2.1.0" value={communityRef} onChange={(e) => setCommunityRef(e.target.value)} />
          </div>
          <div>
            <Label htmlFor="community-digest">sha256 digest of the manifest</Label>
            <Input
              id="community-digest"
              placeholder="64-hex sha256"
              value={communityDigest}
              onChange={(e) => setCommunityDigest(e.target.value)}
              data-testid="community-digest-input"
            />
            {digestProblem !== null ? (
              <p className="mt-1 text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {digestProblem}
              </p>
            ) : null}
          </div>
          <Button variant="secondary" disabled={busy || digestProblem !== null || communityUrl.trim() === ""} onClick={() => void stageCommunity()}>
            {busy ? <Spinner /> : null} Stage skill import
          </Button>
        </div>
      </Card>

      <Card>
        <h3 className="p-4 pb-2 text-sm font-semibold">Upload a skill manifest</h3>
        <div className="grid gap-3 p-4 pt-0" data-testid="manual-skill-form">
          <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
            Manual uploads are digest-computed server-side; content is untrusted data
            (RAG-class). The upload lands PENDING — nothing executes without admin consent.
          </p>
          <textarea
            className="min-h-40 rounded-[var(--m-radius-container)] border p-2 text-[12px] font-mono"
            style={{ borderColor: "var(--m-border)", background: "var(--m-canvas)" }}
            aria-label="Skill manifest JSON"
            placeholder='{"name":"ticket-triage","version":"1.0.0","description":"…"}'
            value={manualManifest}
            onChange={(e) => setManualManifest(e.target.value)}
            data-testid="manual-skill-manifest"
          />
          <Button variant="secondary" disabled={busy || manualManifest.trim() === ""} onClick={() => void stageManual()}>
            {busy ? <Spinner /> : null} Stage manual upload
          </Button>
        </div>
      </Card>
    </div>
  );
}
