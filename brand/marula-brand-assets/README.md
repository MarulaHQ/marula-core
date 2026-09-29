# Marula brand assets

Version 0.1 (concept). Everything here was produced during the Marula identity work: the logo system, app icons, illustration, patterns, social assets, colour and type. Open `_overview.png` for a one-glance contact sheet.

Status to keep in mind before shipping anything:

- The name is not yet cleared. A formal trademark search (South Africa, US, EU), a check on the Amarula liqueur brand in South Africa, and a native-speaker check on the Kannada meaning of "marula" are still open.
- The Ndebele-inspired and shweshwe-inspired patterns are our own interpretations. If they go into production, commission Ndebele artists or license real shweshwe from its makers.

## What's inside

| Folder | Contents |
|---|---|
| `01-logo` | Mark, lockup, wordmark and fruit-dot in colour, on-dark and one-colour versions. SVG plus transparent PNGs. `edition-lockups` has Core, Self-Hosted, Cloud and Enterprise labels |
| `02-app-icons` | iOS, Android (adaptive, monochrome, legacy), web (favicon, touch icon, PWA, maskable) and desktop (Tauri or Electron, tray) |
| `03-illustrations` | Painted orchard (gouache), collage and midcentury posters, sun-baked street poster, spot illustrations (sheet plus six individual files), torn-paper overlays |
| `04-patterns-textures` | Marula pop repeat, talavera, zellige, Ndebele-inspired, two shweshwe-inspired prints, marula skin, cream and kraft paper |
| `05-social-and-web` | Avatars, LinkedIn, X, GitHub and YouTube banners, and the link-preview (Open Graph) image |
| `06-colour` | Palette (JSON, CSS, swatch sheet) and the shadcn/ui theme in tweakcn export format |
| `07-fonts` | Fraunces and Figtree variable fonts (Latin subset) with their licences |
| `08-motion` | The logo pop-in animation and its timing spec |
| `09-docs` | `design.md`, the full design spec |

## Logo

- Files include clear space. Every logo and mark file has padding equal to the fruit's radius built into the canvas. If you want the artwork only, use `01-logo/svg/tight`.
- The wordmark is outlined, so no font install is needed to use any logo file.
- Which file to use: `lockup-color` on white or cream, `lockup-color-on-dark` on ink, `lockup-mono-ink` on gold or any light colour, `lockup-mono-cream` or `lockup-mono-white` on dark colours and photos.
- Minimum size: lockup 96 px wide on screen. Mark alone 16 px, and below 32 px use `fruit-dot-gold` (the favicon version drops the leaf and highlight).
- Wordmark is always lowercase "marula", set in Fraunces Medium (500) with letter-spacing of -0.03 em. In sentences write "Marula".
- Don't recolour the fruit, outline it, add gradients or shadows, rotate it, or set the wordmark in another typeface.

## App icons

- iOS: `ios/AppIcon-1024-ink.png` is the default. It is opaque and square with no rounding, because iOS applies its own mask. The gold version is an alternate.
- Android: `ic_launcher_foreground` and `ic_launcher_background` are the 432 px adaptive layers (the mark sits inside the 264 px safe circle). `ic_launcher_monochrome` is for themed icons. `play-store-icon-512.png` is for the store listing.
- Web: paste `web/head-snippet.html` into your page head and serve `site.webmanifest`. `maskable-512.png` keeps the mark inside the maskable safe zone.
- Desktop: `icon-1024.png` follows the macOS body-in-canvas convention. `icon.icns` and `icon.ico` are ready for Tauri or Electron. The tray template icon is black on transparent, as macOS expects.

## Colour

Core palette: gold `#E8B83A`, blush `#D9822B`, cream `#F6EFD9`, canopy `#5E7F3A`, lime `#A9C25D`, sand `#CDB38B`, bark `#7C746A`, ink `#2B2A33`.

- Gold is never text on a light background (it fails contrast). Use ink, or deep amber `#9A5B0F`, for gold-coloured text.
- Cream or white text on green fills below 24 px uses deep canopy `#4A6A2A`, not `#5E7F3A`.
- Talavera blue, terracotta and kraft are illustration accents only, and never outweigh the core palette. The product UI uses the core palette alone.
- `06-colour/marula-theme-shadcn-tweakcn.css` is the full light and dark token set. Paste it into `globals.css` or import it into tweakcn.

## Type

- Fraunces (500) for the wordmark, marketing headlines and page greetings. Never in dense UI.
- Figtree (400, 500, 600) for the product interface.
- The bundled files are the Latin subset. For full language coverage, use the families from Google Fonts. Both are licensed under the SIL Open Font License (see the OFL files).

## Illustration and patterns

- The painted orchard and collage poster are raster (JPG) at 2x. The posters, spots and overlays are vector, with a subtle paper grain embedded.
- Pattern files are large backgrounds (1200 by 800), not guaranteed seamless tiles. For repeating web backgrounds, crop a tile or ask for a seamless export.
- Use one illustration style per composition, two at most, keep gold as the brightest element, and leave calm space for type. Avoid stock "Africa" imagery.
- Textures sit under type at low contrast. Don't put illustration behind dense product UI.

## Not in this folder

The page designs from the canvas (landing hero, ad mockups, website sections, docs, letterhead, invoice, pitch deck and the app screens) are layouts rather than brand assets, so they live on the identity canvas. The interactive app mockups are a separate HTML file.
