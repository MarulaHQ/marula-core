// usePopoutDirtyGuard — save-bar semantics for popout close (Wave 38).
//
// Purpose: a popout window with unsaved edits confirms before closing —
// the same honest guard the in-app save bars give, never silent data loss.
// Wire it wherever a popout-capable surface owns a dirty editor:
//
//   const dirty = useEditorDirtyState(...);
//   usePopoutDirtyGuard(dirty);
//
// Non-purpose: no autosave, no diffing — the caller owns dirtiness.

"use client";

import { useEffect } from "react";
import { setPopoutDirty } from "../../components/shell/popout-shell";

export function usePopoutDirtyGuard(dirty: boolean): void {
  useEffect(() => {
    setPopoutDirty(dirty);
    return () => setPopoutDirty(false);
  }, [dirty]);
}
