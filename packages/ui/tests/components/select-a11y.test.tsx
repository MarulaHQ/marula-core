// The Select popup's scroll container is a kit-level contract, not a page
// patch (Wave 47 lane A22).
//
// Radix renders the popup as `listbox > viewport (scrolls) > items`, and the
// viewport's `role` and tab stop decide which of two axe rules fires:
//   - no tab stop           → `scrollable-region-focusable` (serious)
//   - tab stop, generic div → `aria-required-children` (critical)
// `role="group"` + `tabIndex={0}` satisfies both: a listbox may own a group,
// so axe keeps flattening it to the options, and the element is in the tab
// order. These assertions are the cheap half of that guard; the browser half
// is e2e/flows/select-a11y.spec.ts, which runs axe against the rendered popup
// in both themes (jsdom has no layout, so the scroll rule cannot fire here).
import { useState, type ReactNode } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "../../src/components/select";

// Long enough that a real popup scrolls: the rule only matches when the list
// overflows its own box.
const OPTIONS = Array.from({ length: 40 }, (_, i) => `node-${i + 1}`);

function Harness(): ReactNode {
  const [value, setValue] = useState(OPTIONS[0]);
  return (
    <Select value={value} onValueChange={setValue}>
      <SelectTrigger aria-label="Node">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {OPTIONS.map((option) => (
          <SelectItem key={option} value={option}>
            {option}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

describe("Select popup accessibility contract", () => {
  it("gives the scroll viewport a role a listbox may own, and a tab stop", async () => {
    render(<Harness />);
    await userEvent.click(screen.getByRole("combobox", { name: "Node" }));

    const listbox = await screen.findByRole("listbox");
    const viewport = listbox.querySelector<HTMLElement>("[data-radix-select-viewport]");
    expect(viewport).not.toBeNull();

    // The tab stop clears `scrollable-region-focusable`...
    expect(viewport).toHaveAttribute("tabindex", "0");
    // ...and the group role keeps the listbox owning options, not a generic
    // div, which is what cleared the critical `aria-required-children` before.
    expect(viewport).toHaveAttribute("role", "group");
  });

  it("keeps every option reachable inside the listbox", async () => {
    render(<Harness />);
    await userEvent.click(screen.getByRole("combobox", { name: "Node" }));

    const listbox = await screen.findByRole("listbox");
    expect(screen.getAllByRole("option")).toHaveLength(OPTIONS.length);
    expect(listbox).toContainElement(screen.getAllByRole("option")[0] as HTMLElement);
  });
});
