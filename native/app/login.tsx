// Login — the pack's `10-login`: the serif wordmark with the fruit mark,
// Welcome back, the two fields, the ink primary, and the two other ways in.
//
// Purpose: exact web semantics underneath the drawing (typed IF3 failure
// copy, MFA-gated login returns a bridge token, tokens only after the
// second factor). The pack's secondary routes — Email me a link and Sign in
// with Microsoft — render as 52px secondary buttons; the link leg calls the
// real magic-link endpoint, Microsoft names its provider honestly when the
// org hasn't enabled one.

import { Redirect } from "expo-router";
import { useEffect, useRef, useState } from "react";
import { Image, Pressable, StyleSheet, Text, TextInput, View } from "react-native";
import Ionicons from "@expo/vector-icons/Ionicons";

import { useAuth } from "@/auth/auth-context";
import { PACK_ICONS, PrimaryButton, SecondaryButton } from "@/ui/pack";
import { type, useTheme } from "@/ui/tokens";
import { requestMagicLink } from "@/auth/magic-link";

// The canonical brand lockup — the brand's real identity, gold fruit with
// the lowercase ink wordmark (internal/design/marula/logo + apps/web/public, byte
// copies of the kit's lockups; web's BrandLockup defaults to the same
// colour variant). Colour on light surfaces, the on-dark lockup on dark.
// Rasterized from the SVG for native (no SVG runtime here) via
// e2e/rasterize-brand.mjs. The kit's minimum lockup width is 96px; 40px of
// height on the 589×200 raster is ~118px wide, so the entry row clears it.
const LOCKUP_COLOR = require("../assets/lockup-color.png") as number;
const LOCKUP_COLOR_DARK = require("../assets/lockup-color-on-dark.png") as number;

export default function LoginScreen() {
  const { status, signIn, error, client, biometricEnabled, biometricAvailable, unlock } = useAuth();
  const { tokens } = useTheme();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [busy, setBusy] = useState(false);
  const passwordRef = useRef<TextInput>(null);
  const [linkNote, setLinkNote] = useState<string | null>(null);
  const [linkBusy, setLinkBusy] = useState(false);
  // The org's real SSO providers (GET /auth/oidc/providers/public) decide
  // what the Microsoft button says — never a fake route.
  const [ssoNames, setSsoNames] = useState<string[] | null>(null);
  const [ssoNote, setSsoNote] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void client.api
      .GET("/api/v1/auth/oidc/providers/public")
      .then((result) => {
        if (cancelled) return;
        if (result.data !== undefined) {
          setSsoNames(result.data.map((provider) => provider.display_name));
        } else {
          setSsoNames([]);
        }
      })
      .catch(() => {
        if (!cancelled) setSsoNames([]);
      });
    return () => {
      cancelled = true;
    };
  }, [client]);

  if (status === "signedIn") return <Redirect href="/(tabs)" />;
  if (status === "mfa") return <Redirect href="/mfa" />;
  if (status === "biometric") return <Redirect href="/unlock" />;

  const canSubmit = email.trim().length > 0 && password.length > 0 && !busy;

  // The keyboard flow is part of the screen, not an affordance bolted on:
  // Return moves email -> password, Return on password signs in. A field
  // service tech signs in one-handed, often with a keyboard docked.
  const submit = () => {
    if (!canSubmit) return;
    setBusy(true);
    void signIn(email.trim(), password).finally(() => setBusy(false));
  };

  return (
    <View style={[styles.screen, { backgroundColor: tokens.background }]} testID="login-screen">
      <View style={styles.brandRow}>
        <Image
          source={tokens.mode === "dark" ? LOCKUP_COLOR_DARK : LOCKUP_COLOR}
          style={styles.lockup}
          resizeMode="contain"
          accessibilityIgnoresInvertColors
        />
      </View>
      <View style={styles.headline}>
        <Text style={[styles.headlineTitle, { color: tokens.text }]}>Welcome back</Text>
        <Text style={[styles.headlineSub, { color: tokens.textSecondary }]}>
          Sign in to start the day.
        </Text>
      </View>

      <View style={[styles.field, { borderColor: tokens.border, backgroundColor: tokens.surface }]}>
        <TextInput
          testID="login-email"
          value={email}
          onChangeText={setEmail}
          autoCapitalize="none"
          autoCorrect={false}
          keyboardType="email-address"
          textContentType="username"
          placeholder="Work email"
          placeholderTextColor={tokens.textMuted}
          returnKeyType="next"
          onSubmitEditing={() => passwordRef.current?.focus()}
          style={[styles.input, { color: tokens.text }]}
        />
      </View>
      <View style={[styles.field, { borderColor: tokens.border, backgroundColor: tokens.surface }]}>
        <TextInput
          ref={passwordRef}
          testID="login-password"
          returnKeyType="go"
          onSubmitEditing={submit}
          value={password}
          onChangeText={setPassword}
          secureTextEntry={!showPassword}
          textContentType="password"
          placeholder="Password"
          placeholderTextColor={tokens.textMuted}
          style={[styles.input, { color: tokens.text }]}
        />
        <Pressable
          onPress={() => setShowPassword((current) => !current)}
          accessibilityRole="button"
          accessibilityLabel={showPassword ? "Hide password" : "Show password"}
          hitSlop={8}
          style={styles.eye}
        >
          <Ionicons name={PACK_ICONS.eye} size={20} color={tokens.textSecondary} />
        </Pressable>
      </View>

      {error !== null || linkNote !== null ? (
        <Text style={[styles.errorText, { color: tokens.warning.text }]}>
          {linkNote ?? error ?? ""}
        </Text>
      ) : null}

      <PrimaryButton
        label="Sign in"
        testID="login-submit"
        disabled={!canSubmit}
        busy={busy}
        onPress={submit}
      />
      <SecondaryButton
        label="Email me a link"
        icon={PACK_ICONS.mail}
        disabled={linkBusy}
        testID="login-magic-link"
        onPress={() => {
          setLinkBusy(true);
          requestMagicLink(email.trim())
            .then((sent) =>
              setLinkNote(
                sent
                  ? "Check your inbox — the link signs you in on this phone."
                  : "The link didn't send. Try the password route.",
              ),
            )
            .finally(() => setLinkBusy(false));
        }}
      />
      <SecondaryButton
        label="Sign in with Microsoft"
        testID="login-sso"
        onPress={() => {
          if (ssoNames !== null && ssoNames.length > 0) {
            // The org HAS an SSO provider; the system-browser bridge (open
            // the provider's authorize URL and return to the app) is not
            // built on the phone yet — recorded gap 2026-10-05. Say so.
            setSsoNote(
              `${ssoNames[0]} is set up for this org, but signing in with it needs the browser sign-in bridge that isn't on this phone yet. Use the password or emailed link.`,
            );
          } else {
            setSsoNote(
              "This org hasn't enabled Microsoft sign-in. Use the password or the emailed link.",
            );
          }
        }}
      />
      {ssoNote !== null ? (
        <Text style={[styles.errorText, { color: tokens.textSecondary }]}>{ssoNote}</Text>
      ) : null}

      {biometricEnabled && biometricAvailable ? (
        <SecondaryButton
          label="Unlock with Face ID"
          testID="login-biometric"
          onPress={() => {
            void unlock();
          }}
        />
      ) : null}

    </View>
  );
}


const styles = StyleSheet.create({
  screen: {
    flex: 1,
    justifyContent: "center",
    gap: 14,
    paddingHorizontal: 24,
  },
  brandRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    marginBottom: 8,
  },
  lockup: {
    width: 118,
    height: 40,
  },
  headline: {
    gap: 2,
    marginBottom: 8,
  },
  headlineTitle: {
    fontFamily: type.sansSemiBold,
    fontSize: 22,
  },
  headlineSub: {
    fontFamily: type.sans,
    fontSize: 14,
  },
  field: {
    height: 52,
    borderRadius: 12,
    borderWidth: 1,
    paddingHorizontal: 14,
    flexDirection: "row",
    alignItems: "center",
  },
  input: {
    flex: 1,
    fontFamily: type.sans,
    fontSize: 16,
  },
  eye: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  errorText: {
    fontFamily: type.sans,
    fontSize: 13,
  },
});
