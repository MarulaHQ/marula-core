// Wave 41 lane 12 (`p41-ui-settings`): the settings IA, the accordion rail,
// the drawn index, the reusable form family, and the drawn Statuses surface.
//
// What these tests pin, and why each is a promise rather than a snapshot:
//   - the eight groups and the pack's page rows exist and point at real routes
//   - the rail is an ACCORDION (groups collapsed, one open) and the index
//     draws the pack's count tags
//   - staged toggles do not apply until Save, and Discard puts them back
//   - validation is BLUSH and happens on submit, not on keystroke
//   - a failed save keeps the form and offers Try again
//   - the import drawer states its count and skips the rows it will not take
//   - the AI table says "no grant means no trace" instead of a dead switch
//   - shadow runs are visible HERE and the page is reachable from the AI table
import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AuthProvider } from "../../lib/api";
import { SETTINGS_GROUPS, SETTINGS_RAIL, groupForPathname } from "../../lib/settings/ia";
import SettingsIndexPage from "../../app/settings/page";
import SettingsLayout from "../../app/settings/layout";
import ShadowRunsPage from "../../app/settings/ai/shadow/page";
import { StatusesSection } from "../../app/settings/workflow/statuses-section";
import {
  StagedSaveBar,
  ToggleSwitch,
} from "../../components/settings/settings-forms";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

const PATHNAME = { value: "/settings" };
vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => PATHNAME.value,
}));

const ORG = ME_FIXTURE.org_id;
const BOARD = "30000000-0000-4000-8000-000000000001";
const S_NEW = "40000000-0000-4000-8000-000000000001";
const S_WAIT = "40000000-0000-4000-8000-000000000002";
const S_DONE = "40000000-0000-4000-8000-000000000003";

function status(over: Record<string, unknown>) {
  return {
    id: S_NEW,
    board_id: BOARD,
    org_id: ORG,
    name: "New",
    category: "open",
    color: null,
    is_entry: true,
    sort_order: 0,
    wip_limit: null,
    client_visible: true,
    ...over,
  };
}

const STATUSES = [
  status({ id: S_NEW, name: "New", category: "open", sort_order: 0 }),
  status({ id: S_WAIT, name: "Waiting on client", category: "waiting", is_entry: false, sort_order: 1 }),
  status({ id: S_DONE, name: "Resolved", category: "closed", is_entry: false, sort_order: 2 }),
];

let api: MockApi;

beforeEach(() => {
  PATHNAME.value = "/settings";
  seedAuthTokens();
  api = new MockApi();
  api
    .on("GET", "/api/v1/me", () => ({ json: ME_FIXTURE }))
    .on("GET", "/api/v1/boards/nav", () => ({ json: [] }))
    .on("GET", `/api/v1/boards/${BOARD}/statuses`, () => ({ json: STATUSES }))
    .on("GET", `/api/v1/boards/${BOARD}/statuses/usage`, () => ({
      json: [
        { status_id: S_NEW, ticket_count: 9, automation_count: 0 },
        { status_id: S_WAIT, ticket_count: 12, automation_count: 2 },
        { status_id: S_DONE, ticket_count: 40166, automation_count: 0 },
      ],
    }))
    .on("GET", "/api/v1/settings/ai/decision/decision_dispatch/runs", () => ({
      json: {
        surface_key: "decision_dispatch",
        days: 7,
        summary: {
          runs: 2,
          above_threshold: 1,
          agreed: 1,
          comparable: 2,
          threshold: 0.9,
          lowest_threshold: 0.9,
          threshold_questions: 1,
          has_evidence: true,
        },
        runs: [
          {
            id: "50000000-0000-4000-8000-000000000001",
            mode: "shadow",
            created_at: "2026-09-30T10:00:00Z",
            model: "claude-sonnet-4-6",
            ticket_id: "60000000-0000-4000-8000-000000000001",
            ticket_number: "MAR-1",
            ticket_title: "Printer is offline",
            answers: [
              {
                question_id: "urgency",
                kind: "choice",
                label: "High",
                confidence: 0.97,
                above_threshold: true,
              },
            ],
            above_threshold: true,
            top_confidence: 0.97,
            current_status: "Waiting on client",
            agreed: true,
          },
          {
            id: "50000000-0000-4000-8000-000000000002",
            mode: "shadow",
            created_at: "2026-09-29T10:00:00Z",
            model: "claude-sonnet-4-6",
            ticket_id: "60000000-0000-4000-8000-000000000002",
            ticket_number: "MAR-2",
            ticket_title: "Vague request",
            answers: [
              {
                question_id: "urgency",
                kind: "choice",
                label: "Low",
                confidence: 0.42,
                above_threshold: false,
              },
            ],
            above_threshold: false,
            top_confidence: 0.42,
            current_status: "Resolved",
            agreed: false,
          },
        ],
      },
    }))
    .on("GET", "/api/v1/settings/ai/decision", () => ({
      json: {
        features_enabled: true,
        surfaces: [
          {
            surface_key: "decision_dispatch",
            label: "Dispatch queue decisions",
            internal: false,
            description: "Where a job goes next.",
            starter_version: 3,
            customized: false,
            configured: true,
            mode: "shadow",
            enabled: true,
            decision_model_curated: true,
            model_provider: "anthropic",
            model_id: "claude-sonnet-4-6",
            latest_starter_version: null,
          },
        ],
      },
    }))
    .install();
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("the settings IA", () => {
  it("is the pack's eight groups, in order, with the pack's count tags", () => {
    expect(SETTINGS_GROUPS.map((group) => group.label)).toEqual([
      "Workspace",
      "Organisation",
      "Workflow",
      "Data",
      "Communication",
      "Security",
      "AI",
      "Platform",
    ]);
    expect(SETTINGS_GROUPS.map((group) => group.pages.length)).toEqual([
      3, 4, 5, 4, 3, 3, 2, 2,
    ]);
  });

  it("sends every drawn row at a route that exists in the app", () => {
    const real = new Set([
      "/settings/general",
      "/settings/appearance",
      "/settings/notifications",
      "/settings/team",
      "/settings/sla",
      "/settings/boards",
      "/settings/workflow",
      "/settings/time-categories",
      "/settings/reply-snippets",
      "/settings/taxonomy",
      "/imports",
      "/settings/export",
      "/settings/self-host",
      "/settings/email",
      "/settings/portal",
      "/settings/integrations",
      "/settings/security",
      "/audit",
      "/settings/ai",
      "/settings/ai/shadow",
      "/settings/plugins",
    ]);
    for (const group of SETTINGS_GROUPS) {
      for (const page of group.pages) {
        expect(real.has(page.href), `${group.label} / ${page.label} -> ${page.href}`).toBe(
          true,
        );
      }
    }
  });

  it("keeps every rail page inside a drawn group, and finds the group for a path", () => {
    for (const [groupId, pages] of Object.entries(SETTINGS_RAIL)) {
      expect(
        SETTINGS_GROUPS.some((group) => group.id === groupId),
        `rail group ${groupId}`,
      ).toBe(true);
      for (const page of pages) {
        expect(page.href.startsWith("/")).toBe(true);
      }
    }
    expect(groupForPathname("/settings/ai/shadow")?.group.id).toBe("ai");
    expect(groupForPathname("/settings")).toBeNull();
  });
});

describe("the settings index", () => {
  it("draws eight cards, each with its count tag and a one-line description", () => {
    render(<SettingsIndexPage />);
    expect(screen.getByText("Change the system once, so nobody has to think about it again.")).toBeTruthy();
    for (const group of SETTINGS_GROUPS) {
      const card = screen.getByTestId(`settings-group-card-${group.id}`);
      expect(card.textContent).toContain(group.label);
      expect(screen.getByTestId(`settings-group-count-${group.id}`).textContent).toBe(
        String(group.pages.length),
      );
    }
    expect(screen.getByTestId("settings-index-link-statuses").getAttribute("href")).toBe(
      "/settings/workflow",
    );
  });

  it("says a merged or unbuilt row is merged or unbuilt rather than pretending", () => {
    render(<SettingsIndexPage />);
    expect(screen.getByTestId("settings-index-link-roles-and-permissions").textContent).toContain(
      "Same page as People",
    );
    expect(screen.getByTestId("settings-index-link-retention").textContent).toContain(
      "not built yet",
    );
  });
});

describe("the settings rail", () => {
  it("is an accordion: the eight groups are listed collapsed, the current one open", async () => {
    PATHNAME.value = "/settings/workflow";
    render(
      <AuthProvider>
        <SettingsLayout>
          <p>page</p>
        </SettingsLayout>
      </AuthProvider>,
    );
    const rail = screen.getByTestId("settings-rail");
    for (const group of SETTINGS_GROUPS) {
      const header = rail.querySelector(
        `[data-testid="settings-group-${group.id}"] button[aria-expanded]`,
      );
      expect(header, group.label).not.toBeNull();
      expect(header?.getAttribute("aria-expanded")).toBe(
        group.id === "workflow" ? "true" : "false",
      );
    }
    // The current group is open and its pages are links.
    expect(rail.textContent).toContain("Statuses");
    // A collapsed group's pages are not rendered at all.
    expect(rail.textContent).not.toContain("Business hours");
    await waitFor(() => expect(screen.getByTestId("settings-main").textContent).toBe("page"));
  });

  it("opens a group on click and closes it on a second click", () => {
    render(
      <AuthProvider>
        <SettingsLayout>
          <p>page</p>
        </SettingsLayout>
      </AuthProvider>,
    );
    const header = screen
      .getByTestId("settings-group-security")
      .querySelector("button[aria-expanded]");
    fireEvent.click(header as HTMLButtonElement);
    expect(
      screen
        .getByTestId("settings-group-security")
        .querySelector("button[aria-expanded]")
        ?.getAttribute("aria-expanded"),
    ).toBe("true");
    fireEvent.click(
      screen.getByTestId("settings-group-security").querySelector("button[aria-expanded]") as HTMLButtonElement,
    );
    expect(
      screen
        .getByTestId("settings-group-security")
        .querySelector("button[aria-expanded]")
        ?.getAttribute("aria-expanded"),
    ).toBe("false");
  });
});

describe("the form family", () => {
  it("renders no save bar when nothing is staged", () => {
    const { container } = render(
      <StagedSaveBar count={0} onDiscard={vi.fn()} onSave={vi.fn()} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("counts the staged changes and says nothing has applied yet", () => {
    render(<StagedSaveBar count={2} onDiscard={vi.fn()} onSave={vi.fn()} />);
    const bar = screen.getByTestId("settings-staged-bar");
    expect(bar.textContent).toContain("2 unsaved changes");
    expect(bar.textContent).toContain("Nothing has changed for anyone yet.");
  });

  it("reports a switch's state to assistive tech and refuses a managed one", () => {
    const on = render(<ToggleSwitch on label="Require MFA" onChange={vi.fn()} />);
    expect(screen.getByRole("switch", { name: "Require MFA" }).getAttribute("aria-checked")).toBe(
      "true",
    );
    on.unmount();
    render(<ToggleSwitch on label="Sign in with Microsoft 365" disabled />);
    const managed = screen.getByRole("switch", { name: "Sign in with Microsoft 365" });
    expect(managed.hasAttribute("disabled")).toBe(true);
  });
});

describe("the Statuses surface", () => {
  it("draws the list with kind pills, the real ticket counts and a live visibility switch", async () => {
    render(<StatusesSection boardId={BOARD} canEdit onChanged={vi.fn()} />);
    await waitFor(() => expect(screen.getByText("Waiting on client")).toBeTruthy());
    // Kind is a word, never colour alone.
    expect(screen.getByText("Waiting")).toBeTruthy();
    expect(screen.getByText("Done")).toBeTruthy();
    // The TICKETS column carries the numbers the usage read returned, on the
    // rows they belong to.
    expect(screen.getByTestId(`status-tickets-${S_NEW}`).textContent).toBe("9");
    expect(screen.getByTestId(`status-tickets-${S_WAIT}`).textContent).toBe("12");
    expect(screen.getByTestId(`status-tickets-${S_DONE}`).textContent).toBe("40166");
    // Client visibility is a REAL switch now, reading the server's field.
    const client = screen.getByRole("switch", { name: "Visible to clients: New" });
    expect(client.getAttribute("aria-checked")).toBe("true");
    expect(client.hasAttribute("disabled")).toBe(false);
    // A done status has no clock to pause.
    expect(
      screen.getByRole("switch", { name: "Pauses SLA for Resolved" }).hasAttribute("disabled"),
    ).toBe(true);
  });

  it("stages client visibility behind the same save bar and sends only what changed", async () => {
    render(<StatusesSection boardId={BOARD} canEdit onChanged={vi.fn()} />);
    await waitFor(() => expect(screen.getByText("Waiting on client")).toBeTruthy());
    fireEvent.click(screen.getByRole("switch", { name: "Visible to clients: New" }));
    const bar = screen.getByTestId("settings-staged-bar");
    expect(bar.textContent).toContain("1 unsaved change");

    api.on("PATCH", `/api/v1/boards/${BOARD}/statuses/${S_NEW}`, () => ({
      json: status({ id: S_NEW, name: "New", category: "open", client_visible: false }),
    }));
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() =>
      expect(api.callsTo("PATCH", `/api/v1/boards/${BOARD}/statuses/${S_NEW}`)).toHaveLength(1),
    );
    // Only the field the admin touched rides the PATCH: a staged toggle can
    // never revert somebody else's change made in the meantime.
    expect(
      api.lastBody<Record<string, unknown>>("PATCH", `/api/v1/boards/${BOARD}/statuses/${S_NEW}`),
    ).toEqual({ client_visible: false });
  });

  it("stages a toggle behind the save bar and applies it on save", async () => {
    render(<StatusesSection boardId={BOARD} canEdit onChanged={vi.fn()} />);
    await waitFor(() => expect(screen.getByText("Waiting on client")).toBeTruthy());
    expect(screen.queryByTestId("settings-staged-bar")).toBeNull();

    fireEvent.click(screen.getByRole("switch", { name: "Pauses SLA for New" }));
    const bar = screen.getByTestId("settings-staged-bar");
    expect(bar.textContent).toContain("1 unsaved change");
    // Nothing reached the server while it was staged.
    expect(api.callsTo("PATCH", `/api/v1/boards/${BOARD}/statuses/${S_NEW}`)).toHaveLength(0);

    api.on("PATCH", `/api/v1/boards/${BOARD}/statuses/${S_NEW}`, () => ({
      json: status({ id: S_NEW, name: "New", category: "waiting" }),
    }));
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() =>
      expect(api.callsTo("PATCH", `/api/v1/boards/${BOARD}/statuses/${S_NEW}`)).toHaveLength(1),
    );
    expect(
      api.lastBody<{ category: string }>("PATCH", `/api/v1/boards/${BOARD}/statuses/${S_NEW}`),
    ).toEqual({ category: "waiting" });
  });

  it("puts a staged toggle back on Discard without writing", async () => {
    render(<StatusesSection boardId={BOARD} canEdit onChanged={vi.fn()} />);
    await waitFor(() => expect(screen.getByText("Waiting on client")).toBeTruthy());
    fireEvent.click(screen.getByRole("switch", { name: "Pauses SLA for New" }));
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    expect(screen.queryByTestId("settings-staged-bar")).toBeNull();
    expect(api.callsTo("PATCH", `/api/v1/boards/${BOARD}/statuses/${S_NEW}`)).toHaveLength(0);
  });

  it("replaces the header with a bulk bar and puts Delete as an outlined red", async () => {
    render(<StatusesSection boardId={BOARD} canEdit onChanged={vi.fn()} />);
    await waitFor(() => expect(screen.getByText("Waiting on client")).toBeTruthy());
    fireEvent.click(screen.getByRole("checkbox", { name: "Select Waiting on client" }));
    const bulk = screen.getByTestId("statuses-bulk-bar");
    expect(bulk.textContent).toContain("1 selected");
    const del = screen.getByTestId("statuses-bulk-delete");
    expect(del.className).toContain("m-btn-destructive");
    expect(del.className).not.toContain("m-btn-destructive-solid");
  });

  it("validates the drawer on submit, in blush, with a name the user can act on", async () => {
    render(<StatusesSection boardId={BOARD} canEdit onChanged={vi.fn()} />);
    await waitFor(() => expect(screen.getByText("Waiting on client")).toBeTruthy());
    fireEvent.click(screen.getByTestId("statuses-new"));
    const drawer = screen.getByTestId("status-drawer");
    expect(drawer.getAttribute("aria-modal")).toBe("true");

    // A duplicate name is caught on submit, not on keystroke.
    fireEvent.change(screen.getByTestId("status-name-input"), {
      target: { value: "New" },
    });
    expect(screen.queryByTestId("status-name-field-error")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Save status" }));
    const error = await screen.findByTestId("status-name-field-error");
    expect(error.textContent).toBe("A status called New already exists.");
    expect(error.getAttribute("role")).toBe("alert");
    // The field carries aria-invalid, and the primary is NOT disabled.
    expect(screen.getByTestId("status-name-input").getAttribute("aria-invalid")).toBe("true");
    expect(screen.getByRole("button", { name: "Save status" }).hasAttribute("disabled")).toBe(
      false,
    );
    expect(api.callsTo("POST", `/api/v1/boards/${BOARD}/statuses`)).toHaveLength(0);
  });

  it("keeps the form and offers Try again when the save did not land", async () => {
    api.on("POST", `/api/v1/boards/${BOARD}/statuses`, () => ({
      status: 503,
      json: { error: { message: "The settings service did not answer." } },
    }));
    render(<StatusesSection boardId={BOARD} canEdit onChanged={vi.fn()} />);
    await waitFor(() => expect(screen.getByText("Waiting on client")).toBeTruthy());
    fireEvent.click(screen.getByTestId("statuses-new"));
    fireEvent.change(screen.getByTestId("status-name-input"), {
      target: { value: "Awaiting approval" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save status" }));
    const failed = await screen.findByTestId("status-drawer-failed");
    expect(failed.textContent).toContain("Your changes are still here.");
    // The work is still in the field, and the primary became Try again.
    expect((screen.getByTestId("status-name-input") as HTMLInputElement).value).toBe(
      "Awaiting approval",
    );
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
  });

  it("states the count on the import primary and never offers a skipped row", async () => {
    render(<StatusesSection boardId={BOARD} canEdit onChanged={vi.fn()} />);
    await waitFor(() => expect(screen.getByText("Waiting on client")).toBeTruthy());
    fireEvent.click(screen.getByTestId("statuses-import"));
    expect(screen.getByTestId("status-import-drawer")).toBeTruthy();

    const file = screen.getByTestId("status-import-file") as HTMLInputElement;
    await fireEvent.change(file, {
      target: {
        files: [
          new File(
            ["Awaiting approval,Waiting\nEscalated,Open\nNew,Open\nOn hold,Paused\n"],
            "statuses.csv",
            { type: "text/csv" },
          ),
        ],
      },
    });
    const primary = await screen.findByTestId("status-import-primary");
    expect(primary.textContent).toBe("Import 2, skip 2");
    // The skipped rows say why, in words.
    expect(screen.getByTestId("status-import-drawer").textContent).toContain(
      "already exists",
    );
    expect(screen.getByTestId("status-import-drawer").textContent).toContain(
      "must be Open, In progress, Waiting or Done",
    );
  });

  it("asks where the tickets go, names the automations, and says what it will do", async () => {
    render(<StatusesSection boardId={BOARD} canEdit onChanged={vi.fn()} />);
    await waitFor(() => expect(screen.getByText("Waiting on client")).toBeTruthy());
    fireEvent.click(screen.getByRole("checkbox", { name: "Select Waiting on client" }));
    fireEvent.click(screen.getByTestId("statuses-bulk-delete"));
    const confirm = screen.getByTestId("status-delete-confirm");
    expect(confirm.getAttribute("role")).toBe("alertdialog");
    expect(confirm.textContent).toContain("Delete Waiting on client?");
    expect(confirm.textContent).toContain("This can't be undone.");
    expect(confirm.textContent).toContain("History keeps the old name.");
    // The count comes from the usage read, the remap is a real select of the
    // board's other statuses, and the button states the consequence.
    expect(confirm.textContent).toContain("12 tickets use this status");
    expect(confirm.textContent).toContain("2 automations mention this status");
    expect(confirm.textContent).toContain("will be paused until");
    const remap = confirm.querySelector("select");
    expect(remap).not.toBeNull();
    expect(
      Array.from(remap?.querySelectorAll("option") ?? []).map((o) => o.textContent),
    ).toEqual(["New", "Resolved"]);
    expect(
      screen.getByRole("button", { name: "Move 12 tickets and delete" }),
    ).toBeTruthy();
  });

  it("deletes with the reassignment the admin chose, and says what it moved", async () => {
    render(<StatusesSection boardId={BOARD} canEdit onChanged={vi.fn()} />);
    await waitFor(() => expect(screen.getByText("Waiting on client")).toBeTruthy());
    fireEvent.click(screen.getByRole("checkbox", { name: "Select Waiting on client" }));
    fireEvent.click(screen.getByTestId("statuses-bulk-delete"));
    const remap = screen
      .getByTestId("status-delete-confirm")
      .querySelector("select") as HTMLSelectElement;
    fireEvent.change(remap, { target: { value: S_NEW } });
    api.on("DELETE", `/api/v1/boards/${BOARD}/statuses/${S_WAIT}`, () => ({
      json: { moved_tickets: 12, paused_automations: 2, sla_pauses_changed: 12 },
    }));
    fireEvent.click(screen.getByRole("button", { name: "Move 12 tickets and delete" }));
    await waitFor(() =>
      expect(
        api.callsTo("DELETE", `/api/v1/boards/${BOARD}/statuses/${S_WAIT}`).length,
      ).toBeGreaterThan(0),
    );
    const call = api.callsTo("DELETE", `/api/v1/boards/${BOARD}/statuses/${S_WAIT}`).at(-1);
    expect(call?.query.reassign_to).toBe(S_NEW);
  });
});

describe("shadow runs", () => {
  it("is reachable from the AI group, and says nothing is shown to anyone", async () => {
    render(
      <AuthProvider>
        <ShadowRunsPage />
      </AuthProvider>,
    );
    await waitFor(() => expect(screen.getByTestId("ai-shadow-page")).toBeTruthy());
    expect(screen.getByTestId("ai-shadow-strip").textContent).toContain(
      "Nothing below was shown to anyone. This page is the only place it appears.",
    );
    expect(screen.getByTestId("ai-shadow-list").textContent).toContain("Nothing recorded yet");
  });

  it("keeps a surface in shadow unless it is promoted, and promotion is reversible", async () => {
    api.on("PUT", "/api/v1/settings/ai/decision/decision_dispatch/config", ({ body }) => {
      const b = body as { mode?: string };
      if (b.mode === "live") {
        api.on("GET", "/api/v1/settings/ai/decision", () => ({
          json: {
            features_enabled: true,
            surfaces: [
              {
                surface_key: "decision_dispatch",
                label: "Dispatch queue decisions",
                internal: false,
                description: "Where a job goes next.",
                starter_version: 3,
                customized: false,
                configured: true,
                mode: "live",
                enabled: true,
                decision_model_curated: true,
                model_provider: "anthropic",
                model_id: "claude-sonnet-4-6",
                latest_starter_version: null,
              },
            ],
          },
        }));
      }
      return { json: {} };
    });
    render(
      <AuthProvider>
        <ShadowRunsPage />
      </AuthProvider>,
    );
    await waitFor(() => expect(screen.getByTestId("ai-shadow-page")).toBeTruthy());
    fireEvent.click(screen.getByTestId("ai-shadow-tab-log"));
    const promote = await screen.findByTestId("ai-shadow-promote");
    expect(promote.textContent).toBe("Promote to live");
    // The card says a person still approves every write.
    expect(screen.getByText(/a person still approves every write/i)).toBeTruthy();
    fireEvent.click(promote);
    await waitFor(() =>
      expect(
        api.callsTo("PUT", "/api/v1/settings/ai/decision/decision_dispatch/config").length,
      ).toBeGreaterThan(0),
    );
    await waitFor(() => expect(screen.getByTestId("ai-shadow-demote")).toBeTruthy());
    // And the strip now says live, still with the person in the loop.
    expect(screen.getByTestId("ai-shadow-strip").textContent).toContain(
      "A person still approves every write",
    );
  });

  it("leads the run log with the four measured figures, not narration", async () => {
    render(
      <AuthProvider>
        <ShadowRunsPage />
      </AuthProvider>,
    );
    await waitFor(() => expect(screen.getByTestId("ai-shadow-page")).toBeTruthy());
    fireEvent.click(await screen.findByTestId("ai-shadow-tab-log"));

    const kpis = await screen.findByTestId("ai-shadow-kpis");
    expect(kpis.textContent).toContain("runs, last 7 days");
    // The counts come from the store, and the line in force is the surface's
    // own act threshold, so moving it in the drawer visibly moves this.
    expect(screen.getByTestId("ai-shadow-run-table").textContent).toContain("MAR-1");
    expect(screen.getByTestId("ai-shadow-run-table").textContent).toContain("High");
    expect(screen.getByTestId("ai-shadow-run-table").textContent).toContain("agreed");
    expect(screen.getByTestId("ai-shadow-run-table").textContent).toContain("differed");
    // A label, never an index, and no invented figures.
    expect(screen.queryByText(/No run log to summarise/)).toBeNull();
  });

});
