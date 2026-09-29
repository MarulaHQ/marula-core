// /settings/self-host — self-hoster visibility into the ops stack (Wave 36 lane 3).
//
// Purpose: one owner/admin pane answering the self-hoster's real question —
// "does the operator ever see the stack?" It renders the API's stack sweep
// (each loop's existing /healthz), the ClamAV scan state with the recently
// quarantined messages, which GlitchTip DSNs are configured (presence only,
// never a value), external-UI link cards (Grafana, Tempo, Loki, collector
// metrics) and the Garage CLI-first facts. Read-only by design.
//
// The payload is fetched with a manual authenticated request instead of the
// typed SDK: this route ships with the lane, and the OpenAPI pin/SDK
// regeneration runs in the lane's gated verification sequence
// (scripts/regen-openapi-pins.sh). The local interfaces below mirror the
// server DTOs in crates/core-api/src/routes/self_host.rs.
//
// Non-purpose: no scanner probing (the raw clamd TCP PING stays a documented
// CLI probe), no secret values, no per-service status machinery.
"use client";

import { Button, Card, Spinner, Tag } from "@marula-core/ui";
import { useCallback, useEffect, useState } from "react";
import {
  apiBaseUrl,
  authHeaders,
  useRequireAuth,
} from "../../../lib/api";
import {
  NotAvailable,
  SettingsHeader,
} from "../../../components/settings/settings-shell";

type ServiceState = "up" | "down" | "not_configured" | "unknown";

interface ServiceView {
  id: string;
  label: string;
  state: ServiceState;
  detail: string | null;
  loops: Record<string, unknown> | null;
}

interface DsnRow {
  service: string;
  state: "configured" | "unset" | "unknown";
  detail: string | null;
}

interface DetectionFile {
  filename: string;
  content_type: string;
  size_bytes: number;
  sha256: string;
}

interface DetectionView {
  message_id: string;
  subject: string | null;
  from_address: string;
  verdict: string;
  ingest_source: string;
  occurred_at: string;
  attachments: DetectionFile[];
}

interface ScanView {
  provider: string;
  clamd_addr: string | null;
  last_scan_at: string | null;
  last_detection_at: string | null;
  detection_count: number;
}

interface LinksView {
  glitchtip: string | null;
  grafana: string | null;
  tempo: string | null;
  loki: string | null;
  otel_metrics: string | null;
}

interface GarageView {
  volumes: string[];
  container_paths: string[];
  bootstrap_commands: string[];
  admin_api_docs_url: string;
  s3_endpoint: string | null;
  s3_bucket: string | null;
}

interface SelfHostView {
  scan: ScanView;
  detections: DetectionView[];
  services: ServiceView[];
  glitchtip_dsns: DsnRow[];
  links: LinksView;
  garage: GarageView;
}

function stateTone(state: ServiceState): "success" | "danger" | "neutral" {
  if (state === "up") return "success";
  if (state === "down") return "danger";
  return "neutral";
}

function stateWord(state: ServiceState): string {
  switch (state) {
    case "up":
      return "up";
    case "down":
      return "down";
    case "not_configured":
      return "not configured";
    default:
      return "unknown";
  }
}

function dotColor(state: ServiceState): string {
  if (state === "up") return "var(--m-status-success-fg)";
  if (state === "down") return "var(--m-status-danger-fg)";
  return "var(--m-text-muted)";
}

function StateDot({ state }: { state: ServiceState }) {
  return (
    <span
      aria-hidden="true"
      className="inline-block h-2 w-2 shrink-0 rounded-full"
      style={{ background: dotColor(state) }}
    />
  );
}

function when(iso: string | null): string {
  if (iso === null) return "never";
  const parsed = new Date(iso);
  return Number.isNaN(parsed.getTime()) ? iso : parsed.toLocaleString();
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

/** One read-only label/value row inside a card. */
function FactRow({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: React.ReactNode;
  mono?: boolean;
}) {
  return (
    <div
      className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 border-b py-2 last:border-b-0"
      style={{ borderColor: "var(--m-border)" }}
    >
      <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
        {label}
      </span>
      <span
        className={mono ? "text-xs font-mono" : "text-[13px]"}
        style={{ color: "var(--m-text)" }}
      >
        {value}
      </span>
    </div>
  );
}

/** One external-tool link card: link when configured, honest state when not. */
function LinkCard({
  title,
  description,
  url,
  envVar,
  openLabel,
}: {
  title: string;
  description: string;
  url: string | null;
  envVar: string;
  openLabel: string;
}) {
  return (
    <Card className="p-4 flex flex-col gap-2">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
          {title}
        </h3>
        <Tag tone={url !== null ? "success" : "outline"}>
          {url !== null ? "configured" : "not configured"}
        </Tag>
      </div>
      <p className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
        {description}
      </p>
      {url !== null ? (
        <a
          className="text-xs underline underline-offset-2"
          style={{ color: "var(--m-accent)" }}
          href={url}
          target="_blank"
          rel="noreferrer"
        >
          {openLabel} ↗
        </a>
      ) : (
        <p className="text-xs font-mono" style={{ color: "var(--m-text-muted)" }}>
          Set {envVar} on core-api
        </p>
      )}
    </Card>
  );
}

export default function SelfHostSettingsPage() {
  const auth = useRequireAuth();
  const isOwnerOrAdmin =
    (auth.me?.permissions ?? []).includes("special:manage_users") ?? false;
  const [view, setView] = useState<SelfHostView | null>(null);
  const [denied, setDenied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadedAt, setLoadedAt] = useState<Date | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetch(`${apiBaseUrl()}/api/v1/settings/self-host`, {
        headers: authHeaders(),
        cache: "no-store",
      });
      if (res.status === 403) {
        setDenied(true);
        return;
      }
      if (!res.ok) {
        setError(`The stack report could not be loaded (${res.status}).`);
        return;
      }
      setView((await res.json()) as SelfHostView);
      setLoadedAt(new Date());
    } catch {
      setError("The stack report could not be loaded — check the connection and try again.");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (auth.status !== "authed" || !isOwnerOrAdmin) return;
    void load();
  }, [auth.status, isOwnerOrAdmin, load]);

  if (auth.status === "authed" && !isOwnerOrAdmin) {
    return (
      <div className="grid gap-6 max-w-3xl">
        <SettingsHeader
          title="Self-host services"
          description="Read-only visibility into this deployment's ops stack."
        />
        <NotAvailable
          what="Self-host services"
          hint="this page is limited to org owners and admins"
        />
      </div>
    );
  }

  const services = view?.services ?? [];
  const upCount = services.filter((s) => s.state === "up").length;
  const downCount = services.filter((s) => s.state === "down").length;
  const summary =
    services.length === 0
      ? "No checks reported yet"
      : downCount > 0
        ? `${downCount} ${downCount === 1 ? "check needs" : "checks need"} attention`
        : "Stack up — every configured check answered";

  return (
    <div className="grid gap-6 max-w-3xl">
      <SettingsHeader
        title="Self-host services"
        description="What this deployment is running, and where to look when something isn't. Read-only: probes and links come from the API process's environment."
      >
        <Button variant="secondary" onClick={() => void load()} disabled={loading}>
          {loading ? "Checking…" : "Refresh"}
        </Button>
      </SettingsHeader>

      {denied ? (
        <NotAvailable
          what="Self-host services"
          hint="this page is limited to org owners and admins"
        />
      ) : null}

      {error !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </p>
      ) : null}

      {view === null && !denied && error === null ? <Spinner /> : null}

      {view !== null ? (
        <>
          {/* --- health sweep ------------------------------------------------- */}
          <Card className="p-4 grid gap-2">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex items-center gap-2">
                <StateDot state={downCount > 0 ? "down" : "up"} />
                <h2 className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
                  {summary}
                </h2>
              </div>
              <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                {upCount}/{services.length} up
                {loadedAt !== null ? ` · checked ${loadedAt.toLocaleTimeString()}` : ""}
              </span>
            </div>
            {services.map((service) => (
              <div
                key={service.id}
                data-testid={`self-host-service-${service.id}`}
                className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1 border-b py-2 last:border-b-0"
                style={{ borderColor: "var(--m-border)" }}
              >
                <div className="flex items-center gap-2">
                  <StateDot state={service.state} />
                  <span className="text-[13px]" style={{ color: "var(--m-text)" }}>
                    {service.label}
                  </span>
                </div>
                <div className="flex items-center gap-3 min-w-0">
                  <span
                    className="text-[11px] font-mono truncate max-w-64"
                    title={service.detail ?? undefined}
                    style={{ color: "var(--m-text-muted)" }}
                  >
                    {service.detail ?? ""}
                  </span>
                  <Tag tone={stateTone(service.state)}>{stateWord(service.state)}</Tag>
                </div>
              </div>
            ))}
          </Card>

          {/* --- malware scanning + detections -------------------------------- */}
          <Card className="p-4 grid gap-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <h2 className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
                Malware scanning (ClamAV)
              </h2>
              <Tag tone={view.scan.provider === "none" ? "neutral" : "success"}>
                {view.scan.provider === "none" ? "none mode" : view.scan.provider}
              </Tag>
            </div>
            <div>
              <FactRow
                label="Scanner address"
                value={view.scan.clamd_addr ?? "—"}
                mono={view.scan.clamd_addr !== null}
              />
              <FactRow label="Last inbound scan" value={when(view.scan.last_scan_at)} />
              <FactRow label="Last quarantine" value={when(view.scan.last_detection_at)} />
              <FactRow label="Quarantined messages" value={String(view.scan.detection_count)} />
            </div>
            {view.scan.provider === "none" ? (
              <p className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                No scanner is wired (EMAIL_SCAN is unset), so inbound messages record a
                skipped scan and nothing is quarantined. The self-host how-to covers the
                stage2 ClamAV slot.
              </p>
            ) : null}
            <div className="grid gap-2">
              <h3 className="text-xs font-semibold" style={{ color: "var(--m-text-secondary)" }}>
                Recently quarantined messages
              </h3>
              {view.detections.length === 0 ? (
                <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                  Nothing is quarantined in this organization&rsquo;s ledger.
                </p>
              ) : (
                view.detections.map((detection) => (
                  <div
                    key={detection.message_id}
                    data-testid={`self-host-detection-${detection.message_id}`}
                    className="border-b pb-2 last:border-b-0 grid gap-1"
                    style={{ borderColor: "var(--m-border)" }}
                  >
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      <span className="text-[13px]" style={{ color: "var(--m-text)" }}>
                        {detection.subject ?? "(no subject)"}
                      </span>
                      <div className="flex items-center gap-2">
                        <Tag tone="danger">quarantined</Tag>
                        <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                          {when(detection.occurred_at)}
                        </span>
                      </div>
                    </div>
                    <span className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                      from {detection.from_address} · ingested via{" "}
                      <span className="font-mono">{detection.ingest_source}</span>
                    </span>
                    {detection.attachments.length > 0 ? (
                      <span className="text-[11px] font-mono" style={{ color: "var(--m-text-muted)" }}>
                        {detection.attachments
                          .map((f) => `${f.filename} (${formatBytes(f.size_bytes)})`)
                          .join(" · ")}
                      </span>
                    ) : null}
                  </div>
                ))
              )}
              <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                The scanner returns one verdict per message. It covers the body and every
                attachment byte, so these rows show what was quarantined with the files
                that rode it.
              </p>
            </div>
          </Card>

          {/* --- external tools ------------------------------------------------ */}
          <section className="grid gap-3">
            <h2 className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
              External tools
            </h2>
            <div className="grid gap-3 sm:grid-cols-2">
              <Card className="p-4 grid gap-2">
                <div className="flex items-center justify-between gap-2">
                  <h3 className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
                    GlitchTip
                  </h3>
                  <Tag tone={view.links.glitchtip !== null ? "success" : "outline"}>
                    {view.links.glitchtip !== null ? "configured" : "not configured"}
                  </Tag>
                </div>
                <p className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
                  Error tracking keeps its own UI; this deployment reports into it once
                  each service's DSN is set.
                </p>
                {view.links.glitchtip !== null ? (
                  <a
                    className="text-xs underline underline-offset-2"
                    style={{ color: "var(--m-accent)" }}
                    href={view.links.glitchtip}
                    target="_blank"
                    rel="noreferrer"
                  >
                    Open GlitchTip ↗
                  </a>
                ) : (
                  <p className="text-xs font-mono" style={{ color: "var(--m-text-muted)" }}>
                    Set MERIDIAN_GLITCHTIP_URL on core-api
                  </p>
                )}
                <div
                  className="grid gap-1 border-t pt-2"
                  style={{ borderColor: "var(--m-border)" }}
                >
                  <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                    DSN configured (presence only)
                  </span>
                  {view.glitchtip_dsns.map((row) => (
                    <div
                      key={row.service}
                      data-testid={`self-host-dsn-${row.service}`}
                      className="flex items-center justify-between gap-2"
                    >
                      <span className="text-xs font-mono" style={{ color: "var(--m-text-secondary)" }}>
                        {row.service}
                      </span>
                      <Tag
                        tone={
                          row.state === "configured"
                            ? "success"
                            : row.state === "unknown"
                              ? "outline"
                              : "neutral"
                        }
                      >
                        {row.state}
                      </Tag>
                    </div>
                  ))}
                </div>
              </Card>

              <LinkCard
                title="Grafana"
                description="Dashboards over the Tempo, Loki and MeridianMetrics datasources."
                url={view.links.grafana}
                envVar="MERIDIAN_GRAFANA_URL"
                openLabel="Open Grafana"
              />
              <LinkCard
                title="Collector metrics"
                description="The OTel collector's Prometheus scrape target."
                url={view.links.otel_metrics}
                envVar="MERIDIAN_OTEL_METRICS_URL"
                openLabel="Open metrics"
              />
              <LinkCard
                title="Tempo"
                description="Trace storage and query UI for exported spans."
                url={view.links.tempo}
                envVar="MERIDIAN_TEMPO_URL"
                openLabel="Open Tempo"
              />
              <LinkCard
                title="Loki"
                description="Log storage and query UI for exported service logs."
                url={view.links.loki}
                envVar="MERIDIAN_LOKI_URL"
                openLabel="Open Loki"
              />
            </div>
          </section>

          {/* --- Garage -------------------------------------------------------- */}
          <Card className="p-4 grid gap-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <h2 className="text-[13px] font-semibold" style={{ color: "var(--m-text)" }}>
                Garage object storage
              </h2>
              <Tag tone={view.garage.s3_endpoint !== null ? "success" : "neutral"}>
                {view.garage.s3_endpoint !== null ? "S3 mode" : "local fs mode"}
              </Tag>
            </div>
            <p className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
              Garage&rsquo;s admin API is CLI-first, so this pane links out instead of forking
              it. In the shipped compose stack the data lives in two named volumes
              (the compose project prefix applies at runtime) and the bootstrap runs once.
            </p>
            <div>
              <FactRow label="Data volume" value={view.garage.volumes[0] ?? "—"} mono />
              <FactRow label="Meta volume" value={view.garage.volumes[1] ?? "—"} mono />
              {view.garage.s3_endpoint !== null ? (
                <>
                  <FactRow label="S3 endpoint" value={view.garage.s3_endpoint} mono />
                  <FactRow label="Bucket" value={view.garage.s3_bucket ?? "—"} mono />
                </>
              ) : null}
            </div>
            <div className="grid gap-1">
              <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                Bootstrap (run once, from the repo root)
              </span>
              <pre
                className="text-[11px] font-mono overflow-x-auto p-2 rounded-[3px]"
                style={{
                  background: "var(--m-surface)",
                  border: "1px solid var(--m-border)",
                  color: "var(--m-text-secondary)",
                }}
              >
                {view.garage.bootstrap_commands.join("\n")}
              </pre>
            </div>
            <a
              className="text-xs underline underline-offset-2"
              style={{ color: "var(--m-accent)" }}
              href={view.garage.admin_api_docs_url}
              target="_blank"
              rel="noreferrer"
            >
              Garage admin API reference ↗
            </a>
          </Card>
        </>
      ) : null}
    </div>
  );
}
