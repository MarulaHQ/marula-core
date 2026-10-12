// Unlock — the biometric gate on a restored session.
//
// Purpose: neutral, not red — nobody is at fault and there is nothing to
// approve. It says what is happening (Face ID is on for this phone), names
// the phone it is on, and gives two ways out: unlock with Face ID, or sign
// out. The unlock leg is real — it runs the same biometric check the setting
// behind it promises.
//
// Renamed from `/locked` (ADR 0021). The screen used to claim the pack's
// `09-locked` semantics — a phone awaiting service-desk approval, an admin
// already asked, a notification promised, a "Check again" that checked
// nothing. No device-approval concept exists in the API, the database or the
// domain, so those claims were removed rather than renamed. Device approval
// is a product want with its own lane; if it lands, it arrives as a new auth
// status, not as this screen's copy.
//
// The composition is inherited: grey lock in a soft circle, a neutral
// sentence, a facts table, one primary action.

import { Redirect } from "expo-router";
import { useState } from "react";
import { StyleSheet, Text, View } from "react-native";
import Ionicons from "@expo/vector-icons/Ionicons";
import * as Device from "expo-device";

import { useAuth } from "@/auth/auth-context";
import { BootScreen } from "@/ui/boot";
import { FactRow, Facts, LinkButton, PACK_ICONS, PrimaryButton } from "@/ui/pack";
import { type, useTheme } from "@/ui/tokens";

export default function UnlockScreen() {
  const { status, unlock, signOut } = useAuth();
  const { tokens } = useTheme();
  const [checking, setChecking] = useState(false);

  if (status === "signedIn") return <Redirect href="/(tabs)" />;
  if (status === "signedOut") return <Redirect href="/login" />;
  if (status === "mfa") return <Redirect href="/mfa" />;
  if (status === "loading") return <BootScreen />;

  return (
    <View
      style={[styles.screen, { backgroundColor: tokens.background }]}
      testID="unlock-screen"
    >
      <View style={[styles.lockDisc, { backgroundColor: tokens.surfaceHover }]}>
        <Ionicons name={PACK_ICONS.lock} size={40} color={tokens.textSecondary} />
      </View>
      <Text style={[styles.title, { color: tokens.text }]}>Unlock Marula</Text>
      <Text style={[styles.body, { color: tokens.textSecondary }]}>
        You turned on Face ID for this phone. Unlock to pick up where you left off — your session
        is still signed in.
      </Text>
      <Facts style={styles.facts}>
        <FactRow first label="Phone">
          {Device.modelName ?? "This phone"}
        </FactRow>
        <FactRow label="Unlock with">Face ID</FactRow>
      </Facts>
      <PrimaryButton
        label="Unlock with Face ID"
        testID="unlock-check"
        disabled={checking}
        busy={checking}
        onPress={() => {
          setChecking(true);
          unlock().finally(() => setChecking(false));
        }}
      />
      <LinkButton
        label="Sign out"
        testID="unlock-signout"
        onPress={() => {
          void signOut();
        }}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  screen: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    gap: 12,
    paddingHorizontal: 28,
  },
  lockDisc: {
    width: 88,
    height: 88,
    borderRadius: 999,
    alignItems: "center",
    justifyContent: "center",
  },
  title: {
    fontFamily: type.sansSemiBold,
    fontSize: 24,
    textAlign: "center",
    marginTop: 4,
  },
  body: {
    fontFamily: type.sans,
    fontSize: 14,
    lineHeight: 22,
    textAlign: "center",
  },
  facts: {
    alignSelf: "stretch",
    marginTop: 8,
  },
});
