// Root layout: fonts, global styles, auth provider, theme bootstrap.
//
// Purpose: the only place global CSS and providers are attached; an inline
// pre-hydration script applies the persisted theme before first paint so
// neither mode ever flashes. Hosts the app shell (Wave 15 lane 6): the
// dark left icon rail + slim top bar + scrolling content + the dark bottom
// work bar, all composed in AppShell. Headerless surfaces (login, portal)
// are excluded inside the shell components themselves.
import type { Metadata, Viewport } from "next";
import { Inter, JetBrains_Mono, Source_Serif_4 } from "next/font/google";
import "./globals.css";
import { AuthProvider } from "../lib/api";
import { AppShell } from "../components/shell/app-shell";
import { cn } from "@/lib/utils";
import { builtinPresetCss } from "@marula-core/ui/theme-tokens";

// Wave 24: the theme-token schema v1 preset range ships as a head style
// block (scoped by data-preset; the unset root stays the tokens.css
// Tangerine baseline — zero visual change without an explicit preset).
const presetCss = builtinPresetCss();

// Tangerine typography (docs/theme.md §4): Source Serif 4 headings,
// Inter body, JetBrains Mono code/data. next/font serves the real
// webfonts and self-hosts them at build time.
const sourceSerifHeading = Source_Serif_4({ subsets: ["latin"], variable: "--font-heading" });
const jetbrainsMono = JetBrains_Mono({ subsets: ["latin"], variable: "--font-mono" });
const inter = Inter({ subsets: ["latin"], variable: "--font-sans" });

export const metadata: Metadata = {
  title: "Meridian",
  description: "Open-core PSA — tickets, time, dispatch.",
  manifest: "/manifest.webmanifest",
  // PWA store-readiness (Wave 20 lane 1): iOS standalone-install posture.
  // No service worker on purpose — the app is authed + dynamic; posture
  // recorded in design/qa-p20-ga-packaging/LANDING.md.
  appleWebApp: {
    capable: true,
    title: "Meridian",
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
    { media: "(prefers-color-scheme: light)", color: "#E8EBED" },
    { media: "(prefers-color-scheme: dark)", color: "#1C2433" },
  ],
};

const themeBootstrap =`(function(){try{var t=localStorage.getItem("meridian-theme");if(t!=="light"&&t!=="dark"){t=window.matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light";}document.documentElement.dataset.theme=t;var p=localStorage.getItem("meridian-theme-preset");if(p){document.documentElement.dataset.preset=p;}}catch(e){}})();`;

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning className={cn(sourceSerifHeading.variable, jetbrainsMono.variable)}>
      <head>
        <script dangerouslySetInnerHTML={{ __html: themeBootstrap }} />
        <style id="m-theme-presets" dangerouslySetInnerHTML={{ __html: presetCss }} />
      </head>
      <body className={inter.variable}>
        <AuthProvider>
          <AppShell>{children}</AppShell>
        </AuthProvider>
      </body>
    </html>
  );
}
