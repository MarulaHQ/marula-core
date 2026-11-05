/**
 * Skeleton — the pack's flat loading blocks (S4).
 * Purpose: the shape, the announcement, and the sheet's quiet geometry.
 * Non-purpose: pixel rendering (jsdom does no layout); the CSS assertions read
 * the stylesheet's declarations, which is what the drawing fixes.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Skeleton, SkeletonGroup } from "../../src/components/skeleton";

const css = readFileSync(join(process.cwd(), "src/components.css"), "utf8");
const sheet = css.slice(
  css.indexOf("Skeleton (surface pack S4)"),
  css.indexOf(".m-spinner"),
);

describe("Skeleton", () => {
  it("defaults to the text line and stays out of the accessibility tree", () => {
    const { container } = render(<Skeleton />);
    const el = container.firstElementChild!;
    expect(el).toHaveClass("m-skel", "m-skel-line");
    expect(el).toHaveAttribute("aria-hidden", "true");
  });

  it("renders the block and circle shapes", () => {
    const { container } = render(
      <>
        <Skeleton shape="block" />
        <Skeleton shape="circle" />
      </>,
    );
    expect(container.querySelector(".m-skel-block")).toBeTruthy();
    expect(container.querySelector(".m-skel-circle")).toBeTruthy();
  });

  it("takes an explicit width and height for a chart or a column", () => {
    const { container } = render(<Skeleton shape="block" width="60%" height={22} />);
    const el = container.firstElementChild as HTMLElement;
    expect(el.style.width).toBe("60%");
    expect(el.style.height).toBe("22px");
  });
});

describe("SkeletonGroup", () => {
  it("announces one pending region and shows the default sentence", () => {
    render(
      <SkeletonGroup label="Loading the alert feed">
        <Skeleton />
      </SkeletonGroup>,
    );
    const region = screen.getByRole("status");
    expect(region).toHaveAttribute("aria-busy", "true");
    expect(region).toHaveAttribute("aria-label", "Loading the alert feed");
    expect(region).toHaveTextContent("Loading the alert feed…");
  });

  it("accepts the caller's sentence", () => {
    render(
      <SkeletonGroup label="Loading runs" sentence="Loading workflow runs…">
        <Skeleton />
      </SkeletonGroup>,
    );
    expect(screen.getByText("Loading workflow runs…")).toBeInTheDocument();
  });
});

describe("the sheet's skeleton geometry", () => {
  it("draws a 22px block, a 12px line and a 10px stack", () => {
    expect(sheet).toContain("height: 22px");
    expect(sheet).toContain("border-radius: 8px");
    expect(sheet).toContain("height: 12px");
    expect(sheet).toContain("gap: 10px");
  });

  it("is still: the family declares no animation or shimmer", () => {
    expect(sheet).not.toMatch(/animation\s*:/);
    expect(sheet).not.toMatch(/@keyframes/);
  });
});
