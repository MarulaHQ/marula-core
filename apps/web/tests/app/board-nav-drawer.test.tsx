// Wave 23 lane 1, item 1: BoardNav mobile drawer — below the md breakpoint
// the rail hides and a ☰ Boards chip opens a scrim + slide-in panel; the
// drawer closes on scrim tap and on link pick. The ≥md rail renders as
// before (same tree, same files).
import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import { cleanup, render, screen, fireEvent, within } from "@testing-library/react";
import { BoardNav } from "../../components/board-nav";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

const BOARDS = [
  {
    id: "b-1",
    group_id: null,
    name: "Delivery",
    description: null,
    is_default: false,
    favorite: true,
    my_role: "member" as const,
  },
  {
    id: "b-2",
    group_id: null,
    name: "Backlogish",
    description: null,
    is_default: false,
    favorite: false,
    my_role: null,
  },
];

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function setup() {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/boards/nav", () => ({ json: BOARDS }));
  mock.on("GET", "/api/v1/board-groups", () => ({ json: [] }));
  mock.on("POST", "/api/v1/boards/favorite", () => ({ json: {} }));
  mock.install();
  return mock;
}

describe("BoardNav mobile drawer (item 1)", () => {
  it("renders the chip, opens the drawer, closes via scrim", async () => {
    setup();
    render(<BoardNav />);
    const toggle = screen.getByTestId("board-nav-toggle");
    expect(toggle).toHaveClass("md:hidden");
    expect(screen.queryByTestId("board-nav-drawer")).toBeNull();
    fireEvent.click(toggle);
    const drawer = await screen.findByTestId("board-nav-drawer");
    expect(screen.getByTestId("board-nav-scrim")).toBeInTheDocument();
    // Starred section + Ungrouped boards section both list it.
    expect(within(drawer).getAllByText("Delivery").length).toBe(2);
    expect(within(drawer).getAllByText("Backlogish").length).toBe(1);
    fireEvent.click(screen.getByTestId("board-nav-scrim"));
    expect(screen.queryByTestId("board-nav-drawer")).toBeNull();
  });

  it("closes the drawer when the ✕ button is pressed", () => {
    setup();
    render(<BoardNav />);
    fireEvent.click(screen.getByTestId("board-nav-toggle"));
    fireEvent.click(screen.getByRole("button", { name: "Close boards drawer" }));
    expect(screen.queryByTestId("board-nav-drawer")).toBeNull();
  });

  it("keeps the ≥md rail in the DOM as the non-drawer surface", () => {
    setup();
    render(<BoardNav />);
    expect(screen.getByRole("navigation", { name: "Boards" })).toHaveClass("md:block");
  });
});
