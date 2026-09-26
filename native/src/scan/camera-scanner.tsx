// Camera scanner — the real lens, an honest simulator state, one API.
//
// Purpose: the physical device decodes a QR/Code128 and hands the raw string
// to the caller. The simulator has no camera, so it says exactly that and
// points at the injector; the resolver owns the meaning in both cases (Wave
// 32 lane 5 contract: POST the string verbatim, never parse locally).
//
// Non-purpose: no resolution, no routing, no side effects. A scan returns
// text; the caller stages it.

import Ionicons from "@expo/vector-icons/Ionicons";
import { CameraView, useCameraPermissions } from "expo-camera";
import * as Device from "expo-device";
import { useEffect, useRef } from "react";
import { Modal, Pressable, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { Button } from "@/ui/primitives";
import { shape, type, useTheme } from "@/ui/tokens";

/** Barcode formats the product prints (labels lane 5) plus retail codes. */
const BARCODE_TYPES = ["qr", "code128", "code39", "ean13", "ean8", "upc_a"] as const;

export function CameraScanner({
  visible,
  onClose,
  onScan,
}: {
  visible: boolean;
  onClose: () => void;
  onScan: (text: string) => void;
}) {
  const { tokens } = useTheme();
  const insets = useSafeAreaInsets();
  const [permission, requestPermission] = useCameraPermissions();
  const locked = useRef(false);

  // Re-arm the single-shot lock each time the sheet opens (the component
  // instance survives close).
  useEffect(() => {
    if (visible) locked.current = false;
  }, [visible]);

  // Only mount the preview while open: only one Camera preview may exist.
  if (!visible) return null;

  const simulator = !Device.isDevice;

  return (
    <Modal visible animationType="fade" onRequestClose={onClose} transparent={false}>
      <View style={[styles.root, { backgroundColor: tokens.background }]}>
        {simulator || permission === null || !permission.granted ? (
          <View style={[styles.fallback, { paddingTop: insets.top + 24 }]}>
            <Ionicons name="camera-outline" size={34} color={tokens.textMuted} />
            <Text style={[styles.fallbackTitle, { color: tokens.text }]}>
              {simulator ? "This device has no camera" : "Camera access needed"}
            </Text>
            <Text style={[styles.fallbackBody, { color: tokens.textMuted }]}>
              {simulator
                ? "The simulator has no lens. Type or paste the code instead — every scan path behaves the same."
                : "Meridian scans labels on this device only. Nothing is uploaded until you confirm a movement."}
            </Text>
            {!simulator && permission !== null && !permission.granted ? (
              <Button
                label={permission.canAskAgain ? "Allow camera" : "Open settings"}
                testID="camera-permission"
                onPress={() => {
                  void requestPermission();
                }}
              />
            ) : null}
            <Button label="Close" variant="secondary" testID="camera-close" onPress={onClose} />
          </View>
        ) : (
          <View style={styles.preview}>
            <CameraView
              style={StyleSheet.absoluteFill}
              facing="back"
              enableTorch={false}
              barcodeScannerSettings={{ barcodeTypes: [...BARCODE_TYPES] }}
              onBarcodeScanned={(result) => {
                if (locked.current) return;
                locked.current = true;
                onScan(result.data);
              }}
            />
            {/* The scan frame: four corner marks, no chrome over the lens. */}
            <View pointerEvents="none" style={styles.frameWrap}>
              <View style={[styles.frame, { borderColor: tokens.overlay }]}>
                <View style={[styles.corner, styles.tl, { borderColor: tokens.primary }]} />
                <View style={[styles.corner, styles.tr, { borderColor: tokens.primary }]} />
                <View style={[styles.corner, styles.bl, { borderColor: tokens.primary }]} />
                <View style={[styles.corner, styles.br, { borderColor: tokens.primary }]} />
              </View>
              <View style={[styles.hintPill, { backgroundColor: tokens.surface }]}>
                <Text style={[styles.hint, { color: tokens.text }]}>
                  Point at the label — the scan fills the form, you confirm.
                </Text>
              </View>
            </View>
            <Pressable
              testID="camera-close"
              onPress={onClose}
              accessibilityRole="button"
              accessibilityLabel="Close the camera"
              style={[styles.close, { top: insets.top + 12, backgroundColor: tokens.surface }]}
            >
              <Ionicons name="close" size={22} color={tokens.text} />
            </Pressable>
          </View>
        )}
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  root: {
    flex: 1,
  },
  fallback: {
    flex: 1,
    padding: 24,
    gap: 12,
    alignItems: "flex-start",
    justifyContent: "center",
  },
  fallbackTitle: {
    fontFamily: type.sansSemiBold,
    fontSize: 20,
  },
  fallbackBody: {
    fontFamily: type.sans,
    fontSize: 14,
    lineHeight: 20,
  },
  preview: {
    flex: 1,
  },
  frameWrap: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    alignItems: "center",
    justifyContent: "center",
    gap: 16,
  },
  frame: {
    width: 232,
    height: 232,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: shape.radiusCard,
  },
  corner: {
    position: "absolute",
    width: 28,
    height: 28,
    borderWidth: 3,
  },
  tl: { top: -2, left: -2, borderRightWidth: 0, borderBottomWidth: 0, borderTopLeftRadius: 12 },
  tr: { top: -2, right: -2, borderLeftWidth: 0, borderBottomWidth: 0, borderTopRightRadius: 12 },
  bl: { bottom: -2, left: -2, borderRightWidth: 0, borderTopWidth: 0, borderBottomLeftRadius: 12 },
  br: { bottom: -2, right: -2, borderLeftWidth: 0, borderTopWidth: 0, borderBottomRightRadius: 12 },
  hintPill: {
    paddingHorizontal: 12,
    paddingVertical: 8,
    borderRadius: shape.radiusPill,
    maxWidth: 300,
  },
  hint: {
    fontFamily: type.sansMedium,
    fontSize: 13,
    textAlign: "center",
  },
  close: {
    position: "absolute",
    right: 16,
    width: 44,
    height: 44,
    borderRadius: 22,
    alignItems: "center",
    justifyContent: "center",
  },
});
