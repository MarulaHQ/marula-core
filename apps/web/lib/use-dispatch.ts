// Dispatch board data + realtime leg (TASKLIST 2.19).
//
// Purpose: one hook owning the /dispatch page's server state: day-view
// fetches, the org-channel socket subscription so appointment
// created/updated/cancelled events reconcile the board live, presence
// queries for the online dots, and the drop handlers that wrap
// bulk-move/assign calls with optimistic confirm/reject dispatches.
//
// Non-purpose: no rendering and no pointer/keyboard plumbing (the page
// maps DOM events onto reducer actions); no availability editing; drag
// ghost math lives in the reducer.

"use client";

import type { components } from "@meridian/sdk";
import { api, loadTokens, useAuth } from "./api";
import {
  dispatchReducer,
  initialDispatchState,
  type Block,
  type BoardState,
  type QueueItem,
  type Tech,
} from "./dispatch-reducer";
import { useCallback, useEffect, useReducer, useRef, useState } from "react";

type TechnicianView = components["schemas"]["TechnicianView"];
type SdkBlock = components["schemas"]["DispatchAppointment"];
type SdkQueueItem = components["schemas"]["MyWorkItem"];

export interface RawDayView {
  date: string;
  technicians: TechnicianView[];
}

function wsUrl(): string {
  if (process.env.NEXT_PUBLIC_WS_URL) return process.env.NEXT_PUBLIC_WS_URL;
  const base = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8080";
  const u = new URL(base);
  u.protocol = u.protocol === "https:" ? "wss:" : "ws:";
  u.port = process.env.NEXT_PUBLIC_WS_PORT ?? "8081";
  u.pathname = "/ws";
  return u.toString();
}

function sdkToTech(t: TechnicianView): Tech {
  return {
    user_id: t.user_id,
    display_name: t.display_name,
    available_minutes: t.available_minutes,
    booked_minutes: t.booked_minutes,
    utilization_percent: t.utilization_percent,
    appointments: (t.appointments ?? []).map(sdkToBlock),
    availability_windows: (t.availability_windows ?? []).map((w) => ({
      weekday: w.weekday,
      start_local: w.start_local,
      end_local: w.end_local,
      timezone: w.timezone,
    })),
    exclusions: (t.exclusions ?? []).map((x) => ({
      id: x.id,
      starts_at: x.starts_at,
      ends_at: x.ends_at,
    })),
  };
}

export function sdkToBlock(a: SdkBlock): Block {
  return {
    id: a.id,
    user_id: a.user_id,
    ticket_id: a.ticket_id ?? null,
    ticket_number: a.ticket_number ?? null,
    client_name: a.client_name ?? null,
    title: a.title,
    starts_at: a.starts_at,
    ends_at: a.ends_at,
    status: a.status,
    travel_minutes: a.travel_minutes,
  };
}

function sdkQueueItem(t: SdkQueueItem): QueueItem {
  return {
    id: t.id,
    number: t.number,
    title: t.title,
    priority: t.priority,
    due_at: t.due_at ?? null,
    sla_resolution_due: t.sla_resolution_due ?? null,
  };
}

/** Typed rejection details the API merges into `error` on 409. */
interface RejectionBody {
  error?: { message?: string; blocker_title?: string };
}

async function readError(response: Response): Promise<string> {
  try {
    const body = (await response.json()) as RejectionBody;
    const detail = body.error?.blocker_title
      ? `${body.error.message}: ${body.error.blocker_title}`
      : body.error?.message;
    if (detail) return detail;
  } catch {
    // fall through to generic text
  }
  return `request failed (${response.status})`;
}

export function useDispatchBoard(initialDate: string) {
  const auth = useAuth();
  const [state, dispatch] = useReducer(dispatchReducer, undefined, () =>
    initialDispatchState(),
  );
  const [date, setDate] = useState(initialDate);
  const [rawView, setRawView] = useState<RawDayView | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  // Ref mirror of pending temps so event frames can route confirms.
  const stateRef = useRef<BoardState>(state);
  stateRef.current = state;

  const load = useCallback(async (forDate: string) => {
    setLoading(true);
    setLoadError(null);
    try {
      const { data, error } = await api.api.GET("/api/v1/dispatch/day", {
        params: { query: { date: forDate as never } },
      });
      if (error !== undefined || !data) {
        setLoadError("Failed to load the dispatch board");
        return;
      }
      setRawView({
        date: data.date,
        technicians: data.technicians,
      });
      dispatch({
        type: "loaded",
        technicians: data.technicians.map(sdkToTech),
        unassigned: data.unassigned.map(sdkQueueItem),
      });
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (auth.status === "authed") void load(date);
  }, [auth.status, date, load]);

  // Live leg: org channel reconcile + presence. Same reconnect/backoff
  // contract as the notification dock.
  useEffect(() => {
    if (auth.status !== "authed" || !auth.me) return;
    let closed = false;
    let attempt = 0;
    let retry: ReturnType<typeof setTimeout> | undefined;

    const connect = () => {
      const tokens = loadTokens();
      if (!tokens || closed) return;
      const sock = new WebSocket(`${wsUrl()}?token=${tokens.access_token}`);
      sock.onopen = () => {
        attempt = 0;
        sock.send(
          JSON.stringify({ type: "subscribe", channel: `org:${auth.me!.org_id}` }),
        );
        sock.send(JSON.stringify({ type: "presence_list" }));
      };
      sock.onmessage = (m) => {
        try {
          const frame = JSON.parse(String(m.data)) as {
            type: string;
            users?: string[];
            event?: {
              type?: string;
              payload?: Record<string, unknown>;
            };
          };
          if (frame.type === "presence" && Array.isArray(frame.users)) {
            dispatch({ type: "presence", userIds: frame.users });
            return;
          }
          const evt = frame.event;
          if (frame.type !== "event" || !evt?.type) return;
          const payload = (evt.payload ?? {}) as Record<string, unknown>;
          const appt = payload.appointment as SdkBlock | undefined;
          if (
            (evt.type === "appointment.created" ||
              evt.type === "appointment.updated") &&
            appt
          ) {
            // Tenancy is enforced at delivery time by the hub (same-org
            // check against the token claim); nothing extra here.
            dispatch({ type: "reconcile_upsert", block: sdkToBlock(appt) });
          } else if (evt.type === "appointment.cancelled") {
            const id = payload.appointment_id as string | undefined;
            if (id) dispatch({ type: "reconcile_remove", apptId: id });
          }
        } catch {
          // malformed frames ignored; REST remains source of truth
        }
      };
      sock.onclose = () => {
        if (closed) return;
        attempt += 1;
        retry = setTimeout(connect, Math.min(1000 * 2 ** attempt, 15000));
      };
      return sock;
    };

    const sockRef = { current: null as WebSocket | null };
    sockRef.current = connect() ?? null;

    // Periodic capacity resync — optimistic stats drift after remote
    // creates outside this browser's view window.
    const resync = setInterval(() => {
      if (!closed && document.visibilityState === "visible") void load(date);
    }, 60_000);

    return () => {
      closed = true;
      clearInterval(resync);
      if (retry) clearTimeout(retry);
      sockRef.current?.close();
    };
  }, [auth.status, auth.me, date, load]);

  // --- drop handlers wrapping mutations with optimistic bookkeeping -----

  const dropMove = useCallback(
    async (apptId: string, userId: string, startsAt: string, endsAt: string) => {
      dispatch({ type: "drop_move", apptId, userId, startsAt, endsAt });
      const response = await api.api.POST("/api/v1/dispatch/bulk-move", {
        body: {
          moves: [{ appointment_id: apptId, user_id: userId, starts_at: startsAt, ends_at: endsAt }],
        },
      });
      if (response.error === undefined && response.data) {
        const moved = response.data.moved[0];
        if (moved) dispatch({ type: "reconcile_upsert", block: sdkToBlock(moved) });
        dispatch({ type: "confirm", key: apptId });
      } else {
        const err = await readError(response.response);
        dispatch({ type: "reject", key: apptId, message: err });
      }
    },
    [],
  );

  const dropAssign = useCallback(
    async (
      ticket: QueueItem,
      userId: string,
      startsAt: string,
      endsAt: string,
      travelMinutes: number,
    ) => {
      const tempId =
        typeof crypto !== "undefined" && "randomUUID" in crypto
          ? crypto.randomUUID()
          : `tmp-${Date.now()}`;
      dispatch({
        type: "drop_assign",
        tempId,
        ticket,
        userId,
        startsAt,
        endsAt,
      });
      const response = await api.api.POST("/api/v1/dispatch/assign", {
        body: {
          ticket_id: ticket.id,
          user_id: userId,
          starts_at: startsAt,
          ends_at: endsAt,
          travel_minutes: travelMinutes,
        },
      });
      if (response.error === undefined && response.data) {
        dispatch({ type: "reconcile_upsert", block: sdkToBlock(response.data) });
        dispatch({ type: "confirm", key: tempId });
      } else {
        const err = await readError(response.response);
        dispatch({ type: "reject", key: tempId, message: err });
      }
    },
    [],
  );

  return {
    state,
    rawView,
    date,
    setDate,
    loading,
    loadError,
    reload: useCallback(() => load(date), [date, load]),
    dispatch,
    dropMove,
    dropAssign,
  };
}
