// The main area when it does not show the call: a room seen from outside
// (who is there, join), a call the server ended, or no space at all.
import { cn } from "cn";
import type { RoomView, SpaceView } from "@/api";
import { Button } from "@/components/ui/button";
import { hostOf } from "@/format";
import { useLanguage } from "@/lang";
import { inCall, isMultiRoom, refOf, whereLabel } from "@/model";
import type { AppState } from "@/state";
import { RoomHeader } from "./CallView";

/** A room this device is not in: who sits there, and a button to join.
    In a call elsewhere, it says which room joining leaves. */
export function PeekView({ app, space, room }: { app: AppState; space: SpaceView; room: RoomView }) {
  const { t } = useLanguage();
  const multi = isMultiRoom(space);
  const call = inCall(app.call) ? app.call : null;
  const callSpace = call && app.spaces?.find((s) => s.instance === call.instance && s.id === call.spaceId);
  const who = room.occupants;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <RoomHeader title={space.name} sub={multi ? `# ${room.name} · ${hostOf(space.instance)}` : hostOf(space.instance)} />
      <div className="flex min-h-0 flex-1 flex-col justify-center gap-5.5 px-14 pb-15">
        {!app.gateChosen && <CalibrateCard app={app} />}
        {who && who.length > 0 && (
          <>
            <div className="flex flex-col gap-1.5">
              {who.map((name) => (
                <span key={name} className="text-[2.125rem] font-semibold tracking-[-0.03em] text-dim">
                  {name}
                </span>
              ))}
            </div>
            <span className="text-sm text-muted-foreground">{t("peek.inRoom")}</span>
          </>
        )}
        {who && who.length === 0 && (
          <span className="text-[2.125rem] font-semibold tracking-[-0.03em] text-hush">{t("peek.empty")}</span>
        )}
        <div className="flex items-center gap-3.5">
          <Button size="xl" disabled={app.joining} onClick={() => app.join(refOf(space, room))}>
            {app.joining ? t("peek.joining") : t("peek.join")}
          </Button>
          {call && (
            <span className="text-[0.8125rem] text-muted-foreground">
              {t("peek.switching", {
                where: whereLabel(call.spaceName, call.roomName, callSpace ? isMultiRoom(callSpace) : true),
              })}
            </span>
          )}
        </div>
        {app.joinError && <p className="text-[0.8125rem] text-danger">{app.joinError}</p>}
      </div>
    </div>
  );
}

/** The server ended the call: this device joined elsewhere, or was removed. */
export function EndedView({ app, space, room }: { app: AppState; space: SpaceView | null; room: RoomView | null }) {
  const { t } = useLanguage();
  const call = app.call!;
  const multi = space ? isMultiRoom(space) : true;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <RoomHeader
        title={call.spaceName}
        sub={multi ? `# ${room?.name ?? call.roomName} · ${hostOf(call.instance)}` : hostOf(call.instance)}
      />
      <div role="alert" className="flex min-h-0 flex-1 flex-col justify-center gap-2 px-14 pb-15">
        <span className="text-[2.125rem] font-semibold tracking-[-0.03em]">{t("ended.title")}</span>
        <span className="text-sm text-subtle">{t("ended.help")}</span>
        <div className="mt-3.5 flex gap-2">
          <Button
            size="xl"
            disabled={app.joining}
            onClick={() => app.join({ instance: call.instance, spaceId: call.spaceId, roomId: call.roomId })}
          >
            {t("ended.rejoin")}
          </Button>
          <Button size="xl" variant="secondary" onClick={app.leave} className="px-4.5 font-medium">
            {t("ended.close")}
          </Button>
        </div>
        {app.joinError && <p className="text-[0.8125rem] text-danger">{app.joinError}</p>}
      </div>
    </div>
  );
}

/** Asks for the voice gate's calibration until a threshold was chosen: the
    default cannot suit every microphone (ADR 0034). */
export function CalibrateCard({ app, className }: { app: AppState; className?: string }) {
  const { t } = useLanguage();
  return (
    <section
      className={cn(
        "flex max-w-115 flex-col gap-2.5 rounded-2xl border border-primary/30 bg-primary/6 p-4.5",
        className,
      )}
    >
      <h2 className="text-base font-semibold">{t("calibrate.title")}</h2>
      <p className="text-[0.8125rem] text-subtle">{t("calibrate.help")}</p>
      <Button onClick={() => app.openSettings("voice")} className="h-10 self-start rounded-xl px-4 text-sm font-semibold">
        {t("calibrate.go")}
      </Button>
    </section>
  );
}
