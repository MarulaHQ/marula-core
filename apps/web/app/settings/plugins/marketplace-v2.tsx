// Marketplace v2 (Wave 22): verified publishers + ratings, ADDITIVE over
// the ADR 0009 v1 surface files (never restructures the v1 formats).
//
// * PublishersViewTab — the org's publisher registry: verified rows carry
//   the org's badge; admins vouch (normalize server-side) and withdraw.
// * RatingTargets — per-artifact-release aggregate from the org's
//   ratings, plus each staff member's own 1–5 star + comment upsert.
"use client";

import type { components } from "@marula-core/sdk";
import { Button, Card, EmptyState, Input, Spinner, Tag } from "@marula-core/ui";
import { useEffect, useState } from "react";
import { api } from "../../../lib/api";
import { reasonCopy } from "../../../lib/error-copy";

type PublisherVerificationView = components["schemas"]["PublisherVerificationView"];
type RatingListView = components["schemas"]["RatingListView"];

/** Mirrors the server's normalize_publisher: registry://slug verbatim,
 * github.com/{owner}/{repo} → github.com/{owner}, anything else → host. */
function publisherIdentityOf(sourceUrl: string): string {
  const value = sourceUrl.trim().toLowerCase();
  if (value.startsWith("registry://")) return value;
  const noScheme = value.replace(/^https?:\/\//, "").replace(/\/+$/, "");
  const [host, owner] = noScheme.split("/", 2);
  if (host === "github.com" && owner !== undefined && owner !== "") {
    return `github.com/${owner}`;
  }
  return host ?? "";
}

function Stars({
  value,
  onPick,
  disabled,
}: {
  value: number;
  onPick?: (v: number) => void;
  disabled?: boolean;
}) {
  return (
    <div className="flex items-center gap-0.5" role="img" aria-label={`${value} of 5 stars`}>
      {[1, 2, 3, 4, 5].map((star) => (
        <button
          key={star}
          type="button"
          disabled={disabled}
          onClick={() => onPick?.(star)}
          aria-label={`Rate ${star} star${star === 1 ? "" : "s"}`}
          data-testid={`rate-${star}`}
          className="px-0.5 text-[16px] leading-none transition-opacity duration-100"
          style={{
            color: star <= value ? "var(--m-accent)" : "var(--m-border-strong, var(--m-border))",
            opacity: disabled ? 0.5 : 1,
            cursor: onPick ? "pointer" : "default",
          }}
        >
          ★
        </button>
      ))}
    </div>
  );
}

function ScoreBadge({ count, average }: { count: number; average: number | null | undefined }) {
  return (
    <span className="flex items-center gap-1.5 text-[12px]" style={{ color: "var(--m-text-secondary)" }} data-testid="rating-summary">
      <span aria-hidden="true" style={{ color: "var(--m-accent)" }}>★</span>
      {average === undefined || average === null
        ? "no ratings"
        : `${average.toFixed(1)} · ${count} rating${count === 1 ? "" : "s"}`}
    </span>
  );
}

export function VerifiedBadge({ sourceUrl }: { sourceUrl: string }) {
  const [verified, setVerified] = useState<Set<string> | null>(null);
  useEffect(() => {
    let live = true;
    void (async () => {
      const { data } = await api.api.GET("/api/v1/plugins/distribution/publishers");
      if (live && data !== undefined) {
        setVerified(new Set(data.filter((p) => p.status === "verified").map((p) => p.publisher)));
      }
    })();
    return () => {
      live = false;
    };
  }, []);
  if (verified === null || verified.size === 0) return null;
  if (!verified.has(publisherIdentityOf(sourceUrl))) return null;
  return (
    <Tag tone="success" data-testid="verified-badge">
      ✓ verified publisher
    </Tag>
  );
}

export function RatingPanel({ sourceUrl, digest }: { sourceUrl: string; digest: string }) {
  const [list, setList] = useState<RatingListView | null>(null);
  const [stars, setStars] = useState(0);
  const [comment, setComment] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    let live = true;
    void (async () => {
      const { data, error: fetchError } = await api.api.GET("/api/v1/plugins/distribution/ratings", {
        params: { query: { source_url: sourceUrl, artifact_digest: digest } },
      });
      if (!live) return;
      if (fetchError === undefined && data !== undefined) {
        setList(data);
        setStars(data.summary.mine?.rating ?? 0);
        setComment(data.summary.mine?.comment ?? "");
      }
    })();
    return () => {
      live = false;
    };
  }, [sourceUrl, digest]);

  const save = async (rating: number) => {
    setBusy(true);
    setSaved(false);
    setError(null);
    setStars(rating);
    const { error: putError, data } = await api.api.PUT("/api/v1/plugins/distribution/ratings", {
      body: {
        source_kind: "manual_upload",
        source_url: sourceUrl,
        artifact_digest: digest,
        rating,
        comment,
      },
    });
    if (putError !== undefined) {
      const envelope = (putError as { error?: { reason?: string } } | undefined)?.error;
      setError(reasonCopy(envelope?.reason ?? "validation_failed", envelope));
      setBusy(false);
      return;
    }
    setStars(data?.mine?.rating ?? rating);
    setSaved(true);
    setBusy(false);
    const { data: refreshed } = await api.api.GET("/api/v1/plugins/distribution/ratings", {
      params: { query: { source_url: sourceUrl, artifact_digest: digest } },
    });
    if (refreshed !== undefined) setList(refreshed);
  };

  return (
    <div className="mt-2 rounded-[var(--m-radius-container)] border p-2.5" style={{ borderColor: "var(--m-border)" }} data-testid="rating-panel">
      <div className="flex flex-wrap items-center gap-2">
        {list !== null ? <ScoreBadge count={list.summary.count} average={list.summary.average} /> : <Spinner />}
        <div className="ml-auto">
          <Stars value={stars} onPick={(v) => void save(v)} disabled={busy} />
        </div>
      </div>
      <div className="mt-1.5 flex items-center gap-2">
        <Input
          placeholder="What worked / what didn't (optional)"
          value={comment}
          onChange={(e) => setComment(e.target.value)}
          aria-label="Rating comment"
          className="min-w-56 flex-1"
        />
        <Button
          size="sm"
          variant="ghost"
          disabled={busy || stars === 0}
          onClick={() => void save(stars)}
          data-testid="rating-save"
        >
          {busy ? <Spinner /> : null} {saved || stars !== 0 ? "Save rating" : "Rate"}
        </Button>
      </div>
      {error !== null ? (
        <p role="alert" className="mt-1 text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </p>
      ) : null}
    </div>
  );
}

export function MarketplaceViewTab({ canManage, onFocus }: { canManage: boolean; onFocus?: () => void }) {
  const [publishers, setPublishers] = useState<PublisherVerificationView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [publisherInput, setPublisherInput] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [busy, setBusy] = useState(false);

  const refresh = async () => {
    const { data, error: fetchError } = await api.api.GET("/api/v1/plugins/distribution/publishers");
    if (fetchError !== undefined || data === undefined) {
      setError("Failed to load the publisher registry.");
      return;
    }
    setPublishers(data);
    setError(null);
  };

  useEffect(() => {
    void refresh();
  }, []);

  const verify = async () => {
    setBusy(true);
    setError(null);
    const { error: postError } = await api.api.POST("/api/v1/plugins/distribution/publishers", {
      body: { publisher: publisherInput, display_name: displayName },
    });
    setBusy(false);
    if (postError !== undefined) {
      const envelope = (postError as { error?: { reason?: string } } | undefined)?.error;
      setError(reasonCopy(envelope?.reason ?? "validation_failed", envelope));
      return;
    }
    setPublisherInput("");
    setDisplayName("");
    await refresh();
    onFocus?.();
  };

  const revoke = async (id: string) => {
    setBusy(true);
    const { error: deleteError } = await api.api.DELETE("/api/v1/plugins/distribution/publishers/{id}", {
      params: { path: { id } },
    });
    setBusy(false);
    if (deleteError !== undefined) {
      const envelope = (deleteError as { error?: { reason?: string } } | undefined)?.error;
      setError(reasonCopy(envelope?.reason ?? "not_found", envelope));
      return;
    }
    await refresh();
  };

  if (publishers === null) return <Spinner />;

  return (
    <div className="grid gap-3" data-testid="publishers-tab">
      {canManage && (
        <Card>
          <div className="flex flex-wrap items-end gap-2 p-4">
            <div className="grid flex-1 gap-1 min-w-64">
              <label htmlFor="pub-input" className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                Publisher identity (github.com/owner/repo, registry://slug, or zip host)
              </label>
              <Input id="pub-input" value={publisherInput} onChange={(e) => setPublisherInput(e.target.value)} placeholder="github.com/an-owner/repo" data-testid="publisher-input" />
            </div>
            <div className="grid gap-1 min-w-44">
              <label htmlFor="pub-name" className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                Display name
              </label>
              <Input id="pub-name" value={displayName} onChange={(e) => setDisplayName(e.target.value)} placeholder="Marula Labs" data-testid="publisher-name" />
            </div>
            <Button size="sm" variant="primary" disabled={busy || publisherInput.trim() === "" || displayName.trim() === ""} onClick={() => void verify()} data-testid="publisher-verify">
              {busy ? <Spinner /> : null} Vouch publisher
            </Button>
          </div>
        </Card>
      )}
      {error !== null ? (
        <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </p>
      ) : null}
      {publishers.length === 0 ? (
        <EmptyState message="No publishers vouched yet. Vouching is per-org — this org's badge only ever reflects who THIS org vetted." />
      ) : (
        publishers.map((pub) => (
          <Card key={pub.id}>
            <div className="flex flex-wrap items-center gap-2 p-4" data-testid={`publisher-row-${pub.publisher}`}>
              <span className="text-[13px] font-semibold">{pub.display_name}</span>
              <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>{pub.publisher}</span>
              <Tag tone={pub.status === "verified" ? "success" : "danger"}>{pub.status}</Tag>
              {pub.note !== "" ? (
                <span className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>{pub.note}</span>
              ) : null}
              {canManage ? (
                <Button
                  size="sm"
                  variant="ghost"
                  className="ml-auto"
                  disabled={busy || pub.status !== "verified"}
                  onClick={() => void revoke(pub.id)}
                  data-testid={`publisher-revoke-${pub.display_name.length}`}
                >
                  Withdraw vouch
                </Button>
              ) : null}
            </div>
          </Card>
        ))
      )}
    </div>
  );
}
