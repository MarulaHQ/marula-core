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

import type { components } from "@marula-core/sdk";
import {
  Badge,
} from "@marula-core/ui";

type ConsentView = components["schemas"]["ConsentView"];
type ConsentDiffView = components["schemas"]["ConsentDiffView"];

const SOURCE_KIND_LABEL: Record<string, string> = {
  marketplace_registry: "Marketplace registry",
  github_repo: "GitHub repository",
  local_zip: "Local zip upload",
  skill_community: "Community skill source",
  manual_upload: "Manual upload",
};

/** Sources whose catalog carries a publisher signature block. A local zip or a
 * manual skill upload makes no publisher claim at all, so the manifest-coverage
 * label renders only where such a claim exists. */
const SIGNED_SOURCES = new Set(["marketplace_registry", "github_repo"]);

/** Whether the publisher's signature covers the manifest the approver reads, or
 * only the artifact bytes beside it (record
 * `plugin-distribution.registry-signature-binds-digest-only`). The weaker claim
 * is shown, not hidden: a legacy digest-only entry and a zip import both land
 * here as unsigned. */
export function ManifestCoverageBadge({
  authenticated,
  sourceKind,
}: {
  authenticated: boolean;
  /** Omitted where the surface knows the verdict but not the source it came
   * from (the install row); the badge then renders unconditionally. */
  sourceKind?: string;
}) {
  if (sourceKind !== undefined && !SIGNED_SOURCES.has(sourceKind)) return null;
  return (
    <Badge
      tone={authenticated ? "success" : "warning"}
      data-testid="manifest-coverage"
    >
      {authenticated ? "manifest signed" : "manifest unsigned"}
    </Badge>
  );
}

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
  manifestAuthenticated,
}: {
  artifactKind: string;
  sourceKind: string;
  sourceUrl: string;
  fetchedRef?: string | null;
  digest: string;
  uploader?: string | null;
  /** Present only for sources that make a signature claim. */
  manifestAuthenticated?: boolean;
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
      {manifestAuthenticated !== undefined ? (
        <div className="mt-2">
          <ManifestAuthenticationNote
            authenticated={manifestAuthenticated}
            sourceKind={sourceKind}
          />
        </div>
      ) : null}
    </dl>
  );
}

/** The verdict and its sentence, ONE rendering for every surface that shows
 * it — the staged-approval row, the consent screen, and the install row
 * (`F49`: an install from a GitHub pull is staged unsigned and has to keep
 * saying so after it is approved). Keeping the copy here is what stops the
 * three surfaces growing three vocabularies for the same fact.
 *
 * `sourceKind` is optional because the install row carries a verdict without
 * carrying the source it came from; the sources that make no signature claim
 * at all still render nothing. */
export function ManifestAuthenticationNote({
  authenticated,
  sourceKind,
}: {
  authenticated: boolean;
  sourceKind?: string;
}) {
  if (sourceKind !== undefined && !SIGNED_SOURCES.has(sourceKind)) return null;
  return (
    <div className="flex flex-wrap items-center gap-2">
      <ManifestCoverageBadge authenticated={authenticated} sourceKind={sourceKind} />
      <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
        {authenticated
          ? "The publisher's signature covers this manifest."
          : "The signature covers only the artifact bytes — read the manifest as text the publisher has not signed."}
      </span>
    </div>
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
  tone: "success" | "danger" | "outline" | "neutral";
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
            <Badge tone={tone}>
              {variations[index] === "+" ? "+ " : variations[index] === "−" ? "− " : ""}
              {value}
            </Badge>
          </li>
        ))}
      </ul>
    </div>
  );
}

/**
 * ADR 0009 §6 full semantics — the modeled connector governance profile
 * rendered VERBATIM on the consent screen: the webhook endpoints the
 * connector declares address the platform's egress posture; the scopes
 * are exactly the SaaS authorizations the admin is consenting to. Absent
 * connector profiles render nothing (an absent profile is the valid
 * default posture for connectors without webhooks).
 */
export function ConnectorProfileBlock({
  connectorProfile,
}: {
  connectorProfile?: unknown;
}) {
  const profile = connectorProfile as
    | { webhook_endpoints?: string[]; scopes?: string[] }
    | null
    | undefined;
  if (!profile) return null;
  const endpoints = profile.webhook_endpoints ?? [];
  const scopes = profile.scopes ?? [];
  if (endpoints.length === 0 && scopes.length === 0) return null;
  return (
    <div className="rounded-md border border-dashed p-3" style={{ borderColor: "var(--m-border)" }}>
      <p
        className="text-[11px] uppercase tracking-wide"
        style={{ color: "var(--m-text-muted)" }}
      >
        Connector profile
      </p>
      <ul className="mt-2 grid gap-1">
        {endpoints.map((endpoint) => (
          <li key={endpoint} className="font-mono text-[12px]">
            webhook → {endpoint}
          </li>
        ))}
        {scopes.map((scope) => (
          <li key={scope} className="font-mono text-[12px]">
            scope → {scope}
          </li>
        ))}
      </ul>
      <p className="mt-2 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
        Declared webhook endpoints seed the connector&apos;s outbound posture;
        scopes are the upstream SaaS permissions the connector asks for.
        Consenting approves exactly what is declared above.
      </p>
    </div>
  );
}

/** ADR 0006 leg (d) — the ciphertext-only vault:ingress consent note:
 * the ONE special warning a capability key can carry (the core never
 * accepts plaintext secrets from plugin surfaces).
 *
 * Wave 32 lane 7 adds the routing note: routing packs read scheduling
 * context and never write it — the consent surface states that plainly. */
const CAPABILITY_NOTES: Record<string, string> = {
  "vault:ingress":
    "Ciphertext-only ingress — the plugin may submit ENCRYPTED payloads into a scoped vault folder. The core never accepts plaintext secrets, and the folder's existing RBAC grants still gate reads.",
  "routing.matrix":
    "Routing provider — reads scheduling context (coordinates, times) and returns travel durations and distances. Routing scopes resolve to the scheduling VIEW gate and never confer scheduling edits.",
  "ui.widgets":
    "Widgets — the plugin may register dashboard and ticket-pane widgets. Widgets are declarative cards rendered by Marula's own components; the plugin ships no code the browser runs.",
  "automation.actions":
    "Automation actions — the plugin may register action kinds for the automations editor. Each action runs a consented built-in effect (such as adding an internal comment) with typed parameters.",
  "automation.triggers":
    "Automation triggers — the plugin may declare event types it emits, so a rule can fire on them. Each declared type is checked against the event registry at grant time; the trigger is a declaration, not code the engine runs raw.",
  "automation.reads":
    "Read nodes — the plugin may fetch EXTERNAL data through credentials it already holds, under declared caps: a per-run call ceiling, a minimum interval between this org's reads, a response-size ceiling, and never a vault secret.",
  "reports.render":
    "Report renders — the plugin may contribute render kinds to the report builder. Rendering stays in Marula's trusted component set.",
  "fields.validate":
    "Field validators — the plugin may contribute JSON-Schema validators for board custom fields. Marula runs the checks; the plugin never sees field values.",
};

/** The capability notes block — prints the vault:ingress posture under
 * the capability list when the declared set carries it (minimal consent
 * surface; no other UI work rides this lane). */
export function CapabilityNotes({ capabilities }: { capabilities: string[] | null | undefined }) {
  const notes = (capabilities ?? [])
    .map((c) => CAPABILITY_NOTES[c])
    .filter((n): n is string => n !== undefined);
  if (notes.length === 0) return null;
  return (
    <div className="rounded-md border border-dashed p-3" style={{ borderColor: "var(--m-border)" }}>
      {notes.map((note, i) => (
        <p key={i} className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
          {note}
        </p>
      ))}
    </div>
  );
}

/** §4.6 (Wave 34 lane 4) — the declarative UI-surface profile rendered
 * VERBATIM on consent: the widgets, automation action kinds, report
 * render kinds, and field validators the plugin contributes. Absent or
 * empty profiles render nothing (a plugin with no surface declarations
 * gets the baseline consent only). */
export function SurfaceProfileBlock({ surfaceProfile }: { surfaceProfile?: unknown }) {
  const profile = surfaceProfile as
    | {
        widgets?: { id: string; title: string; slot: string; kind: string }[];
        actions?: { id: string; label: string; description: string; effect: string }[];
        render_kinds?: { id: string; label: string; description: string }[];
        field_validators?: { id: string; label: string }[];
        triggers?: {
          id: string;
          label: string;
          description: string;
          emitted_event_type: string;
          payload_schema?: unknown;
        }[];
        reads?: {
          id: string;
          label: string;
          description: string;
          guardrails?: {
            max_calls_per_run?: number;
            min_interval_secs?: number;
            max_output_bytes?: number;
            may_return_secrets?: boolean;
          };
        }[];
      }
    | null
    | undefined;
  if (!profile) return null;
  const sections: [string, string[]][] = [
    ["Widgets", (profile.widgets ?? []).map((w) => `${w.title} — ${w.kind} on ${w.slot.replace("_", " ")}`)],
    ["Automation actions", (profile.actions ?? []).map((a) => `${a.label} — runs "${a.effect}"`)],
    ["Report render kinds", (profile.render_kinds ?? []).map((r) => `${r.label} — ${r.description}`)],
    ["Field validators", (profile.field_validators ?? []).map((v) => v.label)],
  ];
  // Wave 43 lane 5: TRIGGER surfaces (lane 4) and the event types they emit,
  // shown beside the UI surfaces above so an approver sees the full surface
  // the plugin contributes — including the half a rule can fire on.
  const triggers = profile.triggers ?? [];
  const emittedTypes = [...new Set(triggers.map((t) => t.emitted_event_type))];
  if (triggers.length > 0) {
    sections.push([
      "Automation triggers",
      triggers.map((t) => `${t.label} — ${t.description} · emits “${t.emitted_event_type}”`),
    ]);
  }
  // Read nodes (lane 4, ADR 0017 §5) — the caps are part of what consent
  // approves, so they render with the declaration, not beside it.
  const reads = profile.reads ?? [];
  if (reads.length > 0) {
    sections.push([
      "Read nodes",
      reads.map((r) => {
        const g = r.guardrails;
        const caps =
          g === undefined
            ? "guardrails not declared"
            : `≤${g.max_calls_per_run ?? "?"} calls/run · ≥${g.min_interval_secs ?? "?"}s apart · ≤${g.max_output_bytes ?? "?"} bytes back · secrets ${g.may_return_secrets === true ? "allowed" : "never"}`;
        return `${r.label} — ${r.description} (${caps})`;
      }),
    ]);
  }
  const total = sections.reduce((n, [, rows]) => n + rows.length, 0);
  if (total === 0) return null;
  return (
    <div className="rounded-md border border-dashed p-3" style={{ borderColor: "var(--m-border)" }}>
      <p className="text-[11px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
        Declared surfaces
      </p>
      <ul className="mt-2 grid gap-1">
        {sections
          .filter(([, rows]) => rows.length > 0)
          .map(([name, rows]) => (
            <li key={name} className="text-[12px]">
              <span style={{ color: "var(--m-text-secondary)" }}>{name}:</span>{" "}
              {rows.join("; ")}
            </li>
          ))}
      </ul>
      {emittedTypes.length > 0 ? (
        <p className="mt-2 text-[12px]" style={{ color: "var(--m-text-secondary)" }} data-testid="emitted-event-types">
          Emitted event types: {emittedTypes.map((t) => (
            <code key={t} className="mr-2">{t}</code>
          ))}
        </p>
      ) : null}
      <p className="mt-2 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
        Declarative registrations only — widgets render through Marula&apos;s
        own components, automation actions run consented built-in effects,
        triggers name the event types the plugin publishes for rules to fire
        on, and validators are JSON-Schema checks. The plugin contributes no
        executable code to these surfaces.
      </p>
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
  connectorProfile,
  surfaceProfile,
  grants,
  onToggleGrant,
}: {
  artifact: ConsentView & { uploaded_by?: string };
  diff?: ConsentDiffView | null;
  settingsSchema?: unknown;
  connectorProfile?: unknown;
  surfaceProfile?: unknown;
  grants?: string[] | null;
  onToggleGrant?: (capability: string) => void;
}) {
  const hasDiff = diff !== undefined && diff !== null;
  const noteCapabilities = hasDiff
    ? [...diff!.added].concat(diff!.unchanged)
    : artifact.declared_capabilities;
  return (
    <div className="grid gap-3">
      <ProvenanceSummary
        artifactKind={artifact.artifact_kind}
        sourceKind={artifact.source_kind}
        sourceUrl={artifact.source_url}
        fetchedRef={artifact.fetched_ref}
        digest={artifact.artifact_digest}
        uploader={artifact.uploaded_by ?? null}
        manifestAuthenticated={
          SIGNED_SOURCES.has(artifact.source_kind) ? artifact.manifest_authenticated : undefined
        }
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
              tone="neutral"
            />
          </>
        ) : (
          <CapabilityList
            title="Declared capabilities"
            variations={artifact.declared_capabilities.map(() => undefined)}
            values={artifact.declared_capabilities}
            tone="neutral"
          />
        )}
        <CapabilityNotes capabilities={noteCapabilities} />
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
        <ConnectorProfileBlock connectorProfile={connectorProfile} />
        <SurfaceProfileBlock surfaceProfile={surfaceProfile} />
        <SettingsSchemaBlock schema={settingsSchema} />
      </div>
    </div>
  );
}
