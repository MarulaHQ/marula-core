// Shell chrome visibility — lets a pushed screen take the tab bar and the
// timer pill away for itself.
//
// Purpose: the pack's rule that a TICKET replaces the tab bar and the pill
// with its own bottom action bar. The tabs layout owns both surfaces, so the
// screen that needs the room asks for it here and returns it on unmount.

import { createContext, useContext } from "react";

export type ShellChrome = {
  /** Hide the timer pill and the tab bar (a ticket's action bar replaces both). */
  setVisible: (visible: boolean) => void;
  visible: boolean;
};

export const ShellChromeContext = createContext<ShellChrome>({
  setVisible: () => {},
  visible: true,
});

export function useShellChrome(): ShellChrome {
  return useContext(ShellChromeContext);
}
