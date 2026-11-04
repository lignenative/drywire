// What opens over the call: the voice gate, one person, the connection.
// The contents only; CallView places them in popovers, beside what was
// clicked in the wide window and across the narrow one.
import { cn } from "cn";
import type { CallView, Participant } from "@/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Slider } from "@/components/ui/slider";
import { formatCount, formatDb, formatMs, GATE_MAX_DBFS, GATE_MIN_DBFS, meterPercent, volumePercent } from "@/format";
import { useDragged } from "@/hooks";
import { useLanguage } from "@/lang";
import type { GateStatus } from "@/model";
import { api } from "@/api";

/** The panel shared by every popover of the canvas. */
export const PANEL =
  "gap-2.5 rounded-2xl border border-border bg-popover p-4 shadow-[0_16px_40px_rgba(0,0,0,0.5)] ring-0";

/** The level meter with the gate's threshold on it, as a white mark the
    person drags. The meter spans −90 to 0 dBFS; the mark moves within the
    range the Rust side accepts, so the slider covers only that part. */
export function GateMeter({
  levelDbfs,
  gateDbfs,
  onGate,
  fill,
  size = "md",
  ringClassName = "shadow-[0_0_0_2px_var(--popover)]",
}: {
  /** Null while no level is known (out of a call). */
  levelDbfs: number | null;
  gateDbfs: number;
  onGate: (value: number) => void;
  fill: string;
  size?: "md" | "sm";
  ringClassName?: string;
}) {
  const { t } = useLanguage();
  const left = meterPercent(GATE_MIN_DBFS);
  const width = meterPercent(GATE_MAX_DBFS) - left;
  return (
    <div className={cn("relative", size === "md" ? "h-[26px]" : "h-[22px]")}>
      <div
        className={cn(
          "absolute inset-x-0 h-2 overflow-hidden rounded-[4px] bg-track",
          size === "md" ? "top-[9px]" : "top-[7px]",
        )}
      >
        <div
          className={cn("h-full rounded-[4px]", fill)}
          style={{ width: `${levelDbfs === null ? 0 : meterPercent(levelDbfs)}%` }}
        />
      </div>
      <Slider
        aria-label={t("gate.title")}
        min={GATE_MIN_DBFS}
        max={GATE_MAX_DBFS}
        step={1}
        value={gateDbfs}
        onValueChange={(v) => onGate(Array.isArray(v) ? v[0] : v)}
        thumbAlignment="center"
        className="absolute inset-y-0 h-full cursor-ew-resize"
        style={{ left: `${left}%`, width: `${width}%` }}
        trackClassName="bg-transparent data-horizontal:h-full overflow-visible"
        indicatorClassName="bg-transparent"
        thumbClassName={cn(
          "w-1 rounded-[2px] bg-foreground hover:ring-0 active:ring-0 after:-inset-x-3",
          size === "md" ? "h-[22px]" : "h-5",
          ringClassName,
        )}
      />
    </div>
  );
}

/** The meter's colour: green while sending, grey below the gate, darker
    while muted. */
export function meterFill(status: GateStatus): string {
  return status === "muted" ? "bg-hush" : status === "sending" ? "bg-primary" : "bg-below";
}

export function GateDetails({
  call,
  status,
  onCalibrate,
}: {
  call: CallView;
  status: GateStatus;
  onCalibrate: () => void;
}) {
  const { t } = useLanguage();
  const [gate, setGate] = useDragged(call.gateDbfs, (v) => api.setGate(v).catch(() => {}));
  const pill = {
    muted: { label: t("me.muted"), className: "bg-alarm/14 text-danger" },
    sending: { label: t("gate.sending"), className: "bg-primary/14 text-primary" },
    below: { label: t("gate.below"), className: "bg-line text-muted-foreground" },
  }[status];
  return (
    <>
      <div className="flex items-center gap-2">
        <span className="flex-1 text-sm font-semibold">{t("gate.title")}</span>
        <Badge className={cn("h-auto px-2 py-0.5 text-[11.5px] font-semibold", pill.className)}>{pill.label}</Badge>
        <span className="font-mono text-[13px] font-medium">{formatDb(gate)}</span>
      </div>
      <GateMeter
        levelDbfs={call.inputDbfs}
        gateDbfs={gate}
        onGate={setGate}
        fill={meterFill(call.muted ? "muted" : call.bound && call.inputDbfs >= gate ? "sending" : "below")}
      />
      <p className="text-[12.5px] text-subtle">{t("gate.help")}</p>
      {!call.gateChosen && (
        <p className="flex items-center gap-2 text-[12.5px] text-warning-foreground">
          <span className="size-[7px] shrink-0 rounded-full bg-warning" />
          {t("gate.default")}
        </p>
      )}
      <Button
        variant="link"
        onClick={onCalibrate}
        className="h-auto self-start p-0 text-[13px] font-semibold no-underline hover:no-underline"
      >
        {t("gate.calibrate")}
      </Button>
    </>
  );
}

export function PersonDetails({ person }: { person: Participant }) {
  const { lang, t } = useLanguage();
  const [volume, setVolume] = useDragged(volumePercent(person.volume), (v) =>
    api.setVolume(person.publicKey, v / 100).catch(() => {}),
  );
  const n = (value: number) => formatCount(lang, value);
  return (
    <>
      <div className="flex items-baseline gap-2.5">
        <span className="text-lg font-semibold">{person.name}</span>
        <span className={cn("flex-1 text-[12.5px]", person.speaking ? "text-primary" : "text-muted-foreground")}>
          {person.speaking ? t("person.speaking") : t("person.silent")}
        </span>
      </div>
      <div className="flex flex-col gap-1">
        <span className="text-xs text-muted-foreground">{t("person.mouthToEar")}</span>
        <div className="flex items-center gap-2">
          <span className="font-mono text-[22px] font-semibold tracking-[-0.02em]">
            {person.estimateMs !== null ? `≈ ${formatMs(person.estimateMs)}` : "–"}
          </span>
          <Badge variant="estimate">{t("person.estimate")}</Badge>
        </div>
        <span className="text-xs text-muted-foreground">{t("person.estimateHelp")}</span>
      </div>
      <div className="flex flex-col gap-1.5">
        <span className="flex justify-between text-[12.5px] text-subtle">
          <span>{t("person.volume")}</span>
          <span className="font-mono text-foreground">{volume} %</span>
        </span>
        <Slider
          aria-label={t("person.volumeOf", { name: person.name })}
          min={0}
          max={200}
          step={5}
          value={volume}
          onValueChange={(v) => setVolume(Array.isArray(v) ? v[0] : v)}
          className="h-4"
        />
      </div>
      <span className="font-mono text-[11.5px] leading-normal text-muted-foreground">
        {t("person.stats", {
          received: n(person.counters.received),
          played: n(person.counters.played),
          concealed: n(person.counters.concealed),
          late: n(person.counters.late),
          dropped: n(person.counters.dropped),
          depth: n(person.counters.depth),
        })}
      </span>
    </>
  );
}

export function LinkDetails({ call, poor }: { call: CallView; poor: boolean }) {
  const { t } = useLanguage();
  const up = call.bound && !call.reconnecting && !call.ended;
  const badLoss = call.uplinkLossPct > 1;
  return (
    <>
      <span className="text-sm font-semibold">{t("link.title")}</span>
      <div className="flex flex-col">
        <div className="flex items-center gap-2 border-b border-raised py-2 text-[13px]">
          <span className="flex-1 text-subtle">{t("link.rtt")}</span>
          <span className={cn("font-mono", poor && "text-danger")}>
            {up && call.rttMs !== null
              ? t("link.rttValue", {
                  rtt: formatMs(call.rttMs),
                  // The canvas writes the lowest without its unit: "14 ms (min. 11)".
                  min: call.minRttMs !== null ? String(Math.round(call.minRttMs)) : "–",
                })
              : "–"}
          </span>
          <Badge variant="measured" className="text-[10.5px] leading-[15px]">
            {t("badge.measured")}
          </Badge>
        </div>
        <div className="flex items-center gap-2 border-b border-raised py-2 text-[13px]">
          <span className="flex-1 text-subtle">{t("link.device")}</span>
          <span className="font-mono">{formatMs(call.deviceMs)}</span>
          <Badge variant={call.deviceMeasured ? "measured" : "estimate"} className="text-[10.5px] leading-[15px]">
            {call.deviceMeasured ? t("badge.measured") : t("badge.reported")}
          </Badge>
        </div>
        <div className="flex items-center gap-2 py-2 text-[13px]">
          <span className="flex-1 text-subtle">{t("link.loss")}</span>
          <span className={cn("font-mono", badLoss && "text-danger")}>{Math.round(call.uplinkLossPct)} %</span>
        </div>
      </div>
      {call.devices.map((d) => (
        <div key={d.direction} className="flex flex-col gap-px">
          <span className="text-[13px]">
            <span className="text-muted-foreground">{d.direction === "input" ? t("link.input") : t("link.output")}</span>{" "}
            · {d.name}
          </span>
          <span className="font-mono text-[11.5px] text-muted-foreground">
            {t("link.detail", { mode: d.mode, buffer: formatMs(d.bufferMs), latency: formatMs(d.latencyMs) })}
          </span>
        </div>
      ))}
    </>
  );
}
