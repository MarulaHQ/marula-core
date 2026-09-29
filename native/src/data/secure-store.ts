// Token persistence and the biometric-unlock preference.
//
// Purpose: the refresh token pair lives in the platform secure store
// (Keychain on iOS, Keystore on Android), never in the offline cache. On
// web (the fast-iteration surface only) it falls back to localStorage with
// the honest caveat that web is not a secure-token target.
//
// Non-purpose: no crypto, no token parsing beyond expiry.

import * as SecureStore from "expo-secure-store";
import { Platform } from "react-native";

export interface StoredSession {
  accessToken: string;
  refreshToken: string;
  sessionId: string;
}

const SESSION_KEY = "marula.session.v1";
const BIOMETRIC_KEY = "marula.biometric-unlock.v1";

const secureAvailable = Platform.OS === "ios" || Platform.OS === "android";

async function readRaw(key: string): Promise<string | null> {
  if (secureAvailable) return SecureStore.getItemAsync(key);
  if (Platform.OS === "web" && typeof localStorage !== "undefined") {
    return localStorage.getItem(key);
  }
  return null;
}

async function writeRaw(key: string, value: string): Promise<void> {
  if (secureAvailable) {
    await SecureStore.setItemAsync(key, value);
    return;
  }
  if (Platform.OS === "web" && typeof localStorage !== "undefined") {
    localStorage.setItem(key, value);
  }
}

async function removeRaw(key: string): Promise<void> {
  if (secureAvailable) {
    await SecureStore.deleteItemAsync(key);
    return;
  }
  if (Platform.OS === "web" && typeof localStorage !== "undefined") {
    localStorage.removeItem(key);
  }
}

export const sessionStore = {
  async load(): Promise<StoredSession | null> {
    const raw = await readRaw(SESSION_KEY);
    if (raw === null) return null;
    try {
      const parsed = JSON.parse(raw) as StoredSession;
      if (
        typeof parsed.accessToken !== "string" ||
        typeof parsed.refreshToken !== "string" ||
        typeof parsed.sessionId !== "string" ||
        parsed.refreshToken === ""
      ) {
        return null;
      }
      return parsed;
    } catch {
      await removeRaw(SESSION_KEY);
      return null;
    }
  },

  async save(session: StoredSession): Promise<void> {
    await writeRaw(SESSION_KEY, JSON.stringify(session));
  },

  async clear(): Promise<void> {
    await removeRaw(SESSION_KEY);
  },
};

export const biometricPreference = {
  async isEnabled(): Promise<boolean> {
    return (await readRaw(BIOMETRIC_KEY)) === "1";
  },
  async setEnabled(enabled: boolean): Promise<void> {
    if (enabled) {
      await writeRaw(BIOMETRIC_KEY, "1");
    } else {
      await removeRaw(BIOMETRIC_KEY);
    }
  },
};

/** Access-token expiry (seconds since epoch), read from the JWT payload.
 * Returns null when the token is not a decodable JWT. */
export function tokenExpiresAt(accessToken: string): number | null {
  const parts = accessToken.split(".");
  if (parts.length !== 3) return null;
  try {
    const payload = JSON.parse(
      base64UrlDecode(parts[1] as string),
    ) as { exp?: unknown };
    return typeof payload.exp === "number" ? payload.exp : null;
  } catch {
    return null;
  }
}

function base64UrlDecode(input: string): string {
  const padded = input.replace(/-/g, "+").replace(/_/g, "/");
  const withPadding = padded + "=".repeat((4 - (padded.length % 4)) % 4);
  if (typeof atob === "function") return atob(withPadding);
  return Buffer.from(withPadding, "base64").toString("binary");
}
