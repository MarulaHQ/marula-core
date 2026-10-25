// Device storage ownership: one user's durable keys are unreachable from
// another user's session, and sign-out leaves nothing behind.

import { beforeEach, describe, expect, it } from "vitest";

import AsyncStorage from "./async-storage-stub";

import { asyncStore, clearUserStorage, setStorageOwner } from "@/data/storage";

describe("device storage — per-user keys", () => {
  beforeEach(() => {
    AsyncStorage.reset();
    setStorageOwner(null);
  });

  it("scopes a namespace to the signed-in user", async () => {
    setStorageOwner("u1");
    await asyncStore("cache").set("marula.rn.cache.tickets.my-work", "[]");
    expect(AsyncStorage.rawKeys()).toEqual([
      "@marula-core/u/u1/cache/marula.rn.cache.tickets.my-work",
    ]);
  });

  it("gives a second user no view of the first user's cache", async () => {
    setStorageOwner("u1");
    const cache = asyncStore("cache");
    const scans = asyncStore("scans");
    await cache.set("marula.rn.cache.me", '{"user_id":"u1"}');
    await scans.set("scan.saved", '[{"id":"s1"}]');

    // Same device, next sign-in.
    setStorageOwner("u2");
    expect(await asyncStore("cache").get("marula.rn.cache.me")).toBeNull();
    expect(await asyncStore("scans").get("scan.saved")).toBeNull();
    expect(await asyncStore("cache").keys()).toEqual([]);
    expect(await asyncStore("scans").keys()).toEqual([]);
  });

  it("keeps the same user's cache across a relaunch and a token refresh", async () => {
    setStorageOwner("u1");
    await asyncStore("cache").set("marula.rn.cache.me", '{"user_id":"u1"}');

    // Unbind (a process start resolves no session yet), then rebind the same id.
    setStorageOwner(null);
    setStorageOwner("u1");
    expect(await asyncStore("cache").get("marula.rn.cache.me")).toBe('{"user_id":"u1"}');
  });

  it("reads nothing and writes nothing while no user is bound", async () => {
    await asyncStore("cache").set("k", "v");
    expect(await asyncStore("cache").get("k")).toBeNull();
    expect(await asyncStore("cache").keys()).toEqual([]);
    expect(AsyncStorage.rawKeys()).toEqual([]);
  });
});

describe("device storage — sign-out teardown", () => {
  beforeEach(() => {
    AsyncStorage.reset();
    setStorageOwner(null);
  });

  it("deletes every namespace the engine and the screens own", async () => {
    setStorageOwner("u1");
    await asyncStore("cache").set("marula.rn.cache.me", "{}");
    await asyncStore("queue").set("marula.rn.queue.ops", "[]");
    await asyncStore("versions").set("marula.rn.versions", "{}");
    await asyncStore("scans").set("scan.saved", "[]");
    await asyncStore("live").set("activity.push.s1", "token");
    await asyncStore("prefs").set("live.activity", "0");
    expect(AsyncStorage.rawKeys()).toHaveLength(6);

    await clearUserStorage();
    expect(AsyncStorage.rawKeys()).toEqual([]);
  });

  it("deletes the unprefixed keys an older build wrote", async () => {
    AsyncStorage.seed("@marula-core/cache/marula.rn.cache.me", "{}");
    AsyncStorage.seed("@marula-core/scans/scan.saved", "[]");

    await clearUserStorage();
    expect(AsyncStorage.rawKeys()).toEqual([]);
  });

  it("unbinds the owner, so post-sign-out writes reach no key", async () => {
    setStorageOwner("u1");
    await clearUserStorage();
    expect(await asyncStore("cache").get("k")).toBeNull();
    await asyncStore("cache").set("k", "v");
    expect(AsyncStorage.rawKeys()).toEqual([]);
  });
});
