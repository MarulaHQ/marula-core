// Command palette in Ask mode, and the panel handoff (K02 -> K03 -> K04 -> K05).
//
// The grant is granted here, so Ask exists. A question streams an answer with
// its sources; an assist_proposed frame becomes a dashed row that says it
// waits for approval and links to the agent platform — the palette never
// offers a second approval path. "Open as a panel" carries the same message
// state into the slide-up panel, which collapses to the tab and back.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import { CommandPaletteHost } from "../../components/command-palette";
import { panelTransform } from "../../components/command-palette/chat-panel";
import { actions } from "../../components/command-palette/store";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

const push = vi.hoisted(() => vi.fn());
const runChatTurn = vi.hoisted(() => vi.fn());

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push, replace: vi.fn(), prefetch: vi.fn() }),
  usePathname: () => "/tickets",
}));

vi.mock("../../lib/agent-chat", () => ({ runChatTurn }));

const TICKET_ONE = "71000000-0000-4000-8000-000000000001";
const PROPOSAL_ID = "60000000-0000-4000-8000-000000000001";
const ANSWER = "Two of the twelve are close. #18402 has already breached.";

function setupApi(): MockApi {
  const mock = new MockApi();
  mock.on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }));
  mock.on("GET", "/api/v1/ai/me/surfaces", () => ({
    json: {
      surfaces: [
        {
          surface_key: "general_chat",
          label: "General chat",
          kind: "chat",
          granted: true,
          models_available: true,
          picked: null,
          models: [],
        },
      ],
    },
  }));
  mock.on("GET", "/api/v1/search", () => ({
    json: {
      query: "vpn",
      total: 0,
      total_capped: false,
      kinds: ["ticket"],
      limit: 30,
      offset: 0,
      next_cursor: null,
      results: [],
    },
  }));
  return mock.install();
}

function renderHost() {
  seedAuthTokens();
  return render(
    <AuthProvider>
      <CommandPaletteHost />
    </AuthProvider>,
  );
}

beforeEach(() => {
  actions.resetForTests();
  push.mockClear();
  runChatTurn.mockReset();
  runChatTurn.mockImplementation(
    async (
      _sessionId: string,
      _turn: unknown,
      onFrame?: (frame: { event: string; data: Record<string, unknown> }) => void,
    ) => {
      onFrame?.({ event: "assistant_delta", data: { delta: "Two of the twelve are close. " } });
      onFrame?.({ event: "assistant_delta", data: { delta: "#18402 has already breached." } });
      onFrame?.({
        event: "tool_invocation",
        data: { tool: "search", capability: "tickets:read", outcome: "executed" },
      });
      onFrame?.({
        event: "assist_proposed",
        data: { proposal_id: PROPOSAL_ID, ticket_id: TICKET_ONE },
      });
      onFrame?.({ event: "turn_completed", data: { outcome: "completed", text: ANSWER } });
      return { frames: [], text: ANSWER };
    },
  );
});

async function openAsk(user: ReturnType<typeof userEvent.setup>) {
  await user.keyboard("{Meta>}k{/Meta}");
  const dialog = await screen.findByRole("dialog");
  await user.keyboard("{Tab}");
  await waitFor(() => {
    expect(within(dialog).getByRole("radio", { name: "Ask" }).getAttribute("aria-checked")).toBe(
      "true",
    );
  });
  return dialog;
}

describe("command palette with the general_chat grant", () => {
  it("switches to Ask on Tab and shows what the chat can see", async () => {
    setupApi();
    renderHost();
    const user = userEvent.setup();
    const dialog = await openAsk(user);

    expect(within(dialog).getByTestId("palette-context-chip").textContent).toContain(
      "This screen: Tickets",
    );
  });

  it("streams an answer with sources and a dashed proposal that waits for approval", async () => {
    setupApi();
    renderHost();
    const user = userEvent.setup();
    const dialog = await openAsk(user);

    const input = within(dialog).getByRole("textbox", { name: "Ask about this screen" });
    await user.type(input, "Which of these will breach before lunch?{Enter}");

    await waitFor(() => {
      expect(within(dialog).getByText(ANSWER)).toBeTruthy();
    });
    expect(within(dialog).getByText(/Used search · executed/)).toBeTruthy();

    const proposal = within(dialog).getByTestId("palette-proposal");
    expect(proposal.textContent).toContain("A change is proposed");
    expect(proposal.textContent).toContain("Waits for your approval. Nothing is sent.");
    expect(within(proposal).getByRole("link", { name: "Review" }).getAttribute("href")).toBe(
      "/settings/agent-platform",
    );
    // The agent platform owns approval; this surface must not grow a second
    // path to it.
    expect(within(proposal).queryByRole("button", { name: /approve/i })).toBeNull();

    const call = runChatTurn.mock.calls[0];
    expect(call).toBeDefined();
    const [sessionId, turn] = call as [string, {
      surface_key: string;
      session_opener: boolean;
      messages: Array<{ role: string; content: string }>;
    }];
    expect(sessionId).toMatch(/^[0-9a-f-]{36}$/);
    expect(turn.surface_key).toBe("general_chat");
    expect(turn.session_opener).toBe(true);
    expect(turn.messages[0]?.role).toBe("system");
    expect(turn.messages.at(-1)).toEqual({
      role: "user",
      content: "Which of these will breach before lunch?",
    });
  });

  it("carries the conversation into the panel, collapses to the tab, and Esc collapses again", async () => {
    setupApi();
    renderHost();
    const user = userEvent.setup();
    const dialog = await openAsk(user);

    const input = within(dialog).getByRole("textbox", { name: "Ask about this screen" });
    await user.type(input, "Which of these will breach before lunch?{Enter}");
    await waitFor(() => {
      expect(within(dialog).getByText(ANSWER)).toBeTruthy();
    });

    await user.click(within(dialog).getByRole("button", { name: "Open as a panel" }));

    const panel = await screen.findByTestId("chat-panel");
    expect(screen.queryByRole("dialog", { name: "Ask about this screen" })).toBeTruthy();
    expect(within(panel).getByText(ANSWER)).toBeTruthy();
    expect(within(panel).getByTestId("chat-panel-context").textContent).toContain("Tickets");
    await waitFor(() => {
      expect(screen.getByTestId("chat-panel").getAttribute("data-phase")).toBe("open");
    });

    await user.click(within(panel).getByRole("button", { name: "Collapse the chat" }));
    const tab = await screen.findByTestId("chat-panel-tab");
    expect(tab.textContent).toContain("Ask");
    expect(tab.textContent).toContain("Two of the twelve are close.");
    expect(screen.queryByTestId("chat-panel")).toBeNull();

    await user.click(tab);
    await waitFor(() => {
      expect(screen.getByTestId("chat-panel").getAttribute("data-phase")).toBe("open");
    });

    await user.keyboard("{Escape}");
    await waitFor(() => {
      expect(screen.queryByTestId("chat-panel")).toBeNull();
    });
    expect(screen.getByTestId("chat-panel-tab")).toBeTruthy();
  });
});

describe("panel geometry", () => {
  it("starts mid-expansion (K04) and settles to its anchor (K03)", () => {
    expect(panelTransform("entering")).toBe("translateY(58%)");
    expect(panelTransform("open")).toBe("translateY(0)");
  });
});
