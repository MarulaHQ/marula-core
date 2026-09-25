// MFA challenge — TOTP (or recovery code) entry on the bridge token.
//
// Purpose: the second-factor step from POST /auth/login's mfa_required
// response. The code is a six-digit TOTP by default; recovery codes are
// accepted in the same field because the server validates either. Nothing
// is stored client-side until mfaVerify returns the rotating pair.

import { Redirect, router } from "expo-router";
import { useState } from "react";
import { StyleSheet, Text } from "react-native";

import { useAuth } from "@/auth/auth-context";
import { Button, Card, ErrorNote, Field, Screen } from "@/ui/primitives";
import { type, useTheme } from "@/ui/tokens";

export default function MfaScreen() {
  const { status, mfaMethods, error, submitMfa, cancelMfa } = useAuth();
  const { tokens } = useTheme();
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);

  if (status === "signedIn") return <Redirect href="/" />;
  if (status !== "mfa") return <Redirect href="/login" />;

  const methodLabel = mfaMethods.includes("totp")
    ? "Enter the 6-digit code from your authenticator app."
    : "Enter your second-factor code.";

  return (
    <Screen>
      <Card>
        <Text style={[styles.title, { color: tokens.text }]}>Two-factor check</Text>
        <Text style={[styles.body, { color: tokens.textMuted }]}>{methodLabel}</Text>
        <Field
          label="Code"
          testID="mfa-code"
          value={code}
          onChangeText={setCode}
          keyboardType="number-pad"
          autoComplete="one-time-code"
          textContentType="oneTimeCode"
          autoFocus
          maxLength={12}
          placeholder="123456"
        />
        {error !== null ? <ErrorNote text={error} /> : null}
        <Button
          label="Verify"
          testID="mfa-submit"
          fullWidth
          loading={busy}
          disabled={code.trim().length < 6 || busy}
          onPress={() => {
            setBusy(true);
            void submitMfa(code.trim()).finally(() => setBusy(false));
          }}
        />
        <Button
          label="Use a different account"
          variant="ghost"
          fullWidth
          testID="mfa-cancel"
          onPress={() => {
            cancelMfa();
            router.replace("/login");
          }}
        />
      </Card>
    </Screen>
  );
}

const styles = StyleSheet.create({
  title: {
    fontFamily: type.sansSemiBold,
    fontSize: 18,
  },
  body: {
    fontFamily: type.sans,
    fontSize: 14,
  },
});
