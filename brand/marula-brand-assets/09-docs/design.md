# Marula design spec

Version 0.1 (concept). Covers the marketing website, the web and desktop app, and the mobile app.

Marula is the agentic operating system for MSPs, shipped as one product in several editions (Core, Enterprise, Cloud). This file is the single source of truth for how it looks, feels and moves. If something here conflicts with a component's defaults, this file wins.

---

## 1. Principles

1. **Warm, not loud.** The palette is ripe fruit and bushveld, not neon SaaS. Colour is used with intent; most of every screen is calm neutrals.
2. **One screen, one job.** Techs juggle a lot. Every view should make the next action obvious and keep the rest quiet.
3. **The assistant is the special thing.** AI suggestions get the one inverted surface on a screen (ink on light mode, cream on dark mode). Nothing else uses that treatment.
4. **Friendly under pressure.** Copy, motion and colour should never add stress. No alarm-red unless something is genuinely broken.
5. **Accessible by default.** WCAG 2.2 AA is the floor, not the goal.

---

## 2. Stack

| Layer | Choice | Notes |
|---|---|---|
| Components | shadcn/ui | Source is copied into the repo, so we own it. Theme through CSS variables only; avoid editing component internals unless a pattern below requires it. |
| Primitives | Radix UI (recommended) or Base UI | See note below. |
| Styling | Tailwind CSS v4 | Tokens live in `globals.css` as CSS variables in OKLCH, mapped via `@theme inline`. |
| Theme authoring | tweakcn | The source theme is built and exported from tweakcn, then committed. Section 4.4 is the canonical export. |
| Icons | lucide-react | shadcn's default set. Outline style, 1.5px stroke, 16/20/24px. |
| Toasts | sonner (via shadcn) | |
| Command palette | cmdk (shadcn `Command`) | |
| Charts | shadcn Charts (Recharts) | Use `--chart-1` to `--chart-5`. |

**Radix vs Base UI.** shadcn/ui now supports both primitive libraries, and `shadcn init` defaults to Base UI unless you pass `-b radix`. Radix is not deprecated and shadcn ships every component for both. Decide once, record it in `components.json` (`style: "radix-*"` or `"base-*"`), and pin that flag in any scaffolding scripts so CI doesn't silently switch. Recommendation: Radix, for the larger ecosystem of shadcn blocks, unless the team prefers Base UI's newer combobox and menu APIs.

**Mobile.** This spec assumes the mobile app is React Native (Expo). If so, use a shadcn-style RN library on NativeWind so the same tokens and component names carry across; tokens in section 4 are platform-agnostic. If the mobile app ends up native Swift/Kotlin, export the same tokens as platform colour assets. **Open question:** confirm the mobile stack.

---

## 3. Brand

### 3.1 Name

- Written **Marula** in sentences; the wordmark is always lowercase **marula**.
- Editions: **Marula Core** (AGPL, self-hosted), **Marula Enterprise**, **Marula Cloud** (hosted). Never "Marula OSS" in public copy; "Core" is friendlier.
- Say "in Marula", not "on Marula" ("I changed the template in Marula").

### 3.2 Logo

- **Mark:** gold fruit (circle), bark stem, green leaf, cream highlight.
- **Lockup:** mark left of the wordmark, gap = 0.18 × mark height.
- **Clear space:** at least the fruit's radius on all sides.
- **Minimum size:** lockup 96px wide on screen; mark alone 16px (favicon version drops the leaf and highlight: gold dot on ink).
- **Colour versions:** full colour on white or cream; full colour with lime leaf on ink; one-colour ink on gold.
- **Don't:** recolour the fruit, outline it, add gradients or shadows, rotate it, or set the wordmark in any other typeface.

### 3.3 Edition colour cues

Ripeness signals the edition, used only in lockups, badges and pricing:

| Edition | Cue colour | Token |
|---|---|---|
| Core | Canopy green | `--edition-core` |
| Enterprise | Ripe amber | `--edition-enterprise` |
| Cloud | Bark grey | `--edition-cloud` |

---

## 4. Colour

### 4.1 Brand palette

| Name | Hex | OKLCH | Role |
|---|---|---|---|
| Marula gold | `#E8B83A` | `oklch(0.805 0.146 86.8)` | Hero accent, primary in dark mode |
| Ripe blush | `#D9822B` | `oklch(0.686 0.144 60.4)` | Warm accent, warnings, "due" states |
| Flesh cream | `#F6EFD9` | `oklch(0.952 0.030 92.3)` | Soft surfaces, dark-mode text |
| Canopy green | `#5E7F3A` | `oklch(0.555 0.104 130.3)` | Success, Core edition |
| Unripe lime | `#A9C25D` | `oklch(0.774 0.131 121.1)` | Success on dark, highlights |
| Bushveld sand | `#CDB38B` | `oklch(0.779 0.062 78.3)` | Borders, dividers, illustration |
| Bark grey | `#7C746A` | `oklch(0.563 0.018 73.5)` | Neutral mid-tone, Cloud edition |
| Dusk ink | `#2B2A33` | `oklch(0.290 0.016 290.4)` | Text, dark-mode base |

### 4.2 Rules

- **Gold is never text on light backgrounds** (1.7:1 on cream, fails). On light mode, use Dusk ink for text and "deep amber" `#9A5B0F` where gold-coloured text is needed (5.4:1 on white).
- Gold text on Dusk ink is excellent (7.7:1), which is why dark mode is gold-forward.
- Status colours always pair with a label or icon, never colour alone.
- Red (`--destructive`) is reserved for destructive actions and genuine failures, not for "overdue".
- Green means done, healthy or alive, never an action. Cream or white text on green fills below 24px uses deep canopy `#4A6A2A` (5.4:1), not `#5E7F3A` (3.99:1).
- Form control borders use `--input` (3:1 against their background); decorative dividers use `--border`.
- Disabled controls use `muted` fill with `muted-foreground` text rather than 50% opacity, so they stay readable.

### 4.3 Semantic mapping

| Token | Light | Dark |
|---|---|---|
| `background` | `#FBF7EC` | `#2B2A33` |
| `foreground` | `#2B2A33` | `#F6EFD9` |
| `card` / `popover` | `#FFFFFF` | `#34333D` |
| `primary` | `#2B2A33` (ink) | `#E8B83A` (gold) |
| `primary-foreground` | `#E8B83A` | `#2B2A33` |
| `secondary` | `#F3EAD2` | `#3A3944` |
| `muted` | `#F3EAD2` | `#34333D` |
| `muted-foreground` | `#5F584F` (6.6:1) | `#BEB5A2` (7.0:1) |
| `accent` | `#F7E3A8` | `#3A3944` |
| `accent-foreground` | `#6B4308` (6.8:1) | `#E8B83A` |
| `destructive` | `#B42318` | `#FB8A80` |
| `border` | `#E6DCC2` | `#43424D` |
| `input` (form control borders, 3:1) | `#857A66` | `#8A8378` |
| `ring` | `#A8641A` | `#E8B83A` |
| `sidebar` | `#F3EAD2` | `#222129` |

The primary flips between modes on purpose: an ink button with gold text on light, a gold button with ink text on dark. Both exceed 7:1.

### 4.4 `globals.css` (tweakcn export format)

```css
:root {
  --radius: 0.75rem;

  --background: oklch(0.976 0.015 90.2);
  --foreground: oklch(0.290 0.016 290.4);
  --card: oklch(1 0 0);
  --card-foreground: oklch(0.290 0.016 290.4);
  --popover: oklch(1 0 0);
  --popover-foreground: oklch(0.290 0.016 290.4);
  --primary: oklch(0.290 0.016 290.4);
  --primary-foreground: oklch(0.805 0.146 86.8);
  --secondary: oklch(0.938 0.033 89.9);
  --secondary-foreground: oklch(0.290 0.016 290.4);
  --muted: oklch(0.938 0.033 89.9);
  --muted-foreground: oklch(0.464 0.017 74.2);
  --accent: oklch(0.918 0.079 91.5);
  --accent-foreground: oklch(0.420 0.087 69.4);
  --destructive: oklch(0.500 0.182 29.5);
  --border: oklch(0.896 0.036 89.5);
  --input: oklch(0.584 0.032 82.5);
  --ring: oklch(0.568 0.120 62.0);

  --chart-1: oklch(0.805 0.146 86.8);
  --chart-2: oklch(0.555 0.104 130.3);
  --chart-3: oklch(0.686 0.144 60.4);
  --chart-4: oklch(0.563 0.018 73.5);
  --chart-5: oklch(0.774 0.131 121.1);

  --sidebar: oklch(0.938 0.033 89.9);
  --sidebar-foreground: oklch(0.290 0.016 290.4);
  --sidebar-primary: oklch(0.290 0.016 290.4);
  --sidebar-primary-foreground: oklch(0.805 0.146 86.8);
  --sidebar-accent: oklch(0.918 0.079 91.5);
  --sidebar-accent-foreground: oklch(0.420 0.087 69.4);
  --sidebar-border: oklch(0.896 0.036 89.5);
  --sidebar-ring: oklch(0.568 0.120 62.0);

  /* Marula extensions */
  --success: oklch(0.484 0.099 131.4);
  --success-soft: oklch(0.930 0.046 119.3);
  --warning: oklch(0.434 0.101 54.8);
  --warning-soft: oklch(0.910 0.048 67.2);
  --gold-text: oklch(0.532 0.116 63.2);
  --edition-core: oklch(0.555 0.104 130.3);
  --edition-enterprise: oklch(0.568 0.120 62.0);
  --edition-cloud: oklch(0.464 0.017 74.2);
  --assistant: oklch(0.290 0.016 290.4);
  --assistant-foreground: oklch(0.952 0.030 92.3);

  --font-sans: "Figtree", ui-sans-serif, system-ui, sans-serif;
  --font-serif: "Fraunces", ui-serif, Georgia, serif;
  --font-mono: "JetBrains Mono", ui-monospace, monospace;
}

.dark {
  --background: oklch(0.290 0.016 290.4);
  --foreground: oklch(0.952 0.030 92.3);
  --card: oklch(0.326 0.018 290.0);
  --card-foreground: oklch(0.952 0.030 92.3);
  --popover: oklch(0.326 0.018 290.0);
  --popover-foreground: oklch(0.952 0.030 92.3);
  --primary: oklch(0.805 0.146 86.8);
  --primary-foreground: oklch(0.290 0.016 290.4);
  --secondary: oklch(0.350 0.019 289.5);
  --secondary-foreground: oklch(0.952 0.030 92.3);
  --muted: oklch(0.326 0.018 290.0);
  --muted-foreground: oklch(0.775 0.028 85.7);
  --accent: oklch(0.350 0.019 289.5);
  --accent-foreground: oklch(0.805 0.146 86.8);
  --destructive: oklch(0.754 0.139 26.2);
  --border: oklch(0.384 0.019 289.7);
  --input: oklch(0.613 0.018 79.3);
  --ring: oklch(0.805 0.146 86.8);

  --sidebar: oklch(0.252 0.015 291.0);
  --sidebar-foreground: oklch(0.952 0.030 92.3);
  --sidebar-primary: oklch(0.805 0.146 86.8);
  --sidebar-primary-foreground: oklch(0.290 0.016 290.4);
  --sidebar-accent: oklch(0.350 0.019 289.5);
  --sidebar-accent-foreground: oklch(0.805 0.146 86.8);
  --sidebar-border: oklch(0.384 0.019 289.7);
  --sidebar-ring: oklch(0.805 0.146 86.8);

  --success: oklch(0.774 0.131 121.1);
  --success-soft: oklch(0.407 0.078 130.4);
  --warning: oklch(0.686 0.144 60.4);
  --warning-soft: oklch(0.420 0.087 69.4);
  --gold-text: oklch(0.805 0.146 86.8);
  --assistant: oklch(0.952 0.030 92.3);
  --assistant-foreground: oklch(0.290 0.016 290.4);
}

@theme inline {
  --color-success: var(--success);
  --color-success-soft: var(--success-soft);
  --color-warning: var(--warning);
  --color-warning-soft: var(--warning-soft);
  --color-gold-text: var(--gold-text);
  --color-assistant: var(--assistant);
  --color-assistant-foreground: var(--assistant-foreground);
  --color-edition-core: var(--edition-core);
  --color-edition-enterprise: var(--edition-enterprise);
  --color-edition-cloud: var(--edition-cloud);
}
```

The standard shadcn `@theme inline` mappings (`--color-background: var(--background)` and so on) stay as generated; only the Marula extensions above are added. To iterate on the theme, paste this into tweakcn's import, adjust, and export back over this block.

---

## 5. Typography

| Role | Typeface | Usage |
|---|---|---|
| Display | Fraunces (500) | Wordmark, marketing headlines, page greetings, slide titles. Never in dense UI. |
| Interface | Figtree (400 / 500 / 600) | Everything in the product. |
| Mono | JetBrains Mono | Code, scripts, log output, IDs. |

All three are on Google Fonts; self-host via `next/font` (web) or bundle with Expo Font (mobile).

**Scale (web app, px / line height):** 12/16 caption, 14/20 small and table text, 16/24 body, 18/28 lead, 20/28 H4, 24/32 H3, 30/36 H2, 36/44 H1 (Fraunces for H1 and page greetings only).

**Scale (marketing):** body 18/30, H2 44/52 Fraunces, hero 72 to 96 Fraunces with -0.03em tracking.

**Scale (mobile):** body 16/24 minimum, secondary text 13px minimum, tab bar labels 12px minimum, headings 20 to 28, respect Dynamic Type / font scaling up to 200%.

Numbers that tick (timers, totals) use `font-variant-numeric: tabular-nums`.

---

## 6. Space, shape, elevation

- **Spacing:** Tailwind's 4px base. Card padding 20 to 24, page gutters 32 to 36 on desktop, 16 to 20 on mobile.
- **Radius:** `--radius: 0.75rem` (12px). Buttons and inputs `rounded-lg` (10 to 12px), cards `rounded-2xl` (16px), pills `rounded-full`.
- **Elevation:** flat by default; separation comes from surface colour and 1px borders. Shadows only for floating layers (popover, dropdown, dialog): `0 8px 24px oklch(0.29 0.016 290 / 0.12)`.
- **Touch targets:** 44 × 44 minimum everywhere, including desktop icon buttons.

---

## 7. Motion

- Default easing `cubic-bezier(0.65, 0, 0.35, 1)`; springy moments `cubic-bezier(0.34, 1.56, 0.64, 1)`.
- Durations: 120ms micro (hover, press), 200ms small (popover, toggle), 300ms medium (sheet, dialog), 800ms max for brand moments.
- **Logo pop-in** (splash, login, first load; roughly 1.9s total):
  1. 0 to 750ms: fruit drops about 1.4 × its height from above, squashes to 1.14 × 0.86 on landing, rebounds to 0.94 × 1.06, settles.
  2. 500ms: stem grows (scaleY 0 to 1, 250ms).
  3. 600ms: leaf springs out from its base (scale 0 to 1, rotate -40° to 0°, 550ms).
  4. 850ms: highlight fades in to 75% (400ms).
  5. 1100ms: mark slides from centre to its lockup position (800ms), letters rise 24px with opacity, staggered 70ms from 1250ms.
- Honour `prefers-reduced-motion` (and the OS setting on mobile): skip straight to the final state.
- Never animate the logo inside the running app chrome; only on entry moments.

---

## 8. Components and patterns

**Buttons.** `default` = primary (ink/gold in light, gold/ink in dark). `secondary` for supporting actions. `outline` uses `--border`. `destructive` only for delete/revoke. One primary button per view region.

**Status badges.** Always text plus colour:

| Status | Light (bg / text) | Dark (bg / text) |
|---|---|---|
| In progress | `#FBEBBF` / `#6B4308` | `#4A3A1A` / `#F2CD6B` |
| Due today | `#F8DCC0` / `#7A3F0C` | `#4D2F14` / `#F0B27A` |
| Waiting on client | `#F3EAD2` / `#5C4631` | `#3A342A` / `#E2CFA9` |
| Resolved | `#D5E3C5` / `#2F4A1E` | `#26361A` / `#BFD99A` |
| Scheduled | `#ECE7DF` / `#4A443D` | `#3F3D45` / `#D8CFBA` |
| Failed / breached | `destructive` soft / `destructive` | same pattern |

**Assistant card.** The only inverted surface on a screen (`bg-assistant text-assistant-foreground`). Always shows what the agent did or proposes, and always offers a human decision (Approve / Edit / Dismiss). Never auto-commit client-facing output from this card without approval.

**Timer.** Large tabular numerals in `--gold-text`. The running timer is persistent (sidebar footer on desktop, pinned bar on mobile) so it's never more than a glance away.

**Tables.** 14px text, 48px rows, sticky header, zebra striping off; hover row uses `muted`.

**Sidebar.** shadcn `Sidebar`. Active item uses `sidebar-accent`. Edition badge sits at the bottom.

**Command palette.** `Cmd/Ctrl + K` everywhere in the app. The assistant is reachable from it.

**Empty states.** A small line illustration (fruit, leaf or canopy in sand and bark tones), one sentence, one action.

---

## 9. Platform specs

### 9.1 Marketing website

- Framework-agnostic but assumed Next.js with the same `globals.css`.
- Light mode is the default presentation; respect the visitor's system theme.
- **Hero:** Fraunces headline (72 to 96px), one-sentence subhead in Figtree 20px, primary CTA ("Get Marula Core" or "Book a demo") plus secondary CTA, and a product screenshot in a soft cream frame. Gold appears as the accent, not the background.
- Max content width 1200px, 12-column grid, 24px gutters, 80px desktop margins.
- Sections alternate `background` and `secondary` surfaces; no gradients.
- Every claim with a number needs a real source. Use placeholders until then.
- Open-source messaging (Core, AGPL, self-hosted) gets its own section; it's a trust signal for MSP buyers.

### 9.2 Web and desktop app

- Same codebase for web and desktop (desktop wrapper assumed Tauri or Electron; **open question**).
- Layout: collapsible sidebar (232px expanded, 64px collapsed), content max 1440px, 3-column dashboard grid that collapses to 2 then 1.
- Dark mode is a first-class citizen and likely the most used; design and review every screen in both.
- Desktop specifics: native title bar integration, tray icon uses the ink-and-gold mark, system notifications for SLA and timer events, global shortcut to start or stop the timer.
- Density: a "compact" preference reduces row height to 40px and card padding to 16px.

### 9.3 Mobile app

- Primary use cases: see my day, start/stop timer, approve drafted time entries, respond to tickets on-site, get alerted.
- **Navigation:** bottom tab bar with 4 items (My day, Tickets, Time, More) plus a raised timer button in the centre. Tab icons 24px lucide.
- **Screens:** 390 × 844 reference frame (iPhone 14/15/16 base); test to 360dp on Android.
- Honour safe areas; no content under the home indicator.
- Timer shows as a pinned bar above the tab bar while running.
- Assistant approvals are swipeable cards: swipe right approve, left dismiss, tap to edit, with buttons as an accessible alternative.
- Haptics: light tick on timer start/stop and approval.
- Follow system theme; dark mode is gold-on-ink as on desktop.

---

## 10. Accessibility checklist

- Text contrast 4.5:1 (3:1 for 24px+ or 18.5px bold). All token pairs above are verified.
- Non-text UI contrast 3:1 (borders on inputs, focus rings, chart lines).
- Visible focus ring on every interactive element: 2px `--ring` with 2px offset.
- Everything reachable and operable by keyboard; primitives (Radix/Base UI) handle most of this, so don't rebuild dialogs or menus by hand.
- Status never conveyed by colour alone.
- Reduced motion respected on every platform.
- Mobile: support font scaling to 200% without clipping; VoiceOver and TalkBack labels on all icon buttons.

---

## 11. Voice and tone (short version)

- Plain, warm, a little wry. Written like a senior tech who's got your back.
- Say what happened and what to do next. "Couldn't reach the Graph API. Retry, or check the tenant's consent." Not "An error occurred."
- The assistant speaks in first person and always makes clear what it did versus what it's proposing.
- No hype words ("revolutionary", "supercharge"). No blame ("you failed to...").

---

## 12. Open questions

1. Primitive library: Radix or Base UI? Record in `components.json`.
2. Mobile stack: React Native (Expo) or native?
3. Desktop wrapper: Tauri or Electron?
4. Final trademark and linguistic clearance on the name (Kannada meaning, Amarula in SA).
5. Illustration style guide (commission once the name is locked).
