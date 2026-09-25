// Login — email + password, with the MFA bridge handoff.
//
// Purpose: the exact web semantics (typed IF3 failure copy, MFA-gated login
// returns a bridge token, tokens only after the second factor) on a
// touch-first screen. Biometric unlock appears when a session is stored and
// the operator enabled it.

import { Redirect } from "expo-router";
import { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { useAuth } from "@/auth/auth-context";
import { Button, Card, ErrorNote, Field, Screen } from "@/ui/primitives";
import { shape, type, useTheme } from "@/ui/tokens";

export default function LoginScreen() {
  const { status, signIn, error, biometricEnabled, biometricAvailable, unlock } = useAuth();
  const { tokens } = useTheme();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);

  if (status === "signedIn") return <Redirect href="/" />;
  if (status === "mfa") return <Redirect href="/mfa" />;
  if (status === "locked") return <Redirect href="/locked" />;

  const canSubmit = email.trim().length > 0 && password.length > 0 && !busy;

  return (
    <Screen>
      <View style={styles.brand}>
        <Text style={[styles.wordmark, { color: tokens.text }]}>Meridian</Text>
        <Text style={[styles.tagline, { color: tokens.textMuted }]}>
          Field service, on your phone.
        </Text>
      </View>
      <Card>
        <Text style={[styles.cardTitle, { color: tokens.text }]}>Sign in</Text>
        <Field
          label="Work email"
          testID="login-email"
          value={email}
          onChangeText={setEmail}
          autoCapitalize="none"
          autoCorrect={false}
          keyboardType="email-address"
          textContentType="username"
          placeholder="you@company.com"
        />
        <Field
          label="Password"
          testID="login-password"
          value={password}
          onChangeText={setPassword}
          secureTextEntry
          textContentType="password"
          placeholder="••••••••"
        />
        {error !== null ? <ErrorNote text={error} /> : null}
        <Button
          label="Sign in"
          testID="login-submit"
          fullWidth
          loading={busy}
          disabled={!canSubmit}
          onPress={() => {
            setBusy(true);
            void signIn(email.trim(), password).finally(() => setBusy(false));
          }}
        />
      </Card>
      {biometricEnabled && biometricAvailable ? (
        <Button
          label="Unlock with Face ID"
          variant="secondary"
          fullWidth
          testID="login-biometric"
          onPress={() => {
            void unlock();
          }}
        />
      ) : null}
    </Screen>
  );
}

const styles = StyleSheet.create({
  brand: {
    paddingTop: 48,
    paddingBottom: 8,
    gap: 4,
  },
  wordmark: {
    fontFamily: type.sansSemiBold,
    fontSize: 32,
    letterSpacing: -0.6,
  },
  tagline: {
    fontFamily: type.sans,
    fontSize: 14,
  },
  cardTitle: {
    fontFamily: type.sansSemiBold,
    fontSize: 18,
    marginBottom: shape.cardPadding / 4,
  },
});
