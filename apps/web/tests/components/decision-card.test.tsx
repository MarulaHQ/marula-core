// Decision workbench suggestion card (Wave 23 lane 3): renders recorded
// decision answers with threshold states (act/pending/ignore + shadow
// badge), offers Apply only for live act-state board suggestions,
// degrades to nothing when no answers exist — over the MockApi router.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { DecisionSuggestionsCard } from "../../components/workbench-v2/decision-card";
import { MockApi } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/tickets",
}));

const TICKET_ID = "30000000-0000-4000-8000-000000000001";
const BOARD_ID = "00000000-0000-4000-8000-00000000ba01";

const ANSWERS = {
  ticket_id: TICKET_ID,
  suggestions: [
    {
      surface: "decision_ticket_triage",
      mode: "live",
      provider: "typesafe",
      model: "jev-1.13.0",
      recorded_at: "2026-09-21T12:00:00Z",
      answers: [
        {
          question_id: "actionable",
          answer: { kind: "noul", probability: 0.9 },
          act: 0.8,
          review: 0.5,
          state: "act",
        },
        {
          question_id: "urgency",
          answer: { kind: "score", level: 3 },
          act: 4.0,
          review: 3.0,
          state: "pending",
        },
        {
          question_id: "frustrated",
          answer: { kind: "noul", probability: 0.3 },
          act: 0.75,
          review: 0.5,
          state: "ignore",
        },
        {
          question_id: "board",
          answer: { kind: "choice", picked: 0, picked_label: "Service Desk" },
          act: 0.7,
          review: 0.45,
          state: "act",
        },
      ],
    },
    {
      surface: "decision_alert_triage",
      mode: "shadow",
      provider: "typesafe",
      model: "jev-1.13.0",
      recorded_at: "2026-09-21T12:00:00Z",
      answers: [
        {
          question_id: "severity",
          answer: { kind: "score", level: 4 },
          act: 4.0,
          review: 3.0,
          state: "act",
        },
      ],
    },
  ],
};

function setupApi(suggestions: unknown): MockApi {
  const mock = new MockApi();
  mock.on("GET", `/api/v1/tickets/${TICKET_ID}/decision-suggestions`, () => ({
    json: suggestions,
  }));
  mock.on("GET", "/api/v1/boards/nav", () => ({
    json: [{ id: BOARD_ID, name: "Service Desk", description: null, is_default: false, group_id: null, group_name: null, color: null, sort_order: 0, ticket_count: 0 }],
  }));
  mock.on("PATCH", `/api/v1/tickets/${TICKET_ID}`, () => ({ json: { id: TICKET_ID } }));
  mock.install();
  return mock;
}

describe("DecisionSuggestionsCard", () => {
  beforeEach(() => {
    cleanup();
  });

  it("renders suggestion rows with threshold states and confidence", async () => {
    setupApi(ANSWERS);
    render(<DecisionSuggestionsCard ticketId={TICKET_ID} />);
    expect(await screen.findByTestId("wb-decision-card")).toBeDefined();
    expect(screen.getByText(/Service Desk/)).toBeDefined();
    expect(screen.getAllByTestId("wb-decision-state-act").length).toBeGreaterThan(0);
    expect(screen.getByTestId("wb-decision-state-pending")).toBeDefined();
    expect(screen.getByTestId("wb-decision-state-ignore")).toBeDefined();
    expect(screen.getByText(/90% likely/)).toBeDefined();
    expect(screen.getByText(/Level 4 of 5/)).toBeDefined();
  });

  it("renders the honest shadow badge for shadow-mode surfaces", async () => {
    setupApi(ANSWERS);
    render(<DecisionSuggestionsCard ticketId={TICKET_ID} />);
    await screen.findByTestId("wb-decision-card");
    expect(screen.getByText("SHADOW")).toBeDefined();
  });

  it("offers Apply only for live act-state board suggestions and PATCHes it", async () => {
    const mock = setupApi(ANSWERS);
    render(<DecisionSuggestionsCard ticketId={TICKET_ID} />);
    const apply = await screen.findByTestId("wb-decision-apply-board");
    expect(screen.queryByTestId("wb-decision-apply-urgency")).toBeNull();
    fireEvent.click(apply);
    await waitFor(() => {
      expect(mock.callsTo("PATCH", `/api/v1/tickets/${TICKET_ID}`).length).toBeGreaterThan(0);
    });
    const body = mock.lastBody<{ board_id?: string }>("PATCH", `/api/v1/tickets/${TICKET_ID}`);
    expect(body.board_id).toBe(BOARD_ID);
  });

  it("renders nothing when the org has no recorded answers (degrade-quiet)", () => {
    setupApi({ ticket_id: TICKET_ID, suggestions: [] });
    render(<DecisionSuggestionsCard ticketId={TICKET_ID} />);
    expect(screen.queryByTestId("wb-decision-card")).toBeNull();
  });

  it("renders nothing when the fetch fails (never an empty shell)", () => {
    const mock = new MockApi();
    mock.on("GET", `/api/v1/tickets/${TICKET_ID}/decision-suggestions`, () => ({
      status: 500,
      json: { error: { reason: "boom" } },
    }));
    mock.install();
    render(<DecisionSuggestionsCard ticketId={TICKET_ID} />);
    expect(screen.queryByTestId("wb-decision-card")).toBeNull();
  });
});
