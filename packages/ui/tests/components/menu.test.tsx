/**
 * Menu — the anchored action list. These tests pin the promises the component
 * makes to every surface that uses it: the trigger carries the ARIA, the
 * keyboard walks the rows, Escape returns focus, and an outside press or an
 * ancestor scroll closes rather than leaving a detached menu open.
 */
import { describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Menu } from "../../src/components/menu";

function setup(onSelect = vi.fn(), onSecond = vi.fn()) {
  render(
    <Menu
      label="Message actions"
      trigger={<button type="button">More</button>}
      items={[
        { label: "Share to channel", onSelect },
        { label: "Copy link", onSelect: onSecond },
        { label: "Delete", onSelect: vi.fn(), disabled: true, tone: "danger" },
      ]}
    />,
  );
  return { trigger: screen.getByRole("button", { name: "More" }), onSelect, onSecond };
}

describe("Menu", () => {
  it("renders the trigger with the menu ARIA and no panel before opening", () => {
    const { trigger } = setup();
    expect(trigger).toHaveAttribute("aria-haspopup", "menu");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("opens on a press and lists the items", async () => {
    const user = userEvent.setup();
    const { trigger } = setup();
    await user.click(trigger);
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    const menu = screen.getByRole("menu", { name: "Message actions" });
    expect(within(menu).getAllByRole("menuitem")).toHaveLength(3);
    expect(menu).toHaveFocus();
  });

  it("walks enabled rows with the arrow keys and runs one with Enter", async () => {
    const user = userEvent.setup();
    const { trigger, onSecond } = setup();
    await user.click(trigger);
    const menu = screen.getByRole("menu", { name: "Message actions" });
    const items = within(menu).getAllByRole("menuitem");

    // Opens on the first row.
    expect(menu).toHaveAttribute("aria-activedescendant", items[0]?.id);

    await user.keyboard("{ArrowDown}");
    expect(menu).toHaveAttribute("aria-activedescendant", items[1]?.id);

    // The disabled third row is skipped, so Down wraps back to the first.
    await user.keyboard("{ArrowDown}");
    expect(menu).toHaveAttribute("aria-activedescendant", items[0]?.id);

    await user.keyboard("{ArrowUp}");
    expect(menu).toHaveAttribute("aria-activedescendant", items[1]?.id);

    await user.keyboard("{Enter}");
    expect(onSecond).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("Escape closes and returns focus to the trigger", async () => {
    const user = userEvent.setup();
    const { trigger } = setup();
    await user.click(trigger);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
  });

  it("Tab closes the menu without trapping focus", async () => {
    const user = userEvent.setup();
    const { trigger } = setup();
    await user.click(trigger);
    await user.keyboard("{Tab}");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("closes on an outside pointerdown", async () => {
    const user = userEvent.setup();
    const { trigger } = setup();
    await user.click(trigger);
    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("closes when an ancestor scrolls", async () => {
    const user = userEvent.setup();
    const { trigger } = setup();
    await user.click(trigger);
    fireEvent.scroll(document);
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("a row with an href is a link that opens away from the product", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    render(
      <Menu
        label="Providers"
        trigger={<button type="button">Providers</button>}
        items={[
          {
            label: "NinjaOne",
            href: "https://ninja.example/device/1",
            onSelect,
          },
        ]}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Providers" }));
    const link = screen.getByRole("menuitem", { name: "NinjaOne" });
    expect(link).toHaveAttribute("href", "https://ninja.example/device/1");
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noreferrer");
  });

  it("can be controlled from a second affordance", () => {
    function Harness() {
      const [open, setOpen] = useState(true);
      return (
        <Menu
          label="Controlled"
          open={open}
          onOpenChange={setOpen}
          trigger={<button type="button">Open</button>}
          items={[{ label: "Only row", onSelect: () => {} }]}
        />
      );
    }
    render(<Harness />);
    expect(screen.getByRole("menu", { name: "Controlled" })).toBeInTheDocument();
  });
});
