// Day-surface invariants: optimistic booking state, travel drafts, photo
// notes, and the clock parser behind the travel form.
//
// The booking PATCH carries no IF envelope on the wire, so the queue's
// optimistic render is what makes accept/finish feel instant; these tests
// pin the render, the revert, and the no-cache no-op. Travel drafts follow
// the same optimistic pattern with a real idempotency key.

import type { MeridianClient } from "@meridian/sdk";
import { describe, expect, it } from "vitest";

import { ReadCache } from "@/core/cache";
import { MODELS, type DispatchDay, type TravelDraft } from "@/core/models";
import { MemoryStore } from "@/core/storage";
import {
  WRITE_OPERATIONS,
  addTicketNote,
  createTravelDraft,
  deleteTravelDraft,
  setAppointmentStatus,
  submitTravelDraft,
  type WriteContext,
} from "@/core/writes";
import { parseClock } from "@/day/clock";

import { FakeApi } from "./helpers";

const DAY_KEY = MODELS.dispatchDay("2026-09-26");

function dayFixture(status: "scheduled" | "in_progress" | "done" = "scheduled"): DispatchDay {
  return {
    date: "2026-09-26",
    from: "2026-09-26T00:00:00Z",
    to: "2026-09-27T00:00:00Z",
    technicians: [
      {
        user_id: "user-1",
        display_name: "Tech",
        timezone: "UTC",
        available_minutes: 480,
        booked_minutes: 120,
        utilization_percent: 25,
        appointments: [
          {
            id: "appt-1",
            user_id: "user-1",
            ticket_id: "ticket-1",
            ticket_number: "T-1001",
            client_id: "client-1",
            client_name: "Acme",
            title: "Boiler service",
            starts_at: "2026-09-26T09:00:00Z",
            ends_at: "2026-09-26T11:00:00Z",
            kind: "job",
            status,
            travel_minutes: 18,
            dispatch_status: null,
          },
        ],
        on_call: [],
        exclusions: [],
        availability_windows: [],
      },
    ],
    unassigned: [],
  } as unknown as DispatchDay;
}

function context(cache: ReadCache): WriteContext {
  return {
    cache,
    uuid: () => "00000000-0000-4000-8000-000000000001",
    now: () => new Date("2026-09-26T09:05:00Z"),
  };
}

function cacheWithDay(status: "scheduled" | "in_progress" | "done" = "scheduled") {
  const cache = new ReadCache(new MemoryStore());
  return cache.set(DAY_KEY, dayFixture(status)).then(() => cache);
}

describe("setAppointmentStatus", () => {
  it("renders the accepted state optimistically and restores it on revert", async () => {
    const cache = await cacheWithDay("scheduled");
    const args = {
      appointmentId: "appt-1",
      appointmentTitle: "Acme",
      date: "2026-09-26",
      status: "in_progress" as const,
      previousStatus: "scheduled" as const,
    };
    await setAppointmentStatus.optimistic?.(args, { idempotencyKey: "k1" }, context(cache));
    let data = cache.get(DAY_KEY)?.data as DispatchDay;
    expect(data.technicians[0]!.appointments[0]!.status).toBe("in_progress");

    await setAppointmentStatus.revert?.(args, { idempotencyKey: "k1" }, context(cache));
    data = cache.get(DAY_KEY)?.data as DispatchDay;
    expect(data.technicians[0]!.appointments[0]!.status).toBe("scheduled");
  });

  it("leaves the cache untouched when the day was never loaded", async () => {
    const cache = new ReadCache(new MemoryStore());
    await setAppointmentStatus.optimistic?.(
      {
        appointmentId: "appt-1",
        date: "2026-09-26",
        status: "done",
        previousStatus: "in_progress",
      },
      { idempotencyKey: "k2" },
      context(cache),
    );
    expect(cache.keys()).not.toContain(DAY_KEY);
  });

  it("sends the status PATCH against the appointment", async () => {
    const api = new FakeApi();
    const args = {
      appointmentId: "appt-1",
      date: "2026-09-26",
      status: "in_progress" as const,
      previousStatus: "scheduled" as const,
    };
    const body = setAppointmentStatus.build(args, { idempotencyKey: "k3" });
    await setAppointmentStatus.send(api.asClient<MeridianClient>(), body);
    expect(api.calls).toEqual([
      { method: "PATCH", path: "/api/v1/appointments/{id}", body: { status: "in_progress" } },
    ]);
  });
});

describe("travel drafts", () => {
  const args = {
    ticketId: "ticket-1",
    ticketNumber: "T-1001",
    startedAt: "2026-09-26T08:20:00Z",
    endedAt: "2026-09-26T08:52:00Z",
    distance: 12.4,
    distanceUnit: "km",
    vehicle: "Van",
    billable: true,
    notes: "Ring road",
  };

  it("renders a pending row optimistically and swaps in the server row", async () => {
    const cache = new ReadCache(new MemoryStore());
    await cache.set(MODELS.travelDrafts, [] as TravelDraft[]);
    const ctx = context(cache);

    await createTravelDraft.optimistic?.(args, { idempotencyKey: "draft-key" }, ctx);
    const pending = cache.get(MODELS.travelDrafts)?.data as TravelDraft[];
    expect(pending).toHaveLength(1);
    expect(pending[0]!.id).toBe("pending-draft-key");
    expect(pending[0]!.status).toBe("draft");

    await createTravelDraft.apply?.(
      { id: "draft-1", ticket_id: "ticket-1", status: "draft" },
      args,
      { idempotencyKey: "draft-key" },
      ctx,
    );
    const applied = cache.get(MODELS.travelDrafts)?.data as TravelDraft[];
    expect(applied).toHaveLength(1);
    expect(applied[0]!.id).toBe("draft-1");
  });

  it("puts a rejected delete back", async () => {
    const cache = new ReadCache(new MemoryStore());
    const row = {
      id: "draft-9",
      ticket_id: "ticket-1",
      user_id: "user-1",
      started_at: "2026-09-26T08:20:00Z",
      ended_at: "2026-09-26T08:52:00Z",
      distance: 4,
      distance_unit: "mi",
      vehicle: "Truck",
      billable: false,
      notes: null,
      status: "draft",
      submitted_time_entry_id: null,
      expense_id: null,
      created_at: "2026-09-26T08:55:00Z",
      updated_at: "2026-09-26T08:55:00Z",
    } as unknown as TravelDraft;
    await cache.set(MODELS.travelDrafts, [row]);
    const ctx = context(cache);

    await deleteTravelDraft.optimistic?.({ draftId: "draft-9" }, { idempotencyKey: "k" }, ctx);
    expect(cache.get(MODELS.travelDrafts)?.data).toEqual([]);

    await deleteTravelDraft.revert?.(
      { draftId: "draft-9", snapshot: row },
      { idempotencyKey: "k" },
      ctx,
    );
    expect(cache.get(MODELS.travelDrafts)?.data).toEqual([row]);
  });

  it("posts the draft body and submits through the draft path", async () => {
    const api = new FakeApi();
    const createBody = createTravelDraft.build(args, { idempotencyKey: "draft-key" });
    expect(createBody.idempotency_key).toBe("draft-key");
    await createTravelDraft.send(api.asClient<MeridianClient>(), createBody);
    expect(api.calls[0]?.method).toBe("POST");
    expect(api.calls[0]?.path).toBe("/api/v1/dispatch/travel-drafts");
    const posted = api.calls[0]?.body as Record<string, unknown>;
    expect(posted["ticket_id"]).toBe("ticket-1");
    expect(posted["distance_unit"]).toBe("km");

    const submitBody = submitTravelDraft.build({ draftId: "draft-1" }, { idempotencyKey: "s1" });
    await submitTravelDraft.send(api.asClient<MeridianClient>(), submitBody);
    expect(api.calls[1]?.method).toBe("POST");
    expect(api.calls[1]?.path).toBe("/api/v1/dispatch/travel-drafts/{id}/submit");
    expect((api.calls[1]?.body as Record<string, unknown>)["idempotency_key"]).toBe("s1");
  });

  it("registers every new operation for relaunch replay", () => {
    for (const kind of [
      "appointment.status",
      "travel.create",
      "travel.submit",
      "travel.delete",
    ]) {
      expect(WRITE_OPERATIONS).toHaveProperty(kind);
    }
  });
});

describe("photo notes", () => {
  it("carries staged attachment ids on the comment create", () => {
    const body = addTicketNote.build(
      {
        ticketId: "ticket-1",
        body: "Panel swapped",
        kind: "internal",
        attachmentIds: ["att-1", "att-2"],
      },
      { idempotencyKey: "note-key" },
    );
    expect(body.attachment_ids).toEqual(["att-1", "att-2"]);
    expect(body.idempotency_key).toBe("note-key");
  });

  it("omits the field when nothing was staged", () => {
    const body = addTicketNote.build(
      { ticketId: "ticket-1", body: "No photos" },
      { idempotencyKey: "note-key" },
    );
    expect(body).not.toHaveProperty("attachment_ids");
  });
});

describe("parseClock", () => {
  const day = new Date("2026-09-26T15:00:00");

  it("reads the touch-friendly forms", () => {
    expect(parseClock("08:30", day)?.getHours()).toBe(8);
    expect(parseClock("8:05", day)?.getMinutes()).toBe(5);
    expect(parseClock(" 17:45 ", day)?.getHours()).toBe(17);
  });

  it("rejects nonsense instead of coercing it", () => {
    expect(parseClock("", day)).toBeNull();
    expect(parseClock("24:00", day)).toBeNull();
    expect(parseClock("12:60", day)).toBeNull();
    expect(parseClock("noon", day)).toBeNull();
  });
});
