// The web client's development server. The port is fixed, on 127.0.0.1
// only, and never moves on its own: CLAUDE.md, "Local ports".
import { defineConfig } from "vite";

const port = Number(process.env.WEB_PORT ?? 10057);
const api = `http://${process.env.DRYWIRE_HTTP_ADDR ?? "127.0.0.1:10050"}`;

export default defineConfig({
  clearScreen: false,
  server: {
    host: "127.0.0.1",
    port,
    strictPort: true,
    // The control plane answers no cross-origin request: in development
    // the page reaches it through this server (src/control.ts).
    proxy: { "/v1": api },
  },
  build: { target: "es2022", outDir: "dist" },
});
