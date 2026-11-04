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
  InvitationView,
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
import { Groups } from "./group";
import { fromSeed, type Identity, saveIdentity } from "./identity";
import { invitationLink } from "./links";
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
  /** The spaces' MLS groups in this browser (ADR 0052). */
  const groups = new Groups(identity);
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
    group: s.spaceKey === "",
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
    // A space whose keys come from its MLS group: this browser joins the
    // group first, which a member's app may have to let it into.
    const roomKey =
      space.spaceKey === ""
        ? async () => {
            const owner = space.role === "owner";
            const deadline = Date.now() + 120_000;
            for (;;) {
              const state = await groups.sync(control(instance), spaceId, owner).catch((e) => {
                throw (e as { code?: string }).code === "old_key"
                  ? appError("group_browser", "this browser's key is from before accounts: sign in again")
                  : asAppError(e);
              });
              if (state.ready) {
                const key = await groups.roomKey(spaceId, roomId);
                if (key) return key;
              }
              if (Date.now() > deadline) {
                throw appError("group_waiting", "no member's app let this browser into the space yet");
              }
              await new Promise((r) => setTimeout(r, 1000));
            }
          }
        : undefined;
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
      roomKey,
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

    createSpace: async (instance: string, creatorCode: string | null, name: string, displayName: string) => {
      const code = creatorCode?.trim() || null;
      let answer;
      try {
        answer = await control(instance).createSpace(name.trim(), displayName.trim(), code);
      } catch (e) {
        if (e instanceof ControlError && e.code === "not_found") throw appError("creator_code_unknown", e.message);
        if (e instanceof ControlError && e.code === "refused" && code && /: 400 /.test(e.message)) {
          throw appError("bad_code", e.message);
        }
        if (e instanceof ControlError && e.code === "refused" && /: 403 /.test(e.message)) {
          throw appError("creator_code_needed", e.message);
        }
        throw asAppError(e);
      }
      // An account with a user name gets a space with an MLS group; a
      // key-only one, 256 random bits only the members will ever hold.
      const me = await control(instance).me().catch(() => null);
      const signedIn = Boolean((me as { username?: string } | null)?.username);
      const spaceKey = signedIn ? "" : hex(crypto.getRandomValues(new Uint8Array(32)));
      const stored: StoredSpace = {
        instance,
        id: answer.space.id,
        name: answer.space.name,
        role: answer.role,
        spaceKey,
        rooms: answer.rooms.map((r) => ({ id: r.id, name: r.name })),
      };
      saveSpaces([...loadSpaces(), stored]);
      void sealRecoveryAgain();
      return view(stored, null);
    },

    createInvitation: async (instance: string, spaceId: string): Promise<InvitationView> => {
      const known = loadSpaces().find((s) => s.instance === instance && s.id === spaceId);
      if (!known) throw appError("not_found", "no such space in this browser");
      try {
        const created = await control(instance).createInvitation(spaceId);
        return {
          link: invitationLink(unhex(created.code), unhex(known.spaceKey), instance),
          uses: created.invitation.max_uses,
          expiresAt: created.invitation.expires_at,
        };
      } catch (e) {
        throw asAppError(e);
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

    // Accounts by e-mail (ADR 0055): the page signs in to the instance
    // that serves it (in development, the one Vite forwards to).
    defaultInstance: async () => (import.meta.env.DEV ? __DRYWIRE_PROXIED__ : location.origin),

    emailCode: async (instance: string, email: string, lang: string) => {
      try {
        await control(instance).emailCode(email.trim(), lang);
      } catch (e) {
        throw asAppError(e);
      }
    },

    emailSignIn: async (instance: string, email: string, code: string) => {
      try {
        const c = control(instance);
        const answer = await c.emailVerify(email.trim(), code);
        // The account's spaces: their keys come from their groups.
        const known = loadSpaces();
        const added: StoredSpace[] = [];
        for (const m of await c.spaces()) {
          if (known.some((s) => s.instance === instance && s.id === m.space.id)) continue;
          const space = await c.space(m.space.id);
          added.push({
            instance,
            id: m.space.id,
            name: m.space.name,
            role: m.role,
            spaceKey: "",
            rooms: space.rooms.map((r) => ({ id: r.id, name: r.name })),
          });
        }
        if (added.length > 0) saveSpaces([...known, ...added]);
        return { newAccount: answer.new_account, username: answer.username || null };
      } catch (e) {
        throw asAppError(e);
      }
    },

    setUsername: async (instance: string, username: string, displayName: string) => {
      try {
        return (await control(instance).setUsername(username.trim(), displayName.trim())).username;
      } catch (e) {
        throw asAppError(e);
      }
    },

    pendingInvitations: async (instance: string) => {
      try {
        const list = await control(instance).invitations();
        return list.map((i) => ({ instance, spaceId: i.space_id, spaceName: i.space_name, invitedBy: i.invited_by }));
      } catch (e) {
        throw asAppError(e);
      }
    },

    answerInvitation: async (instance: string, spaceId: string, accept: boolean) => {
      try {
        if (!accept) {
          await control(instance).declineInvitation(spaceId);
          return null;
        }
        const answer = await control(instance).acceptInvitation(spaceId);
        const stored: StoredSpace = {
          instance,
          id: answer.space.id,
          name: answer.space.name,
          role: answer.role,
          spaceKey: "",
          rooms: answer.rooms.map((r) => ({ id: r.id, name: r.name })),
        };
        const others = loadSpaces().filter((s) => !(s.instance === instance && s.id === stored.id));
        saveSpaces([...others, stored]);
        return view(stored, null);
      } catch (e) {
        throw asAppError(e);
      }
    },

    accountInfo: async (instance: string) => {
      try {
        const me = await control(instance).me();
        if (!me) throw appError("not_found", "this browser has no account here");
        return { username: me.username ?? "", email: me.email ?? "", name: me.name };
      } catch (e) {
        throw asAppError(e);
      }
    },

    accountDevices: async (instance: string) => {
      try {
        return (await control(instance).devices()).map((d) => ({
          publicKey: d.public_key,
          thisDevice: d.this_device,
          lastSeenAt: d.last_seen_at,
        }));
      } catch (e) {
        throw asAppError(e);
      }
    },

    removeDevice: async (instance: string, publicKey: string) => {
      try {
        await control(instance).removeDevice(publicKey);
      } catch (e) {
        throw asAppError(e);
      }
    },

    signOut: async (instance: string) => {
      try {
        await stopCall();
        const c = control(instance);
        await c.removeDevice(hex(identity.publicKey));
        c.forgetToken();
        saveSpaces(loadSpaces().filter((s) => s.instance !== instance));
      } catch (e) {
        throw asAppError(e);
      }
    },

    inviteUser: async (instance: string, spaceId: string, username: string) => {
      try {
        return (await control(instance).inviteUser(spaceId, username.trim())).name;
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
  return api;
}
