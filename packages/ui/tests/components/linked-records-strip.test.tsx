/**
 * LinkedRecordsStrip — §6.5. Each row stays its own configuration with its own
 * chips, the list collapses at `max`, and a caller can suppress the strip for a
 * ticket whose client has nothing to link.
 */
import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  LinkedRecordsStrip,
  type LinkedRecord,
} from "../../src/components/linked-records-strip";

function record(index: number, overrides: Partial<LinkedRecord> = {}): LinkedRecord {
  return {
    id: `rec-${String(index)}`,
    name: `Dell Latitude ${String(index)}`,
    subtitle: `NIN-${String(1000 + index)}`,
    capabilityGroups: [
      {
        kind: "rmm",
        providers: [
          {
            connectionId: `conn-${String(index)}`,
            label: "NinjaOne",
            connector: "ninjaone",
            status: "connected",
            isPrimary: true,
            actions: [
              { kind: "open", label: "Open in NinjaOne", url: `https://ninja.example/${String(index)}` },
            ],
          },
        ],
      },
    ],
    ...overrides,
  };
}

describe("LinkedRecordsStrip", () => {
  it("renders one row per configuration and never merges their chips", () => {
    render(
      <LinkedRecordsStrip
        records={[
          record(1, {
            capabilityGroups: [
              {
                kind: "rmm",
                providers: [
                  {
                    connectionId: "c1",
                    label: "NinjaOne",
                    connector: "ninjaone",
                    status: "connected",
                    actions: [{ kind: "open", label: "Open", url: "https://ninja.example/1" }],
                  },
                ],
              },
              {
                kind: "remote_access",
                providers: [
                  {
                    connectionId: "c2",
                    label: "ScreenConnect",
                    connector: "screenconnect",
                    status: "connected",
                    actions: [{ kind: "launch", label: "Start session" }],
                  },
                ],
              },
            ],
          }),
          record(2),
        ]}
        onAdd={vi.fn()}
      />,
    );

    expect(screen.getByTestId("linked-record-rec-1")).toBeInTheDocument();
    expect(screen.getByTestId("linked-record-rec-2")).toBeInTheDocument();
    expect(
      within(screen.getByTestId("linked-record-rec-1")).getAllByTestId("capability-chip"),
    ).toHaveLength(2);
    expect(
      within(screen.getByTestId("linked-record-rec-2")).getAllByTestId("capability-chip"),
    ).toHaveLength(1);
  });

  it("collapses past max and expands on Show all", async () => {
    const user = userEvent.setup();
    render(
      <LinkedRecordsStrip
        records={[1, 2, 3, 4, 5, 6].map((index) => record(index))}
        max={4}
        onAdd={vi.fn()}
      />,
    );
    expect(screen.getAllByTestId(/linked-record-rec-/)).toHaveLength(4);
    await user.click(screen.getByRole("button", { name: "Show all (6)" }));
    expect(screen.getAllByTestId(/linked-record-rec-/)).toHaveLength(6);
    await user.click(screen.getByRole("button", { name: "Show fewer" }));
    expect(screen.getAllByTestId(/linked-record-rec-/)).toHaveLength(4);
  });

  it("renders nothing at all when the caller hides it", () => {
    const { container } = render(
      <LinkedRecordsStrip records={[]} hidden onAdd={vi.fn()} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("shows one empty state with the add action", async () => {
    const user = userEvent.setup();
    const onAdd = vi.fn();
    render(<LinkedRecordsStrip records={[]} onAdd={onAdd} />);

    expect(screen.getByTestId("linked-records-strip")).toBeInTheDocument();
    const addButtons = screen.getAllByRole("button", { name: "Link configuration" });
    expect(addButtons).toHaveLength(1);
    await user.click(addButtons[0] as HTMLElement);
    expect(onAdd).toHaveBeenCalledTimes(1);
  });

  it("opens a configuration through onLinkClick", async () => {
    const user = userEvent.setup();
    const onLinkClick = vi.fn();
    render(<LinkedRecordsStrip records={[record(1)]} onLinkClick={onLinkClick} onAdd={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: /Dell Latitude 1/ }));
    expect(onLinkClick).toHaveBeenCalledWith("rec-1");
  });
});
