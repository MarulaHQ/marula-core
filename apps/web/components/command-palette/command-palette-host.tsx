// The palette host — the one mount point for the palette and the chat panel.
//
// Purpose: render both overlays, read the general_chat grant once, derive the
// screen context, and own the global Cmd/Ctrl+K chord. The chord runs in the
// capture phase on purpose: the command registry still advertises
// `palette.search` with mod+k, and letting both fire would open this overlay
// and then navigate to /search underneath it. Capture stops that one chord
// from reaching the registry; every other chord is untouched.
//
// Non-purpose: no route state and no page hooks. Pages that want to feed the
// context chip call `useScreenContextRegistration` from their own code.

"use client";

import { useEffect } from "react";
import { useAuth } from "../../lib/api";
import { ChatPanel } from "./chat-panel";
import { CommandPalette } from "./command-palette";
import { useGeneralChatGrant } from "./grant";
import { useScreenContext } from "./screen-context";
import { actions, getSnapshot } from "./store";

export function CommandPaletteHost() {
  const auth = useAuth();
  const grant = useGeneralChatGrant();
  const context = useScreenContext();

  // The store outlives a sign-out, so a different account in the same tab
  // would otherwise open someone else's transcript. Clear it when the
  // signed-in user changes, and when the session ends.
  const userId = auth.me?.user_id ?? null;
  useEffect(() => {
    actions.resetSession();
  }, [userId]);

  useEffect(() => {
    if (auth.status !== "authed") return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.altKey) return;
      if (!(event.metaKey || event.ctrlKey)) return;
      if (event.key.toLowerCase() !== "k") return;
      event.preventDefault();
      event.stopImmediatePropagation();
      if (getSnapshot().open) actions.close();
      else actions.open("search");
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [auth.status]);

  return (
    <>
      <CommandPalette grant={grant} context={context} />
      <ChatPanel grant={grant} context={context} />
    </>
  );
}
