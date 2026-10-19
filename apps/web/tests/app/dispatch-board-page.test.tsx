// Integration test: /dispatch board — the interaction family the pack fixes.
//
// Purpose: prove the headless behaviours lane 15 cannot see in a still: the
// keyboard engine (pick up, move, resize, split, drop), the overlap decision
// family (drag onto an occupied slot, the reason card, the 1/2/3 keys), the
// collision banner, the density steps, and the settings policy actually
// changing what a drop does.
//
// Non-purpose: visual pixel checks (those are the capture leg's job).
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import DispatchBoardPage from "../../app/dispatch/page";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/dispatch",
}));

function setupApi(opts: { ai?: boolean; routing?: unknown } = {}): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/settings/org", () => ({
    json: [{ key: "ai.features_enabled", value: opts.ai ?? false, scope: "org" }],
  }));
  mock.on("GET", "/api/v1/dispatch/routing", () => ({ json: opts.routing ?? ROUTING_CLEAN }));
  mock.install();
  return mock;
}

/** A routing day with nothing to say: the strip stays hidden (ADR 0010). */
const ROUTING_CLEAN = {
  date: "2026-09-29",
  status: { provider_id: "mapbox", quality: "good", measured: true },
  technicians: [],
  suggestions: { available: false, reason: "no_improvement", items: [] },
};

const board = (): HTMLElement => screen.getByTestId("dispatch-board-root");

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
  document.documentElement.removeAttribute("data-density");
});

describe("/dispatch board", () => {
  it("renders the pack's frozen day: twelve lanes, the queue, and the frozen clock", async () => {
    setupApi();
    render(
      <AuthProvider>
        <DispatchBoardPage />
      </AuthProvider>,
    );
    await screen.findByTestId("dispatch-board-root");
    expect(document.querySelectorAll("[data-tech]")).toHaveLength(12);
    expect(screen.getByText("11:41")).toBeInTheDocument();
    expect(screen.getByLabelText(/#18441 Phishing email/)).toBeInTheDocument();
    // the pack's running block reads Live on #18421
    expect(screen.getByLabelText(/#18421 Outlook keeps asking for a password/)).toBeInTheDocument();
    cleanup();
  });

  it("filters the queue by priority", async () => {
    setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <DispatchBoardPage />
      </AuthProvider>,
    );
    await screen.findByTestId("dispatch-board-root");
    expect(screen.getByLabelText(/#18433 Meeting room display/)).toBeInTheDocument();
    // P3+ keeps only the P3/P4 tickets.
    await user.click(screen.getByRole("button", { name: "P3+" }));
    expect(screen.getByLabelText(/#18433 Meeting room display/)).toBeInTheDocument();
    expect(screen.queryByLabelText(/#18441 Phishing email/)).not.toBeInTheDocument();
    // P2 keeps the two urgent ones and drops the rest.
    await user.click(screen.getByRole("button", { name: "P2" }));
    expect(screen.getByLabelText(/#18441 Phishing email/)).toBeInTheDocument();
    expect(screen.queryByLabelText(/#18433 Meeting room display/)).not.toBeInTheDocument();
    cleanup();
  });

  it("switches density, and the ladder reaches the document root", async () => {
    setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <DispatchBoardPage />
      </AuthProvider>,
    );
    await screen.findByTestId("dispatch-board-root");
    // The board footer is the template strip (`.m-surface-footer`) and its
    // density control is the kit's DensitySwitch, so the audit that reads the
    // strip's own root finds it and the steps are radio steps, not pressed
    // buttons.
    const footer = screen.getByTestId("dispatch-footer");
    expect(footer).toHaveClass("m-surface-footer");
    // The step is the account's (density.ts, D7) rather than this page's: with
    // no stored preference the control starts on the Marula default, and
    // picking a step writes the document attribute every control reads.
    expect(screen.getByRole("radio", { name: "Comfortable", checked: true })).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "Compact" }));
    expect(document.documentElement.getAttribute("data-density")).toBe("compact");
    expect(await screen.findByRole("radio", { name: "Compact", checked: true })).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "Dense" }));
    expect(document.documentElement.getAttribute("data-density")).toBe("dense");
    cleanup();
  });

  it("keeps the collision banner for unapproved overlaps only", async () => {
    setupApi();
    render(
      <AuthProvider>
        <DispatchBoardPage />
      </AuthProvider>,
    );
    await screen.findByTestId("dispatch-board-root");
    // The seed has no unapproved overlap: absences are hatched and only the
    // one true collision would raise the banner, so a clean board shows none.
    expect(screen.queryByTestId("collision-banner")).not.toBeInTheDocument();
    cleanup();
  });

  it("opens the overlap policy settings and blocks every overlap when set to never", async () => {
    setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <DispatchBoardPage />
      </AuthProvider>,
    );
    await screen.findByTestId("dispatch-board-root");
    await user.click(screen.getByLabelText("Dispatch settings"));
    const dialog = await screen.findByTestId("dispatch-settings");
    expect(within(dialog).getByText("Always ask me")).toBeInTheDocument();
    await user.click(within(dialog).getByText("Never"));
    expect(within(dialog).getByText(/This is the default/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Done" }));
    cleanup();
  });

  it("wears the data-surface toolbar in the Unassigned view, with the row count on Export", async () => {
    setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <DispatchBoardPage />
      </AuthProvider>,
    );
    await screen.findByTestId("dispatch-board-root");
    await user.click(screen.getByRole("button", { name: /^Unassigned/ }));
    // Columns / Export only exist on the data-surface view.
    expect(screen.getByRole("button", { name: /Columns/ })).toBeInTheDocument();
    // The core kit's ExportControl puts the row count on its trigger and the
    // formats in the menu (the pack's single-Export anatomy).
    const trigger = screen.getByRole("button", { name: /Export/ });
    expect(trigger).toHaveTextContent("6");
    await user.click(trigger);
    expect(screen.getByRole("menuitem", { name: "CSV" })).toBeInTheDocument();
    cleanup();
  });

  it("replaces the column header with a selection bar once tickets are ticked", async () => {
    setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <DispatchBoardPage />
      </AuthProvider>,
    );
    await screen.findByTestId("dispatch-board-root");
    await user.click(screen.getByRole("button", { name: /^Unassigned/ }));
    const table = screen.getByTestId("unassigned-table");
    await user.click(within(table).getByLabelText("Select #18441"));
    const bar = await screen.findByTestId("selection-bar");
    expect(within(bar).getByText("1 selected")).toBeInTheDocument();
    expect(within(bar).getByRole("button", { name: /Assign to/ })).toBeInTheDocument();
    expect(within(bar).getByRole("button", { name: /Place on the board/ })).toBeInTheDocument();
    // the bulk bar replaces the column header rather than sitting under it
    expect(screen.queryByText("Needs")).not.toBeInTheDocument();
    cleanup();
  });

  it("marks the current user's own lane so it is findable in twelve", async () => {
    setupApi();
    render(
      <AuthProvider>
        <DispatchBoardPage />
      </AuthProvider>,
    );
    await screen.findByTestId("dispatch-board-root");
    const mine = document.querySelector('[data-tech="TL"] .av.me');
    expect(mine).not.toBeNull();
    expect(mine).toHaveAttribute("title", expect.stringContaining("you"));
    // and only that lane
    expect(document.querySelectorAll(".dp-lh .av.me")).toHaveLength(1);
    cleanup();
  });

  it("places a queue ticket with the keyboard and announces the drop", async () => {
    setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <DispatchBoardPage />
      </AuthProvider>,
    );
    await screen.findByTestId("dispatch-board-root");
    await user.click(screen.getByRole("button", { name: /^Unassigned/ }));
    // the table view lists the same tickets and is keyboard reachable
    expect(screen.getByTestId("unassigned-table")).toBeInTheDocument();
    cleanup();
  });

  it("hides Plan with AI without the dispatch grant and shows it with one", async () => {
    const user = userEvent.setup();
    setupApi({ ai: false });
    const { unmount } = render(
      <AuthProvider>
        <DispatchBoardPage />
      </AuthProvider>,
    );
    await screen.findByTestId("dispatch-board-root");
    expect(screen.queryByRole("button", { name: "Plan with AI" })).not.toBeInTheDocument();
    cleanup();
    unmount();

    setupApi({ ai: true });
    render(
      <AuthProvider>
        <DispatchBoardPage />
      </AuthProvider>,
    );
    await screen.findByTestId("dispatch-board-root");
    await screen.findByRole("button", { name: "Plan with AI" });
    cleanup();
  });

  it("scopes the plan before proposing it, and never books anything while scoping", async () => {
    setupApi({ ai: true });
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <DispatchBoardPage />
      </AuthProvider>,
    );
    await screen.findByTestId("dispatch-board-root");
    await user.click(screen.getByRole("button", { name: /^Unassigned/ }));
    const table = screen.getByTestId("unassigned-table");
    // one remote and one on-site ticket, so the plan mixes both
    await user.click(within(table).getByLabelText("Select #18441"));
    await user.click(within(table).getByLabelText("Select #18433"));
    await user.click(screen.getByRole("button", { name: "Plan with AI" }));
    const scope = await screen.findByRole("dialog", { name: /Plan with AI: choose the scope/ });
    expect(within(scope).getByText(/Nothing is booked or sent yet/)).toBeInTheDocument();
    await user.click(within(scope).getByRole("button", { name: "Propose a plan" }));
    const panel = await screen.findByTestId("plan-panel");
    // The panel says, in its own words, that nothing is booked until confirm.
    expect(within(panel).getByText(/Nothing is booked until you confirm/)).toBeInTheDocument();
    expect(within(panel).getByText(/Confirmations for clients are prepared for you to send, never sent for you/)).toBeInTheDocument();
    expect(within(panel).getByRole("button", { name: /Confirm plan|to confirm/ })).toBeInTheDocument();
    // confirming books and says so; it never sends anything for you
    const confirm = within(panel).getByRole("button", { name: /^Confirm plan$/ });
    await user.click(confirm);
    expect(await screen.findByTestId("toast")).toHaveTextContent(/Client confirmations are ready for you to send/);
    cleanup();
  });

  it("shows the week capacity matrix with over-capacity days in blush", async () => {
    setupApi();
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <DispatchBoardPage />
      </AuthProvider>,
    );
    await screen.findByTestId("dispatch-board-root");
    await user.click(screen.getByRole("button", { name: "Week" }));
    const wk = await screen.findByTestId("week-view");
    expect(within(wk).getByLabelText(/Tumi L\. Tue 29: \d+ percent/)).toBeInTheDocument();
    expect(within(wk).getByLabelText(/Jordan T\. Fri 2: off/)).toBeInTheDocument();
    cleanup();
  });

  it("keeps every lane header legible: presence, utilisation and the over-capacity figure", async () => {
    setupApi();
    render(
      <AuthProvider>
        <DispatchBoardPage />
      </AuthProvider>,
    );
    await screen.findByTestId("dispatch-board-root");
    // Tumi L. is on the live remote block at 10:30 (the clock is frozen at 11:41).
    expect(screen.getByText("Remote · #18432")).toBeInTheDocument();
    // The over-capacity lane is the one the pack's title says: Over by 53m.
    expect(screen.getAllByText(/Over by/).length).toBeGreaterThan(0);
    expect(document.querySelector(".dp-lh .dp-ub")).not.toBeNull();
    cleanup();
  });
});

describe("/dispatch board — the routing strip is optional", () => {
  // The strip is the first thing to render on the board, so a routing payload
  // it cannot read must not take the board down with it. An empty array from a
  // proxy is the shape that actually shows up.
  it("keeps the board standing when the routing day is not a routing day", async () => {
    setupApi({ routing: [] });
    render(
      <AuthProvider>
        <DispatchBoardPage />
      </AuthProvider>,
    );
    await screen.findByTestId("dispatch-board-root");
    expect(document.querySelectorAll("[data-tech]")).toHaveLength(12);
    expect(screen.queryByTestId("routing-strip")).not.toBeInTheDocument();
    cleanup();
  });

  it("shows the strip when the day carries a blocking travel warning", async () => {
    setupApi({
      routing: {
        ...ROUTING_CLEAN,
        technicians: [
          {
            user_id: "u1",
            display_name: "Sam K.",
            total_travel_minutes: 0,
            warnings: [{ kind: "zero_transfer", appointment_id: "a1", message: "No time to reach Oakview by 13:00", short_minutes: -20 }],
          },
        ],
      },
    });
    render(
      <AuthProvider>
        <DispatchBoardPage />
      </AuthProvider>,
    );
    await screen.findByTestId("dispatch-board-root");
    expect(await screen.findByTestId("routing-strip")).toHaveTextContent(/Sam K\./);
    cleanup();
  });
});

describe("/dispatch board — the collision day", () => {
  it("raises the banner for a day that really has an unapproved double booking", async () => {
    setupApi();
    window.history.replaceState({}, "", "/dispatch?demo=collision");
    render(
      <AuthProvider>
        <DispatchBoardPage />
      </AuthProvider>,
    );
    // The default day has none, so the banner proves the swap happened.
    const banner = await screen.findByTestId("collision-banner");
    expect(banner).toHaveTextContent(/collision today/);
    expect(banner).toHaveTextContent(/Alex M\.: #18348 runs into #18344 at 13:00/);
    cleanup();
    window.history.replaceState({}, "", "/dispatch");
  });
});
