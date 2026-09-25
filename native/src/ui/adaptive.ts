// Adaptive layout seam: regular/compact width classes from day one.
//
// Purpose: the same screens serve phones and tablets without a retrofit.
// Width (not device model) decides the class, so iPad Stage Manager
// resizing and split view switch layouts correctly. Regular width gets a
// two-pane master–detail; compact gets push navigation. Pointer affordances
// are gated by capability (iPad trackpad/mouse), never by user-agent
// guessing.
//
// Non-purpose: no navigation state. Routes stay the same in both classes.

import { useWindowDimensions, Platform } from "react-native";

export const REGULAR_MIN_WIDTH = 768;

export type WidthClass = "compact" | "regular";

export interface AdaptiveLayout {
  width: number;
  height: number;
  widthClass: WidthClass;
  isRegular: boolean;
  /** Keyboards and trackpads exist on tablets; hover states are legal there. */
  hasPointer: boolean;
}

export function useAdaptiveLayout(): AdaptiveLayout {
  const { width, height } = useWindowDimensions();
  const isRegular = width >= REGULAR_MIN_WIDTH;
  return {
    width,
    height,
    widthClass: isRegular ? "regular" : "compact",
    isRegular,
    hasPointer: Platform.OS === "ios" && Platform.isPad === true,
  };
}
