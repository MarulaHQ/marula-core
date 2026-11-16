// Integration test: the task workbench pods. The org's role documents are a
// ceiling: a pod granted only to a role the caller does not hold is hidden,
// while a pod no document names stays. Personal hide/order still layers on top.
import { describe, expect, it, beforeEach, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
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

function setupApi(hidden: string[] = []): MockApi {
  const mock = new MockApi();
  // The caller is a technician. `chain` is granted only to project managers,
  // so it is outside the caller's ceiling; `links` is named by no document and
  // stays open.
  mock.on("GET", "/api/v1/me", () => ({
    json: { ...ME_FIXTURE, roles: ["project_tech"] },
  }));
  mock.on("GET", `/api/v1/projects/${PROJECT}/tasks/${TASK}/dependencies`, () => ({
    json: { depends_on: [] },
  }));
  mock.on("GET", "/api/v1/project-pods/settings", () => ({
    json: [settingsRow("project_tech", ["status"]), settingsRow("project_manager", ["chain"])],
  }));
  mock.on("GET", "/api/v1/project-pods/prefs", () => ({
    json: { surface: "project_task", hidden, pod_order: [] },
  }));
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
    // Granted only to project managers; the caller is a technician.
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
