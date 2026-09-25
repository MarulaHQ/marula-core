// Keyboard-wedge detector (Wave 32 lane 5, `p32-labels-scanning`).
//
// A HID barcode scanner behaves like a very fast keyboard: it types the
// payload, then presses Enter (the configured suffix). Humans type slower
// and with variance, so a burst of printable keys whose inter-key gaps all
// sit under the threshold, terminated by Enter, is a scan — and nothing
// else is.
//
// This module is pure: it consumes key events and reports whether a scan
// completed. The ScanBar owns the DOM side (capturing Enter so a form does
// not submit, stripping the injected burst out of a focused field, routing
// the resolved payload). No timers, no focus calls, no side effects — so
// the detection rules are unit-testable and identical on every surface.

/** Max ms between two keys of the same scanner burst. */
export const WEDGE_MAX_GAP_MS = 55;
/** A burst shorter than this cannot be a scan (avoids typo false-positives). */
export const WEDGE_MIN_CHARS = 3;
/** A burst longer than this is discarded (a stuck key is not a scan). */
export const WEDGE_MAX_CHARS = 512;

export interface WedgeState {
  buffer: string;
  lastKeyAt: number;
}

export interface KeyInput {
  key: string;
  at: number;
  ctrlKey?: boolean;
  metaKey?: boolean;
  altKey?: boolean;
}

export type WedgeResult =
  | { kind: "ignored" }
  | { kind: "buffering" }
  | { kind: "scan"; text: string }
  | { kind: "reset" };

export function createWedgeState(): WedgeState {
  return { buffer: "", lastKeyAt: 0 };
}

/**
 * Feeds one key event. Returns the next state plus what happened:
 *  - `scan`      — a burst completed on Enter; `text` is the payload.
 *  - `buffering` — a printable key landed in a live burst.
 *  - `reset`     — the burst broke (gap, modifier, non-printable) or Enter
 *                  arrived without a burst; the buffer is empty again.
 *  - `ignored`   — nothing relevant (pure modifier keys).
 */
export function feedWedge(state: WedgeState, input: KeyInput): { next: WedgeState; result: WedgeResult } {
  const { key, at } = input;
  const isModifierOnly = key === "Shift" || key === "Control" || key === "Alt" || key === "Meta";
  if (isModifierOnly) {
    return { next: state, result: { kind: "ignored" } };
  }
  if (key === "Enter") {
    const burst =
      state.buffer.length >= WEDGE_MIN_CHARS && at - state.lastKeyAt <= WEDGE_MAX_GAP_MS
        ? state.buffer
        : null;
    const next = createWedgeState();
    if (burst !== null) {
      return { next, result: { kind: "scan", text: burst } };
    }
    return { next, result: { kind: "reset" } };
  }
  if (input.ctrlKey === true || input.metaKey === true || input.altKey === true) {
    return { next: createWedgeState(), result: { kind: "reset" } };
  }
  const isPrintable = key.length === 1;
  if (!isPrintable) {
    // Tab, arrows, Escape, function keys: not scanner payload; break the burst.
    return state.buffer === ""
      ? { next: state, result: { kind: "ignored" } }
      : { next: createWedgeState(), result: { kind: "reset" } };
  }
  const continues = state.buffer !== "" && at - state.lastKeyAt <= WEDGE_MAX_GAP_MS;
  const buffer = continues ? state.buffer + key : key;
  if (buffer.length > WEDGE_MAX_CHARS) {
    return { next: createWedgeState(), result: { kind: "reset" } };
  }
  return { next: { buffer, lastKeyAt: at }, result: { kind: "buffering" } };
}

/**
 * True when the burst was typed into a field the scanner did NOT mean to
 * fill: an editable element that is not the scan bar's own input. The
 * caller strips exactly `text` from the end of that element's value (the
 * scan restores the field instead of polluting it) and never moves focus.
 */
export function burstLandedInForeignField(
  target: EventTarget | null,
  scanInput: HTMLElement | null,
): target is HTMLElement {
  if (target === null || !(target instanceof HTMLElement)) return false;
  if (scanInput !== null && target === scanInput) return false;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || target.isContentEditable === true;
}

/**
 * Removes the injected burst from the tail of a field value and notifies
 * React's controlled components through a native setter + input event —
 * the scan must not eat the user's own typing, and must not silently
 * leave scanner garbage in a form.
 */
export function stripBurstFromField(el: HTMLElement, burst: string): void {
  if (burst === "") return;
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {
    if (!el.value.endsWith(burst)) return;
    const next = el.value.slice(0, el.value.length - burst.length);
    const proto = el instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(proto, "value")?.set;
    if (setter !== undefined) {
      setter.call(el, next);
    } else {
      el.value = next;
    }
    el.dispatchEvent(new Event("input", { bubbles: true }));
    return;
  }
  // contenteditable: only strip when the text actually ends with the burst.
  const text = el.textContent ?? "";
  if (!text.endsWith(burst)) return;
  el.textContent = text.slice(0, text.length - burst.length);
  el.dispatchEvent(new Event("input", { bubbles: true }));
}
