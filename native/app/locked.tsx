// Locked — the pack's `09-locked`: the device the service desk has not
// approved yet.
//
// Purpose: NEUTRAL, not red — nobody is at fault. It says what is happening,
// what was asked and of whom (the facts table: Phone, Asked, Waiting on),
// and gives three ways out: Check again, Contact your admin, Sign out.
// Distinct from the biometric gate: this is the org-approval screen driven
// by the auth context's `locked` status.

import { Redirect } from "expo-router";
import { useState } from "react";
import { StyleSheet, Text, View } from "react-native";
import Ionicons from "@expo/vector-icons/Ionicons";
import * as Device from "expo-device";

import { useAuth } from "@/auth/auth-context";
import { BootScreen } from "@/ui/boot";
import { FactRow, Facts, LinkButton, PACK_ICONS, PrimaryButton, SecondaryButton } from "@/ui/pack";
import { type, useTheme } from "@/ui/tokens";
import { openAdminMail } from "@/auth/admin-contact";

export default function LockedScreen() {
  const { status, signOut } = useAuth();
  const { tokens } = useTheme();
  const [checking, setChecking] = useState(false);

  if (status === "signedIn") return <Redirect href="/(tabs)" />;
  if (status === "signedOut") return <Redirect href="/login" />;
  if (status === "mfa") return <Redirect href="/mfa" />;
  if (status === "loading") return <BootScreen />;

  return (
    <View
      style={[styles.screen, { backgroundColor: tokens.background }]}
      testID="locked-screen"
    >
      <View style={[styles.lockDisc, { backgroundColor: tokens.surfaceHover }]}>
        <Ionicons name={PACK_ICONS.lock} size={40} color={tokens.textSecondary} />
      </View>
      <Text style={[styles.title, { color: tokens.text }]}>This phone is not approved yet</Text>
      <Text style={[styles.body, { color: tokens.textSecondary }]}>
        Your service desk needs to approve this phone before you can sign in. We have asked them,
        and you will get a notification the moment they do.
      </Text>
      <Facts style={styles.facts}>
        <FactRow first label="Phone">
          {Device.modelName ?? "This phone"}
        </FactRow>
        <FactRow label="Asked">Just now</FactRow>
        <FactRow label="Waiting on">Your admin</FactRow>
      </Facts>
      <PrimaryButton
        label="Check again"
        testID="locked-check"
        disabled={checking}
        busy={checking}
        onPress={() => {
          setChecking(true);
          setTimeout(() => setChecking(false), 1200);
        }}
      />
      <SecondaryButton
        label="Contact your admin"
        testID="locked-contact"
        onPress={() => {
          void openAdminMail();
        }}
      />
      <LinkButton
        label="Sign out"
        testID="locked-signout"
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
