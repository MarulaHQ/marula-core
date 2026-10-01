/**
 * Interaction tests for interactive primitives.
 * Purpose: verify Radix-backed behavior — toggle, selection, dialog focus
 * management, tooltip hover — with Workbench class wiring intact.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState, type ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import { Button } from "../../src/components/button";
import { Checkbox } from "../../src/components/checkbox";
import { Dialog, DialogContent, DialogTrigger } from "../../src/components/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "../../src/components/select";
import { Tooltip, TooltipProvider } from "../../src/components/tooltip";

function PortalHarness({ children }: { children: ReactNode }): ReactNode {
  return <TooltipProvider>{children}</TooltipProvider>;
}

describe("Button", () => {
  it("fires onClick when enabled and not when disabled", async () => {
    const onClick = vi.fn();
    const { rerender } = render(<Button onClick={onClick}>Assign</Button>);
    await userEvent.click(screen.getByRole("button", { name: "Assign" }));
    expect(onClick).toHaveBeenCalledTimes(1);

    rerender(<Button disabled onClick={onClick}>Assign</Button>);
    await userEvent.click(screen.getByRole("button", { name: "Assign" }));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('defaults to type="button" unless overridden', () => {
    render(<Button>Save</Button>);
    expect(screen.getByRole("button")).toHaveAttribute("type", "button");
  });

  it("renders asChild as the child element", () => {
    render(
      <Button asChild variant="primary">
        <a href="/tickets">Open tickets</a>
      </Button>,
    );
    const link = screen.getByRole("link", { name: "Open tickets" });
    expect(link).toHaveClass("m-btn-primary");
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});

describe("Checkbox", () => {
  it("toggles checked state on click", async () => {
    render(<Checkbox aria-label="Billable" />);
    const box = screen.getByRole("checkbox", { name: "Billable" });
    expect(box).toHaveAttribute("data-state", "unchecked");
    await userEvent.click(box);
    expect(box).toHaveAttribute("data-state", "checked");
    expect(box).toHaveAttribute("aria-checked", "true");
  });

  it("does not toggle when disabled", async () => {
    render(<Checkbox disabled aria-label="Locked" />);
    const box = screen.getByRole("checkbox", { name: "Locked" });
    await userEvent.click(box);
    expect(box).toHaveAttribute("data-state", "unchecked");
  });

  it("supports indeterminate state", () => {
    render(<Checkbox checked="indeterminate" aria-label="Select all" />);
    expect(screen.getByRole("checkbox", { name: "Select all" })).toHaveAttribute(
      "aria-checked",
      "mixed",
    );
  });
});

describe("Select", () => {
  function Harness({ onChange }: { onChange?: (v: string) => void }): ReactNode {
    const [value, setValue] = useState("");
    return (
      <Select
        value={value}
        onValueChange={(v) => {
          setValue(v);
          onChange?.(v);
        }}
      >
        <SelectTrigger aria-label="Priority">
          <SelectValue placeholder="Pick priority" />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="p1">P1</SelectItem>
          <SelectItem value="p2">P2</SelectItem>
        </SelectContent>
      </Select>
    );
  }

  it("selects an option and reflects the value", async () => {
    const onChange = vi.fn();
    render(<Harness onChange={onChange} />);
    await userEvent.click(screen.getByRole("combobox", { name: "Priority" }));
    const option = await screen.findByRole("option", { name: /P1/ });
    await userEvent.click(option);
    expect(onChange).toHaveBeenCalledWith("p1");
    expect(screen.getByRole("combobox", { name: "Priority" })).toHaveTextContent("P1");
  });
});

describe("Dialog", () => {
  it("opens, labels itself, and closes on Escape", async () => {
    render(
      <PortalHarness>
        <Dialog>
          <DialogTrigger asChild>
            <Button>Delete ticket</Button>
          </DialogTrigger>
          <DialogContent title="Delete ticket" description="This cannot be undone.">
            <p>Body</p>
          </DialogContent>
        </Dialog>
      </PortalHarness>,
    );
    await userEvent.click(screen.getByRole("button", { name: "Delete ticket" }));
    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByRole("heading", { name: "Delete ticket" }),
    ).toBeInTheDocument();
    // Wave41 lane 20: the description id is Radix's now (the kit's
    // DialogDescription), not the retired m-dialog-description.
    expect(dialog.getAttribute("aria-describedby")).toBeTruthy();
    expect(within(dialog).getByText("This cannot be undone.")).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(dialog).not.toBeInTheDocument());
  });
});

describe("Tooltip", () => {
  it("shows content on hover and hides on leave", async () => {
    render(
      <PortalHarness>
        <Tooltip trigger={<Button variant="ghost">ⓘ</Button>}>
          Timer keeps running while paused reason is empty
        </Tooltip>
      </PortalHarness>,
    );
    const trigger = screen.getByRole("button", { name: "ⓘ" });
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
    await userEvent.hover(trigger);
    expect(await screen.findByRole("tooltip")).toHaveTextContent(/Timer keeps running/);
    await userEvent.unhover(trigger);
    await waitFor(() => expect(screen.queryByRole("tooltip")).not.toBeInTheDocument());
  });
});
