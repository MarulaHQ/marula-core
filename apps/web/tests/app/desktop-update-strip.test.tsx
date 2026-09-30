// Component test: DesktopUpdateStrip (Wave 34 lane 6).
//
// The strip is the ONLY visible surface of the desktop updater. It must
// stay absent on plain web and on a failed background check, appear when
// an update is available, offer the explicit apply + session dismiss, and
// render the failed state with a retry.
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DesktopUpdateStrip } from "../../components/shell/desktop-update-strip";

type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

const UPDATE = {
  version: "0.2.0",
  currentVersion: "0.1.0",
  date: "2026-09-25T00:00:00Z",
  notes: "Fixes.",
};

function stubShell(invoke: Invoke): void {
  (window as unknown as { __TAURI__?: { core: { invoke: Invoke } } }).__TAURI__ = {
    core: { invoke },
  };
}

afterEach(() => {
  delete (window as unknown as { __TAURI__?: unknown }).__TAURI__;
  cleanup();
});

describe("DesktopUpdateStrip", () => {
  it("renders nothing on plain web", async () => {
    const { container } = render(<DesktopUpdateStrip />);
    await waitFor(() => {
      expect(container.textContent).toBe("");
    });
  });

  it("renders nothing when the background check fails", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    stubShell(async () => {
      throw new Error("channel unreachable");
    });
    const { container } = render(<DesktopUpdateStrip />);
    await waitFor(() => {
      expect(container.textContent).toBe("");
    });
    warn.mockRestore();
  });

  it("shows the available update and applies it on demand", async () => {
    const calls: string[] = [];
    stubShell(async (cmd) => {
      calls.push(cmd);
      if (cmd === "check_desktop_update") return UPDATE;
      if (cmd === "install_desktop_update") return null;
      throw new Error(`unexpected command ${cmd}`);
    });
    render(<DesktopUpdateStrip />);
    expect(
      await screen.findByText("Marula 0.2.0 is ready to install — you're on 0.1.0."),
    ).toBeTruthy();
    await userEvent.click(screen.getByTestId("desktop-update-install"));
    await waitFor(() => {
      expect(calls).toContain("install_desktop_update");
    });
  });

  it("dismisses for the session without checking again", async () => {
    const calls: string[] = [];
    stubShell(async (cmd) => {
      calls.push(cmd);
      return UPDATE;
    });
    const { container } = render(<DesktopUpdateStrip />);
    await screen.findByTestId("desktop-update-strip");
    await userEvent.click(screen.getByTestId("desktop-update-dismiss"));
    await waitFor(() => {
      expect(container.textContent).toBe("");
    });
    expect(calls.filter((c) => c === "check_desktop_update")).toHaveLength(1);
  });

  it("shows the failed state with a retry when install fails", async () => {
    stubShell(async (cmd) => {
      if (cmd === "check_desktop_update") return UPDATE;
      throw new Error("signature check failed");
    });
    render(<DesktopUpdateStrip />);
    await screen.findByTestId("desktop-update-strip");
    await userEvent.click(screen.getByTestId("desktop-update-install"));
    expect(await screen.findByTestId("desktop-update-failed")).toBeTruthy();
    expect(screen.getByText("Update failed — signature check failed")).toBeTruthy();
    expect(screen.getByText("Try again")).toBeTruthy();
  });

  it("offers a way out while installing and re-surfaces a later failure", async () => {
    // Vision-review round 1, checkpoint 3: an in-flight install must not
    // leave an undismissable strip (a stalled download would trap it).
    let rejectInstall: (error: Error) => void = () => {};
    const installGate = new Promise<unknown>((_resolve, reject) => {
      rejectInstall = reject;
    });
    // The strip is dismissed before rejectInstall fires, so nothing is attached
    // to this promise at that moment and Node reports an unhandled rejection,
    // which fails the whole run. A no-op catch marks it handled without
    // changing what the component sees: its own chain still receives the
    // rejection, which is what makes the failed state appear below.
    installGate.catch(() => {});
    stubShell(async (cmd) => {
      if (cmd === "check_desktop_update") return UPDATE;
      return installGate;
    });
    const { container } = render(<DesktopUpdateStrip />);
    await screen.findByTestId("desktop-update-strip");
    await userEvent.click(screen.getByTestId("desktop-update-install"));
    await screen.findByTestId("desktop-update-installing");
    await userEvent.click(screen.getByTestId("desktop-update-hide"));
    await waitFor(() => {
      expect(container.textContent).toBe("");
    });

    rejectInstall(new Error("download stalled out"));
    expect(await screen.findByTestId("desktop-update-failed")).toBeTruthy();
  });
});
