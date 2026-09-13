// /settings/sla — business-hours calendars + escalation ladders (2.6).
//
// Purpose: edit org SLA business-hours calendars (weekday windows,
// holidays, timezone) over the existing /api/v1/sla/calendars endpoints;
// list SLA policies with a plain-language target summary and each
// calendar's referenced-by count; edit the escalation ladder per policy
// (offset % → notification kind, fire-once contract explained) via the
// ladder GET/PUT pair.
//
// Non-purpose: no policy creation/target editing (no admin endpoint
// exists yet — deferred), no SLA evaluation or sweep logic here.
"use client";

import { friendlyMutationText } from "../../../lib/error-copy";
import {
  Button,
  Card,
  EmptyState,
  FieldHint,
  Input,
  Label,
  Tag,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import { useEffect, useMemo, useReducer, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import {
  FIRE_ONCE_NOTE,
  WEEKDAY_LABELS,
  emptyCalendarDraft,
  calendarDraftFromBusinessCalendar,
  calendarReducer,
  kindExplanation,
  ladderPayload,
  ladderReducer,
  policyTargetSummary,
  validateCalendar,
  validateLadder,
  type CalendarDraftState,
  type LadderKind,
  type LadderRow,
} from "../../../lib/admin/sla";

type SlaCalendar = components["schemas"]["SlaCalendar"];
type PolicyView = components["schemas"]["PolicyView"];
type EscalationStep = components["schemas"]["EscalationStep"];

interface BusinessCalendarDoc {
  name?: string;
  timezone?: string;
  windows?: ([string, string] | null)[];
  holidays?: string[];
}

/** Loosely parses the JSONB calendar body on reads. */
function calendarBody(cal: SlaCalendar): BusinessCalendarDoc {
  return (cal.calendar ?? {}) as BusinessCalendarDoc;
}

function draftToBusinessPayload(state: CalendarDraftState): {
  name: string;
  timezone: string;
  windows: ([string, string] | null)[];
  holidays: string[];
} {
  return {
    name: state.name.trim(),
    timezone: state.timezone.trim() || "UTC",
    windows: state.windows.map((w) => (w ? [w[0], w[1]] : null)),
    holidays: [...state.holidays],
  };
}

export default function SlaSettingsPage() {
  const auth = useRequireAuth();
  const [calendars, setCalendars] = useState<SlaCalendar[]>([]);
  const [policies, setPolicies] = useState<PolicyView[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);

  // Calendar editor state: which saved calendar is being edited (null-id =
  // creating a new one).
  const [editingId, setEditingId] = useState<string | null>(null);
  const [calDraft, dispatchCal] = useReducer(calendarReducer, undefined, emptyCalendarDraft);
  const [calError, setCalError] = useState<string | null>(null);
  const [calSaved, setCalSaved] = useState(false);
  const [savingCal, setSavingCal] = useState(false);
  const [holidayInput, setHolidayInput] = useState("");

  // Ladder editor state per open policy.
  const [openPolicyId, setOpenPolicyId] = useState<string | null>(null);
  const [ladder, dispatchLadder] = useReducer(ladderReducer, { rows: [] });
  const [ladderError, setLadderError] = useState<string | null>(null);
  const [ladderSaved, setLadderSaved] = useState(false);
  const [savingLadder, setSavingLadder] = useState(false);

  const refresh = async () => {
    const [calRes, polRes] = await Promise.all([
      api.api.GET("/api/v1/sla/calendars"),
      api.api.GET("/api/v1/sla/policies"),
    ]);
    if (calRes.error !== undefined || calRes.data === undefined) {
      setLoadError("Failed to load SLA configuration.");
      setLoading(false);
      return;
    }
    setCalendars(calRes.data);
    if (polRes.error === undefined && polRes.data !== undefined) setPolicies(polRes.data);
    setLoading(false);
  };

  useEffect(() => {
    if (auth.status !== "authed") return;
    void refresh();
  }, [auth.status]);

  const refCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const p of policies) {
      if (p.calendar_id !== null && p.calendar_id !== undefined) {
        counts.set(p.calendar_id, (counts.get(p.calendar_id) ?? 0) + 1);
      }
    }
    return counts;
  }, [policies]);

  const startEdit = (cal: SlaCalendar) => {
    const body = calendarBody(cal);
    const draft = calendarDraftFromBusinessCalendar(cal.name, {
      timezone: body.timezone ?? "UTC",
      windows: body.windows ?? [],
      holidays: body.holidays ?? [],
    });
    setEditingId(cal.id);
    dispatchCal({ type: "init", draft });
    setCalSaved(false);
    setCalError(null);
  };

  const startCreate = () => {
    setEditingId(null);
    dispatchCal({ type: "init", draft: emptyCalendarDraft() });
    setCalSaved(false);
    setCalError(null);
  };

  const saveCalendar = async () => {
    setSavingCal(true);
    setCalSaved(false);
    const errors = validateCalendar(calDraft);
    const firstCalError = Object.keys(errors)
      .map((k) => errors[k])
      .find((msg) => msg !== undefined);
    if (firstCalError !== undefined) {
      setCalError(firstCalError);
      setSavingCal(false);
      return;
    }
    const payload = draftToBusinessPayload(calDraft);
    try {
      if (editingId === null) {
        const res = await api.api.POST("/api/v1/sla/calendars", {
          body: {
            name: payload.name,
            calendar: {
              name: payload.name,
              timezone: payload.timezone,
              windows: payload.windows,
              holidays: payload.holidays,
            },
          },
        });
        if (res.error || !res.data) throw new Error("the calendar was rejected");
        setEditingId(res.data.id);
      } else {
        const res = await api.api.PATCH("/api/v1/sla/calendars/{id}", {
          params: { path: { id: editingId } },
          body: {
            name: payload.name,
            timezone: payload.timezone,
            windows: payload.windows,
            holidays: payload.holidays,
          },
        });
        if (res.error || !res.data) throw new Error("the calendar was rejected");
      }
      setCalSaved(true);
      setCalError(null);
      await refresh();
    } catch (e) {
      setCalError(friendlyMutationText(e));
    } finally {
      setSavingCal(false);
    }
  };

  const deleteCalendar = async () => {
    if (editingId === null) return;
    try {
      const res = await api.api.DELETE("/api/v1/sla/calendars/{id}", {
        params: { path: { id: editingId } },
      });
      if (res.error) throw new Error("delete failed");
      await refresh();
      startCreate();
    } catch (e) {
      setCalError(friendlyMutationText(e));
    }
  };

  const openLadder = async (policyId: string) => {
    if (openPolicyId === policyId) {
      setOpenPolicyId(null);
      return;
    }
    setOpenPolicyId(policyId);
    setLadderError(null);
    setLadderSaved(false);
    const res = await api.api.GET("/api/v1/sla/policies/{id}/ladder", {
      params: { path: { id: policyId } },
    });
    dispatchLadder({ type: "init", steps: res.data ?? [] });
  };

  const saveLadder = async () => {
    if (openPolicyId === null) return;
    const validation = validateLadder(ladder);
    const firstLadderError = Object.keys(validation.errorsByIndex)
      .map((k) => validation.errorsByIndex[Number(k)])
      .find((msg) => msg !== undefined);
    if (firstLadderError !== undefined) {
      setLadderError(firstLadderError);
      return;
    }
    setSavingLadder(true);
    setLadderError(null);
    try {
      const res = await api.api.PUT("/api/v1/sla/policies/{id}/ladder", {
        params: { path: { id: openPolicyId } },
        body: ladderPayload(ladder),
      });
      if (res.error || !res.data) throw new Error("the ladder was rejected");
      dispatchLadder({ type: "init", steps: res.data });
      setLadderSaved(true);
    } catch (e) {
      setLadderError(friendlyMutationText(e));
    } finally {
      setSavingLadder(false);
    }
  };

  if (auth.status !== "authed") return null;

  const calErrors = validateCalendar(calDraft);

  return (
    <div className="grid gap-6 max-w-3xl">
      <header>
        <h1 className="text-lg font-semibold" style={{ color: "var(--m-text)" }}>
          SLA
        </h1>
        <p className="mt-1 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
          Business-hours calendars govern every SLA clock; escalation ladders fire once per ticket
          when consumed budget crosses an offset.
        </p>
      </header>

      {/* --- Calendars -------------------------------------------------- */}
      <Card className="p-5 grid gap-3">
        <div className="flex items-center justify-between">
          <h2 className="text-sm font-semibold">Business-hours calendars</h2>
          <Button variant="secondary" onClick={startCreate}>
            New calendar
          </Button>
        </div>

        {loading ? (
          <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
            Loading…
          </p>
        ) : (
          <div role="table" aria-label="Calendars" className="grid gap-0">
            {calendars.length === 0 && (
              <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                No calendars yet — create one to start timing SLAs.
              </p>
            )}
            {calendars.map((cal) => (
              <button
                key={cal.id}
                type="button"
                onClick={() => startEdit(cal)}
                className="flex items-center justify-between gap-3 text-left px-2 py-2 rounded-[2px]"
                style={{
                  borderBottom: "1px solid var(--m-border)",
                  background: editingId === cal.id ? "var(--m-surface)" : "transparent",
                  cursor: "pointer",
                }}
                aria-current={editingId === cal.id ? "true" : undefined}
              >
                <span className="flex items-center gap-2">
                  <span className="text-[13px]" style={{ color: "var(--m-text)" }}>
                    {cal.name}
                  </span>
                  <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                    {(calendarBody(cal).timezone ?? "") !== "" && `${calendarBody(cal).timezone}`}
                  </span>
                </span>
                <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                  referenced by{" "}
                  {(refCounts.get(cal.id) ?? 0) > 0
                    ? `${refCounts.get(cal.id)} ${refCounts.get(cal.id) === 1 ? "policy" : "policies"}`
                    : "nothing"}
                </span>
              </button>
            ))}
          </div>
        )}

        {/* Editor surface (keyed remount keeps draft lifecycle honest). */}
        <div className="grid gap-3 pt-2" style={{ borderTop: "1px solid var(--m-border)" }}>
          <label className="grid gap-1 max-w-sm">
            <Label htmlFor="cal-name">Name</Label>
            <Input
              id="cal-name"
              value={calDraft.name}
              onChange={(e) => dispatchCal({ type: "set-name", value: e.target.value })}
            />
            {calErrors["name"] !== undefined && (
              <span role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {calErrors["name"]}
              </span>
            )}
          </label>
          <label className="grid gap-1 max-w-xs">
            <Label htmlFor="cal-tz">Timezone</Label>
            <Input
              id="cal-tz"
              value={calDraft.timezone}
              onChange={(e) => dispatchCal({ type: "set-timezone", value: e.target.value })}
            />
            <FieldHint>IANA zone name, e.g. Europe/Berlin.</FieldHint>
          </label>

          <div role="table" aria-label="Weekly opening hours" className="grid gap-1 max-w-md">
            {WEEKDAY_LABELS.map((day, i) => {
              const window = calDraft.windows[i];
              const dayError = calErrors[`window-${i}`];
              return (
                <div key={day} role="row" className="flex items-center gap-2">
                  <label className="inline-flex items-center gap-2 text-[12px] w-32">
                    <input
                      type="checkbox"
                      checked={window != null}
                      onChange={() => dispatchCal({ type: "toggle-day", index: i })}
                      aria-label={`${day} open`}
                    />
                    {day}
                  </label>
                  {window == null ? (
                    <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                      closed
                    </span>
                  ) : (
                    <>
                      <Input
                        type="time"
                        aria-label={`${day} opens`}
                        value={window[0]}
                        onChange={(e) =>
                          dispatchCal({ type: "set-window", index: i, half: 0, value: e.target.value })
                        }
                        className="w-24"
                      />
                      <span style={{ color: "var(--m-text-muted)" }}>–</span>
                      <Input
                        type="time"
                        aria-label={`${day} closes`}
                        value={window[1]}
                        onChange={(e) =>
                          dispatchCal({ type: "set-window", index: i, half: 1, value: e.target.value })
                        }
                        className="w-24"
                      />
                      {dayError !== undefined && (
                        <span role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                          {dayError}
                        </span>
                      )}
                    </>
                  )}
                </div>
              );
            })}
          </div>

          <div className="grid gap-1">
            <Label htmlFor="cal-holiday">Holidays</Label>
            <form
              className="flex items-center gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                if (/^\d{4}-\d{2}-\d{2}$/.test(holidayInput)) {
                  dispatchCal({ type: "add-holiday", date: holidayInput });
                  setHolidayInput("");
                }
              }}
            >
              <Input
                id="cal-holiday"
                type="date"
                value={holidayInput}
                onChange={(e) => setHolidayInput(e.target.value)}
                className="w-44"
              />
              <Button type="submit" variant="secondary" disabled={!/^\d{4}-\d{2}-\d{2}$/.test(holidayInput)}>
                Add
              </Button>
            </form>
            {calDraft.holidays.length === 0 ? (
              <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                No holidays — clocks run every local day inside open windows.
              </p>
            ) : (
              <div className="flex flex-wrap gap-1 pt-1">
                {calDraft.holidays.map((date) => (
                  <Tag
                    key={date}
                    tone="outline"
                    role="listitem"
                    title="Click to remove"
                    onClick={() => dispatchCal({ type: "remove-holiday", date })}
                  >
                    {date} ✕
                  </Tag>
                ))}
              </div>
            )}
          </div>

          <div className="flex items-center gap-3 flex-wrap">
            <Button disabled={savingCal} onClick={() => void saveCalendar()}>
              {savingCal ? "Saving…" : editingId === null ? "Create calendar" : "Save calendar"}
            </Button>
            {editingId !== null && (
              <Button variant="ghost" onClick={() => void deleteCalendar()}>
                Delete
              </Button>
            )}
            {editingId !== null && (refCounts.get(editingId) ?? 0) > 0 && (
              <span className="text-[11px]" style={{ color: "var(--m-status-warning-fg)" }}>
                Referenced by {refCounts.get(editingId)}{" "}
                {refCounts.get(editingId) === 1 ? "policy" : "policies"} — deleting detaches them.
              </span>
            )}
            {calError !== null && (
              <span role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {calError}
              </span>
            )}
            {calSaved && calError === null && (
              <span className="text-[12px]" style={{ color: "var(--m-status-success-fg)" }}>
                Saved.
              </span>
            )}
          </div>
        </div>
      </Card>

      {/* --- Policies --------------------------------------------------- */}
      <Card className="p-5 grid gap-3">
        <h2 className="text-sm font-semibold">Policies &amp; escalation ladders</h2>
        {loading ? (
          <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
            Loading…
          </p>
        ) : policies.length === 0 ? (
          <EmptyState message="No SLA policies are defined for this organization." />
        ) : (
          <div role="table" aria-label="SLA policies" className="grid gap-0">
            {policies.map((policy) => (
              <div key={policy.id} role="row" className="grid gap-2 py-2" style={{ borderBottom: "1px solid var(--m-border)" }}>
                <div className="flex items-center justify-between gap-3 flex-wrap">
                  <div className="flex items-center gap-2">
                    <span className="text-[13px]" style={{ color: "var(--m-text)" }}>
                      {policy.name}
                    </span>
                    {policy.is_default && <Tag tone="neutral">default</Tag>}
                    {policy.uses_embedded_calendar && <Tag tone="outline">embedded calendar</Tag>}
                  </div>
                  <code className="text-[11px] tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                    {policyTargetSummary(policy)}
                  </code>
                </div>
                <div className="flex items-center gap-2">
                  <Button variant="ghost" onClick={() => void openLadder(policy.id)}>
                    {openPolicyId === policy.id ? "Hide ladder" : "Escalation ladder"}
                  </Button>
                </div>
                {openPolicyId === policy.id && (
                  <div className="grid gap-2 p-3 rounded-[2px]" style={{ border: "1px solid var(--m-border)" }}>
                    <p className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                      Steps fire when consumed budget reaches their offset percent. {FIRE_ONCE_NOTE}
                    </p>
                    <div role="table" aria-label={`Ladder for ${policy.name}`} className="grid gap-2">
                      {ladder.rows.map((row: LadderRow, index) => (
                        <div key={row.key} role="row" className="flex items-center gap-2 flex-wrap">
                          <span className="text-[12px] tabular-nums w-4" style={{ color: "var(--m-text-muted)" }}>
                            {index + 1}.
                          </span>
                          <Input
                            type="number"
                            min={1}
                            max={500}
                            aria-label={`Step ${index + 1} offset percent`}
                            value={String(row.atPct)}
                            onChange={(e) =>
                              dispatchLadder({
                                type: "update",
                                key: row.key,
                                patch: { atPct: Number(e.target.value) },
                              })
                            }
                            className="w-20"
                          />
                          <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                            % of budget →
                          </span>
                          <select
                            aria-label={`Step ${index + 1} notification kind`}
                            value={row.kind ?? ""}
                            onChange={(e) =>
                              dispatchLadder({
                                type: "update",
                                key: row.key,
                                patch: { kind: e.target.value as LadderKind },
                              })
                            }
                            className="text-[12px] px-2 py-1.5"
                            style={{
                              borderRadius: 2,
                              border: "1px solid var(--m-border)",
                              background: "var(--m-surface)",
                              color: "var(--m-text)",
                            }}
                          >
                            <option value="" disabled>
                              pick kind…
                            </option>
                            <option value="risk">risk</option>
                            <option value="escalate">escalate</option>
                            <option value="breach">breach</option>
                          </select>
                          {row.kind !== null && (
                            <span className="text-[11px]" style={{ color: "var(--m-text-muted)" }}>
                              {kindExplanation(row.kind)}
                            </span>
                          )}
                          <label className="inline-flex items-center gap-1.5 text-[12px] ml-auto">
                            <input
                              type="checkbox"
                              checked={row.enabled}
                              onChange={(e) =>
                                dispatchLadder({
                                  type: "update",
                                  key: row.key,
                                  patch: { enabled: e.target.checked },
                                })
                              }
                              aria-label={`Enable step ${index + 1}`}
                            />
                            enabled
                          </label>
                          <Button
                            variant="ghost"
                            aria-label={`Remove step ${index + 1}`}
                            onClick={() => dispatchLadder({ type: "remove", key: row.key })}
                          >
                            ✕
                          </Button>
                        </div>
                      ))}
                      {ladder.rows.length === 0 && (
                        <p className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                          No steps — the policy breaches silently.
                        </p>
                      )}
                    </div>
                    <div className="flex items-center gap-3 flex-wrap">
                      <Button variant="secondary" onClick={() => dispatchLadder({ type: "add-row", defaultAtPct: nextSuggestedOffset(ladder.rows) })}>
                        Add step
                      </Button>
                      <Button disabled={savingLadder} onClick={() => void saveLadder()}>
                        {savingLadder ? "Saving…" : "Save ladder"}
                      </Button>
                      {ladderError !== null && (
                        <span role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                          {ladderError}
                        </span>
                      )}
                      {ladderSaved && ladderError === null && (
                        <span className="text-[12px]" style={{ color: "var(--m-status-success-fg)" }}>
                          Saved.
                        </span>
                      )}
                    </div>
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
        {loadError !== null && (
          <p role="alert" className="text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {loadError}
          </p>
        )}
      </Card>
    </div>
  );
}

/** Next default offset keeps rows ordered: 50 → 75 → 90 → 100… */
function nextSuggestedOffset(rows: { atPct: number }[]): number {
  const used = new Set(rows.map((r) => r.atPct));
  for (const candidate of [50, 75, 90, 100, 150]) {
    if (!used.has(candidate)) return candidate;
  }
  return Math.max(100, ...rows.map((r) => r.atPct)) + 50;
}
