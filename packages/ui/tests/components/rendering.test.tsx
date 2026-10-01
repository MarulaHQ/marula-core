/**
 * Rendering and theming tests.
 * Purpose: static structure of presentational components, tag tone mapping,
 * table semantics, and theme attribute/persistence behavior.
 */
import { act, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Card, CardContent, CardHeader } from "../../src/components/card";
import { EmptyState } from "../../src/components/empty-state";
import { FieldError, FieldHint, Label } from "../../src/components/field";
import { Input } from "../../src/components/input";
import { Kbd } from "../../src/components/kbd";
import { Separator } from "../../src/components/separator";
import { Spinner } from "../../src/components/spinner";
import { Table, Td, Th, Tr } from "../../src/components/table";
import { StatusTag, Tag } from "../../src/components/tag";
import { applyTheme, resolveInitialTheme, useTheme } from "../../src/theme";

describe("Field", () => {
  it("associates label and error with a control via ids", () => {
    render(
      <>
        <Label htmlFor="summary">Summary</Label>
        <Input id="summary" aria-invalid="true" aria-describedby="summary-error summary-hint" />
        <FieldError id="summary-error">Required</FieldError>
        <FieldHint id="summary-hint">One line max</FieldHint>
      </>,
    );
    expect(screen.getByLabelText("Summary")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("Required");
  });
});

describe("Tag", () => {
  it("renders the kit Pill and maps each tone to its kit pair", () => {
    // Wave41 lane 20: Tag wraps the kit Pill now, so it carries the pill slot
    // and the kit's tone classes rather than the retired .m-tag-* classes.
    const expected = {
      success: "bg-mc-green-soft",
      warning: "bg-mc-progress",
      danger: "bg-mc-st-breach",
      neutral: "bg-muted",
      outline: "bg-muted",
    } as const;
    for (const tone of Object.keys(expected) as Array<keyof typeof expected>) {
      const { container } = render(<Tag tone={tone}>x</Tag>);
      const el = container.firstElementChild!;
      expect(el).toHaveAttribute("data-slot", "pill");
      expect(el).toHaveClass(expected[tone]);
    }
  });

  it("StatusTag renders its label", () => {
    render(<StatusTag tone="danger">Breach</StatusTag>);
    expect(screen.getByText("Breach")).toBeInTheDocument();
  });
});

describe("Table", () => {
  it("renders header scope, rows, and selection state", () => {
    render(
      <Table>
        <thead>
          <tr>
            <Th>Ticket</Th>
          </tr>
        </thead>
        <tbody>
          <Tr selected>
            <Td>MAR-42</Td>
          </Tr>
        </tbody>
      </Table>,
    );
    expect(screen.getByRole("columnheader", { name: "Ticket" })).toBeInTheDocument();
    expect(screen.getByRole("cell", { name: "MAR-42" })).toBeInTheDocument();
    expect(screen.getByText("MAR-42").closest("tr")).toHaveAttribute(
      "data-selected",
      "true",
    );
  });
});

describe("misc primitives", () => {
  it("Spinner announces status", () => {
    render(<Spinner />);
    expect(screen.getByRole("status")).toHaveAttribute("aria-label", "Loading");
  });

  it("Separator defaults to horizontal orientation", () => {
    const { container } = render(<Separator />);
    expect(container.firstChild).toHaveClass("m-separator-horizontal");
  });

  it("Card composes header/content sections", () => {
    render(
      <Card>
        <CardHeader>Dispatch</CardHeader>
        <CardContent>12 unassigned</CardContent>
      </Card>,
    );
    expect(screen.getByText("Dispatch")).toBeInTheDocument();
    expect(screen.getByText("12 unassigned")).toBeInTheDocument();
  });

  it("EmptyState renders message and action", () => {
    render(
      <EmptyState
        message="No tickets match this view."
        action={<button type="button">Clear filters</button>}
      />,
    );
    expect(screen.getByText("No tickets match this view.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Clear filters" })).toBeInTheDocument();
  });

  it("Kbd renders the kit's key cap", () => {
    // Wave41 lane 20: Kbd wraps the kit, which draws the cap as a span (the
    // mockups' own markup), in Figtree rather than the old squared mono chip.
    render(<Kbd>⌘K</Kbd>);
    expect(screen.getByText("⌘K")).toHaveAttribute("data-slot", "kbd");
  });
});

describe("theme switching", () => {
  it("applyTheme sets data-theme and persists", () => {
    applyTheme("dark");
    expect(document.documentElement).toHaveAttribute("data-theme", "dark");
    expect(localStorage.getItem("marula-theme")).toBe("dark");
    applyTheme("light");
  });

  it("resolveInitialTheme prefers stored value over system preference", () => {
    const originalMatchMedia = window.matchMedia;
    window.matchMedia = vi.fn().mockReturnValue({
      matches: true,
      addEventListener: () => {},
      removeEventListener: () => {},
    }) as unknown as typeof window.matchMedia;
    try {
      localStorage.setItem("marula-theme", "light");
      expect(resolveInitialTheme()).toBe("light");
      localStorage.removeItem("marula-theme");
      expect(resolveInitialTheme()).toBe("dark");
    } finally {
      window.matchMedia = originalMatchMedia;
      localStorage.removeItem("marula-theme");
    }
  });

  it("useTheme tracks external attribute changes", async () => {
    function Probe(): React.JSX.Element {
      const { theme } = useTheme();
      return <output>{theme}</output>;
    }
    render(<Probe />);
    expect(screen.getByRole("status")).toHaveTextContent("light");
    await act(async () => {
      document.documentElement.setAttribute("data-theme", "dark");
    });
    expect(screen.getByRole("status")).toHaveTextContent("dark");
    document.documentElement.removeAttribute("data-theme");
  });
});
