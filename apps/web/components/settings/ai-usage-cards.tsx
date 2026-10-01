"use client";

// ai-usage-cards — the org usage progress view (this lane).
//
// Purpose: the admin org dashboard over GET /api/v1/settings/ai/usage/org:
// org-wide ring + per-surface totals for the period. The user-facing cap
// ring (GET /api/v1/ai/me/usage) rides the same primitives and renders
// /today-side for any granted user.
//
// Posture: degrade-quiet when no cap rows exist — the ring shows usage
// on its own scale (no cap to bind) instead of pretending a soft bound.
//
// Tokens: docs/theme.md only; JetBrains Mono numerals; motion ≤120ms.

import { useEffect, useState } from "react";
import { Card, Spinner, Tag } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { api } from "../../lib/api";

type AiOrgUsageResponse = components["schemas"]["AiOrgUsageResponse"];

export function CapRing({
  used,
  soft,
  hard,
  size = 48,
  label,
}: {
  used: number;
  soft: number | null;
  hard: number | null;
  size?: number;
  label: string;
}) {
  const bound = soft ?? hard ?? null;
  const pct = bound != null && bound > 0 ? Math.min(1, used / bound) : null;
  const r = (size - 8) / 2;
  const c = 2 * Math.PI * r;
  const tone =
    pct === null
      ? "var(--m-status-neutral-fg)"
      : (soft !== null && used >= soft) || (hard !== null && used >= (hard ?? Infinity))
        ? "var(--m-status-danger-fg)"
        : pct > 0.75
          ? "var(--m-status-warning-fg)"
          : "var(--m-status-success-fg)";
  return (
    <span
      className="relative inline-flex select-none items-center justify-center"
      style={{ width: size, height: size }}
      data-testid={label != null ? `cap-ring-${label}` : undefined}
    >
      <svg width={size} height={size} className="-rotate-90">
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          stroke="var(--m-border)"
          strokeWidth={4}
        />
        {pct !== null && (
          <circle
            cx={size / 2}
            cy={size / 2}
            r={r}
            fill="none"
            stroke={tone}
            strokeWidth={4}
            strokeLinecap="round"
            strokeDasharray={c}
            strokeDashoffset={c * (1 - pct)}
            style={{ transition: "stroke-dashoffset 120ms ease-out" }}
          />
        )}
      </svg>
      <span
        className="absolute text-[10px]"
        style={{ color: "var(--m-text-secondary)" }}
      >
        {pct === null ? "—" : `${Math.round(pct * 100)}%`}
      </span>
    </span>
  );
}

function formatTokens(n: number | null | undefined): string {
  if (n == null) return "—";
  if (n < 1000) return String(n);
  return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
}

export function OrgUsageCard() {
  const [usage, setUsage] = useState<AiOrgUsageResponse | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let alive = true;
    (async () => {
      const { data } = await api.api.GET("/api/v1/settings/ai/usage/org");
      if (!alive) return;
      setUsage(data ?? null);
      setLoading(false);
    })();
    return () => {
      alive = false;
    };
  }, []);

  return (
    <Card className="mb-4">
      <div className="p-4">
        <h2 className="text-sm font-semibold">Usage this period</h2>
        {loading ? (
          <Spinner />
        ) : usage === null ? (
          <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-muted)" }}>
            Usage is not reporting yet — it lights up with the first metered call.
          </p>
        ) : (
          <div className="mt-3 flex items-center gap-4" data-testid="org-usage">
            <CapRing
              used={usage.used_tokens}
              soft={usage.soft_token_limit}
              hard={usage.hard_token_limit}
              size={56}
              label="org"
            />
            <div>
              <p className="text-[13px] font-semibold">
                {usage.active_users} active user{usage.active_users === 1 ? "" : "s"}
                <span className="ml-2 font-normal" style={{ color: "var(--m-text-muted)" }}>
                  {`period ${usage.period}`}
                </span>
              </p>
              <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                {formatTokens(usage.used_tokens)} used
                {usage.soft_token_limit != null ? ` · soft ${formatTokens(usage.soft_token_limit)}` : ""}
                {usage.hard_token_limit != null ? ` · hard ${formatTokens(usage.hard_token_limit)}` : ""}
              </p>
            </div>
            {Object.entries(usage.tokens_by_surface ?? {}).sort(([a], [b]) =>
              a.localeCompare(b),
            ).map(([surface, tokens]) => (
              <Tag key={surface}>
                {surface}: {formatTokens(tokens)}
              </Tag>
            ))}
          </div>
        )}
      </div>
    </Card>
  );
}

type AiMyUsageResponse = components["schemas"]["AiMyUsageResponse"];

/** The signed-in user's own period usage + cap ring (rides me/usage). */
export function MyUsageCard() {
  const [usage, setUsage] = useState<AiMyUsageResponse | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let alive = true;
    (async () => {
      const { data } = await api.api.GET("/api/v1/ai/me/usage");
      if (!alive) return;
      setUsage(data ?? null);
      setLoading(false);
    })();
    return () => {
      alive = false;
    };
  }, []);

  return (
    <Card className="mb-4">
      <div className="p-4">
        <h2 className="text-sm font-semibold">My usage</h2>
        {loading ? (
          <Spinner />
        ) : usage === null ? (
          <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-muted)" }}>
            No personal usage recorded yet this period — calls light it up.
          </p>
        ) : (
          <div className="mt-3 flex items-center gap-4" data-testid="my-usage">
            <CapRing
              used={usage.used_tokens}
              soft={usage.soft_token_limit}
              hard={usage.hard_token_limit}
              size={56}
              label="me"
            />
            <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
              {formatTokens(usage.used_tokens)} used · period {usage.period}
              {usage.soft_token_limit != null ? ` · soft ${formatTokens(usage.soft_token_limit)}` : ""}
              {usage.hard_token_limit != null ? ` · hard ${formatTokens(usage.hard_token_limit)}` : ""}
            </p>
          </div>
        )}
      </div>
    </Card>
  );
}
