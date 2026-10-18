// DayPackPanel — the tech's trip preparation on /today (Wave 32 lane 8).
//
// Purpose: minimum switching. Everything a field tech needs to walk out the
// door lives here: the day's stops split into route-aware trip segments (an
// office touch re-opens a collect point), the consolidated grab list with
// per-line provenance and where to grab it, one-tap checkout into lane 3's
// transactions, and tomorrow's list tonight when the van does not return to
// the office first.
//
// Ticking a line writes a PLAN NOTE (the ledger stays honest; collection is
// the checkout/consume path). Unavailable states are honest: who has it,
// when it is due.
//
// Non-purpose: no board or queue management (the /today queue below owns
// that), no stock editing, no movement recording outside the shared dialogs.
"use client";

import { Button, Card, Spinner } from "@marula-core/ui";
import { formatDayKey } from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { api } from "../../lib/api";
import { friendlyMutationText } from "../../lib/error-copy";
import {
  availabilityLabel,
  availabilityTone,
  fetchDayPack,
  holderText,
  provenanceLabel,
  qtyText,
  sourceLabel,
  startPointNote,
  tickLine,
  timeText,
  tomorrowReasonNote,
  untickLine,
  type DayPackKit,
  type DayPackLine,
  type DayPackReminder,
  type DayPackView,
} from "../../lib/day-pack";
import { CheckoutDialog, type HolderOption } from "../stock/checkout-dialog";
import { MovementDialog } from "../stock/movement-dialog";
import { PickDialog } from "../stock/pick-dialog";

type StockItemView = components["schemas"]["StockItemView"];
type StockLocationView = components["schemas"]["StockLocationView"];
type ToolKitView = components["schemas"]["ToolKitView"];

interface ReferenceData {
  items: StockItemView[];
  locations: StockLocationView[];
  kits: ToolKitView[];
  holders: HolderOption[];
}

interface CheckoutSeed {
  itemId?: string;
  kitId?: string;
  locationId?: string;
  label: string;
}

interface UseSeed {
  line: DayPackLine;
  ticketLabel: string;
}

export interface DayPackPanelProps {
  /** Local date (YYYY-MM-DD) the panel is pinned to. */
  date: string;
  meUserId: string;
  meName: string;
}

export function DayPackPanel({ date, meUserId, meName }: DayPackPanelProps): ReactNode {
  const [view, setView] = useState<DayPackView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busyLine, setBusyLine] = useState<string | null>(null);
  const [reference, setReference] = useState<ReferenceData | null>(null);
  const [checkout, setCheckout] = useState<CheckoutSeed | null>(null);
  const [useSeed, setUseSeed] = useState<UseSeed | null>(null);
  const [pinTicket, setPinTicket] = useState<{
    id: string;
    label: string;
    initialItemId?: string;
  } | null>(null);

  const load = useCallback(async () => {
    try {
      const next = await fetchDayPack(date);
      setView(next);
      setError(null);
    } catch (err) {
      setError(friendlyMutationText(err, "The day pack could not be loaded."));
    }
  }, [date]);

  useEffect(() => {
    void load();
  }, [load]);

  const ensureReference = useCallback(async (): Promise<ReferenceData> => {
    if (reference !== null) return reference;
    const [itemsRes, locationsRes, kitsRes, usersRes] = await Promise.all([
      api.api.GET("/api/v1/stock-items", { params: { query: { limit: 500 } } }),
      api.api.GET("/api/v1/stock-locations", { params: { query: {} } }),
      api.api.GET("/api/v1/tool-kits", { params: { query: { limit: 200 } } }),
      api.api.GET("/api/v1/org/users", { params: { query: { limit: 200 } } }),
    ]);
    const holders: HolderOption[] =
      usersRes.data !== undefined
        ? usersRes.data.users.map((user) => ({ id: user.id, label: user.display_name }))
        : [{ id: meUserId, label: meName }];
    const data: ReferenceData = {
      items: itemsRes.data ?? [],
      locations: locationsRes.data ?? [],
      kits: kitsRes.data ?? [],
      holders: holders.some((h) => h.id === meUserId)
        ? holders
        : [{ id: meUserId, label: meName }, ...holders],
    };
    setReference(data);
    return data;
  }, [reference, meUserId, meName]);

  const ticketLabels = useMemo(() => {
    const labels = new Map<string, string>();
    if (view === null) return labels;
    for (const stop of [...view.trips.flatMap((trip) => trip.stops), ...(view.tomorrow?.stops ?? [])]) {
      if (stop.ticket_id !== null && stop.ticket_id !== undefined && !labels.has(stop.ticket_id)) {
        labels.set(
          stop.ticket_id,
          `${stop.ticket_number ?? "Ticket"} · ${stop.title}`,
        );
      }
    }
    return labels;
  }, [view]);

  const toggleTick = useCallback(
    async (
      targetDate: string,
      line: { line_key: string; ticket_id?: string | null },
      ticked: boolean,
    ) => {
      setBusyLine(line.line_key);
      setError(null);
      const failure =
        ticked === false
          ? await tickLine(targetDate, line)
          : await untickLine(targetDate, line.line_key);
      setBusyLine(null);
      if (failure !== null) {
        setError(failure);
        return;
      }
      await load();
    },
    [load],
  );

  const openCheckout = useCallback(
    async (seed: CheckoutSeed) => {
      await ensureReference();
      setCheckout(seed);
    },
    [ensureReference],
  );

  const openUse = useCallback(
    async (line: DayPackLine) => {
      await ensureReference();
      setUseSeed({
        line,
        ticketLabel: ticketLabels.get(line.ticket_id) ?? "this ticket",
      });
    },
    [ensureReference, ticketLabels],
  );

  const openPin = useCallback(
    async (ticketId: string, initialItemId?: string) => {
      await ensureReference();
      setPinTicket({
        id: ticketId,
        label: ticketLabels.get(ticketId) ?? "this ticket",
        initialItemId,
      });
    },
    [ensureReference, ticketLabels],
  );

  if (view === null) {
    if (error !== null) {
      return (
        <Card className="m-rise p-3 text-xs" aria-label="Day pack">
          <span style={{ color: "var(--m-status-danger-fg)" }} role="alert">
            {error}
          </span>
        </Card>
      );
    }
    return (
      <Card className="m-rise flex items-center gap-2 p-3" aria-label="Day pack">
        <Spinner />
        <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
          Loading the day pack…
        </span>
      </Card>
    );
  }

  const hasAnyPack =
    view.has_stops ||
    view.trips.some(
      (trip) => trip.lines.length > 0 || trip.kits.length > 0 || (trip.reminders ?? []).length > 0,
    ) ||
    (view.tomorrow !== undefined && view.tomorrow !== null);
  if (!hasAnyPack && error === null) return null;

  const renderGrabBody = (
    lines: DayPackLine[],
    kits: DayPackKit[],
    reminders: DayPackReminder[],
    targetDate: string,
  ) => (
    <>
      {kits.length > 0 && (
        <ul className="grid gap-1" role="list">
          {kits.map((kit) => {
            const ticked = kit.tick !== null && kit.tick !== undefined;
            return (
              <li
                key={kit.line_key}
                data-testid="day-pack-kit"
                data-line-key={kit.line_key}
                className="row-hover grid grid-cols-[auto_minmax(0,1fr)_auto] items-start gap-2 py-1.5"
              >
                <TickButton
                  ticked={ticked}
                  busy={busyLine === kit.line_key}
                  label={kit.name}
                  onToggle={() => void toggleTick(targetDate, { line_key: kit.line_key }, ticked)}
                />
                <div className="grid min-w-0 gap-0.5">
                  <span className="flex min-w-0 flex-wrap items-baseline gap-1.5 text-xs">
                    <span
                      className={ticked ? "truncate line-through" : "truncate font-medium"}
                      style={{ color: ticked ? "var(--m-text-muted)" : "var(--m-text)" }}
                    >
                      {kit.name}
                    </span>
 <span className="text-[10px] tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                      {kit.member_count} tools
                    </span>
                  </span>
                  <span className="flex flex-wrap items-center gap-1.5 text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                    <Chip tone="muted">Default kit</Chip>
                    {kit.ready ? (
                      <span>Ready to check out</span>
                    ) : (
                      <span style={{ color: "var(--m-status-warning-fg)" }}>
                        Missing {(kit.missing ?? []).join(", ")}
                      </span>
                    )}
                    {kit.code !== null && kit.code !== undefined && kit.code !== "" && (
                      <span className="">{kit.code}</span>
                    )}
                  </span>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  data-testid="day-pack-kit-checkout"
                  className="!px-1.5"
                  onClick={() => void openCheckout({ kitId: kit.kit_id, label: kit.name })}
                >
                  Check out kit
                </Button>
              </li>
            );
          })}
        </ul>
      )}
      {lines.length > 0 && (
        <ul className="grid gap-0.5" role="list">
          {lines.map((line) => (
            <LineRow
              key={line.line_key}
              line={line}
              busy={busyLine === line.line_key}
              onToggle={() =>
                void toggleTick(
                  targetDate,
                  { line_key: line.line_key, ticket_id: line.ticket_id },
                  line.tick !== null && line.tick !== undefined,
                )
              }
              onCheckout={() =>
                void openCheckout({
                  itemId: line.stock_item_id,
                  locationId: line.source?.location_id,
                  label: line.item_name,
                })
              }
              onUse={() => void openUse(line)}
            />
          ))}
        </ul>
      )}
      {reminders.length > 0 && (
        <ul className="grid gap-0.5" role="list">
          {reminders.map((reminder) => (
            <li
              key={`${reminder.ticket_id}-${reminder.text}`}
              data-testid="day-pack-reminder"
              className="flex items-center gap-2 py-0.5 text-[11px]"
              style={{ color: "var(--m-text-muted)" }}
            >
              <span aria-hidden="true">☐</span>
              <span className="truncate">{reminder.text}</span>
              <Button
                variant="ghost"
                size="sm"
                className="!px-1.5"
                data-testid="day-pack-pin"
                onClick={() => void openPin(reminder.ticket_id)}
              >
                Pin item
              </Button>
            </li>
          ))}
        </ul>
      )}
    </>
  );

  const startNote = startPointNote(view.start_point?.kind, view.start_point?.label);

  return (
    <section data-testid="day-pack" aria-label="Day pack" className="m-rise grid gap-3">
      <Card className="grid gap-3 p-3">
        <div className="flex flex-wrap items-baseline justify-between gap-2">
          <div className="grid gap-0.5">
            <span
              className="text-[10px] font-semibold uppercase tracking-wide"
              style={{ color: "var(--m-text-muted)" }}
            >
              Day pack
            </span>
            <span className="text-[13px] font-medium">
              {view.counts.stops} stop{view.counts.stops === 1 ? "" : "s"} ·{" "}
              {view.counts.lines} to grab · {view.counts.collected} collected
            </span>
          </div>
          <span className="text-[10px]" style={{ color: "var(--m-text-muted)" }}>
            {startNote}
          </span>
        </div>

        {view.trips.map((trip) => (
          <div
            key={trip.index}
            data-testid={`day-pack-trip-${trip.index}`}
            className="grid gap-2 rounded-[var(--m-radius-container)] border p-2"
            style={{ borderColor: "var(--m-border)", background: "var(--m-canvas)" }}
          >
            <div className="flex flex-wrap items-center gap-2">
              <span
                className="text-[10px] font-semibold uppercase tracking-wide"
                style={{ color: "var(--m-text-muted)" }}
              >
                Trip {trip.index}
              </span>
 <span className="text-[10px] tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                {timeText(trip.opens_at)}
              </span>
              <Chip tone="muted" testid="day-pack-office-chip">
                {trip.opens_at_office ? "Collect at the office" : "Van pack"}
              </Chip>
              <span className="text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                {trip.stops.length} stop{trip.stops.length === 1 ? "" : "s"} ·{" "}
                {trip.lines.length + trip.kits.length} to grab
              </span>
              <span className="h-px flex-1" style={{ background: "var(--m-border)" }} aria-hidden="true" />
            </div>

            <ol className="flex flex-wrap gap-x-3 gap-y-1" role="list">
              {trip.stops.map((stop) => (
                <li key={stop.appointment_id} className="flex items-baseline gap-1.5 text-[11px]">
 <span className="tabular-nums" style={{ color: "var(--m-text-muted)" }}>
                    {timeText(stop.starts_at)}
                  </span>
                  {stop.ticket_id !== null && stop.ticket_id !== undefined && (
                    <a
                      href={`/tickets/${stop.ticket_id}`}
                      className="whitespace-nowrap"
                      style={{ color: "var(--m-text-secondary)" }}
                    >
                      {stop.ticket_number ?? "ticket"}
                    </a>
                  )}
                  <span className="truncate" style={{ color: "var(--m-text)" }}>
                    {stop.site_label ?? stop.client_name ?? stop.title}
                  </span>
                </li>
              ))}
            </ol>

            {(trip.coverage_warnings ?? []).map((warning) => (
              <p
                key={warning}
                data-testid="day-pack-coverage-warning"
                className="text-[11px]"
                style={{ color: "var(--m-status-warning-fg)" }}
              >
                {warning}
              </p>
            ))}

            {trip.lines.length + trip.kits.length > 0 ||
            (trip.reminders ?? []).length > 0 ? (
              renderGrabBody(trip.lines, trip.kits, trip.reminders ?? [], view.date)
            ) : (
              <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                Nothing to collect for this trip.
              </p>
            )}
          </div>
        ))}

        {view.has_stops && view.counts.lines === 0 && (
          <p className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
            Nothing to collect for these stops yet. Pin an item from a ticket, or set dispatch
            defaults under Settings → Toolboxes.
          </p>
        )}

        {view.tomorrow !== undefined && view.tomorrow !== null && (
          <div
            data-testid="day-pack-tomorrow"
            className="grid gap-2 rounded-[var(--m-radius-container)] border p-2"
            style={{ borderColor: "var(--m-border)", background: "var(--m-canvas)" }}
          >
            <div className="flex flex-wrap items-center gap-2">
              <span
                className="text-[10px] font-semibold uppercase tracking-wide"
                style={{ color: "var(--m-text-muted)" }}
              >
                Tomorrow ·{" "}
                {formatDayKey(view.tomorrow.date, "day")}
              </span>
              <span className="text-[10px]" style={{ color: "var(--m-text-muted)" }}>
                {view.tomorrow.stops.length} stop{view.tomorrow.stops.length === 1 ? "" : "s"}
              </span>
            </div>
            <p className="text-[11px]" style={{ color: "var(--m-text-secondary)" }}>
              {tomorrowReasonNote(view.tomorrow.reason)}
            </p>
            {renderGrabBody(
              view.tomorrow.lines,
              view.tomorrow.kits,
              view.tomorrow.reminders ?? [],
              view.tomorrow.date,
            )}
          </div>
        )}

        {error !== null && (
          <p className="text-xs" style={{ color: "var(--m-status-danger-fg)" }} role="alert">
            {error}
          </p>
        )}
      </Card>

      <CheckoutDialog
        open={checkout !== null}
        onOpenChange={(open) => {
          if (!open) setCheckout(null);
        }}
        items={reference?.items ?? []}
        locations={reference?.locations ?? []}
        kits={reference?.kits ?? []}
        holders={reference?.holders ?? []}
        defaultHolderId={meUserId}
        initialItemId={checkout?.itemId}
        initialKitId={checkout?.kitId}
        initialLocationId={checkout?.locationId}
        onCheckedOut={() => {
          setCheckout(null);
          void load();
        }}
      />

      {useSeed !== null && (
        <MovementDialog
          open
          onOpenChange={(open) => {
            if (!open) setUseSeed(null);
          }}
          items={reference?.items ?? []}
          locations={reference?.locations ?? []}
          initialKind="consume"
          initialItemId={useSeed.line.stock_item_id}
          initialQty={qtyText(useSeed.line.qty_milli)}
          ticketId={useSeed.line.ticket_id}
          ticketLabel={useSeed.ticketLabel}
          title={`Use on ${useSeed.ticketLabel}`}
          searchableItems
          scanSurface="ticket-parts"
          onRecorded={() => {
            setUseSeed(null);
            void load();
          }}
        />
      )}

      {pinTicket !== null && (
        <PickDialog
          open
          onOpenChange={(open) => {
            if (!open) setPinTicket(null);
          }}
          ticketId={pinTicket.id}
          ticketLabel={pinTicket.label}
          initialItemId={pinTicket.initialItemId}
          items={reference?.items ?? []}
          onPinned={() => {
            setPinTicket(null);
            void load();
          }}
        />
      )}
    </section>
  );
}

function Chip({
  children,
  tone,
  testid,
}: {
  children: ReactNode;
  tone: "clear" | "warning" | "danger" | "muted" | "accent";
  testid?: string;
}) {
  const fg =
    tone === "danger"
      ? "var(--m-status-danger-fg)"
      : tone === "warning"
        ? "var(--m-status-warning-fg)"
        : tone === "accent"
          ? "var(--m-accent)"
          : tone === "clear"
            ? "var(--m-text-secondary)"
            : "var(--m-text-muted)";
  return (
    <span
      data-testid={testid}
      className="inline-flex items-center whitespace-nowrap rounded-[3px] border px-1 text-[10px] leading-4"
      style={{ borderColor: "var(--m-border)", color: fg }}
    >
      {children}
    </span>
  );
}

function TickButton({
  ticked,
  busy,
  label,
  onToggle,
}: {
  ticked: boolean;
  busy: boolean;
  label: string;
  onToggle(): void;
}) {
  return (
    <button
      type="button"
      data-testid="day-pack-tick"
      aria-pressed={ticked}
      aria-label={ticked ? `Mark ${label} not collected` : `Mark ${label} collected`}
      disabled={busy}
      onClick={onToggle}
      className="mt-[1px] grid h-4 w-4 shrink-0 cursor-pointer place-items-center rounded-[3px] border transition-opacity duration-100 disabled:cursor-wait disabled:opacity-50 motion-reduce:transition-none"
      style={
        ticked
          ? {
              background: "var(--m-accent)",
              borderColor: "var(--m-accent)",
              color: "var(--m-on-accent)",
            }
          : {
              borderColor: "var(--m-border)",
              background: "var(--m-surface)",
              color: "transparent",
            }
      }
    >
      <Icon name="check" size={9} />
    </button>
  );
}

function LineRow({
  line,
  busy,
  onToggle,
  onCheckout,
  onUse,
}: {
  line: DayPackLine;
  busy: boolean;
  onToggle(): void;
  onCheckout(): void;
  onUse(): void;
}) {
  const ticked = line.tick !== null && line.tick !== undefined;
  const holder = holderText(line.availability);
  return (
    <li
      data-testid="day-pack-line"
      data-line-key={line.line_key}
      data-item={line.item_name}
      className="row-hover grid grid-cols-[auto_minmax(0,1fr)_auto] items-start gap-2 py-1.5 sm:grid-cols-[auto_minmax(0,1fr)_auto_auto]"
    >
      <TickButton ticked={ticked} busy={busy} label={line.item_name} onToggle={onToggle} />
      <div className="grid min-w-0 gap-0.5">
        <span className="flex min-w-0 flex-wrap items-baseline gap-1.5 text-xs">
          <span
            className={ticked ? "truncate line-through" : "truncate"}
            title={line.item_name}
            style={{ color: ticked ? "var(--m-text-muted)" : "var(--m-text)" }}
          >
            {line.item_name}
          </span>
          {line.secondary !== null && line.secondary !== undefined && line.secondary !== "" && (
            <span className="text-[10px]" style={{ color: "var(--m-text-muted)" }}>
              {line.secondary}
            </span>
          )}
          <span
 className="text-[10px] tabular-nums"
            style={{ color: "var(--m-text-secondary)" }}
          >
            {qtyText(line.qty_milli)} {line.unit}
          </span>
        </span>
        <span
          className="flex flex-wrap items-center gap-1.5 text-[10px]"
          style={{ color: "var(--m-text-muted)" }}
        >
          <Chip tone="muted">{provenanceLabel(line.provenance)}</Chip>
          <span>{sourceLabel(line)}</span>
          {line.note !== null && line.note !== undefined && line.note !== "" && (
            <span className="truncate">· {line.note}</span>
          )}
        </span>
      </div>
      <span className="grid min-w-[7rem] justify-items-end gap-0.5 text-right">
        <Chip tone={availabilityTone(line.availability)} testid="day-pack-availability">
          {availabilityLabel(line.availability)}
        </Chip>
        {holder !== null && (
          <span
            className="text-[10px]"
            style={{
              color: line.availability.holders?.[0]?.overdue
                ? "var(--m-status-danger-fg)"
                : "var(--m-text-muted)",
            }}
          >
            {holder}
          </span>
        )}
      </span>
      <span className="hidden items-center sm:flex">
        {line.is_tool ? (
          <Button
            variant="ghost"
            size="sm"
            data-testid="day-pack-checkout"
            className="!px-1.5"
            onClick={onCheckout}
          >
            Check out
          </Button>
        ) : (
          <Button
            variant="ghost"
            size="sm"
            data-testid="day-pack-use"
            className="!px-1.5"
            onClick={onUse}
          >
            Use on ticket
          </Button>
        )}
      </span>
    </li>
  );
}
