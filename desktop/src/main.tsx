import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { inTauri } from "./api";
import { LanguageProvider } from "./lang";
import { seedSimulatedName } from "./simulated";
import "./index.css";

// In a browser this is `make ui`, the simulated call: it plays the canvas's
// person.
if (!inTauri) seedSimulatedName();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <LanguageProvider>
      <App />
    </LanguageProvider>
  </StrictMode>,
);
