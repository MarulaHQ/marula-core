// React Native bindings for the core's storage seam.

import AsyncStorage from "@react-native-async-storage/async-storage";

import type { KeyValueStore } from "@/core/storage";

/** Namespaces keys so the cache and the queue never collide with other apps
 * on the device or with each other's prefixes. */
export function asyncStore(namespace: string): KeyValueStore {
  const prefix = `@meridian/${namespace}/`;
  return {
    async get(key) {
      return AsyncStorage.getItem(prefix + key);
    },
    async set(key, value) {
      await AsyncStorage.setItem(prefix + key, value);
    },
    async remove(key) {
      await AsyncStorage.removeItem(prefix + key);
    },
    async keys() {
      const all = await AsyncStorage.getAllKeys();
      return all
        .filter((key) => key.startsWith(prefix))
        .map((key) => key.slice(prefix.length));
    },
  };
}
