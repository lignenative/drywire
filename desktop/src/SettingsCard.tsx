// The settings that matter during a match: the global mute shortcut, the
// presence sounds' volume, and the overlay.
import { useEffect, useState, type KeyboardEvent } from "react";
import { api, type Settings } from "./api";
import { errorText, useLanguage } from "./lang";
import { playPresence } from "./sounds";
import { shortcutFromPress } from "./shortcut";

export function SettingsCard({ onSaved }: { onSaved: (settings: Settings) => void }) {
  const { t } = useLanguage();
  const [shortcut, setShortcut] = useState("");
  const [soundVolume, setSoundVolume] = useState(50);
  const [overlay, setOverlay] = useState(true);
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);

  useEffect(() => {
    api.getSettings().then((s) => {
      setShortcut(s.muteShortcut);
      setSoundVolume(Math.round(s.soundVolume * 100));
      setOverlay(s.overlay);
    }, () => {});
  }, []);

  const record = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Tab") return;
    event.preventDefault();
    if (event.key === "Backspace" || event.key === "Delete") {
      setShortcut("");
      return;
    }
    const recorded = shortcutFromPress(event.nativeEvent);
    if (recorded) setShortcut(recorded);
  };

  const save = async () => {
    setMessage(null);
    try {
      const saved = await api.setPreferences({
        muteShortcut: shortcut,
        soundVolume: soundVolume / 100,
        overlay,
      });
      onSaved(saved);
      setMessage({ text: t("settings.saved"), error: false });
    } catch (e) {
      setMessage({ text: errorText(t, e), error: true });
    }
  };

  return (
    <section className="card">
      <h2>{t("settings.title")}</h2>
      <label>
        <span>{t("settings.shortcut")}</span>
        <input
          value={shortcut}
          placeholder={t("settings.shortcutNone")}
          onKeyDown={record}
          onChange={() => {}}
          spellCheck={false}
        />
        <span className="muted small">{t("settings.shortcutHelp")}</span>
      </label>
      <label>
        <span className="row spread">
          <span>{t("settings.sounds")}</span>
          <span className="value">{soundVolume === 0 ? t("settings.off") : `${soundVolume} %`}</span>
        </span>
        <input
          type="range"
          min={0}
          max={100}
          step={5}
          value={soundVolume}
          onChange={(e) => setSoundVolume(Number(e.target.value))}
          onPointerUp={() => playPresence("join", soundVolume / 100)}
        />
      </label>
      <label className="check">
        <input type="checkbox" checked={overlay} onChange={(e) => setOverlay(e.target.checked)} />
        <span>{t("settings.overlay")}</span>
      </label>
      <button onClick={save}>{t("settings.save")}</button>
      {message && <p className={message.error ? "error" : "ok"}>{message.text}</p>}
    </section>
  );
}
