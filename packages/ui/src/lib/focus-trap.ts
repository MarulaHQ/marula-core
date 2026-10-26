import { useEffect, useRef, type RefObject } from "react";

/**
 * Focus trap for overlay dialogs: focus moves in on open, Tab cycles
 * inside the container, Escape dismisses, and focus returns to whatever
 * held it before the overlay opened. One implementation, every modal —
 * `aria-modal` is a promise to assistive tech, and this hook is what
 * keeps it (a modal that declares containment but lets Tab walk through
 * to the background is the defect this exists for).
 *
 * Usage: give the overlay container a ref, call `useFocusTrap(ref, open, onClose)`
 * from inside it, and leave the rest to the hook. The container itself is
 * focused when it has no focusable children (a scrim or empty panel).
 */
export function useFocusTrap(
  container: RefObject<HTMLElement | null>,
  active: boolean,
  onClose: () => void,
): void {
  // The latest close callback, read through the keydown closure: the
  // parent re-creates its handler every render, and re-running this
  // effect for it would refocus and re-stash on every keystroke.
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  useEffect(() => {
    if (!active) return;
    const root = container.current;
    if (!root) return;

    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;

    const focusables = (): HTMLElement[] =>
      Array.from(
        root.querySelectorAll<HTMLElement>(
          'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
      ).filter((el) => el.offsetParent !== null || el === document.activeElement);

    // Move focus in: the first control, or the container itself when it
    // has none (scrim-only overlays).
    const targets = focusables();
    (targets[0] ?? root).focus();

    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        closeRef.current();
        return;
      }
      if (event.key !== "Tab") return;
      const items = focusables();
      if (items.length === 0) {
        // Nothing focusable inside: keep Tab on the container.
        event.preventDefault();
        root.focus();
        return;
      }
      const first = items[0]!;
      const last = items[items.length - 1]!;
      const current = document.activeElement;
      if (event.shiftKey) {
        if (current === first || current === root || !root.contains(current)) {
          event.preventDefault();
          last.focus();
        }
      } else if (current === last || current === root || !root.contains(current)) {
        event.preventDefault();
        first.focus();
      }
    };

    root.addEventListener("keydown", onKey);
    return () => {
      root.removeEventListener("keydown", onKey);
      previous?.focus();
    };
  }, [active, container]);
}
