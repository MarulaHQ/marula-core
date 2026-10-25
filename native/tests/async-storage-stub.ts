// Node-suite stand-in for @react-native-async-storage/async-storage.
//
// The device-storage tests exercise the real prefixing and sweep logic of
// src/data/storage.ts against an in-memory map, so no native module is needed.
// `reset()` gives each test a clean device.

const map = new Map<string, string>();

const AsyncStorage = {
  async getItem(key: string): Promise<string | null> {
    return map.get(key) ?? null;
  },
  async setItem(key: string, value: string): Promise<void> {
    map.set(key, value);
  },
  async removeItem(key: string): Promise<void> {
    map.delete(key);
  },
  async multiRemove(keys: readonly string[]): Promise<void> {
    for (const key of keys) map.delete(key);
  },
  async getAllKeys(): Promise<string[]> {
    return [...map.keys()];
  },
  /** Test seam: empty the device. */
  reset(): void {
    map.clear();
  },
  /** Test seam: seed a raw key an older build would have written. */
  seed(key: string, value: string): void {
    map.set(key, value);
  },
  /** Test seam: the raw keys on the device, for asserting exact prefixes. */
  rawKeys(): string[] {
    return [...map.keys()].sort();
  },
};

export default AsyncStorage;
