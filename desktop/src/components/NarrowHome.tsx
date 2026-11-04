// The narrow window has no room for a permanent side menu: the spaces are
// the first screen, as on a phone, and a bar at the bottom keeps the call
// one click away (the canvas's narrow window).
import { Plus } from "lucide-react";
import { cn } from "cn";
import type { CallView, RoomView, SpaceView } from "@/api";
import { Button } from "@/components/ui/button";
import { useLanguage } from "@/lang";
import { callRoom, inCall, isMultiRoom, lockState, meFirst, refOf, sameRoom, signalOf, whereLabel } from "@/model";
import type { AppState } from "@/state";
import { lockLook } from "./CallDetails";
import { LeaveIcon, LockIcon, Logo, MicIcon, SettingsIcon, SignalBars, SpeakingBars } from "./icons";

export function NarrowHome({ app }: { app: AppState }) {
  const { t } = useLanguage();
  const call = inCall(app.call) ? app.call : null;
  return (
    <div className="flex h-full flex-col overflow-hidden pt-1 pb-3 text-[15px]">
      <header className="flex items-center gap-2.5 py-2 pr-3 pl-5">
        <Logo size={28} />
        <span className="flex-1 text-lg font-bold tracking-[-0.02em]">{t("app.name")}</span>
        <Button variant="ghost" aria-label={t("settings.title")} onClick={() => app.openSettings()} className="size-11 rounded-xl text-subtle">
          <SettingsIcon className="size-5" />
        </Button>
      </header>

      {app.spaces?.length === 0 ? (
        <NoSpaceNarrow app={app} />
      ) : (
        <nav aria-label={t("spaces.label")} className="flex min-h-0 flex-1 flex-col overflow-y-auto px-3 pt-2 pb-4">
          {app.spaces?.map((space, i) => (
            <SpaceBlock key={`${space.instance} ${space.id}`} app={app} space={space} last={i === app.spaces!.length - 1} />
          ))}
          {app.joinError && <p className="px-3 pt-3 text-[13px] text-danger">{app.joinError}</p>}
        </nav>
      )}

      {call ? (
        <CallBar app={app} call={call} />
      ) : (
        app.spaces?.length !== 0 && (
          <div className="px-4">
            <Button
              variant="secondary"
              onClick={() => app.setInviteOpen(true)}
              className="h-14 w-full gap-2 rounded-[18px] text-base font-semibold"
            >
              <Plus className="size-[18px]" strokeWidth={2.2} />
              {t("spaces.join")}
            </Button>
          </div>
        )
      )}
    </div>
  );
}

function SpaceBlock({ app, space, last }: { app: AppState; space: SpaceView; last: boolean }) {
  const { t } = useLanguage();
  const multi = isMultiRoom(space);
  return (
    <div className={cn("flex flex-col pt-3 pb-2.5", !last && "border-b border-line")}>
      {(multi || space.error) && (
        <span className={cn("px-3 pb-1 text-[21px] font-semibold tracking-[-0.02em]", space.error && "text-muted-foreground")}>
          {space.name}
        </span>
      )}
      {space.error && (
        <span role="alert" className="flex items-center gap-2 px-3 text-sm text-warning-foreground">
          <span className="size-[7px] rounded-full bg-warning" />
          <span className="flex-1">{t("spaces.unreachable")}</span>
          <Button
            variant="ghost"
            onClick={app.reloadSpaces}
            className="h-8 rounded-lg px-2.5 text-sm font-semibold text-warning-foreground hover:text-warning-foreground"
          >
            {t("spaces.retry")}
          </Button>
        </span>
      )}
      {space.rooms.map((room) => (
        <RoomBlock key={room.id} app={app} space={space} room={room} multi={multi} />
      ))}
    </div>
  );
}

/** A room and who sits in it. A click opens the call, joining the room
    first when this device is not in it yet. */
function RoomBlock({ app, space, room, multi }: { app: AppState; space: SpaceView; room: RoomView; multi: boolean }) {
  const { t } = useLanguage();
  const ref = refOf(space, room);
  const call = inCall(app.call) ? app.call : null;
  const live = !!call && sameRoom(callRoom(call), ref);
  const people = live ? livePeople(call, t) : (room.occupants ?? []).map((name) => ({ key: name, name, speaking: false, muted: false }));
  const open = async () => {
    if (!live) await app.join(ref);
    app.setNarrowScreen("call");
  };
  return (
    <Button
      variant="ghost"
      disabled={app.joining}
      onClick={open}
      className={cn(
        "h-auto flex-col items-stretch gap-0 rounded-[14px] px-3 py-0 text-left whitespace-normal",
        live ? "bg-popover hover:bg-popover" : "",
      )}
    >
      <span
        className={cn(
          "flex items-center gap-2 tracking-[-0.02em]",
          multi ? "min-h-[46px] text-lg font-medium text-soft" : "min-h-[50px] text-[21px] font-semibold text-foreground",
        )}
      >
        {multi && <span className="text-faint">#</span>}
        <span className="min-w-0 flex-1">{multi ? room.name : space.name}</span>
        {live && <span className="text-[13px] font-semibold tracking-normal text-primary">{t("spaces.hereShort")}</span>}
      </span>
      {people.length > 0 && (
        <span className={cn("flex flex-wrap gap-x-3 gap-y-0.5 pb-1.5", multi && "pl-[22px]")}>
          {people.map((p) => (
            <span
              key={p.key}
              className={cn(
                "flex items-center gap-1.5 text-[15px] font-normal tracking-normal",
                !live ? "text-dim" : p.speaking ? "text-white" : "text-muted-foreground",
              )}
            >
              {p.speaking && <SpeakingBars size="md" label={t("person.speaking")} />}
              {p.muted && (
                <MicIcon muted role="img" aria-hidden={false} aria-label={t("person.muted")} strokeWidth={2.2} className="size-3.5 text-danger" />
              )}
              {p.name}
            </span>
          ))}
        </span>
      )}
    </Button>
  );
}

function livePeople(call: CallView, t: ReturnType<typeof useLanguage>["t"]) {
  return meFirst(call.participants).map((p) => {
    const muted = p.me && call.muted;
    return {
      key: String(p.session),
      name: p.me ? t("me.you", { name: p.name }) : p.name,
      speaking: p.speaking && !muted,
      muted,
    };
  });
}

function CallBar({ app, call }: { app: AppState; call: CallView }) {
  const { t } = useLanguage();
  const signal = signalOf(call);
  const lock = lockState(call);
  const space = app.spaces?.find((s) => s.instance === call.instance && s.id === call.spaceId);
  const status = call.reconnecting
    ? { label: t("voice.reconnecting"), className: "text-warning-foreground" }
    : signal.connecting
      ? { label: t("voice.connecting"), className: "text-soft" }
      : { label: t("voice.inCall"), className: "text-primary" };
  return (
    <div className="mx-3 flex items-center gap-2.5 rounded-[20px] bg-secondary py-2.5 pr-2.5 pl-4">
      <Button
        variant="ghost"
        aria-label={t("voice.show")}
        onClick={() => app.setNarrowScreen("call")}
        className="h-auto min-w-0 flex-1 flex-col items-stretch gap-0 rounded-none p-0 text-left hover:bg-transparent"
      >
        <span className={cn("flex items-center gap-1.5 text-sm font-semibold", status.className)}>
          <SignalBars bars={signal.bars} poor={signal.poor} size={12} />
          {status.label}
        </span>
        <span className="flex min-w-0 items-center gap-1.5 text-[15px] font-normal text-soft">
          <span className="truncate">{whereLabel(call.spaceName, call.roomName, space ? isMultiRoom(space) : true)}</span>
          {lock && (
            <LockIcon
              open={lock !== "verified"}
              role="img"
              aria-hidden={false}
              aria-label={t(lockLook(lock).aria)}
              strokeWidth={2.2}
              className={cn(
                "size-[13px]",
                lock === "verified" ? "text-muted-foreground" : lock === "pending" ? "text-dim" : "text-warning",
              )}
            />
          )}
        </span>
      </Button>
      <Button
        variant="ghost"
        aria-pressed={call.muted}
        aria-label={call.muted ? t("mute.off") : t("mute.on")}
        onClick={app.toggleMute}
        className={cn(
          "size-[52px] rounded-2xl",
          call.muted ? "bg-alarm/20 text-danger hover:bg-alarm/25 hover:text-danger" : "bg-border text-foreground hover:bg-idle",
        )}
      >
        <MicIcon muted={call.muted} className="size-[22px]" />
      </Button>
      <Button variant="leave" aria-label={t("voice.leave")} onClick={app.leave} className="size-[52px] rounded-2xl">
        <LeaveIcon className="size-[22px]" />
      </Button>
    </div>
  );
}

function NoSpaceNarrow({ app }: { app: AppState }) {
  const { t } = useLanguage();
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center gap-2.5 px-4 pt-[100px] text-center">
      <h1 className="text-[22px] font-semibold tracking-[-0.03em]">{t("empty.title")}</h1>
      <p className="max-w-[320px] text-sm text-subtle">{t("empty.help")}</p>
      <Button onClick={() => app.setInviteOpen(true)} className="mt-2.5 h-11 rounded-xl px-[18px] text-sm font-semibold">
        {t("empty.paste")}
      </Button>
    </div>
  );
}
