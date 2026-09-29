# How to use multiple windows

> **Audience:** both — anyone with a second monitor who wants the dispatch
> board on one screen and the ticket on the other.

## Prerequisites

- A signed-in session in the browser app, or the Marula desktop app.
- On the web, a pop-up-permissive browser. If your browser blocks the new
  window, the app tells you right away and offers a direct link — it never
  fails silently.

## 1. Pop a surface out

Surfaces that support this carry a **Pop out** control in context:

- **Ticket detail:** the ⧉ button in the ticket header, next to Classic.
- **Boards:** the card's **Move ▾** menu has "Pop out ticket".
- **Command palette (Cmd/Ctrl+K):** "Pop out ticket" (Alt+P) on a ticket,
  "Pop out surface" (Alt+Shift+P) on the dispatch board, stock lookup, or
  decisions.

The window that opens is a compact shell: no navigation rail, a slim title
bar, and the workbar. It is a full citizen — shortcuts work in it, the
command palette opens in it (Alt+P works there too), and it keeps its own
arrangement.

## 2. Work across windows honestly

Every window is a live peer. Post a note in the popout and it appears in
the main window the moment it lands — no refresh, no stale mirror. The
same goes the other way, and both windows stay in sync through the
realtime stream even when the popout was opened hours ago.

Sign-in is shared: sign out in one window and every window signs out; sign
back in once and they all come back.

## 3. Layout is remembered per window role

Panel arrangements persist per window role — a ticket workspace you popped
out keeps its own split on screen 2, and the main window keeps its own.
Move the popout to another monitor, resize, re-arrange: next time you pop
the same surface out, it comes back the way you left it.

## 4. Closing is safe

Closing a popout with unsaved edits asks first, the same way the in-app
save bar does. Closed windows are cleaned up: no ghost presence on
tickets, and notifications route to whichever window already shows the
target instead of opening a duplicate.

## In the desktop app

The desktop app's windows are real OS windows, so a few things work the
way your other Mac apps do:

- **Same pop-outs, native windows.** Every Pop out control and palette
  command opens a real app window. Alt+D opens the dispatch board window,
  Alt+S opens stock lookup, from anywhere in the app.
- **Positions stick.** Each window role remembers where you put it and how
  big it was. If a monitor is unplugged overnight, windows come back
  on-screen — nothing hides on a display that is no longer there.
- **Window menu.** Minimize and zoom live in the Window menu, along with
  bring-all-to-front, like every Mac app. Tabs do not: Marula windows
  stay separate, because separate windows are the point.
- **Tray.** The menu-bar icon opens a new ticket window, the dispatch
  board, or stock lookup without touching the main window.
- **Links open the right window.** Scanning or opening a `marula://`
  link (from a QR label, say) brings forward the window for that ticket or
  stock item — or opens a new one if no window has it.
- **One app at a time.** Launching Marula a second time just brings the
  running app forward.
- **Updates wait for your work.** If any window still has unsaved edits,
  "Restart & update" tells you which situation you are in instead of
  restarting under you. Save, then run the update again.

## Troubleshooting

- **Nothing opened:** look for the inline "pop-up blocked" hint where you
  clicked; it carries a direct link to the same window. (The desktop app
  has no pop-up blocker to fight — its windows always open.)
- **The popout shows the login page:** your session expired in the main
  window too. Sign in once there; the popout picks it up.
