// TicketTags — the ticket-detail tag chip row (Wave 11 lane 10 close-out).
//
// Purpose: shows the tags applied to one ticket (GET /tickets/{id}/tags)
// and applies/removes them through the idempotent POST/DELETE pair.
// Editing rides tickets:edit — read-only roles see the chips without the
// picker. The apply picker lists the org's tags (GET /tags) minus the
// ones already applied. Rejections (409 duplicate / 404) render inline.
// Non-purpose: no tag CRUD here (Settings → Tags owns that).
"use client";

import { rejectionText } from "../lib/error-copy";
import type { components } from "@meridian/sdk";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@meridian/ui";
import { useCallback, useEffect, useState } from "react";

import { api } from "../lib/api";
import { runEnvelopeMutation } from "../lib/envelope-write";

type Tag = components["schemas"]["Tag"];

export function TagChip({ tag }: { tag: Tag }) {
  const color = tag.color ?? "var(--m-text-muted)";
  return (
    <span
      className="inline-flex items-center rounded-[var(--m-radius-container)] px-2 py-0.5 text-[11px] font-medium"
      style={{ border: `1px solid ${color}`, color, background: "transparent" }}
    >
      {tag.name}
    </span>
  );
}

export function TicketTags({
  ticketId,
  canEdit,
  onChanged,
}: {
  ticketId: string;
  canEdit: boolean;
  /** Fired after a successful apply/remove so parents can refresh. */
  onChanged?: () => void;
}) {
  const [applied, setApplied] = useState<Tag[] | null>(null);
  const [orgTags, setOrgTags] = useState<Tag[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);
  /** Collapsed view keeps the row one line tall; heavy taggers expand. */
  const MAX_COLLAPSED = 8;

  const refresh = useCallback(async () => {
    const { data } = await api.api.GET("/api/v1/tickets/{id}/tags", {
      params: { path: { id: ticketId } },
    });
    if (data !== undefined) setApplied(data);
  }, [ticketId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    let cancelled = false;
    void api.api
      .GET("/api/v1/tags")
      .then(({ data }) => {
        if (cancelled || data === undefined) return;
        setOrgTags(data.map((tv) => tv.tag));
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  const apply = async (tagId: string) => {
    if (tagId === "" || tagId === "__pick") return;
    const tag = orgTags.find((t) => t.id === tagId) ?? null;
    setError(null);
    const before = applied ?? [];
    const result = await runEnvelopeMutation<
      components["schemas"]["ApplyTagRequest"],
      unknown
    >({
      describe: "Apply tag",
      entityId: ticketId,
      optimistic: () => {
        if (tag !== null) setApplied((prev) => [...(prev ?? []), tag]);
      },
      revert: () => setApplied(before),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key, tag_id: tagId }),
      request: (client, body) =>
        client.POST("/api/v1/tickets/{id}/tags", {
          params: { path: { id: ticketId } },
          body,
        }) as Promise<{ data?: unknown; error?: unknown; response: Response }>,
    });
    if (!result.ok) {
      setError(rejectionText(result, "The tag could not be applied."));
    }
    void refresh();
    onChanged?.();
  };

  const remove = async (tagId: string) => {
    setError(null);
    const before = applied ?? [];
    const result = await runEnvelopeMutation<
      components["schemas"]["MutationParams"],
      unknown
    >({
      describe: "Remove tag",
      entityId: ticketId,
      optimistic: () => setApplied((prev) => (prev ?? []).filter((t) => t.id !== tagId)),
      revert: () => setApplied(before),
      buildBody: (base) => ({ idempotency_key: base.idempotency_key }),
      request: (client, body) =>
        client.DELETE("/api/v1/tickets/{id}/tags/{tag_id}", {
          params: { path: { id: ticketId, tag_id: tagId } },
        }) as Promise<{ data?: unknown; error?: unknown; response: Response }>,
    });
    if (!result.ok) {
      setError(rejectionText(result, "The tag could not be removed."));
    }
    void refresh();
    onChanged?.();
  };

  const available = orgTags.filter((t) => !(applied ?? []).some((a) => a.id === t.id));
  const visibleTags = expanded ? (applied ?? []) : (applied ?? []).slice(0, MAX_COLLAPSED);
  const hiddenCount = (applied?.length ?? 0) - visibleTags.length;

  return (
    <div className="flex flex-wrap items-center gap-1.5" data-testid="ticket-tags">
      {visibleTags.map((tag) => (
        <span key={tag.id} className="inline-flex items-center gap-0.5">
          <TagChip tag={tag} />
          {canEdit ? (
            <button
              type="button"
              aria-label={`Remove tag ${tag.name}`}
              title={`Remove tag ${tag.name}`}
              className="cursor-pointer border-none bg-transparent p-0.5 text-[10px] leading-none transition-opacity duration-100"
              style={{ color: tag.color ?? "var(--m-text-muted)" }}
              onClick={() => void remove(tag.id)}
            >
              ✕
            </button>
          ) : null}
        </span>
      ))}
      {hiddenCount > 0 ? (
        <button
          type="button"
          className="cursor-pointer border-none bg-transparent text-[11px] underline"
          style={{ color: "var(--m-text-muted)" }}
          onClick={() => setExpanded(true)}
        >
          +{hiddenCount} more
        </button>
      ) : null}
      {applied !== null && applied.length === 0 && !canEdit ? (
        <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
          No tags.
        </span>
      ) : null}
      {canEdit ? (
        <Select value="" onValueChange={(v) => void apply(v)}>
          <SelectTrigger className="h-6 w-40 text-[11px]" aria-label="Apply tag">
            <SelectValue placeholder="+ Add tag" />
          </SelectTrigger>
          <SelectContent>
            {available.length === 0 ? (
              <SelectItem value="__pick" disabled>
                No more tags — create in Settings → Tags
              </SelectItem>
            ) : (
              available.map((t) => (
                <SelectItem key={t.id} value={t.id}>
                  {t.name}
                </SelectItem>
              ))
            )}
          </SelectContent>
        </Select>
      ) : null}
      {error !== null ? (
        <span role="alert" className="text-[11px]" style={{ color: "var(--m-status-danger-fg)" }}>
          {error}
        </span>
      ) : null}
    </div>
  );
}

export function TagsEmptyHint() {
  return (
    <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
      Tags are org-wide labels — manage them in Settings → Tags.
    </span>
  );
}
