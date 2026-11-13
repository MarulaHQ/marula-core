// Motion primitives — the app's small, shared motion vocabulary, so every
// reveal and every press is the same gesture instead of a per-screen opinion.
//
// Purpose: two things only. `FadeIn` settles a block into place when it
// replaces a skeleton, and `usePressScale` gives a card a physical give under
// the thumb. Both run inside the theme law (docs/theme.md §5: <=120ms, on
// transform and opacity).
//
// Reduce motion is a three-state fact, not a boolean: "unknown" until React
// Native's async probe answers. Nothing animates while it is unknown, and the
// first mount of a cold start therefore paints without motion instead of
// guessing wrong and fading at a reader who asked it not to.
//
// Non-purpose: no gesture animation, no layout animation, no looping
// attention-getters. A spinner already says "working"; a second thing moving on
// screen is noise.

import { useEffect, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import {
  AccessibilityInfo,
  Animated,
  Easing,
  type StyleProp,
  type ViewStyle,
} from "react-native";

import { motion, staggerDelay } from "./motion-rules";

/** What the reader has asked for. "unknown" is the window before the probe answers. */
export type ReducedMotionState = "unknown" | "on" | "off";

// One value for the whole app. Reading it per component would put an
// AccessibilityInfo listener behind every card in a list; the store keeps one
// and re-renders only the subscribers.
let reducedMotionState: ReducedMotionState = "unknown";
const reducedMotionListeners = new Set<() => void>();
let reducedMotionStarted = false;
// A change event that lands while the startup query is in flight is newer than
// that query, so the query's answer must not overwrite it.
let changedSinceQuery = false;

function publishReducedMotion(next: ReducedMotionState): void {
  if (next === reducedMotionState) return;
  reducedMotionState = next;
  for (const listener of reducedMotionListeners) listener();
}

function startReducedMotion(): void {
  if (reducedMotionStarted) return;
  reducedMotionStarted = true;
  // Subscribe first: an event that arrives before the query does still wins.
  AccessibilityInfo.addEventListener("reduceMotionChanged", (enabled: boolean) => {
    changedSinceQuery = true;
    publishReducedMotion(enabled ? "on" : "off");
  });
  void AccessibilityInfo.isReduceMotionEnabled().then((enabled) => {
    if (changedSinceQuery) return;
    publishReducedMotion(enabled ? "on" : "off");
  });
}

function subscribeReducedMotion(onChange: () => void): () => void {
  startReducedMotion();
  reducedMotionListeners.add(onChange);
  return () => {
    reducedMotionListeners.delete(onChange);
  };
}

function readReducedMotion(): ReducedMotionState {
  return reducedMotionState;
}

function readReducedMotionOnServer(): ReducedMotionState {
  return "unknown";
}

/** The reader's reduce-motion preference, including the unresolved window. */
export function useReducedMotionState(): ReducedMotionState {
  return useSyncExternalStore(
    subscribeReducedMotion,
    readReducedMotion,
    readReducedMotionOnServer,
  );
}

/** True only once the reader has actually asked for reduced motion. */
export function useReduceMotion(): boolean {
  return useReducedMotionState() === "on";
}

/**
 * Fade-and-settle an element on mount.
 *
 * `index` opts the element into a stagger: pass its position in the list and
 * the delay comes from the shared rule, so a list's rhythm is one number in one
 * file. Pass nothing for a single block that enters immediately.
 */
export function FadeIn({
  children,
  index,
  delay,
  style,
  testID,
}: {
  children: ReactNode;
  /** Position in a list, for the stagger. Ignored when `delay` is given. */
  index?: number;
  /** Explicit delay in ms, for the rare block that is not in a list. */
  delay?: number;
  style?: StyleProp<ViewStyle>;
  testID?: string;
}): React.JSX.Element {
  const preference = useReducedMotionState();
  // Paint visible unless motion is confirmed allowed, so nothing flashes at
  // opacity 0 while the preference is unknown or reduced.
  const [progress] = useState(() => new Animated.Value(preference === "off" ? 0 : 1));
  // Frozen at mount: a row that moves in a filtered list keeps its own delay
  // instead of replaying its entrance.
  const [start] = useState(() => delay ?? staggerDelay(index ?? 0, preference !== "off"));
  // Set once this element has entered or skipped entering. A later preference
  // change must not replay an entrance the reader has already seen.
  const settled = useRef(false);

  useEffect(() => {
    if (settled.current || preference !== "off") {
      settled.current = true;
      progress.setValue(1);
      return;
    }
    progress.setValue(0);
    const animation = Animated.timing(progress, {
      toValue: 1,
      duration: motion.duration,
      delay: start,
      easing: Easing.out(Easing.quad),
      useNativeDriver: true,
    });
    animation.start(({ finished }) => {
      if (finished) settled.current = true;
    });
    // Leaving mid-animation is fine: the value is discarded with the node.
    return () => animation.stop();
  }, [progress, preference, start]);

  const translateY = progress.interpolate({
    inputRange: [0, 1],
    outputRange: [motion.distance, 0],
  });

  return (
    <Animated.View
      testID={testID}
      style={[{ opacity: progress, transform: [{ translateY }] }, style]}
    >
      {children}
    </Animated.View>
  );
}

/**
 * Press feedback for a touchable surface: the card dips a hair under the
 * finger and comes back. The dip is 1.5% — enough to feel like a physical
 * control, small enough that a list of them does not wobble.
 *
 * Spread the handlers on the `Pressable` and put `style` on an `Animated.View`
 * that wraps it, not on the touch target itself: a transform on the element
 * that carries layout would let the card shrink out of its own row inside a
 * `flex: 1` parent.
 */
export function usePressScale(): {
  onPressIn: () => void;
  onPressOut: () => void;
  style: { transform: { scale: Animated.AnimatedInterpolation<number> }[] };
} {
  const preference = useReducedMotionState();
  const [depth] = useState(() => new Animated.Value(0));
  const canAnimate = preference === "off";

  // A preference that changes mid-press, or before the release lands, would
  // otherwise leave the card at the dip: setValue stops the running animation
  // and restores the resting scale.
  useEffect(() => {
    if (!canAnimate) depth.setValue(0);
  }, [canAnimate, depth]);

  const settle = (toValue: number, duration: number) => {
    Animated.timing(depth, {
      toValue,
      duration,
      easing: Easing.out(Easing.quad),
      useNativeDriver: true,
    }).start();
  };

  return {
    onPressIn: () => {
      if (!canAnimate) return;
      settle(1, motion.pressIn);
    },
    onPressOut: () => {
      if (!canAnimate) {
        depth.setValue(0);
        return;
      }
      settle(0, motion.duration);
    },
    style: {
      transform: [
        {
          scale: depth.interpolate({
            inputRange: [0, 1],
            outputRange: [1, motion.pressScale],
          }),
        },
      ],
    },
  };
}
