// /settings/plugins — shared capability-consent rendering (ADR 0009).
//
// Purpose: ONE consent surface shape consumed by every source kind —
// staged-member approval (registry/github/zip/skill) and the
// update-reconsent/v2 update-proposal — per the lane-7 invariant that the
// capability-consent screen component is reused across all sources.
// Renders the provenance chain summary, the capability diff (added /
// removed / unchanged) or a plain declared list, and the declared
// settings schema JSON-Schema source.
//
// Non-purpose: no mutation wiring here — callers own approve/refuse/
// consent buttons and the wire calls.
"use client";

import type { components } from "@meridian/sdk";
import { Tag } from "@meridian/ui";

type ConsentView = components["schemas"]["ConsentView"];
type ConsentDiffView = components["schemas"]["ConsentDiffView"];

const SOURCE_KIND_LABEL: Record<string, string> = {
  marketplace_registry: "Marketplace registry",
  github_repo: "GitHub repository",
  local_zip: "Local zip upload",
  skill_community: "Community skill source",
  manual_upload: "Manual upload",
};

function digestShort(digest: string): string {
  return digest.length > 18 ? `${digest.slice(0, 10)}…${digest.slice(-6)}` : digest;
}

/** Provenance chain summary — the ONE shape every source kind records. */
export function ProvenanceSummary({
  artifactKind,
  sourceKind,
  sourceUrl,
  fetchedRef,
  digest,
  uploader,
}: {
  artifactKind: string;
  sourceKind: string;
  sourceUrl: string;
  fetchedRef?: string | null;
  digest: string;
  uploader?: string | null;
}) {
  const rows: [string, string][] = [
    ["Origin", SOURCE_KIND_LABEL[sourceKind] ?? sourceKind],
    ["Source", sourceUrl],
    ["Fetched ref", fetchedRef ?? "—"],
    ["Digest", digestShort(digest)],
    ["Uploader", uploader ?? "—"],
  ];
  return (
    <dl className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
      <p className="text-[11px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
        Provenance chain — {artifactKind}
      </p>
      {rows.map(([k, v]) => (
        <div key={k} className="mt-1 flex gap-2" data-testid={`provenance-${k.toLowerCase()}`}>
          <dt className="w-24 shrink-0">{k}</dt>
          <dd
            data-testid={k === "Digest" ? "digest-value" : undefined}
            className={k === "Digest" ? "font-mono text-[12px]" : "truncate"}
          >
            {k === "Digest" ? digestShort(digest) : v}
          </dd>
        </div>
      ))}
    </dl>
  );
}

/** Renders the declared settings JSON-Schema (the consent screen shows
 * the config surface an approver is consenting to). Renders as raw JSON
 * when the manifest declared anything outside the 0079 subset. */
export function SettingsSchemaBlock({ schema }: { schema?: unknown }) {
  return (
    <div>
      <p className="text-[11px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
        Declared settings schema
      </p>
      <pre
        className="mt-1 max-h-40 overflow-auto rounded-[var(--m-radius-container)] border p-2 text-[12px] font-mono"
        style={{ borderColor: "var(--m-border)", background: "var(--m-canvas)" }}
        data-testid="settings-schema-block"
      >
        {schema === null || schema === undefined ? "(none declared)" : JSON.stringify(schema, null, 2)}
      </pre>
    </div>
  );
}

function CapabilityList({
  title,
  variations,
  values,
  tone,
}: {
  title: string;
  variations: ("+" | "−" | "…" | undefined)[];
  values: string[];
  tone: "success" | "danger" | "outline";
}) {
  if (values.length === 0) return null;
  return (
    <div>
      <p className="text-[11px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
        {title}
      </p>
      <ul className="mt-1 flex flex-wrap gap-1.5">
        {values.map((value, index) => (
          <li key={`${value}-${index}`} className="flex items-center">
            <Tag tone={tone}>
              {variations[index] === "+" ? "+ " : variations[index] === "−" ? "− " : ""}
              {value}
            </Tag>
          </li>
        ))}
      </ul>
    </div>
  );
}

/**
 * The consent surface itself: provenance chain + capability consent +
 * declared settings schema. `diff` present ⇒ renders Added / Removed /
 * Unchanged (update re-consent); otherwise the declared list for a
 * first consent. `grant` toggles render when the reviewer owes a choice
 * (staged approval: a subsetting the server accepts as granted set).
 */
export function ConsentSummary({
  artifact,
  diff,
  settingsSchema,
  grants,
  onToggleGrant,
}: {
  artifact: ConsentView & { uploaded_by?: string };
  diff?: ConsentDiffView | null;
  settingsSchema?: unknown;
  grants?: string[] | null;
  onToggleGrant?: (capability: string) => void;
}) {
  const hasDiff = diff !== undefined && diff !== null;
  return (
    <div className="grid gap-3">
      <ProvenanceSummary
        artifactKind={artifact.artifact_kind}
        sourceKind={artifact.source_kind}
        sourceUrl={artifact.source_url}
        fetchedRef={artifact.fetched_ref}
        digest={artifact.artifact_digest}
        uploader={artifact.uploaded_by ?? null}
      />
      <div className="grid gap-3">
        {hasDiff ? (
          <>
            <CapabilityList
              title="Added capabilities"
              variations={diff!.added.map(() => "+" as const)}
              values={diff!.added}
              tone="success"
            />
            <CapabilityList
              title="Removed capabilities"
              variations={diff!.removed.map(() => "−" as const)}
              values={diff!.removed}
              tone="danger"
            />
            <CapabilityList
              title="Unchanged capabilities"
              variations={diff!.unchanged.map(() => "…" as const)}
              values={diff!.unchanged}
              tone="outline"
            />
          </>
        ) : (
          <CapabilityList
            title="Declared capabilities"
            variations={artifact.declared_capabilities.map(() => undefined)}
            values={artifact.declared_capabilities}
            tone="outline"
          />
        )}
        {onToggleGrant !== undefined && !hasDiff ? (
          <div>
            <p className="text-[11px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Grant capabilities to the install
            </p>
            <ul className="mt-1 grid gap-1">
              {artifact.declared_capabilities.map((capability) => (
                <li key={capability}>
                  <label className="flex items-center gap-2 text-[13px]">
                    <input
                      type="checkbox"
                      checked={(grants ?? []).includes(capability)}
                      onChange={() => onToggleGrant!(capability)}
                      aria-label={`Grant ${capability}`}
                    />
                    <span className="font-mono text-[12px]">{capability}</span>
                  </label>
                </li>
              ))}
            </ul>
          </div>
        ) : hasDiff ? null : (
          <CapabilityList
            title="Granted capabilities (what the install gets)"
            variations={[]}
            values={grants ?? []}
            tone="success"
          />
        )}
        <SettingsSchemaBlock schema={settingsSchema} />
      </div>
    </div>
  );
}
