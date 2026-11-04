// @ts-check
import { defineConfig, passthroughImageService } from "astro/config";

// Drywire's block in the machine-wide port convention is 10050-10059
// (CLAUDE.md, "Local ports"). 10051 is the desktop app's Vite, so the
// site's dev server takes 10053, a slot left free because the control
// plane keeps its data in SQLite, not PostgreSQL; the built site is
// served on 10052. SITE_DEV_PORT and SITE_PORT move them for a one-off,
// and scripts/site.sh follows the same variables.
const devPort = Number(process.env.SITE_DEV_PORT ?? 10053);
const previewPort = Number(process.env.SITE_PORT ?? 10052);

export default defineConfig({
  site: "https://drywire.gg",
  // /i is served from i/index.html, as the plain HTML site was.
  build: {
    format: "directory",
    // Every stylesheet in a file of its own: the invitation page's policy
    // (style-src 'self') refuses inline styles.
    inlineStylesheets: "never",
  },
  // No image is processed, so sharp, which brings libvips (LGPL), is never
  // loaded; package.json replaces it with an empty package (ADR 0042).
  image: { service: passthroughImageService() },
  // The toolbar floats over the page: the dev server should look like the built site.
  devToolbar: { enabled: false },
  server: ({ command }) => ({
    // The IPv4 loopback only: nothing reachable from the local network by accident.
    host: "127.0.0.1",
    port: command === "preview" ? previewPort : devPort,
  }),
  vite: {
    build: {
      // Never inline a script, font or image as a data: URL: the invitation
      // page's policy allows files from its own origin only.
      assetsInlineLimit: 0,
    },
    // Fail rather than move to the next free port: a server that relocates
    // on its own cannot be found again.
    server: { strictPort: true },
    preview: { strictPort: true },
  },
});
