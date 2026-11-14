/**
 * CapabilityChip — §6.2/§6.3. The tests pin the four decisions the component
 * exists to make: the body runs the primary, alternatives live in the caret
 * menu, no primary means "Choose a system" instead of a guess, and a failing
 * provider offers the fix rather than a disabled row.
 */
import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  CapabilityChip,
  capabilityLabel,
  type CapabilityProvider,
} from "../../src/components/capability-chip";

function provider(overrides: Partial<CapabilityProvider> = {}): CapabilityProvider {
  return {
    connectionId: "conn-1",
    label: "NinjaOne",
    connector: "ninjaone",
    status: "connected",
    actions: [{ kind: "open", label: "Open in NinjaOne", url: "https://ninja.example/device/1" }],
    ...overrides,
  };
}

describe("capabilityLabel", () => {
  it("names the known kinds and humanises an open vocabulary", () => {
    expect(capabilityLabel("rmm")).toBe("RMM");
    expect(capabilityLabel("remote_access")).toBe("Remote");
    expect(capabilityLabel("inventory_control")).toBe("Inventory control");
  });
});

describe("CapabilityChip", () => {
  it("renders nothing when the capability has no provider", () => {
    const { container } = render(<CapabilityChip kind="rmm" providers={[]} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("a single open provider renders an anchor with no caret", () => {
    render(<CapabilityChip kind="rmm" providers={[provider()]} />);
    const body = screen.getByRole("link", { name: "RMM, NinjaOne" });
    expect(body).toHaveAttribute("href", "https://ninja.example/device/1");
    expect(body).toHaveAttribute("target", "_blank");
    expect(body).toHaveAttribute("rel", "noreferrer");
    expect(screen.queryByTestId("capability-chip-caret")).not.toBeInTheDocument();
  });

  it("a single launch provider is a button that calls onLaunch", async () => {
    const user = userEvent.setup();
    const onLaunch = vi.fn();
    render(
      <CapabilityChip
        kind="remote_access"
        providers={[
          provider({
            label: "ScreenConnect",
            connector: "screenconnect",
            actions: [{ kind: "launch", label: "Start session" }],
          }),
        ]}
        onLaunch={onLaunch}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Remote, ScreenConnect" }));
    expect(onLaunch).toHaveBeenCalledWith("conn-1", "launch");
  });

  it("with several providers the body runs the primary and the caret lists all of them", async () => {
    const user = userEvent.setup();
    const onLaunch = vi.fn();
    render(
      <CapabilityChip
        kind="rmm"
        providers={[
          provider({ connectionId: "conn-1", label: "NinjaOne", isPrimary: true }),
          provider({
            connectionId: "conn-2",
            label: "Datto RMM",
            connector: "datto",
            status: "degraded",
            actions: [{ kind: "launch", label: "Start session" }],
          }),
        ]}
        onLaunch={onLaunch}
      />,
    );

    // Body: the primary (an open link here).
    expect(screen.getByRole("link", { name: "RMM, NinjaOne" })).toHaveAttribute(
      "href",
      "https://ninja.example/device/1",
    );

    const caret = screen.getByRole("button", { name: "RMM providers" });
    await user.click(caret);
    const menu = screen.getByRole("menu", { name: "RMM providers" });
    const rows = within(menu).getAllByRole("menuitem");
    expect(rows).toHaveLength(2);
    expect(rows[0]?.textContent).toContain("NinjaOne");
    expect(rows[0]?.textContent).toContain("Connected");
    // Degraded is still actionable, and says so.
    expect(rows[1]?.textContent).toContain("Datto RMM");
    expect(rows[1]?.textContent).toContain("Degraded");
  });

  it("with no primary the body says 'Choose a system' and opens the menu", async () => {
    const user = userEvent.setup();
    render(
      <CapabilityChip
        kind="rmm"
        providers={[
          provider({ connectionId: "conn-1", label: "NinjaOne" }),
          provider({ connectionId: "conn-2", label: "Datto RMM", connector: "datto" }),
        ]}
      />,
    );

    const body = screen.getByRole("button", { name: "RMM, choose a system" });
    expect(body).toHaveTextContent("Choose a system");
    await user.click(body);
    const menu = screen.getByRole("menu", { name: "RMM providers" });
    expect(within(menu).getAllByRole("menuitem")).toHaveLength(2);
  });

  it("a provider that is not connected offers Fix, not a disabled row", async () => {
    const user = userEvent.setup();
    const onFix = vi.fn();
    render(
      <CapabilityChip
        kind="rmm"
        providers={[
          provider({ connectionId: "conn-1", label: "NinjaOne", isPrimary: true }),
          provider({
            connectionId: "conn-2",
            label: "Datto RMM",
            connector: "datto",
            status: "auth_failed",
          }),
        ]}
        onFix={onFix}
      />,
    );

    await user.click(screen.getByRole("button", { name: "RMM providers" }));
    const fixRow = screen.getByRole("menuitem", { name: /Fix · Datto RMM/ });
    expect(fixRow).not.toHaveAttribute("aria-disabled");
    await user.click(fixRow);
    expect(onFix).toHaveBeenCalledWith("conn-2");
  });

  it("a single failing provider is a Fix body", async () => {
    const user = userEvent.setup();
    const onFix = vi.fn();
    render(
      <CapabilityChip
        kind="rmm"
        providers={[provider({ status: "auth_failed" })]}
        onFix={onFix}
      />,
    );
    const body = screen.getByRole("button", { name: "RMM, NinjaOne, needs attention" });
    expect(body).toHaveTextContent("Fix · NinjaOne");
    await user.click(body);
    expect(onFix).toHaveBeenCalledWith("conn-1");
  });

  it("a launch with no handler does not render a dead button", () => {
    render(
      <CapabilityChip
        kind="remote_access"
        providers={[
          provider({
            label: "ScreenConnect",
            actions: [{ kind: "launch", label: "Start session" }],
          }),
        ]}
      />,
    );
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    expect(screen.getByTestId("capability-chip-body")).toHaveTextContent(
      "Remote: ScreenConnect",
    );
  });
});
