// Root layout: fonts, global styles, auth provider, theme bootstrap.
//
// Purpose: the only place global CSS and providers are attached; an inline
// pre-hydration script applies the persisted theme before first paint so
// neither mode ever flashes. Hosts the app shell (Wave 15 lane 6): the
// dark left icon rail + slim top bar + scrolling content + the dark bottom
// work bar, all composed in AppShell. Headerless surfaces (login, portal)
// are excluded inside the shell components themselves.
import type { Metadata, Viewport } from "next";
import localFont from "next/font/local";
import "./globals.css";
import { AuthProvider } from "../lib/api";
import { AppShell } from "../components/shell/app-shell";
import { cn } from "@/lib/utils";
import { builtinPresetCss } from "@marula-core/ui/theme-tokens";
import { ToastHost } from "../components/toast-host";

// Wave 24: the theme-token schema v1 preset range ships as a head style
// block (scoped by data-preset; the unset root stays the tokens.css
// baseline — zero visual change without an explicit preset).
const presetCss = builtinPresetCss();

/**
 * Marula typography (docs/theme.md §4, brand spec §5).
 *
 * Figtree carries the interface, Fraunces the display, JetBrains Mono the
 * code. Fraunces is DELIBERATELY NARROWED: the wordmark, page greetings and
 * marketing only, never dense UI. The 26px ticket title is Figtree, not
 * Fraunces, because the rendered pack shows it that way.
 *
 * All three are `next/font/local` against woff2 vendored in LEGAL/fonts/,
 * not `next/font/google`. That is a deliberate change: the Google loader
 * fetches at BUILD time, so a build behind a proxy, in CI with no egress, or
 * during a Google outage fails for a reason that has nothing to do with this
 * code. Vendoring makes the build hermetic and the shipped bytes auditable —
 * the SHA-256 of each file matches the brand kit's copy.
 */
const figtree = localFont({
  src: "../../../LEGAL/fonts/figtree/Figtree-variable-latin.woff2",
  weight: "300 900",
  style: "normal",
  variable: "--font-sans",
  display: "swap",
  fallback: ["system-ui", "-apple-system", "Segoe UI", "sans-serif"],
  adjustFontFallback: false,
});

const fraunces = localFont({
  src: "../../../LEGAL/fonts/fraunces/Fraunces-variable-latin.woff2",
  weight: "100 900",
  style: "normal",
  variable: "--font-heading",
  display: "swap",
  fallback: ["Georgia", "serif"],
  adjustFontFallback: false,
});

const jetbrainsMono = localFont({
  src: "../../../LEGAL/fonts/jetbrains-mono/JetBrainsMono-variable-latin.woff2",
  weight: "100 800",
  style: "normal",
  variable: "--font-mono",
  display: "swap",
  fallback: ["ui-monospace", "SFMono-Regular", "Menlo", "monospace"],
  adjustFontFallback: false,
});

export const metadata: Metadata = {
  title: "Marula",
  description: "Open-core PSA — tickets, time, dispatch.",
  manifest: "/manifest.webmanifest",
  // PWA store-readiness (Wave 20 lane 1): iOS standalone-install posture.
  // No service worker on purpose — the app is authed + dynamic; posture
  // recorded in design/qa-p20-ga-packaging/LANDING.md.
  appleWebApp: {
    capable: true,
    title: "Marula",
    statusBarStyle: "black-translucent",
  },
  icons: {
    icon: [
      { url: "/favicon.ico", sizes: "48x48" },
      { url: "/favicon-32.png", type: "image/png", sizes: "32x32" },
      { url: "/favicon-16.png", type: "image/png", sizes: "16x16" },
    ],
    apple: [{ url: "/apple-touch-icon.png", sizes: "180x180" }],
  },
};

// PWA store-readiness (Wave 20 lane 1): theme color follows the active
// mode (splash/status-bar tint both themes).
export const viewport: Viewport = {
  themeColor: [
    { media: "(prefers-color-scheme: light)", color: "#F8F7F4" },
    { media: "(prefers-color-scheme: dark)", color: "#2B2A33" },
  ],
};

const themeBootstrap =`(function(){try{var t=localStorage.getItem("marula-theme");if(t!=="light"&&t!=="dark"){t=window.matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light";}document.documentElement.dataset.theme=(t==="light"?"neutral":"dark");var p=localStorage.getItem("marula-theme-preset");if(p){document.documentElement.dataset.preset=p;}var d=localStorage.getItem("marula-density");if(d==="compact"||d==="dense"||d==="comfortable"){document.documentElement.dataset.density=d;}else{document.documentElement.dataset.density="comfortable";}}catch(e){}})();`;

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning className={cn(figtree.variable, fraunces.variable, jetbrainsMono.variable)}>
      <head>
        <script dangerouslySetInnerHTML={{ __html: themeBootstrap }} />
        <style id="m-theme-presets" dangerouslySetInnerHTML={{ __html: presetCss }} />
      </head>
      <body>
        <AuthProvider>
          <AppShell>{children}</AppShell>
        </AuthProvider>
        {/* The toast surface is mounted here rather than in a shell, because
            a shell is not mounted on every route (the headerless layout skips
            it) and a toast that cannot appear on a page is worse than no
            toast. AppShell's popout path is covered too. */}
        <ToastHost />
      </body>
    </html>
  );
}
