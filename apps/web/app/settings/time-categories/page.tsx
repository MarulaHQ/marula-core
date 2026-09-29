// /settings/time-categories — the org's time taxonomy (Wave 39 lane 1,
// `p39-time-categories`; migration 0110).
//
// Purpose: manage the categories that time entries key their `work_type`
// onto. Each row carries the typed billing semantics the capture surfaces
// read — a default billable flag, whether flipping billable off demands a
// typed reason, and whether T&M rate cards are expected to key onto it.
// Order is the admin's: move a category and every capture picker reorders.
//
// Retire, never delete: entries and rate-card rows keep the slug, so a
// retired category leaves the pickers while historical work keeps its
// meaning. The row's entry count is what makes that honest.
//
// Mutations ride the instant-first envelope (optimistic patch +
// authoritative reconcile) and are billing-admin actions
// (`special:manage_billing`); the list itself is readable by anyone with
// time view. Non-purpose: no rate-card editing here (contracts stay
// contract-owned — the affinity flag is documentation, not a write path).
"use client";

import type { components } from "@marula-core/sdk";
import {
  Button,
  Card,
  Input,
  Label,
  Spinner,
  Table,
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import { useCallback, useEffect, useState } from "react";

import { rejectionText } from "../../../lib/error-copy";
import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { SettingsHeader } from "../../../components/settings/settings-shell";

type TimeCategoryView = components["schemas"]["TimeCategoryView"];
type TimeCategory = components["schemas"]["TimeCategory"];

const MANAGE_BILLING = "special:manage_billing";

/** The typed unbillable reasons the entry vocabulary accepts — shown in
 * the hint so an admin knows what "requires a reason" means downstream. */
const REASONS = [
  "warranty",
  "courtesy",
  "in_retainer_block",
  "sla_covered",
  "internal",
  "mistake",
] as const;

function slugify(label: string): string {
  return label
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .slice(0, 48);
}

export default function TimeCategoriesSettingsPage() {
  const auth = useRequireAuth();
  const [rows, setRows] = useState<TimeCategoryView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [adding, setAdding] = useState(false);
  const [draftLabel, setDraftLabel] = useState("");
  const [draftSlug, setDraftSlug] = useState("");
  const [draftBillable, setDraftBillable] = useState(true);
  const [draftAffinity, setDraftAffinity] = useState(false);

  const canManage = auth.me?.permissions?.includes(MANAGE_BILLING) === true;

  const load = useCallback(async () => {
    try {
      const { data, error: apiError } = await api.api.GET("/api/v1/time-categories", {
        params: { query: { include_retired: true } },
      });
      if (apiError !== undefined || data === undefined) throw new Error("could not load");
      setRows(data.categories);
      setError(null);
    } catch {
      setError("Could not load the time categories.");
    }
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void load();
  }, [auth.status, load]);

  // Live rows first in their admin-managed order, retired rows after
  // them (see the table header note).
  const visible = rows === null ? null : [
    ...rows.filter((r) => !r.retired),
    ...rows.filter((r) => r.retired),
  ];

  async function create() {
    const label = draftLabel.trim();
    if (label === "") return;
    const slug = slugify(draftSlug.trim() === "" ? label : draftSlug);
    if (slug === "") {
      setError("That label does not produce a usable slug. Use letters, digits, or dashes.");
      return;
    }
    setBusy(true);
    const result = await runEnvelopeMutation<
      components["schemas"]["TimeCategoryCreateRequest"],
      components["schemas"]["TimeCategoryMutationResponse"]
    >({
      describe: `Create time category ${slug}`,
      entityId: slug,
      buildBody: (base) => ({
        slug,
        label,
        default_billable: draftBillable,
        rate_card_affinity: draftAffinity,
        ...base,
      }),
      optimistic: () => undefined,
      revert: () => undefined,
      request: (client, typed) =>
        client.POST("/api/v1/time-categories", { body: typed }),
    });
    setBusy(false);
    if (!result.ok) {
      setError(result.friendly ?? rejectionText(result, "Could not create the category."));
      return;
    }
    setAdding(false);
    setDraftLabel("");
    setDraftSlug("");
    await load();
  }

  async function update(row: TimeCategoryView, patch: Partial<TimeCategory>, describe: string) {
    setBusy(true);
    const result = await runEnvelopeMutation<
      components["schemas"]["TimeCategoryUpdateRequest"],
      components["schemas"]["TimeCategoryMutationResponse"]
    >({
      describe,
      entityId: row.id,
      buildBody: (base) => ({ ...patch, ...base }),
      optimistic: () => undefined,
      revert: () => undefined,
      request: (client, typed) =>
        client.PATCH("/api/v1/time-categories/{id}", {
          params: { path: { id: row.id } },
          body: typed,
        }),
    });
    setBusy(false);
    if (!result.ok) {
      setError(result.friendly ?? rejectionText(result, "Could not update the category."));
      return;
    }
    await load();
  }

  async function retire(row: TimeCategoryView) {
    setBusy(true);
    const result = await runEnvelopeMutation<
      components["schemas"]["MutationParams"],
      components["schemas"]["TimeCategoryMutationResponse"]
    >({
      describe: `Retire time category ${row.slug}`,
      entityId: row.id,
      buildBody: (base) => ({ ...base }),
      optimistic: () => undefined,
      revert: () => undefined,
      request: (client) =>
        client.DELETE("/api/v1/time-categories/{id}", {
          params: { path: { id: row.id } },
        }),
    });
    setBusy(false);
    if (!result.ok) {
      setError(result.friendly ?? rejectionText(result, "Could not retire the category."));
      return;
    }
    await load();
  }

  async function move(index: number, delta: -1 | 1) {
    if (visible === null) return;
    const next = [...visible];
    const target = index + delta;
    if (target < 0 || target >= next.length) return;
    const a = next[index];
    const b = next[target];
    if (a === undefined || b === undefined) return;
    next[index] = b;
    next[target] = a;
    setRows(next);
    const result = await runEnvelopeMutation<
      components["schemas"]["ReorderCategoriesRequest"],
      components["schemas"]["TimeCategoryListResponse"]
    >({
      describe: "Reorder time categories",
      entityId: "time-categories",
      buildBody: (base) => ({
        ordered_ids: next.map((r) => r.id),
        ...base,
      }),
      optimistic: () => undefined,
      revert: () => undefined,
      request: (client, typed) =>
        client.POST("/api/v1/time-categories/reorder", { body: typed }),
    });
    if (!result.ok) {
      setError(result.friendly ?? rejectionText(result, "Could not reorder."));
      await load();
    }
  }

  if (auth.status !== "authed") return null;
  const list = visible ?? [];

  return (
    <main className="flex flex-col gap-4" data-testid="time-categories-settings">
      <SettingsHeader
        title="Time categories"
        description="What kind of work each time entry records. Entries key their category onto these; the order here is the order techs see when logging time."
      />

      {canManage ? (
        <div className="flex items-center gap-2">
          <Button onClick={() => setAdding((v) => !v)} data-testid="create-time-category">
            New category
          </Button>
          <Button onClick={() => void load()} data-testid="reload-time-categories">
            Reload
          </Button>
        </div>
      ) : null}

      {error !== null && (
        <p role="alert" style={{ color: "var(--m-status-danger-fg)" }} data-testid="time-category-error">
          {error}
        </p>
      )}

      {adding ? (
        <Card className="flex flex-col gap-3" data-testid="time-category-create">
          <div className="flex flex-col gap-1">
            <Label htmlFor="tc-label">Label</Label>
            <Input
              id="tc-label"
              value={draftLabel}
              placeholder="Field Service"
              onChange={(e) => setDraftLabel(e.target.value)}
            />
          </div>
          <div className="flex flex-col gap-1">
            <Label htmlFor="tc-slug">Slug (optional)</Label>
            <Input
              id="tc-slug"
              value={draftSlug}
              placeholder={slugify(draftLabel) || "field_service"}
              onChange={(e) => setDraftSlug(e.target.value)}
            />
            <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
              The slug is permanent — entries and rate cards key onto it.
            </span>
          </div>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={draftBillable}
              onChange={(e) => setDraftBillable(e.target.checked)}
            />
            Billable by default
          </label>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={draftAffinity}
              onChange={(e) => setDraftAffinity(e.target.checked)}
            />
            T&amp;M rate cards are expected to key onto this
          </label>
          <div className="flex items-center gap-2">
            <Button onClick={() => void create()} disabled={busy} data-testid="save-time-category">
              Create
            </Button>
            <Button onClick={() => setAdding(false)}>Cancel</Button>
          </div>
        </Card>
      ) : null}

      {/* Retired rows group at the BOTTOM for reading: an admin scanning
          this list should see the live taxonomy in its real order first,
          not retired rows interleaved by position. Display-only — the
          stored positions (and therefore the capture pickers) are
          untouched, and each group keeps its own relative order. */}
      {visible !== null ? (
        <Spinner />
      ) : (
        <Table>
          <thead>
            <tr>
              <Th>Order</Th>
              <Th>Category</Th>
              <Th>Billing</Th>
              <Th>Rate card</Th>
              <Th>Entries</Th>
              {canManage ? <Th>Actions</Th> : null}
            </tr>
          </thead>
          <tbody>
            {list.map((row, index) => (
              <Tr key={row.id} data-testid={`time-category-${row.slug}`}>
                <Td>
                  <div className="flex items-center gap-1">
                    {canManage ? (
                      <>
                        <Button
                          size="sm"
                          variant="ghost"
                          aria-label={`Move ${row.label} up`}
                          disabled={index === 0 || busy}
                          onClick={() => void move(index, -1)}
                        >
                          ↑
                        </Button>
                        <Button
                          size="sm"
                          variant="ghost"
                          aria-label={`Move ${row.label} down`}
                          disabled={index === list.length - 1 || busy}
                          onClick={() => void move(index, 1)}
                        >
                          ↓
                        </Button>
                      </>
                    ) : (
                      <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                        {index + 1}
                      </span>
                    )}
                  </div>
                </Td>
                <Td>
                  <div className="flex flex-col">
                    <span style={{ color: "var(--m-text)" }}>{row.label}</span>
                    <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                      {row.slug}
                      {row.retired ? " · retired" : ""}
                    </span>
                  </div>
                </Td>
                <Td>
                  <div className="flex flex-col gap-1">
                    <label className="flex items-center gap-2 text-sm">
                      <input
                        type="checkbox"
                        checked={row.default_billable}
                        disabled={!canManage || busy || row.retired}
                        onChange={(e) =>
                          void update(
                            row,
                            { default_billable: e.target.checked },
                            `Set ${row.slug} billing default`,
                          )
                        }
                      />
                      Billable by default
                    </label>
                    <label className="flex items-center gap-2 text-sm">
                      <input
                        type="checkbox"
                        checked={row.requires_unbillable_reason}
                        disabled={!canManage || busy || row.retired}
                        onChange={(e) =>
                          void update(
                            row,
                            { requires_unbillable_reason: e.target.checked },
                            `Set ${row.slug} reason requirement`,
                          )
                        }
                      />
                      Ask for a reason when non-billable
                    </label>
                    {row.requires_unbillable_reason ? (
                      <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                        Accepted reasons: {REASONS.join(", ")}
                      </span>
                    ) : null}
                  </div>
                </Td>
                <Td>
                  <label className="flex items-center gap-2 text-sm">
                    <input
                      type="checkbox"
                      checked={row.rate_card_affinity}
                      disabled={!canManage || busy || row.retired}
                      onChange={(e) =>
                        void update(
                          row,
                          { rate_card_affinity: e.target.checked },
                          `Set ${row.slug} rate-card affinity`,
                        )
                      }
                    />
                    Keyed by cards
                  </label>
                </Td>
                <Td>
                  <span className="text-sm">{row.entry_count}</span>
                </Td>
                {canManage ? (
                  <Td>
                    {row.retired ? (
                      <Button
                        size="sm"
                        disabled={busy}
                        onClick={() =>
                          void update(row, { retired: false }, `Restore ${row.slug}`)
                        }
                      >
                        Restore
                      </Button>
                    ) : (
                      <Button
                        size="sm"
                        variant="ghost"
                        disabled={busy}
                        onClick={() => void retire(row)}
                        data-testid={`retire-${row.slug}`}
                      >
                        Retire
                      </Button>
                    )}
                  </Td>
                ) : null}
              </Tr>
            ))}
          </tbody>
        </Table>
      )}
    </main>
  );
}
