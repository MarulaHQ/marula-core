// DesktopUpdateStrip — the update-available seam for the Tauri shell
// (Wave 34 lane 6, `p34-open-gaps-collector`).
//
// Purpose: one slim strip under the top bar that appears ONLY when the
// desktop shell reports an available update; it offers the explicit
// apply ("Restart & update") and a session dismiss. A background check
// that finds nothing, cannot run, or fails stays silent — an update
// notice must never be a nag.
//
// Non-purpose: no version math, no channel config, no download UI — the
// shell owns the check/apply (lib/desktop/updater is the typed wire).

"use client";

import { Button } from "@meridian/ui";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  checkDesktopUpdate,
  installDesktopUpdate,
  type DesktopUpdateInfo,
} from "../../lib/desktop/updater";
import { anyWindowDirty, answerRestartChecks } from "../../lib/popout/window-bus";
import { popoutDirtyFlag } from "./popout-shell";

type StripState =
  /** Nothing to say (no update, not the desktop shell, check failed). */
  | { kind: "hidden" }
  | { kind: "available"; update: DesktopUpdateInfo }
  | { kind: "installing"; update: DesktopUpdateInfo }
  | { kind: "failed"; update: DesktopUpdateInfo; message: string };

export function DesktopUpdateStrip() {
  const [state, setState] = useState<StripState>({ kind: "hidden" });
  const [dismissed, setDismissed] = useState(false);
  const alive = useRef(true);

  // Answer sibling restart surveys with THIS window's dirty state (the
  // popout close guard's flag — one truth per window).
  useEffect(() => answerRestartChecks(() => popoutDirtyFlag()), []);

  useEffect(() => {
    alive.current = true;
    void (async () => {
      const result = await checkDesktopUpdate();
      if (!alive.current) return;
      if (result.kind === "available") {
        setState({ kind: "available", update: result.update });
      } else if (result.kind === "error") {
        // Silent in the UI: support reads the console line. A failed
        // background check is not an update notice.
        console.warn("desktop update check failed:", result.message);
      }
    })();
    return () => {
      alive.current = false;
    };
  }, []);

  const install = useCallback(async () => {
    setState((prev) =>
      prev.kind === "available" ? { kind: "installing", update: prev.update } : prev,
    );
    // Multi-window honesty (Wave 38 lane 2): before the shell restarts,
    // ask EVERY window whether it still has unsaved edits (the save-bar
    // state — one truth per window). Any dirty window blocks the restart:
    // the strip flips back with the reason. No window is killed silently.
    const survey = await anyWindowDirty();
    if (survey.dirty) {
      if (!alive.current) return;
      setDismissed(false);
      setState((prev) =>
        prev.kind === "installing"
          ? { kind: "failed", update: prev.update, message: "A Meridian window still has unsaved edits — save or discard them, then restart the update." }
          : prev,
      );
      return;
    }
    const result = await installDesktopUpdate();
    if (!alive.current) return;
    if (result.kind === "error") {
      // A failure outranks an earlier hide: the user gets the notice back.
      setDismissed(false);
      setState((prev) =>
        prev.kind === "installing"
          ? { kind: "failed", update: prev.update, message: result.message }
          : prev,
      );
    }
    // "restarting": the shell relaunches the app, so the installing state
    // is the last thing this window renders.
  }, []);

  if (dismissed || state.kind === "hidden") return null;

  return (
    <div
      data-testid="desktop-update-strip"
      role="status"
      aria-live="polite"
      className="flex min-h-9 items-center gap-3 border-b px-4 py-1.5 text-xs"
      style={{
        borderColor: "var(--m-border)",
        background: "var(--m-bg-subtle)",
        color: "var(--m-text-secondary)",
      }}
    >
      {state.kind === "available" ? (
        <>
          <span className="min-w-0 flex-1 truncate">
            {`Meridian ${state.update.version} is ready to install — you're on ${state.update.currentVersion}.`}
          </span>
          <Button
            size="sm"
            variant="primary"
            onClick={() => void install()}
            data-testid="desktop-update-install"
          >
            Restart &amp; update
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setDismissed(true)}
            data-testid="desktop-update-dismiss"
            aria-label="Dismiss update notice"
          >
            Later
          </Button>
        </>
      ) : null}

      {state.kind === "installing" ? (
        <>
          <span className="min-w-0 flex-1 truncate" data-testid="desktop-update-installing">
            {`Installing Meridian ${state.update.version} — the app restarts when it's done.`}
          </span>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setDismissed(true)}
            data-testid="desktop-update-hide"
            aria-label="Hide update notice"
          >
            Hide
          </Button>
        </>
      ) : null}

      {state.kind === "failed" ? (
        <>
          <span
            className="min-w-0 flex-1 truncate"
            data-testid="desktop-update-failed"
            title={state.message}
          >
            {`Update failed — ${state.message}`}
          </span>
          <Button size="sm" variant="secondary" onClick={() => void install()}>
            Try again
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setDismissed(true)}
            aria-label="Dismiss update notice"
          >
            Later
          </Button>
        </>
      ) : null}
    </div>
  );
}
