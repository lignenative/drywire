// The interface's commands in a browser: the third implementation of the
// desktop app's `Api` (desktop/src/api.ts), after the Rust side and the
// simulated call (ADR 0048). What a browser cannot do (the overlay, a
// global shortcut, launching at login, the loopback test) is refused here
// and left out of the settings.

import type {
  AccountView,
  Api,
  AppError,
  CalibrationView,
  DeviceCodeView,
  LoopbackView,
  Notice,
  Preferences,
  Settings,
  SpaceView,
} from "@/api";
import { AudioEngine, MicrophoneDenied, type PlaybackStats } from "./audio/engine";
import { calibrate } from "./audio/gate";
import { recordLevels, WebCall } from "./call";
import { type EncoderSettings, SAMPLE_RATE } from "./codec";
import { ControlError, ControlPlane } from "./control";
import { fromSeed, type Identity, saveIdentity } from "./identity";
import { hex, type Protocol, ProtocolError, unhex } from "./protocol";
import {
  loadRecovery,
  loadSettings,
  loadSpaces,
  saveRecovery,
  saveSettings,
  saveSpaces,
  type StoredSpace,
} from "./store";
import {
  BadCode,
  formatCode,
  newSecret,
  open as openVault,
  parseCode,
  seal,
  type Secret,
  vaultId,
  type VaultContents,
  WrongCode,
} from "./vault";

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

export function createWebApi(protocol: Protocol, firstIdentity: Identity, codec: EncoderSettings): WebApi {
  /** The account: another one replaces it when a code is opened. */
  let identity = firstIdentity;
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

  /** The instances of the spaces, each once. */
  const instances = () => [...new Set(loadSpaces().map((s) => s.instance))].sort();

  /** What a vault of this account holds. */
  const contents = async (): Promise<VaultContents> => {
    if (!identity.seed) throw appError("not_portable", "this browser's key cannot be copied");
    const first = instances()[0];
    const me = first ? await control(first).me().catch(() => null) : null;
    return {
      account: hex(identity.seed),
      name: me?.name ?? "",
      spaces: loadSpaces().map((s) => ({ instance: s.instance, id: s.id, name: s.name, key: s.spaceKey })),
    };
  };

  /** Seals the recovery vault with `secret` on every instance of the spaces. */
  const putRecovery = async (secret: Secret) => {
    const all = instances();
    if (all.length === 0) throw appError("no_space", "join a space first");
    const plain = new TextEncoder().encode(JSON.stringify(await contents()));
    const id = await vaultId(secret);
    for (const instance of all) await control(instance).putRecoveryVault(id, await seal(secret, plain));
  };

  /** After the spaces changed, if this browser knows the code. A failure is
      left for the next change. */
  const sealRecoveryAgain = async () => {
    const known = loadRecovery();
    if (known) await putRecovery(unhex(known)).catch(() => {});
  };

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

  const api: WebApi = {
    publicKey: async () => hex(identity.publicKey),

    account: async (): Promise<AccountView> => ({
      recoveryCode: loadRecovery() !== null,
      spaces: loadSpaces().length,
      portable: identity.seed !== null,
    }),

    newRecoveryCode: async () => {
      const secret = newSecret();
      try {
        await putRecovery(secret);
      } catch (e) {
        throw "code" in (e as object) && !(e instanceof Error) ? e : asAppError(e);
      }
      saveRecovery(hex(secret));
      return formatCode(secret);
    },

    newDeviceCode: async (): Promise<DeviceCodeView> => {
      const instance = instances()[0];
      if (!instance) throw appError("no_space", "join a space first");
      const secret = newSecret();
      try {
        const plain = new TextEncoder().encode(JSON.stringify(await contents()));
        const waiting = await control(instance).putDeviceVault(await vaultId(secret), await seal(secret, plain));
        return { code: formatCode(secret), instance, expiresInS: waiting.expires_in };
      } catch (e) {
        throw "code" in (e as object) && !(e instanceof Error) ? e : asAppError(e);
      }
    },

    restoreAccount: async (instance: string, code: string, replace: boolean) => {
      let secret: Secret;
      try {
        secret = parseCode(code);
      } catch {
        throw appError("bad_code", "this is not a Drywire code");
      }
      let found: { kind: "recovery" | "device"; contents: VaultContents };
      try {
        const opened = await control(instance).openVault(await vaultId(secret));
        const plain = await openVault(secret, opened.sealed);
        found = { kind: opened.kind, contents: JSON.parse(new TextDecoder().decode(plain)) };
      } catch (e) {
        if (e instanceof ControlError && e.code === "not_found") throw appError("code_unknown", e.message);
        if (e instanceof WrongCode || e instanceof BadCode) throw appError("code_unknown", e.message);
        throw asAppError(e);
      }
      const account = await fromSeed(unhex(found.contents.account));
      const current = loadSpaces();
      const same = hex(account.publicKey) === hex(identity.publicKey);
      if (!same && current.length > 0 && !replace) {
        throw appError("account_exists", `this browser already has an account, with ${current.length} space(s)`);
      }
      await stopCall();
      const brought: StoredSpace[] = found.contents.spaces.map((s) => ({
        instance: s.instance,
        id: s.id,
        name: s.name,
        role: "",
        spaceKey: s.key,
        rooms: [],
      }));
      const kept = same ? current.filter((c) => !brought.some((b) => b.instance === c.instance && b.id === c.id)) : [];
      await saveIdentity(account);
      identity = account;
      controls.clear();
      saveSpaces([...brought, ...kept]);
      saveRecovery(found.kind === "recovery" ? hex(secret) : null);
      return { name: found.contents.name, spaces: await api.listSpaces() };
    },

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
        void sealRecoveryAgain();
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
  return api;
}
