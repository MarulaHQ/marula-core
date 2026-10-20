/**
 * Wave 41 lane 2b — the primitives this lane added.
 *
 * Purpose: pin the BEHAVIOUR the Marula spec names, not the styling. Each test
 * below is a sentence from DESIGN-NOTES §4 or §5 that would fail silently if
 * someone later simplified the component: the current status step is the gold
 * one, a dismissed recommendation can be brought back, a priority column does
 * not change width, an export says how many rows it holds, and a filter chip
 * can be taken off one at a time.
 *
 * Non-purpose: no visual assertions. Colours and radii are the token suite's
 * job (tests/tokens.test.ts); these tests only care what a person can DO.
 */
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  AssistantCard,
  AssistantCardDismissed,
  AssistantCardQuestion,
} from "../../src/components/assistant-card";
import { Badge } from "../../src/components/badge";
import { ColumnChooser } from "../../src/components/column-chooser";
import { ComposerFrame, composerActionLabel } from "../../src/components/composer-frame";
import { DensitySwitch } from "../../src/components/density-switch";
import { EmptyState, EmptyStateMark } from "../../src/components/empty-state";
import { ExportControl } from "../../src/components/export-control";
import { FilterBar, clauseLabel, type FilterClause } from "../../src/components/filter-bar";
import { StatusStepper } from "../../src/components/status-stepper";
import { Th } from "../../src/components/table";
import { ViewSwitch } from "../../src/components/view-switch";

const STEPS = [
  { id: "new", label: "New" },
  { id: "in_progress", label: "In progress" },
  { id: "waiting", label: "Waiting" },
  { id: "resolved", label: "Resolved" },
];

describe("StatusStepper", () => {
  it("marks only the current step, and earlier steps as done", () => {
    render(<StatusStepper steps={STEPS} currentId="waiting" />);
    expect(screen.getByRole("button", { name: /Waiting/ })).toHaveAttribute("aria-current", "step");
    expect(screen.getByRole("button", { name: /New/ })).not.toHaveAttribute("aria-current");
  });

  it("sets a status when any step is clicked, not only the next one", async () => {
    const onSelect = vi.fn();
    render(<StatusStepper steps={STEPS} currentId="new" onSelect={onSelect} />);
    // Straight from New to Resolved: the spec says clicking any step sets it,
    // because a technician who already called the client should not have to
    // walk the ticket through Waiting to get there.
    await userEvent.click(screen.getByRole("button", { name: /Resolved/ }));
    expect(onSelect).toHaveBeenCalledWith("resolved");
  });

  it("does not offer a step it cannot set", () => {
    render(<StatusStepper steps={STEPS} currentId="new" />);
    for (const step of STEPS) {
      expect(screen.getByRole("button", { name: new RegExp(step.label) })).toBeDisabled();
    }
  });

  it("renders no current step rather than marking the wrong row", () => {
    render(<StatusStepper steps={STEPS} currentId="a-status-from-the-future" />);
    expect(screen.queryByRole("button", { current: "step" })).toBeNull();
  });
});

describe("AssistantCard", () => {
  it("carries the eyebrow, the recommendation and its action", () => {
    render(
      <AssistantCard eyebrow="Recommended next step" heading="Restart the VPN adapter" action={<button>Run</button>}>
        <p>Evidence attached.</p>
      </AssistantCard>,
    );
    expect(screen.getByText("Recommended next step")).toBeInTheDocument();
    expect(screen.getByText("Restart the VPN adapter")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Run" })).toBeInTheDocument();
  });

  it("a dismissed recommendation keeps Undo reachable", async () => {
    const onUndo = vi.fn();
    render(<AssistantCardDismissed summary="Dismissed: restart the VPN adapter" onUndo={onUndo} />);
    expect(screen.getByText(/Dismissed: restart/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(onUndo).toHaveBeenCalledOnce();
  });

  it("an answered question keeps the question visible beside the answer", () => {
    const { rerender } = render(
      <AssistantCardQuestion question="Do you want anything sent to the client?" />,
    );
    expect(screen.getByText("Do you want anything sent to the client?")).toBeInTheDocument();
    rerender(
      <AssistantCardQuestion
        question="Do you want anything sent to the client?"
        answer="No — drafts wait for your approval."
      />,
    );
    // A question that vanishes on answer leaves the timeline with no trace of
    // what was asked.
    expect(screen.getByText("Do you want anything sent to the client?")).toBeInTheDocument();
    expect(screen.getByText(/drafts wait for your approval/)).toBeInTheDocument();
  });
});

describe("Badge", () => {
  it("renders the kit Pill for a tone and the kit PriorityChip for a priority", () => {
    // Wave41 lane 20: Badge is now a thin wrapper over the handoff kit, so it
    // carries the kit's slots and tones rather than the old .m-badge classes.
    const { rerender } = render(<Badge tone="success">Healthy</Badge>);
    expect(screen.getByText("Healthy")).toHaveAttribute("data-slot", "pill");
    expect(screen.getByText("Healthy")).toHaveClass("bg-mc-green-soft");
    rerender(<Badge priority="p1">P1</Badge>);
    expect(screen.getByText("P1")).toHaveAttribute("data-slot", "priority-chip");
    expect(screen.getByText("P1")).toHaveClass("bg-destructive");
  });
});

describe("DensitySwitch", () => {
  it("is a radio group, because a density step is a setting and not a panel", () => {
    render(<DensitySwitch value="comfortable" onChange={() => {}} />);
    expect(screen.getByRole("radiogroup", { name: "Density" })).toBeInTheDocument();
  });

  it("reports the step it was told to change to", async () => {
    const onChange = vi.fn();
    render(<DensitySwitch value="comfortable" onChange={onChange} />);
    await userEvent.click(screen.getByRole("radio", { name: /Dense/ }));
    expect(onChange).toHaveBeenCalledWith("dense");
  });

  it("states each step's own measurements", () => {
    render(<DensitySwitch value="comfortable" onChange={() => {}} />);
    expect(
      screen.getByText(/44 pixel controls, 48 pixel rows, 24 pixel card padding/),
    ).toBeInTheDocument();
  });
});

describe("ComposerFrame", () => {
  it("says who can see a reply, by name", () => {
    render(<ComposerFrame mode="reply" clientName="John Smith" />);
    expect(screen.getByText("Visible to John Smith")).toBeInTheDocument();
  });

  it("an internal note says so and offers no status move", () => {
    render(<ComposerFrame mode="internal" />);
    expect(screen.getByText("Only your team can see this")).toBeInTheDocument();
    expect(screen.queryByLabelText(/Set to Waiting/)).toBeNull();
  });

  it("a reply offers the status move", () => {
    render(<ComposerFrame mode="reply" clientName="John Smith" />);
    expect(screen.getByLabelText(/Set to Waiting after send/)).toBeInTheDocument();
  });

  it("names the action after the destination, not the verb", () => {
    expect(composerActionLabel("internal")).toBe("Add note");
    expect(composerActionLabel("reply")).toBe("Send reply");
  });
});

describe("FilterBar", () => {
  const FIELDS = [
    { key: "status", label: "Status" },
    { key: "assignee", label: "Assignee" },
  ];

  it("reads a compound filter back as separate removable chips", () => {
    const clauses: FilterClause[] = [
      { id: "1", field: "status", op: "eq", value: "open" },
      { id: "2", field: "assignee", op: "is_set" },
    ];
    render(<FilterBar fields={FIELDS} clauses={clauses} onChange={() => {}} />);
    // The chip reads as a sentence and the value carries the weight, so the
    // sentence is split across two nodes: "Status is" + a bold "open".
    const chip = screen.getByText("open");
    expect(chip.tagName).toBe("B");
    expect(chip.parentElement?.textContent).toBe("Status is open");
    expect(screen.getByText("Assignee is set")).toBeInTheDocument();
  });

  it("removes one clause at a time, which is the reason chips are the shape", async () => {
    const onChange = vi.fn();
    const clauses: FilterClause[] = [
      { id: "1", field: "status", op: "eq", value: "open" },
      { id: "2", field: "assignee", op: "eq", value: "me" },
    ];
    render(<FilterBar fields={FIELDS} clauses={clauses} onChange={onChange} />);
    await userEvent.click(screen.getByRole("button", { name: "Remove filter: Status is open" }));
    expect(onChange).toHaveBeenCalledWith([clauses[1]]);
  });

  it("renders a list filter readably", () => {
    expect(clauseLabel({ id: "1", field: "status", op: "in", value: "open, waiting" }, FIELDS)).toBe(
      "Status is any of open, waiting",
    );
  });

  it("says so when nothing is filtered", () => {
    render(<FilterBar fields={FIELDS} clauses={[]} onChange={() => {}} />);
    expect(screen.getByText("No filters")).toBeInTheDocument();
  });
});

describe("ExportControl", () => {
  it("states the row count on the trigger before the user asks for the file", async () => {
    render(<ExportControl columns={["ref", "title"]} rowCount={1284} onExport={() => {}} />);
    // A row count discovered after the fact is how a ledger goes to the wrong
    // person. The count sits on the control, and repeats inside the menu.
    const trigger = screen.getByRole("button", { name: /Export/ });
    expect(trigger).toHaveTextContent("1,284");
    await userEvent.click(trigger);
    expect(screen.getByRole("menu", { name: /Export to a file/ })).toHaveTextContent("1,284 rows");
  });

  it("states the filters, sort and column count, not only the rows", async () => {
    render(
      <ExportControl
        columns={["ref", "title", "status"]}
        rowCount={18}
        filters={[{ field: "status" }, { field: "client" }]}
        sort={{ field: "priority", dir: "desc" }}
        onExport={() => {}}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /Export/ }));
    const menu = screen.getByRole("menu", { name: /Export to a file/ });
    expect(menu).toHaveTextContent("18 rows");
    expect(menu).toHaveTextContent("2 filters");
    expect(menu).toHaveTextContent("sorted by Priority, descending");
    expect(menu).toHaveTextContent("3 columns");
    expect(menu).toHaveTextContent("exactly what is on screen");
  });

  it("exports the visible columns in the order the table is showing them", async () => {    const onExport = vi.fn();
    render(
      <ExportControl
        columns={["title", "ref"]}
        rowCount={2}
        onExport={onExport}
        formats={["csv"]}
        sort={{ field: "title", dir: "asc" }}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /Export/ }));
    await userEvent.click(screen.getByRole("menuitem", { name: "CSV" }));
    expect(onExport).toHaveBeenCalledWith(
      expect.objectContaining({ format: "csv", columns: ["title", "ref"], rowCount: 2 }),
    );
  });

  it("will not export nothing", async () => {
    const onExport = vi.fn();
    render(<ExportControl columns={[]} rowCount={0} onExport={onExport} formats={["csv"]} />);
    await userEvent.click(screen.getByRole("button", { name: /Export/ }));
    await userEvent.click(screen.getByRole("menuitem", { name: "CSV" }));
    expect(onExport).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });

  it("says a large export runs in the background rather than pretending", async () => {
    render(<ExportControl columns={["ref"]} rowCount={40000} onExport={() => {}} asyncThreshold={5000} />);
    await userEvent.click(screen.getByRole("button", { name: /Export/ }));
    expect(screen.getByText(/runs in the background/)).toBeInTheDocument();
  });
});

describe("ColumnChooser", () => {
  it("counts the visible columns on its trigger", () => {
    render(
      <ColumnChooser
        columns={[
          { key: "ref", label: "Number", visible: true },
          { key: "title", label: "Title", visible: true },
          { key: "sla", label: "SLA", visible: false },
        ]}
        onChange={() => {}}
      />,
    );
    expect(screen.getByRole("button", { name: /Columns/ })).toHaveTextContent("2");
  });

  it("will not turn off a required column", async () => {
    render(
      <ColumnChooser
        columns={[{ key: "ref", label: "Number", visible: true, required: true }]}
        onChange={() => {}}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /Columns/ }));
    expect(screen.getByRole("checkbox")).toBeDisabled();
  });

  it("states that the choice is saved on every device, with the count", async () => {
    // DESIGN-NOTES-tickets.md §5: the chooser "states that the choice is saved
    // for the user on every device". The pack prints it under the heading.
    render(
      <ColumnChooser
        columns={[
          { key: "ref", label: "Number", visible: true },
          { key: "sla", label: "SLA", visible: false },
        ]}
        onChange={() => {}}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /Columns/ }));
    expect(screen.getByText(/1 shown\. Saved for you, on every device\./)).toBeInTheDocument();
  });
});

describe("ViewSwitch", () => {
  it("renders nothing when a surface ships one view", () => {
    const { container } = render(
      <ViewSwitch views={[{ id: "all", label: "All" }]} activeId="all" onChange={() => {}} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("marks the active view as selected", async () => {
    const onChange = vi.fn();
    render(
      <ViewSwitch
        views={[
          { id: "open", label: "Open" },
          { id: "mine", label: "Mine" },
        ]}
        activeId="open"
        onChange={onChange}
      />,
    );
    // Wave41 lane 20: ViewSwitch is the kit Segmented now, a ToggleGroup of
    // radio items (aria-checked), not a tablist.
    const open = screen.getByRole("radio", { name: "Open" });
    expect(open).toHaveAttribute("aria-checked", "true");
    await userEvent.click(screen.getByRole("radio", { name: "Mine" }));
    expect(onChange).toHaveBeenCalledWith("mine");
  });
});

describe("Table headers", () => {
  it("reorders a column with Alt+Arrow, because a drag is not a keyboard operation", async () => {
    const onReorder = vi.fn();
    render(
      <table>
        <thead>
          <tr>
            <Th columnKey="ref" index={0} columnCount={3} width={100} onReorder={onReorder} onResize={() => {}}>
              Number
            </Th>
          </tr>
        </thead>
      </table>,
    );
    const header = screen.getByRole("columnheader");
    header.focus();
    await userEvent.keyboard("{Alt>}{ArrowRight}{/Alt}");
    expect(onReorder).toHaveBeenCalledWith(0, 1);
  });

  it("resizes with Arrow, and Shift+Arrow for a larger step", async () => {
    const onResize = vi.fn();
    render(
      <table>
        <thead>
          <tr>
            <Th columnKey="ref" index={0} columnCount={1} width={100} onResize={onResize}>
              Number
            </Th>
          </tr>
        </thead>
      </table>,
    );
    const header = screen.getByRole("columnheader");
    header.focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(onResize).toHaveBeenLastCalledWith("ref", 116);
    await userEvent.keyboard("{Shift>}{ArrowRight}{/Shift}");
    // Both steps are measured from the width the PROP carries, not from the
    // previous step: the primitive is controlled, so a parent that does not
    // re-render gets the same base twice. Accumulating locally would make the
    // column drift away from whatever the parent actually holds.
    expect(onResize).toHaveBeenLastCalledWith("ref", 164);
  });

  it("clamps a resize to something a person can read", async () => {
    const onResize = vi.fn();
    render(
      <table>
        <thead>
          <tr>
            <Th columnKey="ref" index={0} columnCount={1} width={100} onResize={onResize}>
              Number
            </Th>
          </tr>
        </thead>
      </table>,
    );
    screen.getByRole("columnheader").focus();
    await userEvent.keyboard("{Shift>}{ArrowLeft}{Shift>}{ArrowLeft}{/Shift}{/Shift}");
    for (const call of onResize.mock.calls) {
      expect(call[1]).toBeGreaterThanOrEqual(64);
    }
  });

  it("leaves a bare Arrow to the surface, because on a sortable table Arrow sorts", async () => {
    const onResize = vi.fn();
    const onKeyDown = vi.fn();
    render(
      <table>
        <thead>
          <tr>
            <Th columnKey="ref" index={0} columnCount={1} onResize={onResize} onKeyDown={onKeyDown}>
              Number
            </Th>
          </tr>
        </thead>
      </table>,
    );
    // No width means no resize behaviour, so the key is not consumed at all.
    screen.getByRole("columnheader").focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(onResize).not.toHaveBeenCalled();
  });
});

describe("EmptyState", () => {
  it("says what is missing and hides the mark from assistive tech", () => {
    render(<EmptyState message="No tickets in this view yet." />);
    expect(screen.getByText("No tickets in this view yet.")).toBeInTheDocument();
    expect(screen.queryByRole("img")).toBeNull();
  });

  it("draws the fruit, leaf and canopy in lines", () => {
    const { container } = render(<EmptyStateMark />);
    const svg = container.querySelector("svg");
    expect(svg).not.toBeNull();
    // Line work, not a picture: no fills anywhere in the mark.
    expect(svg?.getAttribute("fill")).toBe("none");
    expect(svg?.querySelectorAll("path").length).toBeGreaterThanOrEqual(4);
  });
});
