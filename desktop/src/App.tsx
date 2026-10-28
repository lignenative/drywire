// The window: the call when there is one, else this device's spaces, the
// invitation form and the device key.
import { useCallback, useEffect, useState } from "react";
import { api, type CallView } from "./api";
import { CallPanel } from "./CallPanel";
import { Home } from "./Home";
import { LANGUAGES } from "./i18n";
import { useLanguage } from "./lang";

/** How often the call's numbers are read while in a room. */
const POLL_MS = 200;

export function App() {
  const { lang, setLang, t } = useLanguage();
  const [call, setCall] = useState<CallView | null>(null);

  // Polled, never pushed: the numbers are a copy the session thread
  // refreshes, so reading them never touches the audio.
  useEffect(() => {
    if (!call) return;
    const timer = setInterval(() => {
      api.callStatus().then(setCall, () => {});
    }, POLL_MS);
    return () => clearInterval(timer);
  }, [call !== null]);

  // A call that was running before a reload of the window.
  useEffect(() => {
    api.callStatus().then(setCall, () => {});
  }, []);

  const leave = useCallback(async () => {
    await api.leaveRoom();
    setCall(null);
  }, []);

  return (
    <div className="app">
      <header className="header">
        <div className="brand">
          <span className="logo" aria-hidden="true" />
          <div>
            <h1>Drywire</h1>
            <p className="muted small">{t("app.tagline")}</p>
          </div>
        </div>
        <div className="lang" role="group" aria-label={t("language.label")}>
          {LANGUAGES.map((l) => (
            <button
              key={l}
              className={l === lang ? "chip active" : "chip"}
              onClick={() => setLang(l)}
              aria-pressed={l === lang}
            >
              {l.toUpperCase()}
            </button>
          ))}
        </div>
      </header>
      <main>
        {call ? <CallPanel call={call} onLeave={leave} /> : <Home onJoined={setCall} />}
      </main>
    </div>
  );
}
