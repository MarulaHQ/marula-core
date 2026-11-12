// Unit test: NavCounter — the one count chip in the shell chrome.
//
// The rules it must never break: an unknown count renders nothing (a fake zero
// is a lie), zero renders nothing (an empty inbox is not an event), and a large
// count is capped so it cannot widen the bar.
import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { NavCounter } from "../../components/shell/nav-counter";

describe("NavCounter", () => {
  it("renders nothing when the count is unknown or zero", () => {
    for (const count of [null, undefined, 0, Number.NaN]) {
      const { container, unmount } = render(<NavCounter count={count} />);
      expect(container.textContent).toBe("");
      unmount();
    }
  });

  it("draws the count", () => {
    const { container } = render(<NavCounter count={7} />);
    expect(container.textContent).toBe("7");
  });

  it("caps a large count at the default 99+", () => {
    const { container } = render(<NavCounter count={128} />);
    expect(container.textContent).toBe("99+");
  });

  it("honours a caller's own cap", () => {
    const { container } = render(<NavCounter count={12} cap={9} />);
    expect(container.textContent).toBe("9+");
  });

  it("is decoration: the tappable parent owns the accessible name", () => {
    const { container } = render(<NavCounter count={3} />);
    expect(container.firstElementChild?.getAttribute("aria-hidden")).toBe("true");
  });
});
