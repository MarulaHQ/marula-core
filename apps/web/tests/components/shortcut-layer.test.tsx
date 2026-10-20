// Wave 47 lane A25 (`p47-silent-command-noops`) — the dispatcher's trace.
//
// The regression this file pins: Alt+N on /clients used to click a selector
// that matched nothing and return `false` to nobody. Pressing the chord now
// leaves `[data-testid="shortcut-last-outcome"]` with the status, the command
// id and the selector, plus an error line and an entry in
// `window.__marulaCommandFailures` when the target was required and missing.
import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ShortcutLayer } from "@/components/shortcuts/shortcut-layer";

const pushed: string[] = [];
let currentPathname = "/clients";
const push = (href: string) => {
  pushed.push(href);
};

vi.mock("next/navigation", () => ({
  usePathname: () => currentPathname,
  useRouter: () => ({ push }),
}));

function pressAlt(key: string, code: string): void {
  fireEvent.keyDown(window, { key, code, altKey: true });
}

beforeEach(() => {
  pushed.length = 0;
  currentPathname = "/clients";
  delete window.__marulaCommandFailures;
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("a chord whose control exists", () => {
  it("clicks it and records the ran outcome", () => {
    const onClick = vi.fn();
    render(
      <>
        <button data-testid="new-record" onClick={onClick}>
          New client
        </button>
        {/* authed=false keeps the profile read out of this suite. */}
        <ShortcutLayer authed={false} />
      </>,
    );

    pressAlt("n", "KeyN");

    expect(onClick).toHaveBeenCalledTimes(1);
    const marker = screen.getByTestId("shortcut-last-outcome");
    expect(marker).toHaveAttribute("data-command", "new.clients");
    expect(marker).toHaveAttribute("data-status", "ran");
  });
});

describe("a chord whose control is missing", () => {
  it("names the command and the selector instead of doing nothing", () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    render(<ShortcutLayer authed={false} />);

    pressAlt("n", "KeyN");

    const marker = screen.getByTestId("shortcut-last-outcome");
    expect(marker).toHaveAttribute("data-status", "failed");
    expect(marker).toHaveAttribute("data-command", "new.clients");
    expect(marker).toHaveAttribute("data-selector", '[data-testid="new-record"]');
    expect(marker.textContent).toContain("/clients");

    // The failure is loud on two more channels: the console line and the
    // bounded memory an E2E reads after pressing the chord.
    expect(error).toHaveBeenCalledTimes(1);
    expect(String(error.mock.calls[0]?.[0] ?? "")).toContain('[data-testid="new-record"]');
    expect(window.__marulaCommandFailures?.[0]?.command).toBe("new.clients");
  });
});

describe("a chord with nothing to act on", () => {
  it("reports inapplicable, not failed, and stays out of the failure memory", () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    const debug = vi.spyOn(console, "debug").mockImplementation(() => {});
    render(<ShortcutLayer authed={false} />);

    pressAlt("x", "KeyX");

    const marker = screen.getByTestId("shortcut-last-outcome");
    expect(marker).toHaveAttribute("data-status", "inapplicable");
    expect(marker).toHaveAttribute("data-command", "close.topmost");
    expect(error).not.toHaveBeenCalled();
    expect(debug).toHaveBeenCalledTimes(1);
    expect(window.__marulaCommandFailures ?? []).toHaveLength(0);
  });
});
