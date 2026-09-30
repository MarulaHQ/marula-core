// MFA — the pack's `11-mfa`: six boxes, the active one outlined, Verify
// disabled until six digits, a backup-code link, and the 30-second note.
//
// Purpose: the typed MFA flow underneath the drawing — the bridge token
// from a gated login verifies through POST /auth/mfa/verify; recovery codes
// ride the same field because the server accepts either.

import { Redirect, router } from "expo-router";
import { useRef, useState } from "react";
import { Pressable, StyleSheet, Text, TextInput, View } from "react-native";
import Ionicons from "@expo/vector-icons/Ionicons";

import { useAuth } from "@/auth/auth-context";
import { PACK_ICONS, LinkButton, PrimaryButton } from "@/ui/pack";
import { chrome, type, useTheme } from "@/ui/tokens";

const CODE_LEN = 6;

export default function MfaScreen() {
  const { status, error, submitMfa, cancelMfa } = useAuth();
  const { tokens } = useTheme();
  const [digits, setDigits] = useState<string[]>(Array(CODE_LEN).fill(""));
  const [busy, setBusy] = useState(false);
  const inputs = useRef<(TextInput | null)[]>([]);

  if (status === "signedIn") return <Redirect href="/(tabs)" />;
  if (status !== "mfa") return <Redirect href="/login" />;

  const code = digits.join("");

  const setDigit = (index: number, value: string) => {
    const clean = value.replace(/[^0-9]/g, "");
    if (clean.length === 0) {
      setDigits((current) => current.map((d, i) => (i === index ? "" : d)));
      return;
    }
    setDigits((current) => {
      const next = [...current];
      for (const char of clean) {
        if (index >= CODE_LEN) break;
        next[index] = char;
        index += 1;
      }
      return next;
    });
    const target = Math.min(index + clean.length, CODE_LEN - 1);
    inputs.current[target]?.focus();
  };

  const filled = code.replace(/\s/g, "").length;

  return (
    <View style={[styles.screen, { backgroundColor: tokens.background }]} testID="mfa-screen">
      <Pressable
        accessibilityRole="button"
        accessibilityLabel="Back"
        onPress={() => router.replace("/login" as never)}
        style={styles.backDisc}
      >
        <Ionicons name={PACK_ICONS.back} size={22} color={tokens.text} />
      </Pressable>

      <View style={[styles.shield, { backgroundColor: tokens.warning.background }]}>
        <Ionicons name={PACK_ICONS.shield} size={30} color={chrome.goldText} />
      </View>

      <View style={styles.headline}>
        <Text style={[styles.title, { color: tokens.text }]}>Enter your code</Text>
        <Text style={[styles.subtitle, { color: tokens.textSecondary }]}>
          Open your authenticator app and type the 6-digit code for Marula.
        </Text>
      </View>

      <View style={styles.otpRow} accessibilityLabel="6-digit code">
        {digits.map((digit, index) => {
          const active = digit === "" && index === filled;
          return (
            <View
              key={index}
              style={[
                styles.otpBox,
                {
                  borderColor: active ? tokens.primary : tokens.border,
                  borderWidth: active ? 2 : 1,
                },
              ]}
            >
              <Text style={[styles.otpDigit, { color: tokens.text }]}>{digit}</Text>
              <TextInput
                ref={(node) => {
                  inputs.current[index] = node;
                }}
                value={digit}
                onChangeText={(value) => setDigit(index, value)}
                keyboardType="number-pad"
                textContentType="oneTimeCode"
                maxLength={6}
                style={styles.otpInput}
                accessibilityLabel={`Digit ${index + 1}`}
                autoFocus={index === 0}
              />
            </View>
          );
        })}
      </View>

      {error !== null ? (
        <View style={styles.errorRow}>
          <Ionicons name={PACK_ICONS.alert} size={16} color={tokens.warning.text} />
          <Text style={[styles.errorText, { color: tokens.warning.text }]}>{error}</Text>
        </View>
      ) : null}

      <PrimaryButton
        label="Verify"
        testID="mfa-submit"
        disabled={filled < CODE_LEN || busy}
        busy={busy}
        onPress={() => {
          setBusy(true);
          void submitMfa(code).finally(() => setBusy(false));
        }}
      />
      <LinkButton
        label="Use a backup code instead"
        testID="mfa-backup"
        onPress={() => {
          cancelMfa();
          router.replace("/login");
        }}
      />
      <Text style={[styles.note, { color: tokens.textSecondary }]}>
        Codes change every 30 seconds. If it did not work, wait for the next one.
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  screen: {
    flex: 1,
    justifyContent: "center",
    gap: 16,
    paddingHorizontal: 24,
  },
  backDisc: {
    position: "absolute",
    top: 46,
    left: 12,
    width: 44,
    height: 44,
    borderRadius: 12,
    alignItems: "center",
    justifyContent: "center",
  },
  shield: {
    width: 64,
    height: 64,
    borderRadius: 999,
    alignItems: "center",
    justifyContent: "center",
  },
  headline: {
    gap: 6,
  },
  title: {
    fontFamily: type.sansSemiBold,
    fontSize: 22,
    fontWeight: "600",
  },
  subtitle: {
    fontFamily: type.sans,
    fontSize: 14,
    lineHeight: 21,
    maxWidth: 300,
  },
  otpRow: {
    flexDirection: "row",
    justifyContent: "center",
    gap: 8,
    minHeight: 60,
  },
  otpBox: {
    width: 48,
    height: 60,
    borderRadius: 12,
    alignItems: "center",
    justifyContent: "center",
  },
  otpDigit: {
    fontSize: 26,
    fontWeight: "600",
  },
  otpInput: {
    position: "absolute",
    width: "100%",
    height: "100%",
    opacity: 0,
    textAlign: "center",
  },
  errorRow: {
    flexDirection: "row",
    gap: 8,
    alignItems: "flex-start",
  },
  errorText: {
    flex: 1,
    fontSize: 14,
    fontWeight: "600",
  },
  note: {
    fontSize: 13,
    textAlign: "center",
    lineHeight: 20,
  },
});

