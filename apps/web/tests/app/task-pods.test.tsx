// Integration test: the task workbench pods. The org's role documents are a
// ceiling: a pod granted only to a role the caller does not hold is hidden,
// while a pod no document names stays. Personal hide/order still layers on top.
//
// The documents are keyed by what `/me` reports for a holder — a builtin role's
// preset tag or a custom role's name — so the picker's options come from
// `/api/v1/roles`, not from a list of its own.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AuthProvider } from "../../lib/api";
import { TaskPodsPanel } from "../../components/projects/task-pods";
import { ME_FIXTURE, MockApi, seedAuthTokens } from "../helpers/mock-api";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: vi.fn(), push: vi.fn() }),
  usePathname: () => "/projects/p1/tasks/t1",
}));

const PROJECT = "00000000-0000-4000-8000-0000000000a1";
const TASK = "00000000-0000-4000-8000-0000000000b1";

function settingsRow(roleKey: string, pods: string[]) {
  return { surface: "project_task", role_key: roleKey, pods, updated_at: "" };
}

function roleRow(builtin: string | null, name: string) {
  return {
    id: `role-${name}`,
    org_id: "00000000-0000-4000-8000-000000000002",
    name,
    builtin,
    is_system: builtin !== null,
    forked_from: null,
    permissions: {},
    holders: 1,
  };
}

// The org's real roles: two presets and one custom role the admin created.
const ROLES = {
  roles: [
    roleRow("dispatcher", "Dispatcher"),
    roleRow("senior_tech", "Senior Tech"),
    roleRow("technician", "Technician"),
    roleRow(null, "Bench Lead"),
  ],
  presets: [],
  portal_tiers: [],
};

function setupApi(
  hidden: string[] = [],
  admin = false,
  settings: { rows?: unknown[]; fail?: boolean } = {},
): MockApi {
  const mock = new MockApi();
  // The caller is a technician. `chain` is granted only to dispatchers, so it
  // is outside the caller's ceiling; `links` is named by no document and stays
  // open.
  mock.on("GET", "/api/v1/me", () => ({
    json: {
      ...ME_FIXTURE,
      permissions: admin
        ? [...ME_FIXTURE.permissions, "projects:edit"]
        : ME_FIXTURE.permissions,
      roles: ["technician"],
    },
  }));
  mock.on("GET", `/api/v1/projects/${PROJECT}/tasks/${TASK}/dependencies`, () => ({
    json: { depends_on: [] },
  }));
  if (settings.fail === true) {
    mock.on("GET", "/api/v1/project-pods/settings", () => ({
      status: 500,
      json: { error: { reason: "internal", message: "the settings read failed" } },
    }));
  } else {
    mock.on("GET", "/api/v1/project-pods/settings", () => ({
      json: settings.rows ?? [
        settingsRow("technician", ["status"]),
        settingsRow("dispatcher", ["chain"]),
      ],
    }));
  }
  mock.on("GET", "/api/v1/project-pods/prefs", () => ({
    json: { surface: "project_task", hidden, pod_order: [] },
  }));
  mock.on("GET", "/api/v1/roles", () => ({ json: ROLES }));
  mock.on("PUT", "/api/v1/project-pods/settings", ({ body }) => {
    const next = body as { role_key: string; pods: string[] };
    return { json: settingsRow(next.role_key, next.pods) };
  });
  mock.install();
  return mock;
}

function mount() {
  return render(
    <AuthProvider>
      <TaskPodsPanel projectId={PROJECT} taskId={TASK} taskTitle="Cut over mail" />
    </AuthProvider>,
  );
}

beforeEach(() => {
  window.localStorage.clear();
  seedAuthTokens();
});

describe("TaskPodsPanel role ceiling", () => {
  it("hides a pod granted only to a role the caller does not hold", async () => {
    setupApi();
    mount();
    expect(await screen.findByLabelText("Status and blocker")).toBeInTheDocument();
    // Unrestricted: no role document names it, so it stays.
    expect(await screen.findByLabelText("Links")).toBeInTheDocument();
    // Granted only to dispatchers; the caller is a technician.
    await waitFor(() => expect(screen.queryByLabelText("Waiting on")).toBeNull());
    cleanup();
  });

  it("keeps the every-pod-hidden note when the ceiling and the person hide them all", async () => {
    setupApi(["status", "links"]);
    mount();
    // `chain` is already outside the role ceiling; the personal prefs hide the
    // other two, so nothing is left to draw.
    expect(
      await screen.findByText(
        "Every pod is hidden. Turn one back on in Pods below to see it here.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("Status and blocker")).toBeNull();
    expect(screen.queryByLabelText("Waiting on")).toBeNull();
    expect(screen.queryByLabelText("Links")).toBeNull();
    cleanup();
  });
});

describe("TaskPodsPanel role documents", () => {
  it("offers the org's roles and stores a key a holder can match", async () => {
    const mock = setupApi([], true);
    const user = userEvent.setup();
    mount();

    // The default names a role that exists: the technician preset, not the
    // legacy `project_tech` the picker used to hardcode.
    const trigger = await screen.findByLabelText("Role document");
    expect(trigger).toHaveTextContent("Technician");

    // Options come from `/api/v1/roles`, labelled with the display name, and
    // include a custom role — a document for it is keyed by that name.
    await user.click(trigger);
    expect(await screen.findByRole("option", { name: "Senior Tech" })).toBeInTheDocument();
    await user.click(screen.getByRole("option", { name: "Bench Lead" }));

    await waitFor(() => expect(trigger).toHaveTextContent("Bench Lead"));
    await user.click(screen.getByRole("button", { name: "Timer" }));
    await waitFor(() =>
      expect(mock.lastBody("PUT", "/api/v1/project-pods/settings")).toMatchObject({
        role_key: "Bench Lead",
        surface: "project_task",
      }),
    );
    cleanup();
  });
});

describe("TaskPodsPanel when the role documents are absent or unreadable", () => {
  it("treats an empty document set as no ceiling", async () => {
    // No document names a pod, so no document restricts one: the panel must
    // draw all three rather than inferring a ceiling from silence.
    setupApi([], false, { rows: [] });
    mount();
    expect(await screen.findByLabelText("Status and blocker")).toBeInTheDocument();
    expect(await screen.findByLabelText("Waiting on")).toBeInTheDocument();
    expect(await screen.findByLabelText("Links")).toBeInTheDocument();
    cleanup();
  });

  it("draws the pods when the settings read fails instead of holding a spinner", async () => {
    setupApi([], false, { fail: true });
    mount();
    // A failed read degrades to "no documents": the workbench stays usable,
    // which is the only honest behaviour when the ceiling is unknown.
    expect(await screen.findByLabelText("Status and blocker")).toBeInTheDocument();
    expect(await screen.findByLabelText("Links")).toBeInTheDocument();
    cleanup();
  });
});
