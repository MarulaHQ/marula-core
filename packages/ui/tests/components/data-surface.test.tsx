/**
 * The data-surface composers: SurfaceFooter, BulkActionBar / BulkHeaderRow and
 * the FilterChips / FilterTrigger split.
 *
 * These carry the template's laws, so the tests pin the laws rather than the
 * markup: the footer states a count and only offers a page it can serve, the
 * bulk bar replaces the column header rather than sitting above it, and the chip
 * row collapses instead of wrapping.
 */
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { BulkActionBar, BulkHeaderRow } from "../../src/components/bulk-action-bar";
import { DataToolbar } from "../../src/components/data-toolbar";
import { FilterChips, type FilterClause, type FilterField } from "../../src/components/filter-bar";
import { SurfaceFooter } from "../../src/components/surface-footer";

const FIELDS: FilterField[] = [
  { key: "client", label: "Client" },
  { key: "status", label: "Status" },
];

function clause(id: string, field: string, value: string): FilterClause {
  return { id, field, op: "eq", value };
}

describe("SurfaceFooter", () => {
  it("states shown of total with both numbers as figures", () => {
    render(<SurfaceFooter shown={60} total={40213} noun="ticket" />);
    const footer = screen.getByRole("group", { name: "List footer" });
    // The count reads as one sentence with the two numbers emphasised, so it is
    // matched on the count element rather than on a bare text node.
    expect(
      within(footer).getByText((_c, el) => el?.classList.contains("m-surface-footer-count") === true),
    ).toHaveTextContent("60 of 40,213 tickets");
    expect(within(footer).getByText("60").tagName).toBe("B");
    expect(within(footer).getByText("40,213").tagName).toBe("B");
  });

  it("does not invent a denominator when the total is unknown", () => {
    const { container } = render(<SurfaceFooter shown={50} noun="ticket" />);
    expect(container.querySelector(".m-surface-footer-count")?.textContent).toBe("50 tickets");
  });

  it("singularises the noun at one row", () => {
    const { container } = render(<SurfaceFooter shown={1} total={1} noun="client" />);
    expect(container.querySelector(".m-surface-footer-count")?.textContent).toBe("1 of 1 client");
  });

  it("offers load-more only when the caller can serve another page", async () => {
    const onLoadMore = vi.fn();
    const { rerender } = render(<SurfaceFooter shown={50} total={120} noun="ticket" />);
    expect(screen.queryByRole("button", { name: /Load 50 more/ })).not.toBeInTheDocument();

    rerender(
      <SurfaceFooter shown={50} total={120} noun="ticket" onLoadMore={onLoadMore} />,
    );
    await userEvent.click(screen.getByRole("button", { name: /Load 50 more/ }));
    expect(onLoadMore).toHaveBeenCalledTimes(1);
  });

  it("says it is working rather than lying about the count while a page loads", () => {
    render(
      <SurfaceFooter shown={50} noun="ticket" onLoadMore={() => undefined} loadingMore />,
    );
    expect(screen.getByRole("button", { name: "Loading…" })).toBeDisabled();
  });

  it("renders the J/K chips the pack prints, after the density slot", () => {
    const { container } = render(
      <SurfaceFooter
        shown={12}
        total={12}
        noun="ticket"
        density={<span data-testid="density">density</span>}
        hints={[
          { keys: "J K", label: "move" },
          { keys: "X", label: "select" },
        ]}
      />,
    );
    const footer = container.querySelector(".m-surface-footer");
    expect(footer).not.toBeNull();
    const order = Array.from(footer?.children ?? []).map((n) => n.textContent);
    expect(order[order.length - 2]).toContain("density");
    expect(order[order.length - 1]).toContain("move");
    expect(screen.getByText("J")).toBeInTheDocument();
    expect(screen.getByText("K")).toBeInTheDocument();
  });
});

describe("BulkActionBar", () => {
  it("reads the count, keeps Clear last, and does not invert the surface", () => {
    const onClear = vi.fn();
    const { container } = render(
      <BulkActionBar count={3} noun="ticket" onClear={onClear}>
        <button type="button">Assign</button>
      </BulkActionBar>,
    );
    const bar = container.querySelector(".m-bulk-bar");
    expect(bar).not.toBeNull();
    expect(screen.getByText("3 selected")).toBeInTheDocument();
    const buttons = within(bar as HTMLElement).getAllByRole("button");
    expect(buttons.map((b) => b.textContent)).toEqual(["Assign", "Clear selection"]);
    expect(bar?.className).toContain("m-bulk-bar");
  });

  it("renders Merge only when the surface has a merge path, disabled below two rows", () => {
    const { rerender } = render(<BulkActionBar count={1} onClear={() => undefined} />);
    expect(screen.queryByRole("button", { name: "Merge" })).not.toBeInTheDocument();

    rerender(<BulkActionBar count={1} onClear={() => undefined} onMerge={() => undefined} />);
    expect(screen.getByRole("button", { name: "Merge" })).toBeDisabled();

    rerender(<BulkActionBar count={2} onClear={() => undefined} onMerge={() => undefined} />);
    expect(screen.getByRole("button", { name: "Merge" })).toBeEnabled();
  });

  it("takes the column header's row, spanning every column", () => {
    render(
      <table>
        <thead>
          <BulkHeaderRow columnCount={5} count={2} onClear={() => undefined} />
        </thead>
      </table>,
    );
    const cell = screen.getByTestId("bulk-bar");
    expect(cell.tagName).toBe("TH");
    expect(cell.getAttribute("colspan")).toBe("5");
    // The bar wears the header row's class, which is where the tint and the 3px
    // gold edge live — an action bar above the table would not.
    expect(cell.className).toContain("m-bulk-head");
  });

  it("clears the selection from the bar's own button", async () => {
    const onClear = vi.fn();
    render(<BulkHeaderRow columnCount={3} count={2} onClear={onClear} />);
    await userEvent.click(screen.getByRole("button", { name: "Clear selection" }));
    expect(onClear).toHaveBeenCalledTimes(1);
  });
});

describe("FilterChips", () => {
  it("reads the value in bold and removes by id", async () => {
    const onChange = vi.fn();
    render(
      <FilterChips
        fields={FIELDS}
        clauses={[clause("a", "client", "Acme Corporation")]}
        onChange={onChange}
      />,
    );
    const value = screen.getByText("Acme Corporation");
    expect(value.tagName).toBe("B");
    await userEvent.click(screen.getByRole("button", { name: /Remove filter/ }));
    expect(onChange).toHaveBeenCalledWith([]);
  });

  it("collapses past the cap into +N more instead of wrapping", async () => {
    const clauses = [
      clause("a", "client", "One"),
      clause("b", "client", "Two"),
      clause("c", "client", "Three"),
      clause("d", "client", "Four"),
    ];
    render(
      <FilterChips fields={FIELDS} clauses={clauses} onChange={() => undefined} maxChips={3} />,
    );
    expect(screen.queryByText("Four")).not.toBeInTheDocument();
    const more = screen.getByRole("button", { name: "Show 1 more filters" });
    await userEvent.click(more);
    expect(screen.getByText("Four")).toBeInTheDocument();
  });

  it("ends the row with the dashed Add filter chip when one is offered", async () => {
    const onAdd = vi.fn();
    render(
      <FilterChips fields={FIELDS} clauses={[]} onChange={() => undefined} onAdd={onAdd} />,
    );
    await userEvent.click(screen.getByRole("button", { name: /Add filter/ }));
    expect(onAdd).toHaveBeenCalledTimes(1);
  });
});

describe("DataToolbar", () => {
  it("puts the view and the chips left, and Columns, Filter and Export right", () => {
    const { container } = render(
      <DataToolbar
        label="Ticket list controls"
        views={<span>views</span>}
        chips={<span>chips</span>}
        filter={<button type="button">Filter</button>}
        columns={<button type="button">Columns</button>}
        export={<button type="button">Export</button>}
      />,
    );
    const main = container.querySelector(".m-data-toolbar-main");
    expect(main?.querySelector(".m-data-toolbar-chips")?.textContent).toBe("chips");
    const right = main?.querySelector(".m-data-toolbar-right");
    expect(
      Array.from(right?.children ?? []).map((n) => n.textContent),
    ).toEqual(["Columns", "Filter", "Export"]);
    // The strip is one row: no wrap utility, and the CSS nowrap is asserted in
    // the stylesheet rather than here.
    expect(container.querySelector(".m-data-toolbar")?.className).not.toContain("flex-wrap");
  });

  it("still renders a whole self-contained filter control in the middle", () => {
    const { container } = render(
      <DataToolbar label="Legacy" filters={<span>bar</span>} columns={<button type="button">C</button>} />,
    );
    expect(container.querySelector(".m-data-toolbar-chips")?.textContent).toBe("bar");
  });
});
