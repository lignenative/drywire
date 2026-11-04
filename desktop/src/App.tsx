// The window. Wide: the side menu of spaces on the left, and beside it the
// call, a room seen from outside, or the settings. Narrow: one screen at a
// time, the spaces first (the canvas's V3 page, ADR 0037).
import { CallView } from "@/components/CallView";
import { InviteDialog } from "@/components/InviteDialog";
import { NarrowHome } from "@/components/NarrowHome";
import { EndedView, NoSpaceView, PeekView } from "@/components/RoomViews";
import { NarrowSettings, SettingsPage } from "@/components/Settings";
import { SideMenu } from "@/components/SideMenu";
import { useNarrow } from "@/hooks";
import { findRoom } from "@/model";
import { showsCall, useAppState, type AppState } from "@/state";

export function App() {
  const app = useAppState();
  const narrow = useNarrow();
  return (
    <div className="h-screen overflow-hidden bg-background text-sm leading-[1.4] text-foreground">
      {app.page === "settings" ? (
        narrow ? (
          <NarrowSettings app={app} />
        ) : (
          <SettingsPage app={app} />
        )
      ) : narrow ? (
        app.narrowScreen === "call" && app.call ? (
          <div className="flex h-full flex-col">
            <CallView app={app} call={app.call} narrow />
          </div>
        ) : (
          <NarrowHome app={app} />
        )
      ) : (
        <div className="flex h-full">
          <SideMenu app={app} />
          <main className="relative flex min-w-0 flex-1 flex-col">
            <MainArea app={app} />
          </main>
        </div>
      )}
      <InviteDialog app={app} narrow={narrow} />
    </div>
  );
}

function MainArea({ app }: { app: AppState }) {
  if (!app.spaces) return null;
  if (app.call && showsCall(app)) {
    const found = findRoom(app.spaces, app.selected);
    return app.call.ended ? (
      <EndedView app={app} space={found?.space ?? null} room={found?.room ?? null} />
    ) : (
      <CallView app={app} call={app.call} />
    );
  }
  const found = findRoom(app.spaces, app.selected);
  if (found) return <PeekView app={app} space={found.space} room={found.room} />;
  return app.spaces.length === 0 ? <NoSpaceView app={app} /> : null;
}
