# How to get help inside Marula

> **Audience:** both — every signed-in person gets the same help panel;
> admins additionally see the setup and policy guides for their surfaces.

## Prerequisites

- A signed-in session. The help panel lives in the staff top bar, so it
  appears on every normal app surface (auth screens, the client portal,
  and shared links don't carry it).

## Open the help panel

1. Press the **?** button in the top bar, right of the theme toggle. You
   can also reach it with the keyboard: Tab to it, or just look for the
   question mark at the top right of any page.
2. The panel opens on the right and lists the guides for the surface you
   are on. Standing on `/tickets` shows the ticket guides; standing on
   `/settings/fx` shows the FX rate-book guide.

Every entry is a page from the public docs site — the same markdown that
lives at [getmarula.com/docs](https://getmarula.com/docs). Nothing is
written twice: when a docs page moves, the app's index is re-checked in
CI, so a stale entry fails the build instead of sending you to a 404.

## Search all guides

Type in the panel's search box to search every guide title, not just the
ones for the current surface. Results are ranked with the most recently
updated page first.

## Open the full docs site

Each entry has an **Open in docs** link that jumps to the published page.
Use it when you want the complete page with its prerequisites and related
links — the panel entry is the pointer, the docs site is the page.

## Rate a guide

Under each entry there is a small "Was this helpful?" pair. Your answer
is stored with your account settings and reviewed when we curate the
docs. Nothing is sent anywhere else, and admins cannot see who answered
what.

## Turn feature tours off

First visits to some surfaces highlight what matters (the work bar, the
decision cards, the merge workbench) with a short, skippable tour. If
you would rather never see them:

- **Just you:** Settings → Preferences → "Don't show me tours".
- **Whole org:** Settings → Onboarding → "Disable feature tours
  (org-wide)" (admins).

Tours respect your system reduced-motion setting: with reduced motion on
they appear without the highlight animation. Guides from the help panel
always stay available, tours or not.

## Troubleshooting

- **The ? button is missing.** You are on a surface without the staff
  shell (sign-in, portal, a shared link). Open the app proper first.
- **A guide link is wrong or missing.** If you self-host, check that you
  are on a current release; the help index is rebuilt with each release.
- **A tour replayed after I finished it.** Tours remember completion per
  account. If one replays, confirm you are signed in as the same user,
  then finish it once more — the state is stored server-side.
