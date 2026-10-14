# How to use the keyboard shortcuts

> **Audience:** both — every signed-in staff member (owners, dispatchers,
> technicians, bookkeepers) can drive the app from the keyboard. Shortcut
> settings apply to your personal account only.

You can navigate boards, create tickets, copy record links, and search across Marula without taking your hands off the keyboard. If you forget a shortcut, hold down `Alt` anywhere outside a text box to see an on-screen guide for the page you are viewing.

## Prerequisites

- A signed-in session on the web or desktop app. New to Marula? Walk through [Your first ticket](../tutorials/first-ticket.md) on the seeded demo org (`owner@demo.local` / `demo-password-123`).
- A physical keyboard. On touchscreens, every action has a matching button on screen.

## The activation key: Alt

Marula uses `Alt` plus a letter or number (`Alt+N`, `Alt+1`, etc.) for workspace actions. Standard browser and operating system shortcuts (`Ctrl+T`, `Ctrl+W`, `Cmd+Q`, `Alt+Tab`, and browser back/forward) are never overridden. The one exception is the global search palette, which keeps the familiar `Cmd+K` / `Ctrl+K` shortcut.

## Two ways to check available shortcuts

1. **Hold `Alt` for the live page overlay:** Press and hold `Alt` anywhere on a page (when your cursor is not inside a text input). A small overlay lists every shortcut active on the current screen, along with its key combination and scope badge (`Global` or current surface). Let go of `Alt` to hide it.
2. **Press `?` for the full cheat sheet:** Press `?` (`Shift+/`) to open a reference dialog listing every keyboard command across the entire app, grouped by **Everywhere**, **Navigation**, and **On specific surfaces**. Press `Escape` or `Alt+X` to close it. The `?` cheat sheet still opens even if you disable active shortcuts in your preferences.

## Built-in keyboard shortcuts

| Shortcut | Action | Where it works |
|---|---|---|
| `Alt+N` | Create a new record (for example, a new client or a new ticket) | Any screen with a create button |
| `Alt+1` … `Alt+7` | Jump directly to a left-navigation section | Everywhere |
| `Cmd+K` / `Ctrl+K` | Open the command palette and unified search | Everywhere |
| `Alt+J` | Cycle to the next sub-page within the current navigation section | Everywhere |
| `Alt+C` | Copy a direct link to the open record | Ticket and client detail pages (plus device detail) |
| `Alt+X` | Close the topmost dialog or overlay | Everywhere |
| `?` | Open the full keyboard shortcut cheat sheet | Everywhere (even when shortcuts are turned off) |

Each shortcut always performs the same kind of action. `Alt+N` always means "create new" on both the client directory and a service board.

## Keyboard shortcuts respect your permissions

Every keyboard shortcut triggers the exact same action as clicking the button on screen:

- `Alt+N` opens the same creation dialog as clicking **New client** or **New ticket**. If a page has no create button, `Alt+N` does nothing and is hidden from the `Alt` overlay.
- Role permissions and disabled buttons apply identically for both mouse clicks and keyboard shortcuts.

## Turning keyboard shortcuts off

If a shortcut conflicts with assistive software or your personal workflow, you can turn shortcuts off for your account under **Settings → Shortcuts → Keyboard shortcuts profile**:

- **Default:** All `Alt` shortcuts and the hold-`Alt` overlay are active.
- **All off:** Disables `Alt` shortcuts and the hold-`Alt` overlay for your user account (handy during screen shares or presentations). Pressing `?` still opens the read-only cheat sheet.

## Where to go from here

- Customizing the board columns and statuses you jump between: [How to set workflow categories](admin-workflow-categories.md).
- Screen reader announcements and high-contrast themes: [How to use Marula with a keyboard, a screen reader, and themes](staff-accessibility.md).
- Daily ticket workflow: [How to work a ticket](staff-work-a-ticket.md).
