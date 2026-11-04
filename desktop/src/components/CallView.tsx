// The call: large names, who speaks, and three controls (the voice gate,
// mute, leave). A click on a name opens that person; on your own name or
// the gate's button, the voice gate; on the signal chip, the connection;
// on the lock next to the space's name, the end-to-end encryption.
import { Bluetooth, ChevronDown } from "lucide-react";
import { useRef, useState, type ReactNode, type RefObject } from "react";
import { cn } from "cn";
import type { CallView as Call, Participant } from "@/api";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { formatDb, formatMs, hostOf } from "@/format";
import { useLanguage } from "@/lang";
import { gateStatus, isMultiRoom, lockState, othersFirst, signalOf, trustOf, type LockState, type Signal } from "@/model";
import type { AppState } from "@/state";
import { GateDetails, LinkDetails, LockDetails, lockLook, PANEL, PersonDetails } from "./CallDetails";
import { GateIcon, LeaveIcon, LockIcon, Logo, MicIcon, SettingsIcon, SignalBars, SpeakingBars, SpinIcon } from "./icons";

/** Which popover is open: none, the gate, the link, the lock, or a person
    by session. */
type Sheet = "none" | "gate" | "link" | "lock" | number;

export function CallView({ app, call, narrow = false }: { app: AppState; call: Call; narrow?: boolean }) {
  const { t } = useLanguage();
  const [sheet, setSheet] = useState<Sheet>("none");
  const bar = useRef<HTMLDivElement>(null);
  const signal = signalOf(call);
  const status = gateStatus(call);
  const space = app.spaces?.find((s) => s.instance === call.instance && s.id === call.spaceId);
  const multi = space ? isMultiRoom(space) : true;
  const live = !call.ended;
  const toggle = (next: Sheet) => (open: boolean) => setSheet(open ? next : sheet === next ? "none" : sheet);
  const lock = live ? lockState(call) : null;
  const header = useRef<HTMLDivElement>(null);

  // The lock sits right after the space's name; its popover opens under
  // the title in the wide window, across it in the narrow one.
  const lockButton = lock && (
    <Popover open={sheet === "lock"} onOpenChange={toggle("lock")}>
      <PopoverTrigger
        render={<LockButton state={lock} narrow={narrow} />}
        aria-label={t(lockLook(lock).aria)}
        title={t(lockLook(lock).aria)}
        aria-expanded={sheet === "lock"}
      />
      <PopoverContent
        side="bottom"
        align="start"
        sideOffset={8}
        anchor={header}
        collisionPadding={12}
        className={cn(PANEL, "gap-3", narrow ? "w-[calc(100vw-24px)]" : "w-[364px]")}
      >
        <LockDetails call={call} state={lock} />
      </PopoverContent>
    </Popover>
  );

  const chip = (
    <Popover open={sheet === "link"} onOpenChange={toggle("link")}>
      <PopoverTrigger
        render={<SignalChip signal={signal} />}
        aria-label={t("call.link")}
        aria-expanded={sheet === "link"}
      />
      <PopoverContent
        side="bottom"
        align="end"
        sideOffset={narrow ? 10 : 8}
        collisionPadding={12}
        className={cn(PANEL, "gap-3", narrow ? "w-[calc(100vw-24px)]" : "w-[340px]")}
      >
        <LinkDetails call={call} poor={signal.poor} />
      </PopoverContent>
    </Popover>
  );

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      {narrow ? (
        <header className="flex items-center gap-2.5 pt-3.5 pr-2.5 pb-2.5 pl-3.5">
          <Logo label={t("app.name")} />
          <div ref={header} className="flex min-w-0 items-center">
            <Button
              variant="ghost"
              onClick={() => app.setNarrowScreen("spaces")}
              aria-label={t("spaces.back")}
              className="h-9 min-w-0 gap-2 rounded-[10px] px-2.5 text-[15px] font-semibold"
            >
              <span className="truncate">{call.spaceName}</span>
              {multi && (
                <span className="truncate font-normal text-muted-foreground">
                  <span className="text-faint">#</span> {call.roomName}
                </span>
              )}
              <ChevronDown className="size-3.5 text-muted-foreground" strokeWidth={2.2} />
            </Button>
            {lockButton}
          </div>
          <span className="flex-1" />
          {live && chip}
          <Button variant="ghost" size="icon-md" aria-label={t("settings.title")} onClick={() => app.openSettings()} className="text-subtle">
            <SettingsIcon />
          </Button>
        </header>
      ) : (
        <RoomHeader
          title={call.spaceName}
          sub={multi ? `# ${call.roomName} · ${hostOf(call.instance)}` : hostOf(call.instance)}
          titleRef={header}
          afterTitle={lockButton}
        >
          {chip}
        </RoomHeader>
      )}

      <CallBanner call={call} narrow={narrow} />

      <div className={cn("flex min-h-0 flex-1 items-center", narrow ? "justify-center px-5" : "px-14")}>
        {call.ended && narrow ? (
          <EndedNarrow app={app} />
        ) : (
          <div className={cn("flex flex-col", narrow ? "gap-2" : "gap-2.5", call.reconnecting && "opacity-40")}>
            {othersFirst(call.participants).map((p) =>
              p.me ? (
                <NameButton key={p.session} person={p} muted={call.muted} narrow={narrow} trust="ok" onClick={() => setSheet("gate")} />
              ) : (
                <Popover key={p.session} open={sheet === p.session} onOpenChange={toggle(p.session)}>
                  <PopoverTrigger
                    render={<NameButton person={p} muted={false} narrow={narrow} trust={nameTrust(call, p, lock)} />}
                  />
                  <PopoverContent
                    side={narrow ? "top" : "bottom"}
                    align="start"
                    sideOffset={8}
                    anchor={narrow ? bar : undefined}
                    className={cn(PANEL, "gap-3", narrow ? "w-[calc(100vw-24px)]" : "w-[340px]")}
                  >
                    <PersonDetails person={p} />
                  </PopoverContent>
                </Popover>
              ),
            )}
            <AloneNotes call={call} />
          </div>
        )}
      </div>

      <Controls
        app={app}
        call={call}
        narrow={narrow}
        barRef={bar}
        gateOpen={sheet === "gate"}
        onGateOpen={toggle("gate")}
        gateLabel={formatDb(call.gateDbfs)}
        gateIcon={call.muted || signal.bars === 0 ? "text-dim" : status === "sending" ? "text-primary" : "text-muted-foreground"}
      >
        <GateDetails
          call={call}
          status={status}
          onCalibrate={() => {
            setSheet("none");
            app.openSettings("voice");
          }}
        />
      </Controls>

      {/* The canvas dims the call behind an open popover; a click on it
          closes the popover (Base UI's outside press). */}
      {sheet !== "none" && <div aria-hidden="true" className="pointer-events-none absolute inset-0 bg-[rgb(5_5_6/0.35)]" />}
    </div>
  );
}

/** The space's name in large, and where the room is under it. */
export function RoomHeader({
  title,
  sub,
  titleRef,
  afterTitle,
  children,
}: {
  title: string;
  sub: string;
  titleRef?: RefObject<HTMLDivElement | null>;
  afterTitle?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <header className="flex items-center gap-3 px-7 pt-[22px] pb-[18px]">
      <div className="min-w-0 flex-1">
        <div ref={titleRef} className="flex items-center gap-1.5">
          <span className="text-[22px] font-semibold tracking-[-0.02em]">{title}</span>
          {afterTitle}
        </div>
        <div className="text-[13px] text-muted-foreground">{sub}</div>
      </div>
      {children}
    </header>
  );
}

/** The lock next to the space's name: closed grey when everyone heard is
    verified, open while someone is checked, open amber when a voice is
    rejected. A 44 px target in the narrow window, as on the phone. */
function LockButton({
  state,
  narrow,
  className,
  ...props
}: { state: LockState; narrow: boolean } & React.ComponentProps<"button">) {
  const look = lockLook(state);
  if (narrow) {
    return (
      <Button
        variant="ghost"
        {...props}
        className={cn("-ml-2 size-11 rounded-xl hover:bg-transparent aria-expanded:bg-transparent", className)}
      >
        <span className={cn("flex size-[30px] items-center justify-center rounded-[9px]", look.button)}>
          <LockIcon open={state !== "verified"} />
        </span>
      </Button>
    );
  }
  return (
    <Button variant="ghost" {...props} className={cn("size-7 rounded-[8px]", look.button, className)}>
      <LockIcon open={state !== "verified"} className="size-4" />
    </Button>
  );
}

/** What a name shows of its encryption: a rejected voice always, a pending
    one only while the lock is open (silent people do not hold it back). */
function nameTrust(call: Call, person: Participant, lock: LockState | null) {
  const trust = trustOf(call, person);
  if (trust === "failed") return "failed";
  if (trust === "pending" && lock !== "verified") return "pending";
  return "ok";
}

function SignalChip({ signal, className, ...props }: { signal: Signal } & React.ComponentProps<"button">) {
  return (
    <Button
      variant="ghost"
      {...props}
      className={cn(
        "h-[30px] gap-1.5 rounded-full border px-2.5 font-mono text-[13px] font-medium",
        signal.poor
          ? "border-alarm/45 bg-alarm/10 text-danger hover:bg-alarm/15 hover:text-danger"
          : signal.rttMs !== null
            ? "border-border text-foreground"
            : "border-border text-muted-foreground",
        className,
      )}
    >
      <SignalBars bars={signal.bars} poor={signal.poor} />
      {signal.rttMs !== null ? formatMs(signal.rttMs) : signal.connecting ? "…" : "–"}
    </Button>
  );
}

/** Connecting, reconnecting, or a Bluetooth device: one line above the
    names. Bluetooth is said at once, not in the details: it can add more
    latency than everything else together. */
function CallBanner({ call, narrow }: { call: Call; narrow: boolean }) {
  const { t } = useLanguage();
  const bluetooth = call.devices.some((d) => d.bluetooth);
  const banner = call.ended
    ? null
    : call.reconnecting
      ? {
          icon: <SpinIcon />,
          text:
            t("call.reconnecting") +
            (call.retryInS != null ? ` ${t("call.retryIn", { seconds: call.retryInS })}` : ""),
          warn: true,
        }
      : !call.bound
        ? { icon: <SpinIcon />, text: t("call.connecting"), warn: false }
        : bluetooth
          ? { icon: <Bluetooth className="size-4" strokeWidth={2.2} />, text: t("call.bluetooth"), warn: true }
          : null;
  if (!banner) return null;
  return (
    <div className={narrow ? "flex justify-center px-4 pt-1" : "px-7 pb-1"}>
      <div
        role="status"
        className={cn(
          "inline-flex max-w-[460px] items-center gap-2.5 rounded-xl border px-3.5 py-[9px] text-[13px]",
          banner.warn ? "border-warning/35 bg-warning/10 text-warning-foreground" : "border-track bg-muted text-soft",
        )}
      >
        {banner.icon}
        <span>{banner.text}</span>
      </div>
    </div>
  );
}

function NameButton({
  person,
  muted,
  narrow,
  trust,
  className,
  ...props
}: { person: Participant; muted: boolean; narrow: boolean; trust: "ok" | "pending" | "failed" } & React.ComponentProps<"button">) {
  const { t } = useLanguage();
  // A rejected voice is dropped: it is never shown as speaking.
  const speaking = person.speaking && !muted && trust !== "failed";
  return (
    <Button
      variant="ghost"
      {...props}
      className={cn(
        "h-auto justify-start rounded-xl py-0 pr-2.5 pl-0 text-left hover:bg-transparent aria-expanded:bg-transparent",
        narrow ? "gap-3 rounded-[10px] py-0.5 pr-2" : "min-h-[50px] gap-4",
        className,
      )}
    >
      <span className={cn("flex shrink-0 items-center justify-center", narrow ? "h-[18px] w-[18px]" : "h-[26px] w-[22px]")}>
        {trust === "failed" && (
          <LockIcon open role="img" aria-hidden={false} aria-label={t("person.rejected")} className={cn("text-warning", narrow ? "size-[18px]" : "size-5")} />
        )}
        {speaking && <SpeakingBars size={narrow ? "md" : "lg"} label={t("person.speaking")} />}
        {trust === "pending" && !speaking && (
          <LockIcon open role="img" aria-hidden={false} aria-label={t("person.checking")} className={cn("text-dim", narrow ? "size-[18px]" : "size-5")} />
        )}
        {muted && (
          <MicIcon
            muted
            role="img"
            aria-hidden={false}
            aria-label={t("person.muted")}
            className={cn("text-danger", narrow ? "size-5" : "size-[22px]")}
          />
        )}
      </span>
      <span
        className={cn(
          "font-semibold tracking-[-0.03em]",
          narrow ? "text-[22px]" : "text-[34px]",
          trust === "failed" ? "text-hush" : speaking ? "text-white" : "text-dim",
        )}
      >
        {person.name}
      </span>
      {person.me && (
        <span className={cn("self-end font-normal text-muted-foreground", narrow ? "pb-1 text-[13px]" : "pb-2 text-sm")}>
          {t("call.you")}
        </span>
      )}
    </Button>
  );
}

/** Alone in the room, or still on the default threshold: what to know. */
function AloneNotes({ call }: { call: Call }) {
  const { t } = useLanguage();
  const alone = call.bound && call.participants.length === 1;
  if (!alone && call.gateChosen) return null;
  return (
    <div className="mt-3.5 flex flex-col gap-1.5 text-[13px] text-muted-foreground">
      {alone && <span>{t("call.alone")}</span>}
      {!call.gateChosen && (
        <span className="flex items-center gap-2">
          <span className="size-[7px] rounded-full bg-warning" />
          {t("call.gateDefault")}
        </span>
      )}
    </div>
  );
}

function Controls({
  app,
  call,
  narrow,
  barRef,
  gateOpen,
  onGateOpen,
  gateLabel,
  gateIcon,
  children,
}: {
  app: AppState;
  call: Call;
  narrow: boolean;
  barRef: RefObject<HTMLDivElement | null>;
  gateOpen: boolean;
  onGateOpen: (open: boolean) => void;
  gateLabel: string;
  gateIcon: string;
  children: ReactNode;
}) {
  const { t } = useLanguage();
  const ended = !!call.ended;
  const shortcut = app.settings?.muteShortcut.replaceAll("+", " + ") || undefined;
  const gate = (
    <Popover open={gateOpen} onOpenChange={onGateOpen}>
      <PopoverTrigger
        disabled={ended}
        aria-label={t("call.gateButton", { gate: gateLabel })}
        aria-expanded={gateOpen}
        render={
          <Button
            variant="ghost"
            className={cn(
              "relative h-12 gap-2 rounded-[14px] font-mono text-[13px] font-medium",
              narrow
                ? cn("px-3.5", gateOpen ? "bg-track" : "")
                : cn("border px-4", gateOpen ? "border-idle bg-raised" : "border-border bg-secondary"),
            )}
          />
        }
      >
        <GateIcon className={gateIcon} />
        {!narrow && gateLabel}
        {!call.gateChosen && (
          <span className={cn("absolute size-[7px] rounded-full bg-warning", narrow ? "top-[9px] right-[9px]" : "top-2 right-2")} />
        )}
      </PopoverTrigger>
      <PopoverContent
        side="top"
        align="center"
        sideOffset={narrow ? 10 : 18}
        anchor={narrow ? barRef : undefined}
        className={cn(PANEL, narrow ? "w-[calc(100vw-24px)]" : "w-[360px]")}
      >
        {children}
      </PopoverContent>
    </Popover>
  );
  const mute = (
    <Button
      variant="ghost"
      disabled={ended}
      aria-pressed={call.muted}
      title={shortcut}
      onClick={app.toggleMute}
      className={cn(
        "h-12 gap-2.5 rounded-[14px] px-[18px] text-sm font-semibold",
        narrow ? "min-w-0 flex-1" : "border",
        call.muted
          ? cn("text-danger hover:text-danger", narrow ? "bg-alarm/16 hover:bg-alarm/20" : "border-alarm/45 bg-alarm/14 hover:bg-alarm/20")
          : narrow
            ? "bg-raised"
            : "border-border bg-secondary",
      )}
    >
      <MicIcon muted={call.muted} />
      <span className="truncate">{call.muted ? t("mute.off") : t("mute.on")}</span>
    </Button>
  );
  const leave = (
    <Button
      variant="leave"
      disabled={ended}
      aria-label={t("voice.leave")}
      onClick={app.leave}
      className={cn("h-12 rounded-[14px]", narrow ? "w-[52px]" : "w-14")}
    >
      <LeaveIcon className="size-5" />
    </Button>
  );
  if (narrow) {
    return (
      <div className={cn("flex justify-center px-3 pb-3.5", ended && "opacity-35")}>
        <div
          ref={barRef}
          className="flex w-full items-center gap-1.5 rounded-[20px] border border-track bg-muted p-1.5 shadow-[0_12px_32px_rgba(0,0,0,0.4)]"
        >
          {mute}
          {gate}
          {leave}
        </div>
      </div>
    );
  }
  return (
    <div className="flex justify-center gap-2.5 px-7 pt-4 pb-[26px]">
      {gate}
      {mute}
      {leave}
    </div>
  );
}

/** The narrow window's ended call: the canvas keeps the bar, dimmed. */
function EndedNarrow({ app }: { app: AppState }) {
  const { t } = useLanguage();
  const call = app.call!;
  return (
    <div role="alert" className="flex max-w-[360px] flex-col items-center gap-2 text-center">
      <span className="text-[22px] font-semibold tracking-[-0.02em]">{t("ended.title")}</span>
      <span className="text-sm text-subtle">{t("ended.help")}</span>
      <div className="mt-3 flex gap-2">
        <Button
          size="lg"
          onClick={() => app.join({ instance: call.instance, spaceId: call.spaceId, roomId: call.roomId })}
          className="h-10 rounded-xl px-4 text-sm font-semibold"
        >
          {t("ended.rejoinShort")}
        </Button>
        <Button variant="secondary" size="lg" onClick={app.leave} className="h-10 rounded-xl bg-secondary px-4 text-sm">
          {t("ended.close")}
        </Button>
      </div>
    </div>
  );
}
