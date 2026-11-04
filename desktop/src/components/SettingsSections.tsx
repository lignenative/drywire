// The four pages of the settings: voice, sounds, display, advanced. Each
// setting applies at once; nothing waits for a Save button.
import { Play } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { cn } from "cn";
import { api, type CalibrationView, type LoopbackView, type OverlayCorner, type Preferences } from "@/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { DEFAULT_GATE_DBFS, formatDb, formatMs } from "@/format";
import { useDragged } from "@/hooks";
import { LANGUAGES } from "@/i18n";
import { errorText, useLanguage } from "@/lang";
import { inCall } from "@/model";
import { shortcutFromPress } from "@/shortcut";
import { playPresence } from "@/sounds";
import type { AppState } from "@/state";
import { GateMeter } from "./CallDetails";

const CALIBRATION_SECONDS = 10;

/** One setting: its name and help on the left, its control on the right
    (under them in the narrow window). */
function Row({
  title,
  help,
  children,
  narrow,
  last,
  className,
}: {
  title: ReactNode;
  help?: ReactNode;
  children?: ReactNode;
  narrow: boolean;
  last?: boolean;
  className?: string;
}) {
  return (
    <div
      className={cn(
        // Wrapping lets a row's details (the calibration's result) go
        // under it, across the whole width.
        "flex flex-wrap items-center gap-x-4 py-[18px]",
        narrow && "gap-y-3",
        !last && "border-b border-line",
        className,
      )}
    >
      <div className={cn("min-w-0 flex-1", narrow && "basis-full")}>
        <div className="text-[15px] font-medium">{title}</div>
        {help && <div className="text-[13px] text-muted-foreground">{help}</div>}
      </div>
      {children}
    </div>
  );
}

/** Saves the preferences the Rust side keeps together, and returns the
    error to show if it refused them. */
async function savePreferences(app: AppState, change: Partial<Preferences>): Promise<unknown> {
  if (!app.settings) return null;
  const { muteShortcut, soundVolume, overlay, overlayCorner } = { ...app.settings, ...change };
  try {
    app.setSettings(await api.setPreferences({ muteShortcut, soundVolume, overlay, overlayCorner }));
    return null;
  } catch (e) {
    return e;
  }
}

// --- Voice ---------------------------------------------------------------

export function VoiceSection({ app, narrow }: { app: AppState; narrow: boolean }) {
  const { t } = useLanguage();
  const call = inCall(app.call) ? app.call : null;
  const reported = app.settings?.gateOpenDbfs ?? call?.gateDbfs ?? DEFAULT_GATE_DBFS;
  const [gate, setGate] = useDragged(reported, (v) => {
    api.setGate(v).then(() => app.settings && app.setSettings({ ...app.settings, gateOpenDbfs: v }), () => {});
  });
  // The level is known only in a call: the Rust side reads the microphone
  // there and nowhere else (TODO: a level command out of a call).
  const level = call ? call.inputDbfs : null;
  const open = !!call && !call.muted && call.inputDbfs >= gate;

  return (
    <>
      <Row title={t("settings.gate")} help={t("settings.gateHelp")} narrow={narrow} last={false}>
        <div className={narrow ? "min-w-0 flex-1" : "w-[150px]"}>
          <GateMeter
            levelDbfs={level}
            gateDbfs={gate}
            onGate={setGate}
            fill={open ? "bg-primary" : "bg-below"}
            size="sm"
            ringClassName="shadow-[0_0_0_2px_var(--background)]"
          />
        </div>
        <span className="w-[52px] text-right font-mono text-[13px] font-medium">{formatDb(gate)}</span>
        <Calibration app={app} narrow={narrow} />
      </Row>
      <ShortcutRow app={app} narrow={narrow} />
    </>
  );
}

/** The 10-second calibration: silence, controller in hand, then a
    threshold just above that noise (spec section 10). */
function Calibration({ app, narrow }: { app: AppState; narrow: boolean }) {
  const { t } = useLanguage();
  const [left, setLeft] = useState<number | null>(null);
  const [found, setFound] = useState<CalibrationView | null>(null);
  const [error, setError] = useState<string | null>(null);

  // A countdown while the Rust side records.
  useEffect(() => {
    if (left === null || left <= 0) return;
    const timer = setTimeout(() => setLeft(left - 1), 1000);
    return () => clearTimeout(timer);
  }, [left]);

  const run = async () => {
    setFound(null);
    setError(null);
    setLeft(CALIBRATION_SECONDS);
    try {
      setFound(await api.calibrateGate());
    } catch (e) {
      setError(errorText(t, e));
    } finally {
      setLeft(null);
    }
  };

  const apply = async () => {
    if (!found) return;
    try {
      await api.setGate(found.gateDbfs);
      if (app.settings) app.setSettings({ ...app.settings, gateOpenDbfs: found.gateDbfs });
      setFound(null);
    } catch (e) {
      setError(errorText(t, e));
    }
  };

  return (
    <>
      <Button variant="secondary" size="md" disabled={left !== null} onClick={run}>
        {left !== null ? t("settings.calibrating", { seconds: Math.max(left, 0) }) : t("settings.calibrate")}
      </Button>
      {(found || error) && (
        <div className={cn("flex basis-full flex-col gap-2.5 rounded-[14px] bg-card p-4", !narrow && "mt-3 mb-1")}>
          {found && (
            <>
              <span className="text-[13px] text-subtle">{t("settings.calibrateIntro")}</span>
              <div className="flex flex-wrap gap-x-[18px] gap-y-1.5 text-[13px]">
                <span>
                  {t("settings.noise")} <span className="font-mono">{formatDb(found.noiseDbfs)}</span>
                </span>
                <span>
                  {t("settings.peaks")} <span className="font-mono">{formatDb(found.peakDbfs)}</span>
                </span>
                <span className="text-primary">
                  {t("settings.proposed")} <span className="font-mono">{formatDb(found.gateDbfs)}</span>
                </span>
              </div>
              {found.tooLoud && <p className="text-[13px] text-warning-foreground">{t("settings.tooLoud")}</p>}
              <div className="flex gap-2">
                <Button onClick={apply} className="h-9 rounded-[10px] px-3.5 text-[13px] font-semibold">
                  {t("settings.apply")}
                </Button>
                <Button
                  variant="ghost"
                  onClick={() => setFound(null)}
                  className="h-9 rounded-[10px] px-3.5 text-[13px] font-medium text-soft"
                >
                  {t("settings.discard")}
                </Button>
              </div>
            </>
          )}
          {error && <p className="text-[13px] text-danger">{error}</p>}
        </div>
      )}
    </>
  );
}

/** The canvas writes Shift as ⇧; the other keys keep their names. */
function keyLabel(key: string): string {
  return key === "Shift" ? "⇧" : key;
}

function ShortcutRow({ app, narrow }: { app: AppState; narrow: boolean }) {
  const { t } = useLanguage();
  const [recording, setRecording] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const shortcut = app.settings?.muteShortcut ?? "";
  const keys = shortcut ? shortcut.split("+") : [];

  const save = async (next: string) => {
    setRecording(false);
    const failed = await savePreferences(app, { muteShortcut: next });
    setError(failed ? errorText(t, failed) : null);
  };

  const record = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (!recording || event.key === "Tab") return;
    event.preventDefault();
    if (event.key === "Escape") return setRecording(false);
    if (event.key === "Backspace" || event.key === "Delete") return void save("");
    const pressed = shortcutFromPress(event.nativeEvent);
    if (pressed) void save(pressed);
  };

  return (
    <Row
      title={t("settings.shortcut")}
      help={
        <>
          {t("settings.shortcutHelp")}
          {recording && <span className="block">{t("settings.shortcutHint")}</span>}
          {error && <span className="block text-danger">{error}</span>}
        </>
      }
      narrow={narrow}
      last
    >
      <Button
        variant="outline"
        aria-label={t("settings.shortcutEdit", { keys: keys.join(" ") || t("settings.shortcutNone") })}
        aria-pressed={recording}
        onClick={() => setRecording(!recording)}
        onKeyDown={record}
        onBlur={() => setRecording(false)}
        className={cn(
          "h-9 gap-1 rounded-[10px] border-border bg-transparent px-2 dark:border-border dark:bg-transparent",
          recording && "border-primary dark:border-primary",
        )}
      >
        {recording ? (
          <span className="px-1 text-[13px] text-muted-foreground">{t("settings.shortcutRecording")}</span>
        ) : keys.length > 0 ? (
          keys.map((k) => (
            <Kbd key={k} className="h-auto rounded-md bg-raised px-[7px] py-0.5 font-mono text-xs text-foreground">
              {keyLabel(k)}
            </Kbd>
          ))
        ) : (
          <span className="px-1 text-[13px] text-muted-foreground">{t("settings.shortcutNone")}</span>
        )}
      </Button>
    </Row>
  );
}

// --- Sounds --------------------------------------------------------------

export function SoundsSection({ app, narrow }: { app: AppState; narrow: boolean }) {
  const { t } = useLanguage();
  const saved = Math.round((app.settings?.soundVolume ?? 0.5) * 100);
  const [volume, setVolume] = useState(saved);
  useEffect(() => setVolume(saved), [saved]);
  return (
    <div className="flex flex-col gap-3.5 border-b border-line py-[18px]">
      <div>
        <div className="text-[15px] font-medium">{t("settings.sounds")}</div>
        <div className="text-[13px] text-muted-foreground">{t("settings.soundsHelp")}</div>
      </div>
      <div className="flex items-center gap-3.5">
        <Slider
          aria-label={t("settings.soundVolume")}
          min={0}
          max={100}
          step={5}
          value={volume}
          onValueChange={(v) => setVolume(Array.isArray(v) ? v[0] : v)}
          onValueCommitted={(v) => {
            const value = Array.isArray(v) ? v[0] : v;
            playPresence("join", value / 100);
            void savePreferences(app, { soundVolume: value / 100 });
          }}
          className="h-4 flex-1"
        />
        <span className="w-14 text-right font-mono text-[13px] font-medium">
          {volume === 0 ? t("settings.off") : `${volume} %`}
        </span>
      </div>
      <div className={cn("flex gap-2", narrow && "flex-wrap")}>
        {(["join", "leave", "disconnect"] as const).map((kind) => (
          <Button
            key={kind}
            variant="secondary"
            onClick={() => playPresence(kind, Math.max(volume, 20) / 100)}
            className="h-9 gap-2 rounded-full pr-3.5 pl-2.5 text-[13px] font-medium"
          >
            <Play className="size-2.5 fill-primary text-primary" />
            {t(`settings.preview.${kind}`)}
          </Button>
        ))}
      </div>
    </div>
  );
}

// --- Display -------------------------------------------------------------

const CORNERS: readonly OverlayCorner[] = ["top-left", "top-right", "bottom-left", "bottom-right"];

/** The canvas's switch: 42 × 24, green when on. */
function CanvasSwitch(props: React.ComponentProps<typeof Switch>) {
  return (
    <Switch
      {...props}
      className="h-6 w-[42px] p-[3px] data-unchecked:bg-idle dark:data-unchecked:bg-idle [&>span]:size-[18px] [&>span]:bg-white! [&>span]:data-checked:translate-x-[18px]"
    />
  );
}

export function DisplaySection({ app, narrow }: { app: AppState; narrow: boolean }) {
  const { lang, setLang, t } = useLanguage();
  const overlay = app.settings?.overlay ?? true;
  const corner = app.settings?.overlayCorner ?? "top-left";
  const [atLogin, setAtLogin] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.launchAtLogin().then(setAtLogin, () => {});
  }, []);

  const pickLanguage = (next: string) => {
    if (next === "en" || next === "fr") setLang(next);
  };

  return (
    <>
      <Row title={t("settings.overlay")} help={t("settings.overlayHelp")} narrow={narrow}>
        <CanvasSwitch
          aria-label={t("settings.overlay")}
          checked={overlay}
          onCheckedChange={(on) => void savePreferences(app, { overlay: on })}
        />
      </Row>
      <Row title={t("settings.corner")} narrow={narrow} className={cn(!overlay && "opacity-40")}>
        <ToggleGroup
          aria-label={t("settings.corner")}
          value={[corner]}
          disabled={!overlay}
          onValueChange={(v: string[]) => v[0] && void savePreferences(app, { overlayCorner: v[0] as OverlayCorner })}
          spacing={1.5}
        >
          {CORNERS.map((c, i) => (
            <ToggleGroupItem
              key={c}
              value={c}
              aria-label={t(`settings.corner.${c}`)}
              className={cn(
                "group/corner flex h-[30px] w-10 min-w-10 rounded-[8px] border border-border p-1 hover:bg-transparent aria-pressed:border-primary aria-pressed:bg-transparent",
                i < 2 ? "items-start" : "items-end",
                i % 2 === 0 ? "justify-start" : "justify-end",
              )}
            >
              <span className="h-1.5 w-2.5 rounded-[2px] bg-hush group-aria-pressed/corner:bg-primary" />
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </Row>
      <Row title={t("language.label")} narrow={narrow}>
        <ToggleGroup
          aria-label={t("language.label")}
          value={[lang]}
          onValueChange={(v: string[]) => v[0] && pickLanguage(v[0])}
          spacing={0}
          className="rounded-[10px] bg-card p-[3px]"
        >
          {LANGUAGES.map((l) => (
            <ToggleGroupItem
              key={l}
              value={l}
              className="h-7 rounded-[7px]! px-3 text-[13px] font-semibold text-subtle hover:bg-transparent aria-pressed:bg-border aria-pressed:text-foreground"
            >
              {t(`language.${l}`)}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </Row>
      <Row
        title={t("settings.atLogin")}
        help={
          <>
            {t("settings.atLoginHelp")}
            {error && <span className="block text-danger">{error}</span>}
          </>
        }
        narrow={narrow}
      >
        <CanvasSwitch
          aria-label={t("settings.atLogin")}
          checked={atLogin}
          onCheckedChange={async (on) => {
            setError(null);
            try {
              setAtLogin(await api.setLaunchAtLogin(on));
            } catch (e) {
              setError(errorText(t, e));
            }
          }}
        />
      </Row>
    </>
  );
}

// --- Advanced ------------------------------------------------------------

export function AdvancedSection({ narrow }: { narrow: boolean }) {
  return (
    <>
      <DeviceTest narrow={narrow} />
      <DeviceKey narrow={narrow} />
    </>
  );
}

/** The loopback test (spec section 11): the earphone against the
    microphone, nine clicks, their median. */
function DeviceTest({ narrow }: { narrow: boolean }) {
  const { t } = useLanguage();
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<LoopbackView | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      setResult(await api.loopbackTest());
    } catch (e) {
      setError(errorText(t, e));
    } finally {
      setBusy(false);
    }
  };

  const reported = result?.devices.reduce((sum, d) => sum + d.bufferMs + d.latencyMs, 0) ?? 0;
  const measured = result?.medianMs ?? null;
  const scale = Math.max(measured ?? 0, reported, 1);
  return (
    <div className="flex flex-col gap-3 border-b border-line py-[18px]">
      <div className={cn("flex gap-4", narrow ? "flex-wrap items-center" : "items-center")}>
        <div className={cn("min-w-0 flex-1", narrow && "basis-full")}>
          <div className="text-[15px] font-medium">
            {t("settings.deviceTest")}{" "}
            <span className="text-[12.5px] font-normal text-dim">{t("settings.outOfCall")}</span>
          </div>
          <div className="text-[13px] text-muted-foreground">{t("settings.deviceTestHelp")}</div>
        </div>
        <Button variant="secondary" size="md" disabled={busy} onClick={run}>
          {busy ? t("settings.listening") : result ? t("settings.rerun") : t("settings.run")}
        </Button>
      </div>
      {measured !== null && result && (
        <>
          <div className="flex flex-wrap items-baseline gap-2.5">
            <span className="font-mono text-[28px] font-semibold tracking-[-0.02em]">{formatMs(measured)}</span>
            <Badge variant="measured">{t("badge.measured")}</Badge>
            <span className="text-[13px] text-muted-foreground">
              {t("settings.deviceResult", { count: result.delaysMs.length })}
            </span>
          </div>
          <div className="flex flex-col gap-1.5 text-[12.5px]">
            <div className="flex items-center gap-2.5">
              <span className="w-[110px] text-subtle">{t("settings.measured")}</span>
              <span className="flex flex-1">
                <span className="h-2 rounded-[4px] bg-primary" style={{ width: `${(measured / scale) * 100}%` }} />
              </span>
              <span className="w-12 text-right font-mono">{formatMs(measured)}</span>
            </div>
            <div className="flex items-center gap-2.5">
              <span className="w-[110px] text-subtle">{t("settings.systemSays")}</span>
              <span className="flex flex-1">
                <span
                  className="h-1.5 rounded-[4px] border border-dashed border-muted-foreground"
                  style={{ width: `${(reported / scale) * 100}%` }}
                />
              </span>
              <span className="w-12 text-right font-mono text-subtle">{formatMs(reported)}</span>
            </div>
          </div>
          <span className="text-[12.5px] text-muted-foreground">{t("settings.wireless")}</span>
        </>
      )}
      {result && measured === null && <p className="text-[13px] text-warning-foreground">{t("settings.noClick")}</p>}
      {error && <p className="text-[13px] text-danger">{error}</p>}
    </div>
  );
}

/** The key an operator lists to let this device create spaces. */
function DeviceKey({ narrow }: { narrow: boolean }) {
  const { t } = useLanguage();
  const [key, setKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => {
    api.publicKey().then(setKey, (e) => setError(errorText(t, e)));
    return () => clearTimeout(timer.current);
  }, []);

  const copy = async () => {
    if (!key) return;
    await navigator.clipboard.writeText(key);
    setCopied(true);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setCopied(false), 1500);
  };

  return (
    <div className={cn("flex gap-4 py-[18px]", narrow ? "flex-col" : "items-start")}>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <div className="text-[15px] font-medium">{t("settings.key")}</div>
        <div className="text-[13px] text-muted-foreground">{t("settings.keyHelp")}</div>
        {key && <code className="font-mono text-[12.5px] leading-[1.6] break-all text-subtle">{key}</code>}
        {error && <p className="text-[13px] text-danger">{error}</p>}
      </div>
      <Button variant="secondary" size="md" disabled={!key} onClick={copy} className="self-start">
        {copied ? t("settings.copied") : t("settings.copy")}
      </Button>
    </div>
  );
}
