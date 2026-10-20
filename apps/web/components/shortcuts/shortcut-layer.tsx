// ShortcutLayer — the Wave 29 keyboard shell: hold-Alt overlay + ?-cheat
// sheet + the chord dispatcher, mounted once inside the app shell.
//
// Purpose: one listener surface resolving every chord through the registry
// (lib/commands) and rendering its two views — a transient overlay of the
// commands valid on the CURRENT surface while Alt is held, and the full
// registry dialog on `?`. Execution respects surfaces exactly like clicks:
// commands navigate the same hrefs and click the same controls, so
// disabled/unauthorized surfaces behave exactly as they would for a mouse.
//
// Non-purpose: no runtime conflict resolution (the registry lint settles
// chords at authoring time), no remapping (per-command custom is stored in
// the typed `shortcuts.profile` enumeration later; the profile's `all_off`
// value silences chords AND the hold-Alt overlay).

"use client";

import { usePathname, useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "@/lib/api";
import { chordFromEvent, modIsMeta } from "@/lib/commands/chords";
import { reportCommandOutcome } from "@/lib/commands/outcome";
import { commandForChord, commandsForSurface, metaRows } from "@/lib/commands/registry";
import type { CommandOutcome, ShortcutProfile } from "@/lib/commands/types";
import { CheatSheetDialog } from "./cheat-sheet-dialog";
import { ShortcutOverlay } from "./shortcut-overlay";

/** Reads the saved keyboard profile for signed-in users. null = unknown
 * yet (the layer behaves as `default` until proven otherwise). One retry
 * ride-along: the profile read may race the auth rotation at boot, and an
 * all_off preference that lost that race would leak chords anyway. */
function useShortcutProfile(enabled: boolean): ShortcutProfile | null {
  const [profile, setProfile] = useState<ShortcutProfile | null>(null);
  useEffect(() => {
    if (!enabled) {
      setProfile(null);
      return;
    }
    let cancelled = false;
    const load = async () => {
      try {
        const { data } = await api.api.GET("/api/v1/settings/user");
        if (cancelled) return;
        const row = (data ?? []).find((v) => v.key === "shortcuts.profile");
        if (row && (row.value === "default" || row.value === "all_off")) {
          setProfile(row.value);
        } else {
          setProfile("default");
        }
      } catch {
        if (!cancelled) {
          // Boot race with the token rotation: retry, then fall back.
          await new Promise((r) => window.setTimeout(r, 1500));
          if (!cancelled) {
            try {
              const retry = await api.api.GET("/api/v1/settings/user");
              if (cancelled) return;
              const row = retry.data?.find((v) => v.key === "shortcuts.profile");
              setProfile(row?.value === "default" || row?.value === "all_off" ? row.value : "default");
            } catch {
              if (!cancelled) setProfile("default");
            }
          }
        }
      }
    };
    void load();
    const onVisible = () => {
      if (document.visibilityState === "visible") void load();
    };
    window.addEventListener("focus", onVisible);
    return () => {
      cancelled = true;
      window.removeEventListener("focus", onVisible);
    };
  }, [enabled]);
  return profile;
}

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.isContentEditable ||
    target.tagName === "INPUT" ||
    target.tagName === "TEXTAREA" ||
    target.tagName === "SELECT"
  );
}

export function ShortcutLayer({ authed }: { authed: boolean }) {
  const pathname = usePathname();
  const router = useRouter();
  const profile = useShortcutProfile(authed);
  const shortcutsOn = profile !== "all_off";
  const [overlayOpen, setOverlayOpen] = useState(false);
  const [sheetOpen, setSheetOpen] = useState(false);
  const sheetTriggerRef = useRef<EventTarget | null>(null);
  const surfaceCommands = useMemo(() => (shortcutsOn ? commandsForSurface(pathname) : []), [pathname, shortcutsOn]);
  const metaRowsList = useMemo(() => metaRows(), []);

  const ctx = useMemo(
    () => ({
      pathname,
      navigate: (href: string) => router.push(href),
      // Typed, not boolean: the miss carries the selector that produced it,
      // so a renamed control can name itself (Wave 47 lane A25).
      click: (selector: string) => {
        const el = document.querySelector<HTMLElement>(selector);
        if (!el) return { kind: "absent" as const, selector };
        el.click();
        return { kind: "clicked" as const };
      },
      copy: (text: string) => {
        const done = () => true;
        void navigator.clipboard?.writeText(text).then(done, done);
        return true;
      },
    }),
    [pathname, router],
  );

  /** The last press, so a debugger can read what a chord did without a
   * console: rendered as `[data-testid="shortcut-last-outcome"]`. */
  const [lastOutcome, setLastOutcome] = useState<{ command: string; outcome: CommandOutcome } | null>(
    null,
  );

  const dispatchChord = useCallback(
    (event: KeyboardEvent) => {
      if (isEditableTarget(event.target)) return;
      // The platform decides which modifier "mod" means: ⌘ on macOS, Ctrl
      // everywhere else. A pinned `true` meant a Ctrl chord could never fire
      // off macOS, and the `?`/Alt logic below already reads the real key.
      const spec = chordFromEvent(event, modIsMeta());
      if (!spec) return;
      const command = commandForChord(spec, pathname);
      if (command) {
        event.preventDefault();
        event.stopPropagation();
        // One place turns a press into a trace: the outcome is reported
        // (console + DOM event + failure memory) and mirrored into the DOM,
        // so a dead chord is loud where it happens instead of in a
        // downstream timeout. Reporting never throws — see outcome.ts.
        const outcome = command.run(ctx);
        setLastOutcome({ command: command.id, outcome });
        reportCommandOutcome({ command: command.id, pathname, outcome });
      }
    },
    [ctx, pathname],
  );

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      // The cheat-sheet toggle is profile-independent: read-only reference.
      if (
        event.key === "?" &&
        !event.altKey &&
        !event.metaKey &&
        !event.ctrlKey &&
        !isEditableTarget(event.target)
      ) {
        sheetTriggerRef.current = event.target;
        event.preventDefault();
        setSheetOpen(true);
        return;
      }
      if (!shortcutsOn) return;
      // Hold-Alt overlay: appear while Alt is down, dismiss on release.
      if (event.key === "Alt" && !event.repeat) {
        setOverlayOpen(true);
        return;
      }
      dispatchChord(event);
    };    const onKeyUp = (event: KeyboardEvent) => {
      if (event.key === "Alt") setOverlayOpen(false);
    };
    const onBlur = () => setOverlayOpen(false);
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("blur", onBlur);
    };
  }, [dispatchChord, shortcutsOn]);

  return (
    <>
      {authed && profile !== null && (
        <span data-testid="shortcut-profile-ready" data-profile={profile} hidden aria-hidden="true" />
      )}
      {/* Last press, readable from the DOM. `data-status` is the policy line:
          "ran" | "inapplicable" (declared absence) | "failed" (a promised
          control was missing, with the selector attached). */}
      {lastOutcome !== null && (
        <span
          data-testid="shortcut-last-outcome"
          data-command={lastOutcome.command}
          data-status={lastOutcome.outcome.status}
          data-selector={
            lastOutcome.outcome.status === "ran" ? undefined : lastOutcome.outcome.selector
          }
          hidden
          aria-hidden="true"
        >
          {lastOutcome.outcome.status === "ran" ? "" : lastOutcome.outcome.reason}
        </span>
      )}
      {overlayOpen && <ShortcutOverlay commands={surfaceCommands} metaRows={metaRowsList} />}
      <CheatSheetDialog
        open={sheetOpen}
        onClose={() => {
          setSheetOpen(false);
          const restore = sheetTriggerRef.current;
          if (restore instanceof HTMLElement) restore.focus();
        }}
      />
    </>
  );
}
