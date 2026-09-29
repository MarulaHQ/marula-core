/* eslint-disable react-hooks/refs -- the MarulaClient is created once and
   held in a ref (the documented lazy-init idiom) so every callback and the
   provider value share one stable instance; the compiler's refs rule cannot
   see through the once-only write. */
// Auth provider — login, MFA (TOTP entry), refresh rotation, biometric
// unlock, and the single MarulaClient every layer shares.
//
// Purpose: mirror the web auth semantics exactly (typed IF3 failures,
// MFA-gated login via the bridge token, rotating refresh cookies replaced by
// the SecureStore-held refresh pair) while adding the native-only legs:
// platform biometric unlock and a proactive refresh so the realtime socket
// always reconnects with a live access token.
//
// Non-purpose: no routing (expo-router redirects off `status`) and no cache
// ownership (the sync provider clears caches on session end).

import {
  MarulaApiError,
  MarulaClient,
  type LoginResult,
  type MfaRequired,
} from "@marula-core/sdk";
import * as LocalAuthentication from "expo-local-authentication";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { AppState } from "react-native";

import { friendlyMutationError } from "@/core/errors";
import { fetchMe, type Me } from "@/core/models";
import { API_URL } from "@/config";
import { fetchWithAuthRefresh } from "@/data/http";
import { biometricPreference, sessionStore, tokenExpiresAt } from "@/data/secure-store";

export type AuthStatus = "loading" | "locked" | "signedOut" | "mfa" | "signedIn";

export interface AuthContextValue {
  status: AuthStatus;
  me: Me | null;
  mfaMethods: string[];
  biometricAvailable: boolean;
  biometricEnabled: boolean;
  error: string | null;
  client: MarulaClient;
  signIn(email: string, password: string): Promise<void>;
  submitMfa(code: string): Promise<void>;
  cancelMfa(): void;
  signOut(): Promise<void>;
  unlock(): Promise<boolean>;
  setBiometricEnabled(enabled: boolean): Promise<boolean>;
  ensureToken(): Promise<void>;
  accessToken(): string | null;
}

const AuthContext = createContext<AuthContextValue | null>(null);

const REFRESH_MARGIN_SECONDS = 120;
const PROACTIVE_REFRESH_MS = 10 * 60 * 1000;

export function AuthProvider({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<AuthStatus>("loading");
  const [me, setMe] = useState<Me | null>(null);
  const [mfaMethods, setMfaMethods] = useState<string[]>([]);
  const [biometricAvailable, setBiometricAvailable] = useState(false);
  const [biometricEnabled, setBiometricEnabledState] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const mfaTokenRef = useRef<string | null>(null);
  const sessionRef = useRef<{ refreshToken: string; sessionId: string } | null>(null);
  const clientRef = useRef<MarulaClient | null>(null);
  const session = sessionStore;

  const refreshTokens = useCallback(async (): Promise<string | null> => {
    const client = clientRef.current;
    if (client === null || client.tokens.sessionId === null) return null;
    try {
      const tokens = await client.refresh();
      sessionRef.current = {
        refreshToken: tokens.refresh_token,
        sessionId: tokens.session_id,
      };
      await session.save({
        accessToken: tokens.access_token,
        refreshToken: tokens.refresh_token,
        sessionId: tokens.session_id,
      });
      return tokens.access_token;
    } catch {
      return null;
    }
  }, [session]);

  if (clientRef.current === null) {
    clientRef.current = new MarulaClient({
      baseUrl: API_URL,
      fetch: fetchWithAuthRefresh(globalThis.fetch.bind(globalThis), refreshTokens),
    });
  }
  const client = clientRef.current;

  const applySession = useCallback(
    async (tokens: { accessToken: string; refreshToken: string; sessionId: string }) => {
      sessionRef.current = { refreshToken: tokens.refreshToken, sessionId: tokens.sessionId };
      client.setTokens({
        access_token: tokens.accessToken,
        refresh_token: tokens.refreshToken,
        session_id: tokens.sessionId,
        expires_in: 0,
      });
      const principal = await fetchMe(client);
      setMe(principal);
      setStatus("signedIn");
      setError(null);
    },
    [client],
  );

  const persistAndContinue = useCallback(
    async (tokens: { access_token: string; refresh_token: string; session_id: string }) => {
      await session.save({
        accessToken: tokens.access_token,
        refreshToken: tokens.refresh_token,
        sessionId: tokens.session_id,
      });
      await applySession({
        accessToken: tokens.access_token,
        refreshToken: tokens.refresh_token,
        sessionId: tokens.session_id,
      });
    },
    [applySession, session],
  );

  const signOut = useCallback(async () => {
    try {
      await client.logout();
    } catch {
      // A revoked or already-dead session still signs out locally.
    }
    sessionRef.current = null;
    mfaTokenRef.current = null;
    await session.clear();
    setMe(null);
    setMfaMethods([]);
    setStatus("signedOut");
    setError(null);
  }, [client, session]);

  // Boot: restore the session, honoring the biometric gate.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const [hardware, enrolled, enabled, stored] = await Promise.all([
        LocalAuthentication.hasHardwareAsync().catch(() => false),
        LocalAuthentication.isEnrolledAsync().catch(() => false),
        biometricPreference.isEnabled(),
        session.load(),
      ]);
      if (cancelled) return;
      setBiometricAvailable(hardware && enrolled);
      setBiometricEnabledState(enabled);
      if (stored === null) {
        setStatus("signedOut");
        return;
      }
      sessionRef.current = { refreshToken: stored.refreshToken, sessionId: stored.sessionId };
      if (enabled && hardware && enrolled) {
        setStatus("locked");
        return;
      }
      try {
        await applySession(stored);
      } catch {
        sessionRef.current = null;
        await session.clear();
        if (!cancelled) setStatus("signedOut");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [applySession, session]);

  const signIn = useCallback(
    async (email: string, password: string) => {
      setError(null);
      try {
        const result: LoginResult = await client.login({ email, password });
        if (isMfaRequired(result)) {
          mfaTokenRef.current = result.mfa_token;
          setMfaMethods(result.methods);
          setStatus("mfa");
          return;
        }
        await persistAndContinue(result);
      } catch (cause) {
        setError(friendlyMutationError(cause).text);
        throw cause;
      }
    },
    [client, persistAndContinue],
  );

  const submitMfa = useCallback(
    async (code: string) => {
      const mfaToken = mfaTokenRef.current;
      if (mfaToken === null) {
        setError("That sign-in attempt expired — start again.");
        setStatus("signedOut");
        return;
      }
      setError(null);
      try {
        const tokens = await client.mfaVerify({ mfa_token: mfaToken, code });
        mfaTokenRef.current = null;
        await persistAndContinue(tokens);
      } catch (cause) {
        setError(friendlyMutationError(cause).text);
        throw cause;
      }
    },
    [client, persistAndContinue],
  );

  const cancelMfa = useCallback(() => {
    mfaTokenRef.current = null;
    setMfaMethods([]);
    setError(null);
    setStatus("signedOut");
  }, []);

  const unlock = useCallback(async (): Promise<boolean> => {
    const result = await LocalAuthentication.authenticateAsync({
      promptMessage: "Unlock Marula",
      cancelLabel: "Cancel",
      disableDeviceFallback: false,
    }).catch(() => ({ success: false }));
    if (!result.success) return false;
    const stored = await session.load();
    if (stored === null) {
      setStatus("signedOut");
      return false;
    }
    try {
      await applySession(stored);
      return true;
    } catch {
      await session.clear();
      setStatus("signedOut");
      return false;
    }
  }, [applySession, session]);

  const setBiometricEnabled = useCallback(
    async (enabled: boolean): Promise<boolean> => {
      if (enabled) {
        const result = await LocalAuthentication.authenticateAsync({
          promptMessage: "Turn on Face ID unlock",
          cancelLabel: "Cancel",
          disableDeviceFallback: false,
        }).catch(() => ({ success: false }));
        if (!result.success) return false;
      }
      await biometricPreference.setEnabled(enabled);
      setBiometricEnabledState(enabled);
      return true;
    },
    [],
  );

  const ensureToken = useCallback(async () => {
    const accessToken = client.tokens.accessToken;
    if (accessToken === null || accessToken === "") {
      await refreshTokens();
      return;
    }
    const expiresAt = tokenExpiresAt(accessToken);
    const nowSeconds = Date.now() / 1000;
    if (expiresAt !== null && expiresAt - nowSeconds <= REFRESH_MARGIN_SECONDS) {
      await refreshTokens();
    }
  }, [client, refreshTokens]);

  const accessToken = useCallback(() => client.tokens.accessToken, [client]);

  // Proactive rotation while signed in, and on foreground.
  useEffect(() => {
    if (status !== "signedIn") return;
    const interval = setInterval(() => {
      void ensureToken();
    }, PROACTIVE_REFRESH_MS);
    const subscription = AppState.addEventListener("change", (next) => {
      if (next === "active") void ensureToken();
    });
    return () => {
      clearInterval(interval);
      subscription.remove();
    };
  }, [ensureToken, status]);

  const value = useMemo<AuthContextValue>(
    () => ({
      status,
      me,
      mfaMethods,
      biometricAvailable,
      biometricEnabled,
      error,
      client,
      signIn,
      submitMfa,
      cancelMfa,
      signOut,
      unlock,
      setBiometricEnabled,
      ensureToken,
      accessToken,
    }),
    [
      status,
      me,
      mfaMethods,
      biometricAvailable,
      biometricEnabled,
      error,
      client,
      signIn,
      submitMfa,
      cancelMfa,
      signOut,
      unlock,
      setBiometricEnabled,
      ensureToken,
      accessToken,
    ],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthContextValue {
  const value = useContext(AuthContext);
  if (value === null) throw new Error("useAuth must be used inside AuthProvider");
  return value;
}

function isMfaRequired(result: LoginResult): result is MfaRequired {
  return (result as { mfa_required?: unknown }).mfa_required === true;
}

export { MarulaApiError };
