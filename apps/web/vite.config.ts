// The web client's development server and build. The interface is the
// desktop app's own (desktop/src, ADR 0048): "@/…" points there, and every
// package both share resolves to this directory's node_modules, so the page
// holds one React, one Base UI, whatever imports them.
//
// The port is fixed, on 127.0.0.1 only, and never moves on its own:
// CLAUDE.md, "Local ports".
import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";
import pkg from "./package.json" with { type: "json" };

const port = Number(process.env.WEB_PORT ?? 10057);
const api = `http://${process.env.DRYWIRE_HTTP_ADDR ?? "127.0.0.1:10050"}`;
const repository = fileURLToPath(new URL("../..", import.meta.url));
const desktop = fileURLToPath(new URL("../../desktop/src", import.meta.url));
const mlsWasm = fileURLToPath(new URL("./src/mls-wasm/drywire_mls_wasm.js", import.meta.url));

export default defineConfig({
  clearScreen: false,
  plugins: [tailwindcss()],
  // The instance the proxy below reaches: the page talks to it through
  // its own origin (src/api.ts).
  define: { __DRYWIRE_PROXIED__: JSON.stringify(api) },
  resolve: {
    // The browser's OpenMLS module, which scripts/mls-wasm.sh builds.
    alias: { "@": desktop, "drywire-mls-wasm": mlsWasm },
    dedupe: Object.keys(pkg.dependencies),
  },
  server: {
    host: "127.0.0.1",
    port,
    strictPort: true,
    // The control plane answers no cross-origin request: in development
    // the page reaches it through this server (src/control.ts).
    proxy: { "/v1": api },
    // The interface's files live in desktop/, outside this directory.
    fs: { allow: [repository] },
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: { target: "es2022", outDir: "dist" },
});
