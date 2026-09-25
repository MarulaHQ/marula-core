// Locked screen — the biometric gate in front of a stored session.
//
// Purpose: when the operator turns on Face ID unlock, the refresh pair stays
// in the Keychain and is only handed to the client after a successful
// platform authentication. This screen is that gate; "Sign out instead"
// clears the stored session.

import { Redirect } from "expo-router";
import { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { useAuth } from "@/auth/auth-context";
import { BootScreen } from "@/ui/boot";
import { Button, Card, ErrorNote } from "@/ui/primitives";
import { type, useTheme } from "@/ui/tokens";

export default function LockedScreen() {
  const { status, unlock, signOut, biometricAvailable } = useAuth();
  const { tokens } = useTheme();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);

  if (status === "signedIn") return <Redirect href="/" />;
  if (status === "signedOut") return <Redirect href="/login" />;
  if (status === "mfa") return <Redirect href="/mfa" />;
  if (status === "loading") return <BootScreen />;

  return (
    <View style={[styles.screen, { backgroundColor: tokens.background }]}>
      <Text style={[styles.wordmark, { color: tokens.text }]}>Meridian</Text>
      <Card style={styles.card}>
        <Text style={[styles.body, { color: tokens.textSecondary }]}>
          {biometricAvailable
            ? "Unlock with Face ID to load your saved session."
            : "Biometric unlock isn't available on this device."}
        </Text>
        {failed ? <ErrorNote text="Unlock didn't complete — try again." /> : null}
        <Button
          label="Unlock"
          fullWidth
          testID="locked-unlock"
          loading={busy}
          disabled={!biometricAvailable || busy}
          onPress={() => {
            setBusy(true);
            setFailed(false);
            void unlock()
              .then((ok) => {
                if (!ok) setFailed(true);
              })
              .finally(() => setBusy(false));
          }}
        />
        <Button
          label="Sign out instead"
          variant="ghost"
          fullWidth
          testID="locked-signout"
          onPress={() => {
            void signOut();
          }}
        />
      </Card>
    </View>
  );
}

const styles = StyleSheet.create({
  screen: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    padding: 24,
    gap: 24,
  },
  wordmark: {
    fontFamily: type.sansSemiBold,
    fontSize: 30,
  },
  card: {
    alignSelf: "stretch",
  },
  body: {
    fontFamily: type.sans,
    fontSize: 14,
  },
});
