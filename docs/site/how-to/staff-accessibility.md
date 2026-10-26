---
description: "Marula is built to be usable without a mouse. You can run a ticket from the keyboard end to end, hear what changes through a screen reader, and pick a theme…"
---

# How to use Marula with a keyboard, a screen reader, and themes

> **Audience:** both — technicians and admins share the same surfaces, and
> every control described here works the same on either side.

Marula is built to be usable without a mouse. You can run a ticket from
the keyboard end to end, hear what changes through a screen reader, and
pick a theme that you can actually read. This page covers all three.

## Keyboard

Two shortcuts answer most questions:

- **Alt (hold)** shows an overlay listing the shortcuts that work on the
  surface you are on. Release Alt and it goes away. Nothing moves and
  nothing steals focus while it is up.
- **?** (Shift and the question-mark key) opens the full shortcut cheat
  sheet, grouped by scope: what works everywhere, what works on tickets,
  and so on. **Escape** closes it.

A few more worth memorising: **Ctrl+K** (⌘K on a Mac) opens search from
anywhere, **Alt+X** closes the cheat sheet from the keyboard alone, and
the shortcut profile on **Settings → Shortcuts** can turn groups off if a
chord fights your muscle memory. Turning shortcuts off never hides their
documentation; the cheat sheet stays reachable with the `all off` profile.

Every dialog traps focus while it is open and hands it back to the button
you came from when it closes. **Escape** closes any of them. If you tab
past the top of the page, a "Skip to main content" link appears; it jumps
straight past the navigation rail and header.

## Screen reader

Async changes are announced, so you do not have to hunt for them:

- Assistant answers on the agent panel read out their first line when the
  reply lands, not every token while it streams.
- Decision suggestions say how many are waiting when the card loads.
- The settings save bar confirms "Saved." out loud, and a failed save is
  announced assertively so it is not missed.

Dialogs are marked modal, so screen readers announce them as such and
keep reading order inside them. Form errors are wired to their fields,
and icon-only controls carry text labels the reader speaks. Loading
surfaces that take a while show what they are doing, and that status text
is readable too.

## Themes

**Settings → Appearance** holds the theme presets (`Marula` neutral, `Marula Warm`, or `High contrast`), each available in light and dark mode. You can also choose your preferred layout density (**Comfortable**, **Compact**, or **Dense**). Every built-in preset passes WCAG AA contrast (`4.5:1` for text, checked automatically in CI), so pick the combination that reads best for your eyes. Admins can set a default theme preset for the whole organization from the same page, while your personal override always takes priority for your account.

If you need maximum legibility, switch to the **High contrast** preset. Custom theme packs uploaded by an admin are checked against the same contrast floors before they can be installed.

## If something is in the way

- A color pair that is hard to read: switch presets first. If the problem
  survives on High contrast, it is a bug worth reporting.
- A control you cannot reach by keyboard: press **?** to check whether a
  chord exists for it, then report the surface if none does.
- Motion that bothers you: the app honors your system "reduce motion"
  setting. Entrance animations collapse to plain fades under it.
