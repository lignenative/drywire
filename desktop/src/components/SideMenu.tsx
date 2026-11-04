// The wide window's permanent menu, on the left: the spaces and their
// rooms with who sits in each, the call this device is in, and this
// device's own bar (name, mute, settings).
import { Plus } from "lucide-react";
import { cn } from "cn";
import type { CallView, RoomView, SpaceView } from "@/api";
import { Button } from "@/components/ui/button";
import { formatMs } from "@/format";
import { useLanguage } from "@/lang";
import { callRoom, inCall, isMultiRoom, meFirst, refOf, sameRoom, signalOf, whereLabel } from "@/model";
import type { AppState } from "@/state";
import { LeaveIcon, Logo, MicIcon, SettingsIcon, SignalBars, SpeakingBars } from "./icons";

export function SideMenu({ app }: { app: AppState }) {
  const { t } = useLanguage();
  return (
    <aside className="flex w-66 shrink-0 flex-col border-r border-sidebar-border bg-sidebar">
      <div className="flex items-center gap-2.5 px-4 pt-4.5 pb-2.5">
        <Logo label={t("app.name")} />
        <span className="text-[0.9375rem] font-bold tracking-[-0.02em]">{t("app.name")}</span>
      </div>

      <nav aria-label={t("spaces.label")} className="flex min-h-0 flex-1 flex-col overflow-y-auto px-2 pt-1 pb-3">
        {app.spaces?.map((space, i) => (
          <SpaceRows key={`${space.instance} ${space.id}`} app={app} space={space} last={i === app.spaces!.length - 1} />
        ))}
        <Button
          variant="ghost"
          onClick={() => app.setInviteOpen(true)}
          className="mt-2.5 h-8.5 justify-start gap-2 rounded-[0.5rem] px-2.5 text-[0.8125rem] font-medium text-muted-foreground"
        >
          <Plus className="size-3.5" strokeWidth={2.2} />
          {t("spaces.join")}
        </Button>
      </nav>

      {inCall(app.call) && <VoicePanel app={app} call={app.call} />}
      <UserBar app={app} />
    </aside>
  );
}

function SpaceRows({ app, space, last }: { app: AppState; space: SpaceView; last: boolean }) {
  const { t } = useLanguage();
  const multi = isMultiRoom(space);
  return (
    <div className={cn("flex flex-col pt-2.5 pb-1.5", !last && "border-b border-line")}>
      {multi && <span className="px-2.5 pb-1 text-[0.9375rem] font-semibold tracking-[-0.01em]">{space.name}</span>}
      {space.error && (
        <div className="flex flex-col gap-0.5 px-2.5 py-1">
          <span className="text-[0.9375rem] font-semibold text-muted-foreground">{space.name}</span>
          <span role="alert" className="flex items-center gap-2 text-[0.78125rem] text-warning-foreground">
            <span className="size-1.75 rounded-full bg-warning" />
            <span className="flex-1">{t("spaces.unreachable")}</span>
            <Button
              variant="ghost"
              onClick={app.reloadSpaces}
              className="h-6.5 rounded-[0.4375rem] px-2 text-[0.78125rem] font-semibold text-warning-foreground hover:text-warning-foreground"
            >
              {t("spaces.retry")}
            </Button>
          </span>
        </div>
      )}
      {space.rooms.map((room) => (
        <RoomRow key={room.id} app={app} space={space} room={room} multi={multi} />
      ))}
    </div>
  );
}

function RoomRow({ app, space, room, multi }: { app: AppState; space: SpaceView; room: RoomView; multi: boolean }) {
  const { t } = useLanguage();
  const ref = refOf(space, room);
  const selected = sameRoom(app.selected, ref);
  const call = inCall(app.call) ? app.call : null;
  const live = !!call && sameRoom(callRoom(call), ref);
  return (
    <div className="flex flex-col">
      <Button
        variant="ghost"
        aria-current={selected ? "page" : undefined}
        onClick={() => app.select(ref)}
        className={cn(
          "h-auto justify-start gap-2 rounded-[0.625rem] px-2.5 text-left whitespace-normal",
          multi ? "min-h-8.5 text-sm font-medium" : "min-h-10 text-[0.9375rem] font-semibold",
          selected ? "bg-accent text-white hover:bg-accent" : multi ? "text-subtle" : "text-foreground",
        )}
      >
        {multi && <span className="text-faint">#</span>}
        <span className="min-w-0 flex-1">{multi ? room.name : space.name}</span>
        {live && (
          <span
            title={t("spaces.here")}
            className={cn("size-1.75 rounded-full", call.reconnecting ? "bg-warning" : "bg-primary")}
          />
        )}
      </Button>
      {live ? (
        <LivePeople call={call} indent={multi} />
      ) : (
        room.occupants?.map((name) => (
          <PersonRow key={name} name={name} indent={multi} className="text-dim" />
        ))
      )}
    </div>
  );
}

/** The people of the call's room, live: who speaks, whether you are muted. */
function LivePeople({ call, indent }: { call: CallView; indent: boolean }) {
  const { t } = useLanguage();
  return meFirst(call.participants).map((p) => {
    const muted = p.me && call.muted;
    const speaking = p.speaking && !muted;
    return (
      <PersonRow
        key={p.session}
        name={p.name}
        indent={indent}
        className={speaking ? "text-foreground" : "text-muted-foreground"}
      >
        {speaking && <SpeakingBars size="sm" label={t("person.speaking")} />}
        {muted && <MicIcon muted role="img" aria-label={t("person.muted")} aria-hidden={false} strokeWidth={2.2} className="size-3.25 text-danger" />}
      </PersonRow>
    );
  });
}

function PersonRow({
  name,
  indent,
  className,
  children,
}: {
  name: string;
  indent: boolean;
  className?: string;
  children?: React.ReactNode;
}) {
  return (
    <div className={cn("flex h-6 items-center gap-2 pr-2.5 text-[0.8125rem]", indent ? "pl-7.5" : "pl-5", className)}>
      <span className="min-w-0 flex-1 truncate">{name}</span>
      {children}
    </div>
  );
}

/** The call this device is in, whichever room the main area shows: one
    click back to it, one to leave. */
function VoicePanel({ app, call }: { app: AppState; call: CallView }) {
  const { t } = useLanguage();
  const signal = signalOf(call);
  const space = app.spaces?.find((s) => s.instance === call.instance && s.id === call.spaceId);
  const where = whereLabel(call.spaceName, call.roomName, space ? isMultiRoom(space) : true);
  const status = call.reconnecting
    ? { label: t("voice.reconnecting"), className: "text-warning-foreground" }
    : signal.connecting
      ? { label: t("voice.connecting"), className: "text-soft" }
      : { label: t("voice.inCall"), className: "text-primary" };
  return (
    <div className="mx-2 flex items-center gap-2 rounded-xl bg-popover py-2.5 pr-2 pl-3">
      <Button
        variant="ghost"
        onClick={() => {
          app.select(callRoom(call));
          app.closeSettings();
        }}
        aria-label={t("voice.show")}
        className="h-auto min-w-0 flex-1 flex-col items-stretch gap-0 rounded-none p-0 text-left hover:bg-transparent"
      >
        <span className={cn("flex items-center gap-1.5 text-[0.8125rem] font-semibold", status.className)}>
          <SignalBars bars={signal.bars} poor={signal.poor} size={12} />
          {status.label}
          <span className={cn("font-mono text-xs font-medium", signal.poor ? "text-danger" : "text-muted-foreground")}>
            {signal.rttMs !== null ? formatMs(signal.rttMs) : signal.connecting ? "…" : "–"}
          </span>
        </span>
        <span className="truncate text-[0.78125rem] font-normal text-subtle">{where}</span>
      </Button>
      <Button
        variant="ghost"
        size="icon-md"
        aria-label={t("voice.leave")}
        onClick={app.leave}
        className="text-danger hover:text-danger"
      >
        <LeaveIcon />
      </Button>
    </div>
  );
}

function UserBar({ app }: { app: AppState }) {
  const { t } = useLanguage();
  const call = inCall(app.call) ? app.call : null;
  const muted = !!call?.muted;
  return (
    <div className="flex items-center gap-1 pt-2.5 pr-2 pb-3 pl-5">
      <div className="min-w-0 flex-1">
        <div className="truncate text-[0.84375rem] font-semibold">{app.displayName}</div>
        <div className={cn("text-xs", muted ? "text-danger" : call ? "text-muted-foreground" : "text-dim")}>
          {muted ? t("me.muted") : call ? t("voice.inCall") : t("me.idle")}
        </div>
      </div>
      <Button
        variant="ghost"
        size="icon-md"
        aria-pressed={muted}
        aria-label={muted ? t("mute.off") : t("mute.on")}
        title={app.settings?.muteShortcut || undefined}
        disabled={!call}
        onClick={app.toggleMute}
        className={cn(muted ? "bg-alarm/16 text-danger hover:bg-alarm/20 hover:text-danger" : "text-subtle", "disabled:opacity-100")}
      >
        <MicIcon muted={muted} />
      </Button>
      <Button
        variant="ghost"
        size="icon-md"
        aria-label={t("settings.title")}
        onClick={() => app.openSettings()}
        className="text-subtle"
      >
        <SettingsIcon />
      </Button>
    </div>
  );
}
