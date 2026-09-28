# How to use multiple windows

> **Audience:** both — anyone with a second monitor who wants the dispatch
> board on one screen and the ticket on the other.

## Prerequisites

- A signed-in session in the browser app.
- A pop-up-permissive browser. If your browser blocks the new window, the
  app tells you right away and offers a direct link — it never fails
  silently.

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

## Troubleshooting

- **Nothing opened:** look for the inline "pop-up blocked" hint where you
  clicked; it carries a direct link to the same window.
- **The popout shows the login page:** your session expired in the main
  window too. Sign in once there; the popout picks it up.
