// The overlay: a transparent window over the game that lets every click
// through (shell.rs). It shows who is speaking, whether this device is
// muted, and a pill for two seconds when someone joins or leaves. The
// canvas's overlay page draws it over a game: only the pills, no window.
import { StrictMode, useEffect, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { cn } from "cn";
import { api, type CallView, type Notice, type OverlayCorner } from "./api";
import { MicIcon, SpeakingBars, SpinIcon } from "./components/icons";
import { Badge } from "./components/ui/badge";
import { LanguageProvider, useLanguage } from "./lang";
import "./index.css";
import "./overlay.css";

const POLL_MS = 150;
const PILL_MS = 2000;

interface Pill extends Notice {
  id: number;
}

/** One pill: dark glass over any game, a coloured edge for its kind. */
function OverlayPill({ className, children }: { className?: string; children: ReactNode }) {
  return (
    <Badge
      className={cn(
        "h-[30px] gap-2 rounded-full border bg-[rgb(12_13_15/0.84)] px-3 text-[13px] font-medium text-[#e3e4e8] shadow-[0_2px_10px_rgba(0,0,0,0.35)]",
        className,
      )}
    >
      {children}
    </Badge>
  );
}

function Overlay() {
  const { t } = useLanguage();
  const [call, setCall] = useState<CallView | null>(null);
  const [pills, setPills] = useState<Pill[]>([]);
  const [corner, setCorner] = useState<OverlayCorner>("top-left");

  // The corner, read again now and then: the settings may change it.
  useEffect(() => {
    const read = () => api.getSettings().then((s) => setCorner(s.overlayCorner), () => {});
    read();
    const timer = setInterval(read, 2000);
    return () => clearInterval(timer);
  }, []);

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
    <div
      className={cn(
        // The pills hug the corner the window sits in.
        "box-border flex h-screen flex-col gap-1.5 p-1 font-sans leading-[1.3] select-none",
        corner.endsWith("right") ? "items-end" : "items-start",
        corner.startsWith("bottom") && "flex-col-reverse",
      )}
    >
      {call.reconnecting && (
        <OverlayPill className="border-warning/45 pr-3 pl-2.5 font-semibold text-warning-foreground">
          <SpinIcon className="size-3.5" />
          {t("overlay.reconnecting")}
        </OverlayPill>
      )}
      {speaking.map((p) => (
        <OverlayPill key={p.session} className="border-primary/55 pr-3 pl-2.5 text-sm font-semibold text-white">
          <SpeakingBars label={t("person.speaking")} />
          {p.name}
        </OverlayPill>
      ))}
      {call.muted && (
        <OverlayPill className="border-danger/45 bg-[rgb(60_14_17/0.88)] pr-3 pl-2 font-semibold text-[#ffb0b3]">
          <MicIcon muted strokeWidth={2.2} className="size-4" />
          {t("me.muted")}
        </OverlayPill>
      )}
      {pills.map((p) => (
        <OverlayPill key={p.id} className="border-white/10">
          {t(`overlay.${p.kind}`, { name: p.name })}
        </OverlayPill>
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
