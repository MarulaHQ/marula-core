// Native signature pad — the drawing surface at the door.
//
// Geometry is the pack's own, not invented. The Marula pack draws this exact
// surface twice: `design/mockups/marula-app-mockup-images/mobile/{light,dark}/
// states/08-wrap-up-resolved.png` (empty: a dashed box, "Sign here" bottom
// left, "Clear" bottom right) and `11-wrap-up-signed.png` (marked: the stroke
// in the theme's TEXT colour). This component reproduces that, using
// react-native-svg paths over a gesture-handler pan, which is the seam the
// pack's own icon set already assumes (`packages/ui-native/src/icons/
// marula-icons.native.tsx` imports react-native-svg).
//
// Pressure/tilt are DROPPED, explicitly: react-native-svg exposes no pressure
// channel, and gesture-handler's pan reports translationX/translationY only,
// so there is nothing to map. A constant weight on every input is honest,
// where a per-input curve that cannot be fed would be theatre.
//
// The captured mark rasterises through the dependency-free bilevel PNG writer
// in `native/src/signature/png.ts` — React Native has no canvas and no
// `toDataURL`, and the server's attachment store sniffs magic bytes and
// accepts a real PNG or nothing.

import React, { useCallback, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, TextInput, View } from "react-native";
import Svg, { Path } from "react-native-svg";
import { Gesture, GestureDetector } from "react-native-gesture-handler";

import { useTheme } from "../ui/tokens";
import { BASE_LINE_WIDTH, type Stroke, type StrokePoint } from "./stroke";
import { DEFAULT_RASTER, describeMark, hasInk, rasterise } from "./raster";
import { encodeBilevelPng, toBase64 } from "./png";

export interface CapturePayload {
  /** Base64 PNG, or "" for a typed-name agreement. */
  imageBase64: string;
  /** Who signed, as they typed it. */
  signerName: string;
}

export interface SignaturePadProps {
  onCapture: (payload: CapturePayload) => void | Promise<void>;
  consentText: string;
  consentVersion: number;
  disabled?: boolean;
  busy?: boolean;
}

const INK_PALETTE: readonly [readonly [number, number, number], readonly [number, number, number]] = [
  [0, 0, 0],
  [43, 42, 51],
];

export function SignaturePad({
  onCapture,
  consentText,
  consentVersion,
  disabled = false,
  busy = false,
}: SignaturePadProps): React.ReactNode {
  const { tokens } = useTheme();
  const [strokes, setStrokes] = useState<readonly Stroke[]>([]);
  const [typedName, setTypedName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [capturing, setCapturing] = useState(false);

  const marked = useMemo(() => hasInk(strokes), [strokes]);
  const typedMode = typedName.trim().length > 0;

  /**
   * Maps gesture coordinates into the pad's own space. `t` is a device-clock
   * reading, which for a pan gesture is `Date.now()`: gesture-handler's Pan
   * events carry translation and state, not a timestamp, and the captured
   * point set records evidence about the device rather than a trusted clock.
   */
  const local = useCallback(
    (x: number, y: number): StrokePoint => ({ x, y, t: Date.now() }),
    [],
  );

  const pan = useMemo(
    () =>
      Gesture.Pan()
        .minDistance(0)
        .minPointers(1)
        .maxPointers(1)
        .onStart((event) => {
          setStrokes((current) => [...current, [local(event.x, event.y)]]);
        })
        .onUpdate((event) => {
          setStrokes((current) => {
            if (current.length === 0) return current;
            const index = current.length - 1;
            const stroke = current[index];
            if (stroke === undefined) return current;
            const next: Stroke = [
              ...stroke,
              local(event.x, event.y),
            ];
            return [...current.slice(0, index), next];
          });
        })
        .enabled(!disabled && !busy),
    [busy, disabled, local],
  );

  const clear = useCallback(() => {
    setStrokes([]);
    setError(null);
  }, []);

  /** Takes back the last gesture — a customer who wants a clean pad presses
   *  Clear, so undo is deliberately per-stroke. */
  const undo = useCallback(() => {
    setStrokes((current) => current.slice(0, -1));
    setError(null);
  }, []);

  const capture = useCallback(async () => {
    setError(null);
    setCapturing(true);
    try {
      const typed = typedName.trim();
      if (typed !== "") {
        // A typed agreement is the SAME record without the ink; the name the
        // customer typed is the sign-off.
        await onCapture({ imageBase64: "", signerName: typed });
        return;
      }
      if (!marked) {
        setError("The customer needs to sign, or type their name to agree instead.");
        return;
      }
      const pixels = rasterise(strokes, DEFAULT_RASTER);
      const png = encodeBilevelPng({
        width: DEFAULT_RASTER.width,
        height: DEFAULT_RASTER.height,
        palette: INK_PALETTE,
        pixels,
      });
      await onCapture({ imageBase64: toBase64(png), signerName: "" });
    } catch (err) {
      setError(err instanceof Error ? err.message : "The signature could not be captured.");
    } finally {
      setCapturing(false);
    }
    // `strokes` is a dep on purpose: the rasteriser must see the mark the
    // signer finished, and a ref written during render to dodge that is
    // exactly what the render-phase lint rule rejects.
  }, [marked, onCapture, strokes, typedName]);

  const description = describeMark(strokes);
  const idle = !marked && !typedMode;

  return (
    <View
      style={[styles.section, { borderColor: tokens.border }]}
      accessible
      accessibilityLabel={
        marked
          ? `Signature drawn: ${description}`
          : "Empty signature pad. The customer draws here."
      }
    >
      <Text style={[styles.heading, { color: tokens.text }]}>Client sign-off</Text>

      <GestureDetector gesture={pan}>
        <View
          style={[
            styles.pad,
            { borderColor: tokens.border, backgroundColor: tokens.surfaceHover },
          ]}
          testID="signature-pad-surface"
        >
          {/* No viewBox: 1 user unit must equal 1 CSS pixel, or gesture
              coordinates land somewhere other than the finger that made
              them. A viewBox rescales, so it is deliberately absent. */}
          <Svg width="100%" height="100%">
            {strokes.map((stroke, index) => (
              <Path
                key={`stroke-${index}`}
                d={toPathData(stroke)}
                stroke={tokens.text}
                strokeWidth={BASE_LINE_WIDTH}
                strokeLinecap="round"
                strokeLinejoin="round"
                fill="none"
              />
            ))}
          </Svg>
          {!marked && (
            <Text style={[styles.placeholder, { color: tokens.textMuted }]} pointerEvents="none">
              Sign here
            </Text>
          )}
          <Pressable
            onPress={clear}
            disabled={disabled || busy || !marked}
            accessibilityRole="button"
            accessibilityLabel="Clear the signature"
            style={styles.clearHit}
            testID="signature-clear"
          >
            <Text style={[styles.clearLabel, { color: tokens.textMuted }]}>Clear</Text>
          </Pressable>
        </View>
      </GestureDetector>

      {strokes.length > 0 && (
        <Pressable onPress={undo} disabled={disabled || busy} accessibilityRole="button" testID="signature-undo">
          <Text style={[styles.undo, { color: tokens.textMuted }]}>Undo last stroke</Text>
        </Pressable>
      )}

      <Text style={[styles.label, { color: tokens.textSecondary }]}>Customer&rsquo;s name</Text>
      <TextInput
        value={typedName}
        onChangeText={setTypedName}
        placeholder="Dana Whitfield"
        placeholderTextColor={tokens.textMuted}
        editable={!disabled && !busy}
        returnKeyType="done"
        onSubmitEditing={() => void capture()}
        accessibilityLabel="Customer's name"
        style={[
          styles.input,
          { borderColor: tokens.border, backgroundColor: tokens.surface, color: tokens.text },
        ]}
        testID="signature-typed-name"
      />
      <Text style={[styles.hint, { color: tokens.textMuted }]}>
        Typing a name and pressing Sign off records the same agreement without ink.
      </Text>

      <Text style={[styles.consent, { color: tokens.textMuted }]} testID="signature-consent">
        {consentText} (v{consentVersion})
      </Text>

      <View style={styles.footer}>
        <Pressable
          onPress={() => void capture()}
          disabled={disabled || busy || idle}
          accessibilityRole="button"
          style={[
            styles.confirm,
            {
              backgroundColor: idle ? tokens.surfaceHover : tokens.primary,
            },
          ]}
          testID="signature-confirm"
        >
          <Text
            style={[
              styles.confirmLabel,
              { color: idle ? tokens.textMuted : tokens.primaryText },
            ]}
          >
            {capturing || busy ? "Signing…" : "Sign off"}
          </Text>
        </Pressable>
        <Text style={[styles.status, { color: tokens.textMuted }]}>
          {capturing
            ? "Recording"
            : marked
              ? "Ready"
              : typedMode
                ? "Typed agreement"
                : "Waiting for ink"}
        </Text>
      </View>

      {error !== null && (
        <Text style={[styles.error, { color: tokens.danger.text }]} accessibilityLiveRegion="polite">
          {error}
        </Text>
      )}
    </View>
  );
}

/** Builds one SVG path string from one stroke. A lone point becomes a zero
 *  length move so the round cap still draws a visible dot. */
function toPathData(stroke: Stroke): string {
  if (stroke.length === 0) return "";
  const first = stroke[0]!;
  if (stroke.length === 1) return `M ${first.x} ${first.y} L ${first.x} ${first.y}`;
  let d = `M ${first.x} ${first.y}`;
  for (let i = 1; i < stroke.length; i += 1) {
    d += ` L ${stroke[i]!.x} ${stroke[i]!.y}`;
  }
  return d;
}

const styles = StyleSheet.create({
  section: {
    marginTop: 14,
    borderTopWidth: 1,
    paddingTop: 14,
  },
  heading: {
    fontSize: 15,
    fontWeight: "700",
    marginBottom: 8,
  },
  pad: {
    height: 132,
    borderWidth: 1,
    borderStyle: "dashed",
    borderRadius: 14,
    overflow: "hidden",
    justifyContent: "center",
  },
  placeholder: {
    position: "absolute",
    left: 12,
    bottom: 8,
    fontSize: 11,
  },
  clearHit: {
    position: "absolute",
    right: 8,
    bottom: 6,
    paddingHorizontal: 8,
    paddingVertical: 4,
  },
  clearLabel: {
    fontSize: 11,
  },
  undo: {
    marginTop: 6,
    fontSize: 11,
    textDecorationLine: "underline",
  },
  label: {
    marginTop: 12,
    fontSize: 12,
  },
  input: {
    marginTop: 6,
    borderWidth: 1,
    borderRadius: 10,
    paddingHorizontal: 10,
    paddingVertical: 8,
    fontSize: 14,
  },
  hint: {
    marginTop: 6,
    fontSize: 10.5,
    lineHeight: 15,
  },
  consent: {
    marginTop: 8,
    fontSize: 10.5,
    lineHeight: 15,
  },
  footer: {
    marginTop: 10,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  confirm: {
    borderRadius: 10,
    paddingHorizontal: 14,
    paddingVertical: 8,
  },
  confirmLabel: {
    fontSize: 12,
    fontWeight: "700",
  },
  status: {
    fontSize: 10,
  },
  error: {
    marginTop: 8,
    fontSize: 10.5,
  },
});
