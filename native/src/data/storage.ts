// React Native bindings for the core's storage seam.
//
// Purpose: namespace durable device keys, scope every key to the signed-in
// user, and provide the one teardown sign-out awaits. A key written for one
// account resolves to a prefix only that account can read, so cached reads,
// queued writes, and the saved-scan shelf never hydrate for the next user of
// the device. Namespaces stay as they were — the owner is a segment in front
// of them.
//
// Non-purpose: no serialization (the core owns JSON) and no credential
// storage (src/data/secure-store.ts owns the session pair).

import AsyncStorage from "@react-native-async-storage/async-storage";

import type { KeyValueStore } from "@/core/storage";

/** Root every key this app writes shares; the sign-out sweep is rooted here. */
const ROOT = "@marula-core";

/** The signed-in user who owns durable device state, or null before a session
 * is established. `asyncStore` resolves the prefix per call rather than at
 * construction, because several stores are built once at module load (the scan
 * shelf, the live-timer preference) and still have to follow the session. */
let owner: string | null = null;

/** Binds durable device state to the signed-in user. The sync provider calls
 * this as a session starts; `clearUserStorage` unbinds it. */
export function setStorageOwner(userId: string | null): void {
  owner = userId;
}

function prefixFor(namespace: string): string | null {
  if (owner === null) return null;
  return `${ROOT}/u/${owner}/${namespace}/`;
}

/** Namespaces keys so the cache and the queue never collide with other apps
 * on the device or with each other's prefixes — and so every key carries the
 * user it belongs to. Without a signed-in owner there is nothing to read and
 * nothing worth writing: reads answer empty, writes are dropped. */
export function asyncStore(namespace: string): KeyValueStore {
  return {
    async get(key) {
      const prefix = prefixFor(namespace);
      if (prefix === null) return null;
      return AsyncStorage.getItem(prefix + key);
    },
    async set(key, value) {
      const prefix = prefixFor(namespace);
      if (prefix === null) return;
      await AsyncStorage.setItem(prefix + key, value);
    },
    async remove(key) {
      const prefix = prefixFor(namespace);
      if (prefix === null) return;
      await AsyncStorage.removeItem(prefix + key);
    },
    async keys() {
      const prefix = prefixFor(namespace);
      if (prefix === null) return [];
      const all = await AsyncStorage.getAllKeys();
      return all
        .filter((key) => key.startsWith(prefix))
        .map((key) => key.slice(prefix.length));
    },
  };
}

/**
 * Sign-out teardown: unbind the owner, then delete every durable key this app
 * wrote — every user's namespace tree and the unprefixed `@marula-core/<ns>/`
 * family an older build left behind. The sync provider awaits this before a
 * new engine may hydrate, so a second account cannot read the first one's
 * cache even when an earlier sign-out was cut short on this device.
 */
export async function clearUserStorage(): Promise<void> {
  owner = null;
  const all = await AsyncStorage.getAllKeys();
  const stale = all.filter((key) => key.startsWith(`${ROOT}/`));
  if (stale.length > 0) await AsyncStorage.multiRemove(stale);
}
