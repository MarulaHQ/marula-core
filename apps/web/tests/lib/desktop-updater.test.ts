// Unit tests: desktop updater seam (Wave 34 lane 6).
//
// The seam is the wire between the web app and the Tauri shell's
// check/apply commands. These tests pin the honest-state contract: plain
// web answers "unavailable" (never a pretend check), the shell's payload
// is validated before the UI trusts it, and "up to date" is distinct from
// "couldn't check".
import { afterEach, describe, expect, it } from "vitest";
import {
  checkDesktopUpdate,
  installDesktopUpdate,
  type DesktopUpdateInfo,
} from "../../lib/desktop/updater";

type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

function stubShell(invoke: Invoke): void {
  (window as unknown as { __TAURI__?: { core: { invoke: Invoke } } }).__TAURI__ = {
    core: { invoke },
  };
}

afterEach(() => {
  delete (window as unknown as { __TAURI__?: unknown }).__TAURI__;
});

const UPDATE: DesktopUpdateInfo = {
  version: "0.2.0",
  currentVersion: "0.1.0",
  date: "2026-09-25T00:00:00Z",
  notes: "Fixes.",
};

describe("checkDesktopUpdate", () => {
  it("answers unavailable on plain web (no shell global)", async () => {
    expect(await checkDesktopUpdate()).toEqual({ kind: "unavailable" });
  });

  it("maps a null shell answer to up-to-date", async () => {
    stubShell(async () => null);
    expect(await checkDesktopUpdate()).toEqual({ kind: "current" });
  });

  it("returns the update when the shell reports one", async () => {
    stubShell(async (cmd) => {
      expect(cmd).toBe("check_desktop_update");
      return UPDATE;
    });
    expect(await checkDesktopUpdate()).toEqual({ kind: "available", update: UPDATE });
  });

  it("rejects a malformed payload instead of trusting it", async () => {
    stubShell(async () => ({ version: 7 }));
    const result = await checkDesktopUpdate();
    expect(result.kind).toBe("error");
  });

  it("surfaces an IPC failure as a check error, not as up-to-date", async () => {
    stubShell(async () => {
      throw new Error("channel unreachable");
    });
    expect(await checkDesktopUpdate()).toEqual({
      kind: "error",
      message: "channel unreachable",
    });
  });
});

describe("installDesktopUpdate", () => {
  it("answers unsupported on plain web", async () => {
    expect(await installDesktopUpdate()).toEqual({ kind: "unsupported" });
  });

  it("reports restarting after the shell applies and relaunches", async () => {
    stubShell(async (cmd) => {
      expect(cmd).toBe("install_desktop_update");
      return null;
    });
    expect(await installDesktopUpdate()).toEqual({ kind: "restarting" });
  });

  it("surfaces an install failure as an error the UI can retry", async () => {
    stubShell(async () => {
      throw new Error("signature check failed");
    });
    expect(await installDesktopUpdate()).toEqual({
      kind: "error",
      message: "signature check failed",
    });
  });
});
