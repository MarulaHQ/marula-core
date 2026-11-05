// Integration test: /reports/author — "describe it" → a reviewed report draft.
//
// Purpose: the page has real state (idle → drafting → ready / needs_fixes /
// refused → accepted / rejected) and a save path, so this suite drives each
// transition through the real component tree with the SDK-typed mock routes the
// dashboards suite uses. Every assertion is a call count, a rendered string or a
// role query. No snapshots.
//
// What it pins:
//   * one transcript → one POST /api/v1/reports/author, then a row with its
//     status, the server's read-back ground truth and the diff tally;
//   * a typed 403 refusal shows the refusal sentence and leaves nothing
//     saveable;
//   * `valid: false` lists every issue code and leaves nothing saveable;
//   * accept saves once through the EXISTING definition route, with the draft's
//     spec and the typed name, then stops offering Accept;
//   * reject clears the row and issues no request;
//   * the transcript box and Generate are the only way in — no audio control.
//
// Non-purpose: the endpoint contract (core-api flow tests) and the diff
// algorithm (core-api unit tests). This file proves the page wires them.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import ReportAuthorPage from "../../app/reports/author/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

const EMPTY_SEARCH = new URLSearchParams();

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/reports/author",
  useSearchParams: () => EMPTY_SEARCH,
}));

const AUTHOR_PATH = "/api/v1/reports/author";
const SAVE_PATH = "/api/v1/report-definitions";
const SAVED_ID = "00000000-0000-4000-8000-00000000e001";

const SPEC = {
  view: "tickets" as const,
  filters: [],
  group_by: ["priority"],
  measures: [{ agg: "count" as const }],
  viz: "bar" as const,
};

const CARD = {
  size: { span: "half" as const, height: "regular" as const },
  window_days: 30,
  refresh: { every_minutes: 60, reach_days: 0 },
};

const READS =
  "A chart over tickets grouped by priority, showing a count of rows. It spans the " +
  "last 30 days, refreshes every hour, and sits half wide and regular tall.";

/** A validated answer: valid, one draft, a three-entry diff (+2 ~1). */
const READY = {
  valid: true,
  saved: false,
  draft: {
    name: "P1 tickets by priority",
    spec: SPEC,
    card: CARD,
    shape: "chart",
    reads: READS,
  },
  issues: [],
  diff: [
    { op: "add", path: "view", after: "tickets" },
    { op: "add", path: "group_by/priority", after: "priority" },
    { op: "change", path: "card/window_days", before: 30, after: 7 },
  ],
  tool_calls: [{ name: "get_view_columns", argument: "tickets", outcome: "ok" }],
  usage: { prompt_tokens: 180, completion_tokens: 96, steps: 2 },
  model_info: { provider: "openai", model: "marula-test-chat", resolution: "org_allowlist" },
  catalogue: { views: 5, shapes: 3, rules: 3, prompt_chars: 5393 },
};

/** The registry refused the answer: two typed issues, no draft. */
const NEEDS_FIXES = {
  ...READY,
  valid: false,
  draft: null,
  diff: [],
  issues: [
    {
      code: "unknown_column",
      path: "nonsense",
      message: "unknown column 'nonsense' on view 'ops.v1_tickets'",
    },
    {
      code: "window_wider_than_refresh",
      message: "the card shows 30 days but one refresh reads 7",
    },
  ],
};

function setupApi(overrides: { author?: () => { status?: number; json: unknown } } = {}) {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/settings/user", () => ({ json: [] }));
  mock.on("POST", AUTHOR_PATH, overrides.author ?? (() => ({ json: READY })));
  mock.on("POST", SAVE_PATH, () => ({
    json: {
      id: SAVED_ID,
      name: "Q1 breaches",
      spec: SPEC,
      scope: "self",
      created_by: ME_FIXTURE.user_id,
      created_at: "2026-11-05T09:00:00Z",
      updated_at: "2026-11-05T09:00:00Z",
      mutation: { version: 1, seq: 1, applied_at: "2026-11-05T09:00:00Z" },
    },
  }));
  mock.install();
  return mock;
}

function renderPage() {
  render(
    <AuthProvider>
      <ReportAuthorPage />
    </AuthProvider>,
  );
}

/** Types a transcript and clicks the one button that starts a draft. */
async function describeIt(transcript: string) {
  const box = await screen.findByLabelText("What should this report answer?");
  fireEvent.change(box, { target: { value: transcript } });
  fireEvent.click(screen.getByRole("button", { name: "Generate a draft" }));
}

beforeEach(() => {
  window.localStorage.clear();
  window.history.replaceState({}, "", "/reports/author");
  seedAuthTokens();
});

describe("/reports/author integration", () => {
  it("drafts once from one transcript and shows the status, read-back and diff tally", async () => {
    const mock = setupApi();
    renderPage();
    await describeIt("Tickets by priority as a bar chart, last thirty days.");

    const row = await screen.findByTestId("report-draft-row");
    await waitFor(() => expect(row).toHaveAttribute("data-status", "ready"));

    // One transcript, one authoring call, nothing else on the wire.
    expect(mock.callsTo("POST", AUTHOR_PATH)).toHaveLength(1);
    expect(mock.lastBody<{ transcript: string }>("POST", AUTHOR_PATH).transcript).toBe(
      "Tickets by priority as a bar chart, last thirty days.",
    );

    // The row says what state it is in, in words beside the colour.
    expect(within(row).getByText("Ready to accept")).toBeInTheDocument();
    // The read-back is the server's sentence, rendered unchanged.
    expect(within(row).getByText(READS)).toBeInTheDocument();
    // The diff tally counts what changed: two additions, one change.
    expect(within(row).getByText("+2 ~1")).toBeInTheDocument();

    // The diff itself is the structural list, grouped by the part it touches.
    const diff = screen.getByRole("region", { name: "Changes against the current draft" });
    expect(within(diff).getByText("Data")).toBeInTheDocument();
    expect(within(diff).getByText("Grouped by")).toBeInTheDocument();
    expect(within(diff).getByText("Card")).toBeInTheDocument();
    expect(within(diff).getByText("window_days")).toBeInTheDocument();

    // Nothing was saved by generating.
    expect(mock.callsTo("POST", SAVE_PATH)).toHaveLength(0);
  });

  it("shows a typed AI refusal and leaves nothing saveable", async () => {
    const mock = setupApi({
      author: () => ({
        status: 403,
        json: {
          error: {
            reason: "ai_features_disabled",
            message: "AI features are switched off for this organization",
          },
        },
      }),
    });
    renderPage();
    await describeIt("anything at all");

    const row = await screen.findByTestId("report-draft-row");
    await waitFor(() => expect(row).toHaveAttribute("data-status", "refused"));

    // The sentence the server gave, not a generic failure.
    expect(screen.getByRole("alert")).toHaveTextContent(/switched off/i);
    expect(within(row).getByText("Not available")).toBeInTheDocument();
    // No draft means no read-back and no diff to review.
    expect(screen.queryByText(READS)).not.toBeInTheDocument();
    expect(
      screen.queryByRole("region", { name: "Changes against the current draft" }),
    ).not.toBeInTheDocument();

    // The Accept control is offered in no usable form: clicking it writes nothing.
    const accept = screen.getByRole("button", { name: "Accept and save" });
    expect(accept).toBeDisabled();
    fireEvent.click(accept);
    expect(mock.callsTo("POST", SAVE_PATH)).toHaveLength(0);
    expect(mock.callsTo("POST", AUTHOR_PATH)).toHaveLength(1);
  });

  it("lists every issue code and offers no save on a valid:false answer", async () => {
    const mock = setupApi({ author: () => ({ json: NEEDS_FIXES }) });
    renderPage();
    await describeIt("something the registry refuses");

    const row = await screen.findByTestId("report-draft-row");
    await waitFor(() => expect(row).toHaveAttribute("data-status", "needs_fixes"));

    expect(within(row).getByText("Needs fixes")).toBeInTheDocument();
    expect(within(row).getByText("unknown_column")).toBeInTheDocument();
    expect(within(row).getByText("window_wider_than_refresh")).toBeInTheDocument();
    expect(within(row).getByText("What the registry refused")).toBeInTheDocument();
    // The path of a pointed issue is shown beside its code.
    expect(within(row).getByText("nonsense")).toBeInTheDocument();
    // No draft, so nothing to read back or diff.
    expect(screen.queryByText(READS)).not.toBeInTheDocument();

    const accept = screen.getByRole("button", { name: "Accept and save" });
    expect(accept).toBeDisabled();
    fireEvent.click(accept);
    expect(mock.callsTo("POST", SAVE_PATH)).toHaveLength(0);
  });

  it("saves the draft once through the existing definition route, then stops offering Accept", async () => {
    const mock = setupApi();
    renderPage();
    await describeIt("Tickets by priority as a bar chart, last thirty days.");

    const row = await screen.findByTestId("report-draft-row");
    await waitFor(() => expect(row).toHaveAttribute("data-status", "ready"));

    // The page pre-fills the draft's name; the reviewer can type their own.
    const nameBox = screen.getByLabelText("Report name");
    fireEvent.change(nameBox, { target: { value: "Q1 breaches" } });

    const accept = screen.getByRole("button", { name: "Accept and save" });
    // Two clicks in one gesture: the second must not become a second save.
    fireEvent.click(accept);
    fireEvent.click(accept);

    await waitFor(() => expect(mock.callsTo("POST", SAVE_PATH)).toHaveLength(1));
    const body = mock.lastBody<{
      name: string;
      spec: unknown;
      scope: string;
      idempotency_key: string;
    }>("POST", SAVE_PATH);
    expect(body.name).toBe("Q1 breaches");
    expect(body.spec).toEqual(SPEC);
    // A new report is created private, exactly as the builder does it.
    expect(body.scope).toBe("self");
    expect(typeof body.idempotency_key).toBe("string");

    // The row becomes Saved and the decision cluster is gone, so there is no
    // second click to make.
    await waitFor(() =>
      expect(screen.getByTestId("report-draft-row")).toHaveAttribute("data-status", "accepted"),
    );
    expect(within(screen.getByTestId("report-draft-row")).getByText("Saved")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Accept and save" })).not.toBeInTheDocument();
    expect(screen.getByText(/Saved as a new private report/)).toBeInTheDocument();
    expect(mock.callsTo("POST", SAVE_PATH)).toHaveLength(1);
    // Generating is not repeated either.
    expect(mock.callsTo("POST", AUTHOR_PATH)).toHaveLength(1);
  });

  it("clears the draft on reject and issues no request", async () => {
    const mock = setupApi();
    renderPage();
    await describeIt("Tickets by priority as a bar chart.");

    const row = await screen.findByTestId("report-draft-row");
    await waitFor(() => expect(row).toHaveAttribute("data-status", "ready"));
    expect(mock.callsTo("POST", AUTHOR_PATH)).toHaveLength(1);

    fireEvent.click(screen.getByRole("button", { name: "Reject" }));

    // The draft is client state, so rejecting it is a render, not a request.
    await waitFor(() => expect(screen.queryByTestId("report-draft-row")).not.toBeInTheDocument());
    expect(screen.getByText(/The draft is gone/)).toBeInTheDocument();
    expect(screen.queryByText(READS)).not.toBeInTheDocument();
    expect(mock.callsTo("POST", SAVE_PATH)).toHaveLength(0);
    expect(mock.callsTo("POST", AUTHOR_PATH)).toHaveLength(1);
  });

  it("takes a transcript and nothing else: no audio control, no implicit request", async () => {
    const mock = setupApi();
    const { container } = render(
      <AuthProvider>
        <ReportAuthorPage />
      </AuthProvider>,
    );
    const box = await screen.findByLabelText("What should this report answer?");

    // The only entry point is the transcript box plus its one button.
    expect(container.querySelectorAll("textarea")).toHaveLength(1);
    expect(container.querySelectorAll("audio")).toHaveLength(0);
    expect(container.querySelectorAll("input[type='file']")).toHaveLength(0);
    expect(container.querySelectorAll("input[type='audio']")).toHaveLength(0);
    expect(screen.queryByLabelText(/microphone|record|voice note/i)).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /microphone|record|dictate/i })).not.toBeInTheDocument();

    // An empty transcript cannot start a draft, and nothing has been asked for.
    expect(screen.getByRole("button", { name: "Generate a draft" })).toBeDisabled();
    expect(mock.callsTo("POST", AUTHOR_PATH)).toHaveLength(0);

    // The page states the constraint rather than leaving it to the absence of a
    // control.
    expect(box).toHaveAttribute("aria-describedby");
    expect(screen.getByText(/Text only: a voice note is captured elsewhere/)).toBeInTheDocument();
    // No draft row before the first answer.
    expect(screen.queryByTestId("report-draft-row")).not.toBeInTheDocument();
  });
});
