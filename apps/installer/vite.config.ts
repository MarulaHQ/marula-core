import { defineConfig } from "vite";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";

/**
 * The installer's build.
 *
 * `base: "./"` is load-bearing: the native shell serves this bundle from the
 * app's resource directory and the loopback host serves it from `/`, and a
 * root-absolute asset path would break the first. Every asset is relative to
 * the document, so both hosts read the same bytes.
 *
 * No CDN, no remote font, no analytics: `offline:check` fails the build if an
 * emitted file references an http(s) URL, because "works offline after the
 * images are pulled" is a promise the installer makes in its own first screen.
 */
export default defineConfig({
  base: "./",
  plugins: [react(), tailwindcss()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
    assetsInlineLimit: 0,
    sourcemap: false,
  },
  server: { port: 5183, strictPort: true },
});
