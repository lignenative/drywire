// The interface's build. Tauri loads the dev server in development and
// the built files (dist/) in the app. The port is fixed, on 127.0.0.1
// only, and never moves on its own: CLAUDE.md, "Local ports".
import { defineConfig } from "vite";

const port = Number(process.env.DEV_PORT ?? 10051);

export default defineConfig({
  clearScreen: false,
  server: {
    host: "127.0.0.1",
    port,
    strictPort: true,
    // The Rust side rebuilds on its own; Vite need not watch it.
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    // WebView2 on Windows 10 and 11 is a current Chromium.
    target: "es2022",
    outDir: "dist",
    // Two pages: the app's window and the overlay (src-tauri/src/shell.rs).
    rollupOptions: {
      input: {
        main: "index.html",
        overlay: "overlay.html",
      },
    },
  },
});
