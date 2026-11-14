// The connector write-form helpers (§3).
//
// The settings patch is the half of the write contract a form can silently get
// wrong: PATCH merges, so an absent key means "keep" and only `null` removes.
// A form that posts only the changed keys and never a removal cannot clear a
// field, and a form that posts the whole document cannot tell the two apart.
import { describe, expect, it } from "vitest";
import {
  audienceSummary,
  bindingPatch,
  readRoles,
  secretsPatch,
  settingsPatch,
  toggleRole,
} from "./forms";

describe("settingsPatch", () => {
  it("sends only the keys that changed", () => {
    expect(
      settingsPatch(
        { base_url: "https://a.example", timeout: 30, enabled: true },
        { base_url: "https://a.example", timeout: 60, enabled: true },
      ),
    ).toEqual({ timeout: 60 });
  });

  it("sends null for a key the operator cleared", () => {
    expect(settingsPatch({ proxy: "10.0.0.1", timeout: 30 }, { timeout: 30 })).toEqual({
      proxy: null,
    });
  });

  it("sends a newly added key", () => {
    expect(settingsPatch({}, { region: "eu" })).toEqual({ region: "eu" });
  });

  it("compares nested values structurally, not by reference", () => {
    expect(
      settingsPatch(
        { network: { host: "a", ports: [1, 2] } },
        { network: { host: "a", ports: [1, 2] } },
      ),
    ).toEqual({});
  });

  it("never includes a credential, even if the form still carries one", () => {
    expect(
      settingsPatch(
        { api_key: "old", host: "a" },
        { api_key: "new", host: "a" },
        ["api_key"],
      ),
    ).toEqual({});
  });
});

describe("secretsPatch", () => {
  it("sends only fields the operator typed into", () => {
    expect(secretsPatch({ api_key: "new", other: "" })).toEqual({ api_key: "new" });
    expect(secretsPatch({})).toEqual({});
  });
});

describe("bindingPatch", () => {
  it("trims the label and turns a blank parent into null", () => {
    expect(bindingPatch({ label: "  #helpdesk  ", enabled: false, external_parent: "  " })).toEqual({
      label: "#helpdesk",
      enabled: false,
      external_parent: null,
    });
  });
});

describe("audienceSummary", () => {
  const roles = [
    { id: "r-1", name: "Dispatcher" },
    { id: "r-2", name: "Senior technician" },
  ];

  it("reads an empty audience as a grant, never as a lock", () => {
    expect(audienceSummary([], roles)).toBe("All staff with access");
  });

  it("names the roles by id", () => {
    expect(audienceSummary(["r-1", "r-2"], roles)).toBe("Dispatcher, Senior technician");
  });

  it("counts the ids it cannot name rather than printing a uuid", () => {
    expect(audienceSummary(["r-1", "r-9"], roles)).toBe(
      "Dispatcher and 1 role this page cannot name",
    );
    expect(audienceSummary(["r-9"], [])).toBe("1 named role");
  });
});

describe("toggleRole", () => {
  it("adds once and keeps the operator's order", () => {
    expect(toggleRole(["b"], "a", true)).toEqual(["b", "a"]);
    expect(toggleRole(["b", "a"], "a", true)).toEqual(["b", "a"]);
    expect(toggleRole(["b", "a"], "b", false)).toEqual(["a"]);
  });
});

describe("readRoles", () => {
  it("reads the roles array out of the list envelope and drops malformed rows", () => {
    expect(
      readRoles({
        roles: [
          { id: "r-1", name: "Dispatcher", is_system: true, permissions: [] },
          { id: 2, name: "broken" },
        ],
        presets: [],
      }),
    ).toEqual([{ id: "r-1", name: "Dispatcher" }]);
    expect(readRoles(undefined)).toEqual([]);
  });
});
