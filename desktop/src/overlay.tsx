// The overlay: a transparent window over the game that lets every click
// through (shell.rs). It shows who is speaking, whether this device is
// muted, and a pill for two seconds when someone joins or leaves.
import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { api, type CallView, type Notice } from "./api";
import { LanguageProvider, useLanguage } from "./lang";
import "./overlay.css";

const POLL_MS = 150;
const PILL_MS = 2000;

interface Pill extends Notice {
  id: number;
}

function Overlay() {
  const { t } = useLanguage();
  const [call, setCall] = useState<CallView | null>(null);
  const [pills, setPills] = useState<Pill[]>([]);

  useEffect(() => {
    const timer = setInterval(() => api.callStatus().then(setCall, () => {}), POLL_MS);
    return () => clearInterval(timer);
  }, []);

  useEffect(() => {
    let next = 0;
    const unlisten = api.onPresence((notice) => {
      const pill = { ...notice, id: next++ };
      setPills((all) => [...all, pill].slice(-3));
      setTimeout(() => setPills((all) => all.filter((p) => p.id !== pill.id)), PILL_MS);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  if (!call || call.ended) return null;
  const speaking = call.participants.filter((p) => p.speaking && !(p.me && call.muted));
  return (
    <div className="overlay">
      {call.muted && <div className="pill muted">{t("call.muted")}</div>}
      {speaking.map((p) => (
        <div key={p.session} className="pill speaking">
          <span className="dot" />
          {p.name}
        </div>
      ))}
      {pills.map((p) => (
        <div key={p.id} className={`pill notice ${p.kind}`}>
          {t(`overlay.${p.kind}`, { name: p.name })}
        </div>
      ))}
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <LanguageProvider>
      <Overlay />
    </LanguageProvider>
  </StrictMode>,
);
