// The web client: the desktop app's interface (desktop/src, ADR 0048)
// driven by the browser's own implementation of its commands (api.ts),
// or, in a browser that lacks what a call needs, a page that says so.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "@/App";
import { provideApi } from "@/api";
import { offerInvitation } from "@/invite";
import { LanguageProvider } from "@/lang";
import { createWebApi } from "./api";
import { MicDialog } from "./components/MicDialog";
import { Unsupported } from "./components/Unsupported";
import { loadIdentity } from "./identity";
import { loadSpaces } from "./store";
import { Protocol } from "./protocol";
import { checkSupport } from "./support";
import "./web.css";

const root = createRoot(document.getElementById("root")!);
const support = await checkSupport();
// In development, `?scene=unsupported` shows that page in any browser, to
// compare it with the canvas.
if (import.meta.env.DEV && new URLSearchParams(location.search).get("scene") === "unsupported") {
  support.missing = ["WebTransport", "Opus (WebCodecs)"];
}

if (!support.codec || support.missing.length > 0) {
  root.render(
    <LanguageProvider>
      <Unsupported missing={support.missing} />
    </LanguageProvider>,
  );
} else {
  const wasm = await fetch(new URL("/drywire.wasm", location.href)).then((r) => r.arrayBuffer());
  const protocol = await Protocol.load(wasm);
  const identity = await loadIdentity(loadSpaces().length > 0);
  const api = createWebApi(protocol, identity, support.codec);
  provideApi(api, "web");
  // In development, the call's numbers from the console:
  // `await drywire.callStatus()` (README, "Measure").
  if (import.meta.env.DEV) Object.assign(window, { drywire: api });
  // An invitation link opens the client with the link in its fragment,
  // which the browser never sends anywhere: offer it to the form.
  if (location.hash.length > 1) offerInvitation(`https://drywire.gg/i${location.hash}`);
  root.render(
    <StrictMode>
      <LanguageProvider>
        <App />
        <MicDialog />
      </LanguageProvider>
    </StrictMode>,
  );
}
