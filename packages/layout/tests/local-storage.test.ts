/**
 * @module Tests for persistence adapters: per-user keying, round-trips, and
 * failure surfaces. A Map-backed StorageLike fake keeps these Node-pure.
 */

import { describe, expect, it } from "vitest";
import {
  LocalStoragePanelStateStore,
  MemoryPanelStateStore,
  PanelPersistError,
  type PanelStateStore,
  type StorageLike,
} from "../src/persist";
import { encodeLayoutState } from "../src/serialize";
import { createPane } from "../src/tree";

function fakeStorage(): StorageLike & { dump(): Map<string, string> } {
  const map = new Map<string, string>();
  return {
    getItem: (key) => map.get(key) ?? null,
    setItem: (key, value) => {
      map.set(key, value);
    },
    removeItem: (key) => {
      map.delete(key);
    },
    dump: () => map,
  };
}

async function roundTrip(store: PanelStateStore, userId: string): Promise<unknown | null> {
  const payload = await store.load(userId);
  if (payload === null) return null;
  // Payloads are opaque strings; parse here only to assert content survived.
  return JSON.parse(payload) as unknown;
}

describe("LocalStoragePanelStateStore", () => {
  it("keys state per user id inside the namespace", async () => {
    const storage = fakeStorage();
    const store = new LocalStoragePanelStateStore({ storage });
    const alice = createPane({ tabs: ["Alice"] });
    const bob = createPane({ tabs: ["Bob"] });

    await store.save("alice", encodeLayoutState(alice));
    await store.save("bob", encodeLayoutState(bob));

    expect(storage.dump().get("meridian.layout:alice")).toContain("Alice");
    expect((await roundTrip(store, "alice")) as { root: { tabs: unknown[] } }).toMatchObject({
      root: { tabs: [{ title: "Alice" }] },
    });
    expect(await store.load("carol")).toBeNull();

    await store.clear("alice");
    expect(await store.load("alice")).toBeNull();
    expect(await store.load("bob")).not.toBeNull();
  });

  it("supports custom namespaces in the key format", () => {
    const store = new LocalStoragePanelStateStore({ storage: fakeStorage(), namespace: "custom.ns" });
    expect(store.keyFor("u1")).toBe("custom.ns:u1");
  });

  it("passes raw payloads through untouched, including corrupt ones", async () => {
    const storage = fakeStorage();
    const store = new LocalStoragePanelStateStore({ storage });
    await store.save("u1", "{not json at all");
    expect(await store.load("u1")).toBe("{not json at all");
  });

  it("wraps write failures in PanelPersistError", async () => {
    const failing: StorageLike = {
      getItem: () => null,
      setItem: () => {
        throw new Error("quota exceeded");
      },
      removeItem: () => undefined,
    };
    const store = new LocalStoragePanelStateStore({ storage: failing });
    await expect(store.save("u1", "{}")).rejects.toBeInstanceOf(PanelPersistError);
  });
});

describe("MemoryPanelStateStore", () => {
  it("behaves identically for save/load/clear within its lifetime", async () => {
    const store = new MemoryPanelStateStore();
    const paneNode = createPane({ tabs: ["Session"] });
    await store.save("user", encodeLayoutState(paneNode));
    expect(JSON.parse((await store.load("user")) as string)).toMatchObject({
      root: { kind: "pane" },
    });
    await store.clear("user");
    expect(await store.load("user")).toBeNull();
  });
});
