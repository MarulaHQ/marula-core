// Root layout: fonts, global styles, auth provider, theme bootstrap.
//
// Purpose: the only place global CSS and providers are attached; an inline
// pre-hydration script applies the persisted theme before first paint so
// neither mode ever flashes. Hosts the docked active-timer bar (TASKLIST
// 2.9) so it spans every surface.
// Non-purpose: no page chrome — headers live in pages that need them.
import type { Metadata } from "next";
import { Inter } from "next/font/google";
import "./globals.css";
import { AuthProvider } from "../lib/api";
import { TimerDock } from "../components/timer-dock";

const inter = Inter({ subsets: ["latin"], variable: "--m-font-sans" });

export const metadata: Metadata = {
  title: "Meridian",
  description: "Open-core PSA — tickets, time, dispatch.",
};

const themeBootstrap = `(function(){try{var t=localStorage.getItem("meridian-theme");if(t!=="light"&&t!=="dark"){t=window.matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light";}document.documentElement.dataset.theme=t;}catch(e){}})();`;

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: themeBootstrap }} />
      </head>
      <body className={inter.variable}>
        <AuthProvider>
          {children}
          <TimerDock />
        </AuthProvider>
      </body>
    </html>
  );
}
