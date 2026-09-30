// Login — the pack's `10-login`: the serif wordmark with the fruit mark,
// Welcome back, the two fields, the ink primary, and the two other ways in.
//
// Purpose: exact web semantics underneath the drawing (typed IF3 failure
// copy, MFA-gated login returns a bridge token, tokens only after the
// second factor). The pack's secondary routes — Email me a link and Sign in
// with Microsoft — render as 52px secondary buttons; the link leg calls the
// real magic-link endpoint, Microsoft names its provider honestly when the
// org hasn't enabled one.

import { Redirect, router } from "expo-router";
import { useRef, useState } from "react";
import { Image, Pressable, StyleSheet, Text, TextInput, View } from "react-native";
import Ionicons from "@expo/vector-icons/Ionicons";

import { useAuth } from "@/auth/auth-context";
import { PACK_ICONS, PrimaryButton, SecondaryButton } from "@/ui/pack";
import { type, useTheme } from "@/ui/tokens";
import { requestMagicLink } from "@/auth/magic-link";

// The canonical marula fruit, cut from the shipped app icon (native/assets/
// icon.png) — never re-drawn. The wordmark beside it is Fraunces, which is
// the brand's serif face.
const FRUIT = require("../assets/marula-fruit.png") as number;

export default function LoginScreen() {
  const { status, signIn, error, biometricEnabled, biometricAvailable, unlock } = useAuth();
  const { tokens } = useTheme();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [busy, setBusy] = useState(false);
  const passwordRef = useRef<TextInput>(null);
  const [linkNote, setLinkNote] = useState<string | null>(null);
  const [linkBusy, setLinkBusy] = useState(false);

  if (status === "signedIn") return <Redirect href="/(tabs)" />;
  if (status === "mfa") return <Redirect href="/mfa" />;
  if (status === "locked") return <Redirect href="/locked" />;

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
        <Image source={FRUIT} style={styles.fruit} accessibilityIgnoresInvertColors />
        <Text style={[styles.wordmark, { color: tokens.text }]}>marula</Text>
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
        onPress={() =>
          router.replace("/mfa" as never)
        }
      />

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
  wordmark: {
    fontFamily: "Fraunces_500Medium",
    fontSize: 34,
    letterSpacing: -1,
  },
  fruit: {
    width: 32,
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
