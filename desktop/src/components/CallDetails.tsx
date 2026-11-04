// What opens over the call: the voice gate, one person, the connection.
// The contents only; CallView places them in popovers, beside what was
// clicked in the wide window and across the narrow one.
import { Check } from "lucide-react";
import { cn } from "cn";
import type { CallView, Participant } from "@/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Slider } from "@/components/ui/slider";
import { formatCount, formatDb, formatMs, GATE_MAX_DBFS, GATE_MIN_DBFS, meterPercent, volumePercent } from "@/format";
import { useDragged } from "@/hooks";
import { useLanguage } from "@/lang";
import type { Key } from "@/i18n";
import { meFirst, trustNames, trustOf, type GateStatus, type LockState } from "@/model";
import { CheckingIcon, KeyIcon, LockIcon } from "./icons";
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
    <div className={cn("relative", size === "md" ? "h-6.5" : "h-5.5")}>
      <div
        className={cn(
          "absolute inset-x-0 h-2 overflow-hidden rounded-[0.25rem] bg-track",
          size === "md" ? "top-2.25" : "top-1.75",
        )}
      >
        <div
          className={cn("h-full rounded-[0.25rem]", fill)}
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
          "w-1 rounded-[0.125rem] bg-foreground hover:ring-0 active:ring-0 after:-inset-x-3",
          size === "md" ? "h-5.5" : "h-5",
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
        <Badge className={cn("h-auto px-2 py-0.5 text-[0.71875rem] font-semibold", pill.className)}>{pill.label}</Badge>
        <span className="font-mono text-[0.8125rem] font-medium">{formatDb(gate)}</span>
      </div>
      <GateMeter
        levelDbfs={call.inputDbfs}
        gateDbfs={gate}
        onGate={setGate}
        fill={meterFill(call.muted ? "muted" : call.bound && call.inputDbfs >= gate ? "sending" : "below")}
      />
      <p className="text-[0.78125rem] text-subtle">{t("gate.help")}</p>
      {!call.gateChosen && (
        <p className="flex items-center gap-2 text-[0.78125rem] text-warning-foreground">
          <span className="size-1.75 shrink-0 rounded-full bg-warning" />
          {t("gate.default")}
        </p>
      )}
      <Button
        variant="link"
        onClick={onCalibrate}
        className="h-auto self-start p-0 text-[0.8125rem] font-semibold no-underline hover:no-underline"
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
        <span className={cn("flex-1 text-[0.78125rem]", person.speaking ? "text-primary" : "text-muted-foreground")}>
          {person.speaking ? t("person.speaking") : t("person.silent")}
        </span>
      </div>
      <div className="flex flex-col gap-1">
        <span className="text-xs text-muted-foreground">{t("person.mouthToEar")}</span>
        <div className="flex items-center gap-2">
          <span className="font-mono text-[1.375rem] font-semibold tracking-[-0.02em]">
            {person.estimateMs !== null ? `≈ ${formatMs(person.estimateMs)}` : "–"}
          </span>
          <Badge variant="estimate">{t("person.estimate")}</Badge>
        </div>
        <span className="text-xs text-muted-foreground">{t("person.estimateHelp")}</span>
      </div>
      <div className="flex flex-col gap-1.5">
        <span className="flex justify-between text-[0.78125rem] text-subtle">
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
      <span className="font-mono text-[0.71875rem] leading-normal text-muted-foreground">
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
        <div className="flex items-center gap-2 border-b border-raised py-2 text-[0.8125rem]">
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
          <Badge variant="measured" className="text-[0.65625rem] leading-3.75">
            {t("badge.measured")}
          </Badge>
        </div>
        <div className="flex items-center gap-2 border-b border-raised py-2 text-[0.8125rem]">
          <span className="flex-1 text-subtle">{t("link.device")}</span>
          <span className="font-mono">{formatMs(call.deviceMs)}</span>
          <Badge variant={call.deviceMeasured ? "measured" : "estimate"} className="text-[0.65625rem] leading-3.75">
            {call.deviceMeasured ? t("badge.measured") : t("badge.reported")}
          </Badge>
        </div>
        <div className="flex items-center gap-2 py-2 text-[0.8125rem]">
          <span className="flex-1 text-subtle">{t("link.loss")}</span>
          <span className={cn("font-mono", badLoss && "text-danger")}>{Math.round(call.uplinkLossPct)} %</span>
        </div>
      </div>
      {call.devices.map((d) => (
        <div key={d.direction} className="flex flex-col gap-px">
          <span className="text-[0.8125rem]">
            <span className="text-muted-foreground">{d.direction === "input" ? t("link.input") : t("link.output")}</span>{" "}
            · {d.name}
          </span>
          <span className="font-mono text-[0.71875rem] text-muted-foreground">
            {t("link.detail", { mode: d.mode, buffer: formatMs(d.bufferMs), latency: formatMs(d.latencyMs) })}
          </span>
        </div>
      ))}
    </>
  );
}

/** The lock's colours and words for each state of the call. */
export function lockLook(state: LockState): { button: string; title: string; pill: string; aria: Key } {
  const looks: Record<LockState, { button: string; title: string; pill: string; aria: Key }> = {
    verified: {
      button: "text-muted-foreground",
      title: "text-primary",
      pill: "bg-primary/12 text-primary",
      aria: "lock.ariaVerified",
    },
    pending: {
      button: "text-dim",
      title: "text-muted-foreground",
      pill: "bg-line text-muted-foreground",
      aria: "lock.ariaPending",
    },
    failed: {
      button: "bg-warning/12 text-warning hover:bg-warning/16 hover:text-warning",
      title: "text-warning",
      pill: "bg-warning/14 text-warning-foreground",
      aria: "lock.ariaFailed",
    },
  };
  return looks[state];
}

/** End-to-end encryption, explained: whether everyone heard is verified,
    who is and who is not, and the limit a member's key sets. */
export function LockDetails({ call, state }: { call: CallView; state: LockState }) {
  const { t } = useLanguage();
  const look = lockLook(state);
  const names = trustNames(call);
  const pill =
    state === "verified"
      ? t("lock.pillVerified")
      : state === "pending"
        ? t("lock.pillPending")
        : t(names.failed.length > 1 ? "lock.pillFailedMany" : "lock.pillFailedOne", {
            count: Math.max(names.failed.length, 1),
          });
  const text =
    state === "verified"
      ? t("lock.textVerified")
      : state === "pending"
        ? [names.pending.length > 0 && t("lock.textPendingName", { name: names.pending[0] }), t("lock.textPending")]
            .filter(Boolean)
            .join(" ")
        : names.failed.length > 0
          ? t("lock.textFailedName", { name: names.failed[0] })
          : t("lock.textFailed");
  return (
    <>
      <div className="flex items-center gap-2">
        <LockIcon open={state !== "verified"} strokeWidth={2.2} className={cn("size-4", look.title)} />
        <span className="flex-1 text-sm font-semibold">{t("lock.title")}</span>
        <Badge className={cn("h-auto px-2 py-0.5 text-[0.71875rem] font-semibold", look.pill)}>{pill}</Badge>
      </div>
      <p className="text-[0.78125rem] text-subtle">{text}</p>
      <div className="flex items-center gap-2 text-[0.78125rem] text-soft">
        <KeyIcon className="size-3.5 text-muted-foreground" />
        {t("lock.server")}
      </div>
      <div className="flex flex-col border-t border-raised pt-1">
        {meFirst(call.participants).map((p) => {
          const trust = trustOf(call, p) ?? "pending";
          return (
            <div key={p.session} className="flex h-8 items-center gap-2.5 text-[0.8125rem]">
              <span className="flex size-4 shrink-0 items-center justify-center">
                {trust === "ok" && <Check className="size-3.5 text-primary" strokeWidth={2.6} />}
                {trust === "pending" && <CheckingIcon className="text-muted-foreground" />}
                {trust === "failed" && <LockIcon open strokeWidth={2.4} className="size-3.5 text-warning" />}
              </span>
              <span className={cn("min-w-0 flex-1 truncate", trust === "failed" ? "text-muted-foreground" : "text-foreground")}>
                {p.name}
              </span>
              <span className={cn("text-xs", trust === "failed" ? "text-warning-foreground" : "text-muted-foreground")}>
                {p.me
                  ? t("lock.you")
                  : trust === "failed"
                    ? t("lock.rejected")
                    : trust === "pending"
                      ? t("lock.pending")
                      : t("lock.verified")}
              </span>
            </div>
          );
        })}
      </div>
      <p className="border-t border-raised pt-2.5 text-xs text-muted-foreground">{t("lock.limit")}</p>
    </>
  );
}
