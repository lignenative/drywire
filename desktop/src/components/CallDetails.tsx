// What opens over the call: the voice gate, one person, the connection.
// The contents only; CallView places them in popovers, beside what was
// clicked in the wide window and across the narrow one.
import { Check, ChevronLeft, ShieldCheck, TriangleAlert } from "lucide-react";
import { useEffect, useState } from "react";
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
import { loadVerified, safetyCode, saveVerified, verificationOf, type Verified } from "@/safety";

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
export function LockDetails({ call, state, narrow = false }: { call: CallView; state: LockState; narrow?: boolean }) {
  const { t } = useLanguage();
  // In the narrow window a person's safety code is a page of this sheet,
  // in place of the list; in the wide one, their row opens in place.
  const [page, setPage] = useState<string | null>(null);
  const paged = narrow ? call.participants.find((p) => p.publicKey === page && !p.me) : undefined;
  if (paged) return <SafetyCode call={call} person={paged} narrow onBack={() => setPage(null)} />;
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
        {meFirst(call.participants).map((p) => (
          <PersonTrust key={p.session} call={call} person={p} onPage={narrow ? () => setPage(p.publicKey) : undefined} />
        ))}
      </div>
      <p className="border-t border-raised pt-2.5 text-xs text-muted-foreground">{t("lock.limit")}</p>
    </>
  );
}

/** One person in the lock's panel: what the call checked of them, and,
    for the others whose voice verified, the safety code to compare with
    them (ADR 0053): in place, or as a page of the sheet (`onPage`). */
function PersonTrust({ call, person, onPage }: { call: CallView; person: Participant; onPage?: () => void }) {
  const { t } = useLanguage();
  const trust = trustOf(call, person) ?? "pending";
  const [open, setOpen] = useState(false);
  const verified = useVerified();
  const verification = person.me ? "none" : verificationOf(verified.marks, person.publicKey, person.name);

  const icon =
    trust === "failed" ? (
      <LockIcon open strokeWidth={2.4} className="size-3.5 text-warning" />
    ) : trust === "pending" ? (
      <CheckingIcon className="text-muted-foreground" />
    ) : verification === "newKey" ? (
      <TriangleAlert className="size-3.5 text-warning" strokeWidth={2.4} />
    ) : verification === "verified" ? (
      <ShieldCheck className="size-3.5 text-primary" strokeWidth={2.4} />
    ) : (
      <Check className="size-3.5 text-primary" strokeWidth={2.6} />
    );
  const label =
    verification === "verified" ? t("safety.listVerified") : verification === "newKey" ? t("safety.newKey") : t("safety.button");

  return (
    <div className="flex flex-col">
      <div className="flex h-8 items-center gap-2.5 text-[0.8125rem]">
        <span className="flex size-4 shrink-0 items-center justify-center">{icon}</span>
        <span className={cn("min-w-0 flex-1 truncate", trust === "failed" ? "text-muted-foreground" : "text-foreground")}>
          {person.name}
        </span>
        {person.me ? (
          <span className="text-xs text-muted-foreground">{t("lock.you")}</span>
        ) : trust === "failed" ? (
          <span className="text-xs text-warning-foreground">{t("lock.rejected")}</span>
        ) : trust === "pending" ? (
          <span className="text-xs text-muted-foreground">{t("lock.pending")}</span>
        ) : (
          <Button
            variant="ghost"
            aria-expanded={onPage ? undefined : open}
            aria-label={t("safety.title", { name: person.name })}
            onClick={() => (onPage ? onPage() : setOpen(!open))}
            className={cn(
              "h-7 rounded-lg px-2 text-xs font-medium",
              verification === "newKey" ? "text-warning" : verification === "verified" ? "text-primary" : "text-subtle",
            )}
          >
            {label}
          </Button>
        )}
      </div>
      {open && !onPage && trust === "ok" && !person.me && (
        <div className="mb-2 rounded-xl bg-muted p-3">
          <SafetyCode call={call} person={person} narrow={false} />
        </div>
      )}
    </div>
  );
}

/** The marks this device keeps, and a way to change them that every row
    sees at once. */
function useVerified(): { marks: Verified; mark: (person: Participant, on: boolean) => void } {
  const [marks, setMarks] = useState<Verified>(loadVerified);
  useEffect(() => {
    const changed = () => setMarks(loadVerified());
    window.addEventListener("drywire-verified", changed);
    return () => window.removeEventListener("drywire-verified", changed);
  }, []);
  const mark = (person: Participant, on: boolean) => {
    const next = { ...loadVerified() };
    if (on) {
      // A name is verified with one key: an older one stops counting.
      for (const [key, name] of Object.entries(next)) if (name === person.name) delete next[key];
      next[person.publicKey] = person.name;
    } else {
      delete next[person.publicKey];
    }
    saveVerified(next);
    setMarks(next);
    window.dispatchEvent(new Event("drywire-verified"));
  };
  return { marks, mark };
}

/** A person's safety code, what to do with it, and the mark. */
function SafetyCode({
  call,
  person,
  narrow,
  onBack,
}: {
  call: CallView;
  person: Participant;
  narrow: boolean;
  onBack?: () => void;
}) {
  const { t } = useLanguage();
  const me = call.participants.find((p) => p.me);
  const [code, setCode] = useState<string | null>(null);
  const verified = useVerified();
  const verification = verificationOf(verified.marks, person.publicKey, person.name);

  useEffect(() => {
    if (!me) return;
    safetyCode(me.publicKey, person.publicKey).then(setCode, () => setCode(null));
  }, [me?.publicKey, person.publicKey]);

  const groups = (code ?? "····· ····· ····· ·····").split(" ");
  return (
    <div className="flex flex-col gap-2.5">
      {onBack && (
        <Button variant="ghost" onClick={onBack} className="h-8 self-start rounded-lg px-1.5 text-[0.8125rem] font-medium text-subtle">
          <ChevronLeft className="size-4" />
          {t("lock.title")}
        </Button>
      )}
      <span className="text-[0.8125rem] font-semibold">{t("safety.title", { name: person.name })}</span>
      <code
        className={cn(
          "text-center font-mono font-medium tracking-[0.04em] text-foreground",
          narrow ? "py-1 text-[1.375rem] leading-[1.4]" : "text-[1.125rem]",
        )}
      >
        {narrow ? (
          <>
            {groups.slice(0, 2).join(" ")}
            <br />
            {groups.slice(2).join(" ")}
          </>
        ) : (
          groups.join(" ")
        )}
      </code>
      {verification === "newKey" ? (
        <p className="text-xs text-warning">{t("safety.changed", { name: person.name })}</p>
      ) : (
        <p className="text-xs text-subtle">{t("safety.help", { name: person.name })}</p>
      )}
      {verification === "verified" ? (
        <div className="flex items-center gap-2 text-xs text-primary">
          <ShieldCheck className="size-3.5" strokeWidth={2.4} />
          <span className="flex-1">{t("safety.verified")}</span>
          <Button variant="ghost" onClick={() => verified.mark(person, false)} className="h-7 rounded-lg px-2 text-xs text-subtle">
            {t("safety.remove")}
          </Button>
        </div>
      ) : (
        <Button
          disabled={code === null}
          onClick={() => verified.mark(person, true)}
          className={cn("rounded-xl text-[0.8125rem] font-semibold", narrow ? "h-13 w-full text-[0.9375rem]" : "h-9")}
        >
          {t("safety.mark")}
        </Button>
      )}
    </div>
  );
}
