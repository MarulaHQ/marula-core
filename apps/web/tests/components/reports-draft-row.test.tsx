// DraftRow's data-check block (Wave 64 lane 5): what the server's verification
// found, what a repair cleared, and what survives.
//
// Purpose: the component is the reviewer's only evidence that the draft was run
// through the product's data path, so each state it can be handed is pinned:
//   * a clean check states the classes and the rounds, and Accept stays on;
//   * a surviving finding is named with its class and its card, Accept is off,
//     and the same sentence is not printed twice when the server also lists it
//     among `issues`;
//   * `ran: false` says nothing was checked instead of implying success.
//
// Non-purpose: the server's verification (core-api flow tests) and the status
// derivation (lib/report-authoring).
import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import { DraftRow } from "../../components/reports/draft-row";
import type {
  DraftVerification,
  ReportDraft,
} from "../../lib/report-authoring";

const DRAFT: ReportDraft = {
  name: "Tickets by priority",
  spec: {
    view: "tickets",
    filters: [],
    group_by: ["priority"],
    measures: [{ agg: "count" }],
    viz: "bar",
  },
  card: {
    size: { span: "half", height: "regular" },
    window_days: 30,
    refresh: { every_minutes: 60, reach_days: 0 },
  },
  shape: "chart",
  reads:
    "A chart over tickets grouped by priority, showing a count of rows. It spans the " +
    "last 30 days, refreshes every hour, and sits half wide and regular tall.",
};

const CHECKED = [
  "the spec and card pass the same boundary the save and run routes use",
  "a chart hint has a dimension to put on its axis",
  "the window is no wider than one refresh reaches",
  "a chart has rows, a table has headers, a stat has a number",
  "no card's filters resolve to zero rows",
];

function props(overrides: Partial<Parameters<typeof DraftRow>[0]> = {}) {
  return {
    status: "ready" as const,
    draft: DRAFT,
    issues: [],
    diff: [],
    name: "Tickets by priority",
    onNameChange: vi.fn(),
    onAccept: vi.fn(),
    onEdit: vi.fn(),
    onReject: vi.fn(),
    ...overrides,
  };
}

const CLEAN: DraftVerification = {
  ran: true,
  checked: CHECKED,
  attempts: [{ round: 0, findings: [] }],
  repaired: [],
  findings: [],
};

describe("DraftRow verification block", () => {
  it("states what was checked and in how many rounds when nothing failed", () => {
    render(<DraftRow {...props({ verification: CLEAN })} />);
    expect(screen.getByTestId("draft-verification-summary")).toHaveTextContent(
      "Checked 5 classes in 1 round; nothing failed.",
    );
    // The classes are available, not shouted: the summary opens on demand.
    expect(screen.getByText("What was checked")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Accept and save" }),
    ).not.toBeDisabled();
  });

  it("names a surviving finding with its class and its card, and keeps Accept off", () => {
    const finding = {
      kind: "no_rows",
      card_index: 2,
      message: "Card 3's chart has no rows at this window.",
    };
    const verification: DraftVerification = {
      ran: true,
      checked: CHECKED,
      attempts: [{ round: 0, findings: [finding] }],
      repaired: [],
      findings: [finding],
    };
    render(
      <DraftRow
        {...props({
          status: "needs_fixes",
          verification,
          // The server lists a surviving finding in `issues` too. The row must
          // print it once, under the block that owns it.
          issues: [{ code: "no_rows", path: "card[3]", message: finding.message }],
        })}
      />,
    );
    expect(screen.getByText("What the data could not fill")).toBeInTheDocument();
    expect(screen.getByText(finding.message)).toBeInTheDocument();
    expect(screen.getAllByText(finding.message)).toHaveLength(1);
    // The class and the card are machine-readable on the row, not prose.
    const section = screen.getByText("What the data could not fill").parentElement!;
    expect(within(section).getByText("no_rows")).toBeInTheDocument();
    expect(within(section).getByText(/card 3/)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Accept and save" }),
    ).toBeDisabled();
  });

  it("records what a repair cleared, separately from what failed", () => {
    const repaired = {
      kind: "no_rows",
      message: "The chart has no rows at this window.",
    };
    const verification: DraftVerification = {
      ran: true,
      checked: CHECKED,
      attempts: [
        { round: 0, findings: [repaired] },
        { round: 1, findings: [] },
      ],
      repaired: [repaired],
      findings: [],
    };
    render(<DraftRow {...props({ verification })} />);
    expect(screen.getByTestId("draft-verification-summary")).toHaveTextContent(
      "Checked 5 classes in 2 rounds; nothing failed.",
    );
    expect(
      screen.getByText("Repaired before this draft was offered"),
    ).toBeInTheDocument();
    expect(screen.getByText(repaired.message)).toBeInTheDocument();
    expect(screen.queryByText("What the data could not fill")).toBeNull();
  });

  it("keeps the builder reachable when the data check is what failed", () => {
    const finding = {
      kind: "no_rows",
      message: "The chart has no rows at this window.",
    };
    const verification: DraftVerification = {
      ran: true,
      checked: CHECKED,
      attempts: [{ round: 0, findings: [finding] }],
      repaired: [],
      findings: [finding],
    };
    render(
      <DraftRow
        {...props({
          status: "needs_fixes",
          verification,
          issues: [{ code: "no_rows", message: finding.message }],
        })}
      />,
    );
    // The spec is valid; the data is not. A reviewer fixes that in the builder.
    expect(
      screen.getByRole("button", { name: "Edit in the builder" }),
    ).toBeEnabled();
    expect(screen.getByRole("button", { name: "Accept and save" })).toBeDisabled();
    // The data finding is NOT reported as a registry refusal.
    expect(screen.queryByText("What the registry refused")).toBeNull();
  });

  it("keeps the builder shut when the registry refused the spec", () => {
    render(
      <DraftRow
        {...props({
          status: "needs_fixes",
          issues: [
            {
              code: "unknown_column",
              path: "nonsense",
              message: "unknown column 'nonsense' on view 'ops.v1_tickets'",
            },
          ],
        })}
      />,
    );
    expect(
      screen.getByRole("button", { name: "Edit in the builder" }),
    ).toBeDisabled();
    expect(screen.getByText("What the registry refused")).toBeInTheDocument();
  });

  it("says nothing was checked rather than implying a clean check", () => {
    const verification: DraftVerification = {
      ran: false,
      checked: [],
      attempts: [],
      repaired: [],
      findings: [],
    };
    render(<DraftRow {...props({ verification })} />);
    expect(screen.getByTestId("draft-verification-summary")).toHaveTextContent(
      "Not checked: no data check ran for this draft.",
    );
    expect(screen.queryByText(/nothing failed/)).toBeNull();
    expect(screen.queryByText("What was checked")).toBeNull();
  });

  it("renders no verification block at all when the answer carries none", () => {
    render(<DraftRow {...props()} />);
    expect(screen.queryByText("What the check found")).toBeNull();
    expect(screen.queryByTestId("draft-verification-summary")).toBeNull();
  });
});
