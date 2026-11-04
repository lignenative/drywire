// The window's state in one place: the spaces, the call, the settings,
// which room the main area shows and which page is open. The screens get
// it as one object and call its actions; they hold only what is theirs
// (a popover being open, a slider being dragged).
import { useCallback, useEffect, useRef, useState } from "react";
import { api, type CallView, type Settings, type SpaceView } from "./api";
import { offeredInvitation } from "./invite";
import { errorText, useLanguage } from "./lang";
import { callRoom, findRoom, firstRoom, inCall, sameRoom, type RoomRef } from "./model";
import { playPresence } from "./sounds";

/** How often the call's numbers are read while in a room. */
const POLL_MS = 100;

/** The name typed in the last invitation, offered again for the next. */
export const NAME_KEY = "drywire.displayName";

export type SettingsTab = "voice" | "sounds" | "display" | "account" | "advanced";

export const SETTINGS_TABS: readonly SettingsTab[] = ["voice", "sounds", "display", "account", "advanced"];

export interface AppState {
  spaces: SpaceView[] | null;
  call: CallView | null;
  settings: Settings | null;
  /** The room the main area shows: the call's, or one being looked at. */
  selected: RoomRef | null;
  page: "main" | "settings";
  settingsTab: SettingsTab;
  inviteOpen: boolean;
  /** The narrow window shows one screen at a time. */
  narrowScreen: "spaces" | "call";
  joining: boolean;
  /** Why the last join failed, in the interface's language. */
  joinError: string | null;
  /** The name this device goes by, for the bar at the bottom. */
  displayName: string;
  /** Whether the voice gate's threshold was chosen, or is the default. */
  gateChosen: boolean;

  select: (ref: RoomRef) => void;
  join: (ref: RoomRef) => Promise<void>;
  leave: () => Promise<void>;
  toggleMute: () => void;
  /** While reconnecting, joins again now instead of after the countdown. */
  retryNow: () => void;
  reloadSpaces: () => void;
  openSettings: (tab?: SettingsTab) => void;
  closeSettings: () => void;
  setSettingsTab: (tab: SettingsTab) => void;
  setInviteOpen: (open: boolean) => void;
  accepted: (space: SpaceView, name: string) => void;
  /** Another account is this device's now (ADR 0051): its name, its spaces. */
  restored: (name: string, spaces: SpaceView[]) => void;
  setSettings: (settings: Settings) => void;
  setNarrowScreen: (screen: "spaces" | "call") => void;
}

export function useAppState(): AppState {
  const { lang, t } = useLanguage();
  const [spaces, setSpaces] = useState<SpaceView[] | null>(null);
  const [call, setCall] = useState<CallView | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [selected, setSelected] = useState<RoomRef | null>(null);
  const [page, setPage] = useState<"main" | "settings">("main");
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("voice");
  // An invitation the window was opened with fills the form at once.
  const [inviteOpen, setInviteOpen] = useState(() => offeredInvitation() !== null);
  const [narrowScreen, setNarrowScreen] = useState<"spaces" | "call">("spaces");
  const [joining, setJoining] = useState(false);
  const [joinError, setJoinError] = useState<string | null>(null);
  const [displayName, setDisplayName] = useState(() => localStorage.getItem(NAME_KEY) ?? "");
  const soundVolume = useRef(0.5);
  soundVolume.current = settings?.soundVolume ?? soundVolume.current;

  const readSettings = useCallback(() => {
    api.getSettings().then(setSettings, () => {});
  }, []);

  const reloadSpaces = useCallback(() => {
    api.listSpaces().then(setSpaces, () => setSpaces([]));
  }, []);

  useEffect(() => {
    reloadSpaces();
    readSettings();
    // A call that was running before a reload of the window.
    api.callStatus().then(setCall, () => {});
  }, []);

  // The tray menu speaks the interface's language.
  useEffect(() => {
    api.setLanguage(lang).catch(() => {});
  }, [lang]);

  // Who sits in each room, pushed at every change: the side menu lists
  // names under every room, and asking the servers would cost a request
  // per space.
  useEffect(() => {
    const unlisten = api.onSpaces(setSpaces);
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  // The presence sounds. Events, not polling: they arrive even while
  // the window is hidden in the tray and its timers are slowed down.
  useEffect(() => {
    const unlisten = api.onPresence((notice) => playPresence(notice.kind, soundVolume.current));
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  // Polled, never pushed: the numbers are a copy the session thread
  // refreshes, so reading them never touches the audio.
  useEffect(() => {
    if (!call) return;
    const timer = setInterval(() => {
      api.callStatus().then(setCall, () => {});
    }, POLL_MS);
    return () => clearInterval(timer);
  }, [call !== null]);

  // The main area follows the call when one starts, and otherwise shows
  // the first room until the person picks one.
  const callKey = call ? `${call.instance} ${call.spaceId} ${call.roomId}` : null;
  useEffect(() => {
    if (call) setSelected(callRoom(call));
  }, [callKey]);
  useEffect(() => {
    if (!spaces) return;
    setSelected((current) => (current && findRoom(spaces, current) ? current : firstRoom(spaces)));
  }, [spaces]);

  const me = call?.participants.find((p) => p.me);
  useEffect(() => {
    if (me?.name) setDisplayName(me.name);
  }, [me?.name]);

  const join = useCallback(
    async (ref: RoomRef) => {
      setJoining(true);
      setJoinError(null);
      try {
        // The Rust side leaves the current call first.
        setCall(await api.joinRoom(ref.instance, ref.spaceId, ref.roomId));
        setSelected(ref);
      } catch (e) {
        setJoinError(errorText(t, e));
      } finally {
        setJoining(false);
      }
    },
    [t],
  );

  const leave = useCallback(async () => {
    await api.leaveRoom().catch(() => {});
    setCall(null);
    setNarrowScreen("spaces");
    // The gate may have been moved during the call.
    readSettings();
  }, [readSettings]);

  const toggleMute = useCallback(() => {
    if (!inCall(call)) return;
    const muted = !call.muted;
    setCall({ ...call, muted });
    api.setMuted(muted).catch(() => {});
  }, [call]);

  const retryNow = useCallback(() => {
    // The countdown goes at once, so the button does too: no second click
    // on a control that already did its work.
    if (inCall(call)) setCall({ ...call, retryInS: null });
    api.retryNow().catch(() => {});
  }, [call]);

  const openSettings = useCallback(
    (tab?: SettingsTab) => {
      if (tab) setSettingsTab(tab);
      readSettings();
      setPage("settings");
    },
    [readSettings],
  );

  const restored = useCallback((name: string, list: SpaceView[]) => {
    if (name) {
      localStorage.setItem(NAME_KEY, name);
      setDisplayName(name);
    }
    setSpaces(list);
    const first = list.find((s) => s.rooms.length > 0);
    if (first) setSelected({ instance: first.instance, spaceId: first.id, roomId: first.rooms[0].id });
  }, []);

  const accepted = useCallback(
    (space: SpaceView, name: string) => {
      localStorage.setItem(NAME_KEY, name);
      setDisplayName(name);
      api.listSpaces().then((list) => {
        setSpaces(list);
        const fresh = list.find((s) => s.instance === space.instance && s.id === space.id);
        if (fresh && fresh.rooms.length > 0 && !inCall(call)) {
          setSelected({ instance: fresh.instance, spaceId: fresh.id, roomId: fresh.rooms[0].id });
        }
      }, () => {});
    },
    [call],
  );

  const gateChosen = inCall(call) ? call.gateChosen : settings ? settings.gateOpenDbfs !== null : true;

  return {
    spaces,
    call,
    settings,
    selected,
    page,
    settingsTab,
    inviteOpen,
    narrowScreen,
    joining,
    joinError,
    displayName,
    gateChosen,
    select: (ref) => {
      setJoinError(null);
      setSelected(ref);
    },
    join,
    leave,
    toggleMute,
    retryNow,
    reloadSpaces,
    openSettings,
    closeSettings: () => setPage("main"),
    setSettingsTab,
    setInviteOpen,
    accepted,
    restored,
    setSettings,
    setNarrowScreen,
  };
}

/** Whether the main area shows the call itself. */
export function showsCall(app: AppState): boolean {
  return !!app.call && sameRoom(app.selected, callRoom(app.call));
}
