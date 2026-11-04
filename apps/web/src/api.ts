// The interface's commands in a browser: the third implementation of the
// desktop app's `Api` (desktop/src/api.ts), after the Rust side and the
// simulated call (ADR 0048). What a browser cannot do (the overlay, a
// global shortcut, launching at login, the loopback test) is refused here
// and left out of the settings.

import type { Api, AppError, CalibrationView, LoopbackView, Notice, Preferences, Settings, SpaceView } from "@/api";
import { AudioEngine, MicrophoneDenied, type PlaybackStats } from "./audio/engine";
import { calibrate } from "./audio/gate";
import { recordLevels, WebCall } from "./call";
import { type EncoderSettings, SAMPLE_RATE } from "./codec";
import { ControlError, ControlPlane } from "./control";
import type { Identity } from "./identity";
import { hex, type Protocol, ProtocolError } from "./protocol";
import { loadSettings, loadSpaces, saveSettings, saveSpaces, type StoredSpace } from "./store";

const CALIBRATION_SECONDS = 10;

/** The control plane's address for an instance. The page's own origin, or
    the instance `make web`'s Vite forwards to, are reached through the
    page's origin; any other instance directly, which needs the control
    plane to answer cross-origin requests (not yet). */
function baseFor(instance: string): string {
  return instance === location.origin || instance === __DRYWIRE_PROXIED__ ? "" : instance;
}

function appError(code: string, message: string): AppError {
  return { code, message };
}

/** A failure as the interface reads it. */
function asAppError(e: unknown): AppError {
  if (e instanceof ControlError) return appError(e.code, e.message);
  if (e instanceof ProtocolError) return appError("protocol", e.message);
  if (e instanceof MicrophoneDenied) return appError("microphone", e.message);
  return appError("unknown", e instanceof Error ? e.message : String(e));
}

/** The interface's commands, and more:
    - the output's glitches during the call, for measuring (README,
      "Measure");
    - the two commands branch claude/windows-v3-gaps adds to `Api`, ready
      for when it merges: the microphone's level, which stays at the floor
      out of a call (a browser opens the microphone only in one, as the
      microphone dialog promises), and changes to the spaces, which this
      client does not follow yet. */
export type WebApi = Api & {
  playbackStats(): PlaybackStats | null;
  inputLevel(): Promise<number>;
  onSpaces(f: (spaces: SpaceView[]) => void): Promise<() => void>;
};

export function createWebApi(protocol: Protocol, identity: Identity, codec: EncoderSettings): WebApi {
  const controls = new Map<string, ControlPlane>();
  const listeners = new Set<(notice: Notice) => void>();
  let settings = loadSettings();
  let call: WebCall | null = null;
  /** The call being started: leaving or switching waits for it. */
  let starting: Promise<unknown> | null = null;
  /** Joins run one after the other: two at once would share the
      protocol module's one client and leave a microphone open. */
  let joins: Promise<unknown> = Promise.resolve();

  const control = (instance: string) => {
    let c = controls.get(instance);
    if (!c) controls.set(instance, (c = new ControlPlane(baseFor(instance), instance, identity, protocol)));
    return c;
  };

  const view = (s: StoredSpace, error: AppError | null): SpaceView => ({
    instance: s.instance,
    id: s.id,
    name: s.name,
    role: s.role,
    // Who sits in other rooms is not read yet, as in the app (api.ts).
    rooms: s.rooms.map((r) => ({ id: r.id, name: r.name })),
    error,
  });

  const fullSettings = (): Settings => ({
    gateOpenDbfs: settings.gateOpenDbfs,
    volumes: { ...settings.volumes },
    muteShortcut: "",
    soundVolume: settings.soundVolume,
    overlay: false,
    overlayCorner: "top-left",
  });

  const save = () => saveSettings(settings);

  const stopCall = async () => {
    await starting?.catch(() => {});
    const current = call;
    call = null;
    await current?.leave();
  };

  const join = async (instance: string, spaceId: string, roomId: string) => {
    await stopCall();
    const space = loadSpaces().find((s) => s.instance === instance && s.id === spaceId);
    const room = space?.rooms.find((r) => r.id === roomId);
    if (!space || !room) throw appError("not_found", "no such room in this browser");
    const started = WebCall.start({
      protocol,
      identity,
      control: control(instance),
      space,
      roomId,
      roomName: room.name,
      codec,
      settings: () => settings,
      onNotice: (notice) => listeners.forEach((f) => f(notice)),
    });
    starting = started;
    try {
      call = await started;
    } catch (e) {
      throw asAppError(e);
    } finally {
      starting = null;
    }
    return call.view();
  };

  return {
    publicKey: async () => hex(identity.publicKey),

    acceptInvitation: async (link: string, displayName: string) => {
      let invitation;
      try {
        invitation = protocol.invitation(link);
      } catch (e) {
        throw appError("invalid_link", (e as Error).message);
      }
      try {
        const answer = await control(invitation.instance).redeem(invitation.code, displayName);
        const stored: StoredSpace = {
          instance: invitation.instance,
          id: answer.space.id,
          name: answer.space.name,
          role: answer.role,
          spaceKey: hex(invitation.spaceKey),
          rooms: answer.rooms.map((r) => ({ id: r.id, name: r.name })),
        };
        const others = loadSpaces().filter((s) => !(s.instance === stored.instance && s.id === stored.id));
        saveSpaces([...others, stored]);
        // The link served its purpose: it leaves the address bar.
        if (location.hash) history.replaceState(null, "", location.pathname + location.search);
        return view(stored, null);
      } catch (e) {
        throw asAppError(e);
      }
    },

    listSpaces: async () => {
      const spaces = loadSpaces();
      const views = await Promise.all(
        spaces.map(async (s) => {
          try {
            const answer = await control(s.instance).space(s.id);
            s.name = answer.space.name;
            s.role = answer.role;
            s.rooms = answer.rooms.map((r) => ({ id: r.id, name: r.name }));
            return view(s, null);
          } catch (e) {
            return view(s, asAppError(e));
          }
        }),
      );
      saveSpaces(spaces);
      return views;
    },

    joinRoom: (instance: string, spaceId: string, roomId: string) => {
      const next = joins.catch(() => {}).then(() => join(instance, spaceId, roomId));
      joins = next;
      return next;
    },

    leaveRoom: () => stopCall(),

    callStatus: async () => call?.view() ?? null,

    setMuted: async (muted: boolean) => call?.setMuted(muted),

    retryNow: async () => call?.retryNow(),

    setGate: async (openDbfs: number) => {
      settings = { ...settings, gateOpenDbfs: openDbfs };
      save();
      call?.setGate(openDbfs);
    },

    setVolume: async (publicKey: string, volume: number) => {
      settings = { ...settings, volumes: { ...settings.volumes, [publicKey]: volume } };
      save();
      call?.setVolume(publicKey, volume);
    },

    getSettings: async () => fullSettings(),

    setPreferences: async (preferences: Preferences) => {
      settings = { ...settings, soundVolume: preferences.soundVolume };
      save();
      return fullSettings();
    },

    // No tray menu to translate in a browser.
    setLanguage: async () => {},

    loopbackTest: (): Promise<LoopbackView> =>
      Promise.reject(appError("unknown", "the loopback test needs the Drywire app")),

    calibrateGate: async (): Promise<CalibrationView> => {
      let levels: number[];
      try {
        if (call) {
          levels = await call.calibrate(CALIBRATION_SECONDS);
        } else {
          // Out of a call, the microphone opens for the ten seconds only.
          const engine = await AudioEngine.start((codec.frameMs * SAMPLE_RATE) / 1000);
          try {
            levels = await recordLevels(engine, CALIBRATION_SECONDS);
          } finally {
            engine.close();
          }
        }
      } catch (e) {
        throw asAppError(e);
      }
      const found = calibrate(levels);
      if (!found) throw appError("audio", "the microphone gave no sound");
      return { ...found, gateDbfs: Math.round(found.gateDbfs) };
    },

    launchAtLogin: async () => false,

    setLaunchAtLogin: (): Promise<boolean> =>
      Promise.reject(appError("unknown", "a browser does not start with the session")),

    playbackStats: () => call?.playbackStats() ?? null,

    inputLevel: async () => call?.inputLevel() ?? -90,

    onSpaces: async () => () => {},

    onPresence: async (f: (notice: Notice) => void) => {
      listeners.add(f);
      return () => {
        listeners.delete(f);
      };
    },
  };
}
