// Cache invariants: durable across relaunch, stale-marking keeps data,
// optimistic patches reconcile, logout clears everything.

import { describe, expect, it } from "vitest";

import { ReadCache } from "@/core/cache";
import { MemoryStore } from "@/core/storage";

describe("ReadCache", () => {
  it("persists entries and hydrates them in a fresh instance", async () => {
    const store = new MemoryStore();
    const first = new ReadCache(store);
    await first.set("tickets.my-work", [{ id: "t1" }]);
    const second = new ReadCache(store);
    await second.hydrate();
    expect(second.get("tickets.my-work")?.data).toEqual([{ id: "t1" }]);
    expect(second.get("tickets.my-work")?.source).toBe("server");
  });

  it("marks stale without dropping the data (offline keeps rendering)", async () => {
    const cache = new ReadCache(new MemoryStore());
    await cache.set("ticket:t1", { id: "t1", title: "old" });
    await cache.markStale("ticket:t1");
    expect(cache.get("ticket:t1")?.stale).toBe(true);
    expect(cache.get("ticket:t1")?.data).toEqual({ id: "t1", title: "old" });
  });

  it("applies an optimistic patch and reconciles from the server snapshot", async () => {
    const cache = new ReadCache(new MemoryStore());
    await cache.set("ticket:t1:comments", [{ id: "c1", body: "first" }]);
    await cache.patch<{ id: string; body: string; pending?: boolean }[]>(
      "ticket:t1:comments",
      (current) => [...(current ?? []), { id: "local-1", body: "second", pending: true }],
    );
    expect(cache.get("ticket:t1:comments")?.data).toHaveLength(2);
    expect(cache.get("ticket:t1:comments")?.source).toBe("optimistic");
    await cache.set("ticket:t1:comments", [
      { id: "c1", body: "first" },
      { id: "c2", body: "second" },
    ]);
    expect(cache.get("ticket:t1:comments")?.data).toEqual([
      { id: "c1", body: "first" },
      { id: "c2", body: "second" },
    ]);
    expect(cache.get("ticket:t1:comments")?.source).toBe("server");
  });

  it("notifies subscribers per key and per clear", async () => {
    const cache = new ReadCache(new MemoryStore());
    const seen: string[] = [];
    const unsubscribe = cache.subscribe((key) => seen.push(key));
    await cache.set("a", 1);
    await cache.set("b", 2);
    await cache.clear();
    unsubscribe();
    await cache.set("c", 3);
    expect(seen).toEqual(["a", "b", "*"]);
  });

  it("clears durable entries on logout", async () => {
    const store = new MemoryStore();
    const cache = new ReadCache(store);
    await cache.set("me", { user_id: "u1" });
    await cache.clear();
    const rehydrated = new ReadCache(store);
    await rehydrated.hydrate();
    expect(rehydrated.keys()).toEqual([]);
  });
});
