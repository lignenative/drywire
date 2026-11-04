// A call that exists only in this browser tab: the interface's commands
// answered without Tauri, the Rust core or a server (ADR 0037). It lets
// the design be checked and iterated on in any browser (`make ui`).
//
// Nothing here is measured. The numbers are made up to look like a real
// call, so they must never be shown as Drywire's latency anywhere else.
//
// `?scene=` in the address picks one of the canvas's states, so each
// artboard can be compared with the interface: call, muted, alone,
// connecting, reconnecting, ended, poor, e2eePending, e2eeFailed,
// firstRun, empty, unreachable.
import type { UnlistenFn } from "@tauri-apps/api/event";
import type {
  Api,
  CalibrationView,
  CallView,
  DeviceView,
  EncryptionView,
  LoopbackView,
  Notice,
  Participant,
  Preferences,
  Settings,
  SpaceView,
} from "./api";

export type Scene =
  | "idle"
  | "call"
  | "muted"
  | "alone"
  | "connecting"
  | "reconnecting"
  | "ended"
  | "poor"
  | "e2eePending"
  | "e2eeFailed"
  | "firstRun"
  | "empty"
  | "unreachable";

const SCENES: readonly Scene[] = [
  "idle",
  "call",
  "muted",
  "alone",
  "connecting",
  "reconnecting",
  "ended",
  "poor",
  "e2eePending",
  "e2eeFailed",
  "firstRun",
  "empty",
  "unreachable",
];

/** The scene the address asks for; idle outside a browser (the tests). */
function sceneFromAddress(): Scene {
  const asked = typeof location === "undefined" ? null : new URLSearchParams(location.search).get("scene");
  return SCENES.find((s) => s === asked) ?? "idle";
}

/** Who sits in each room before you join, by space and room. */
const OCCUPANTS: Record<string, string[]> = {
  "team/general": ["sam", "kim", "lee"],
  "team/afk": ["ana"],
  "crew/general": ["jo", "max"],
};

/** The same spaces as the design canvas. */
function spaces(scene: Scene): SpaceView[] {
  if (scene === "empty") return [];
  const list: SpaceView[] = [
    space("voice.example.org", "team", "Team", ["general", "scrims", "afk"]),
    space("drywire.example.net", "friday", "Friday night", ["general"]),
    space("old.example.com", "crew", "Old crew", ["general"]),
  ];
  if (scene === "unreachable") {
    list.push({
      ...space("lan.example.com", "lan", "LAN party", []),
      error: { code: "network", message: "dial tcp: connection refused" },
    });
  }
  return list;
}

function space(instance: string, id: string, name: string, rooms: string[]): SpaceView {
  return {
    instance,
    id,
    name,
    role: "member",
    rooms: rooms.map((room) => ({ id: room, name: room, occupants: OCCUPANTS[`${id}/${room}`] ?? [] })),
    error: null,
  };
}

const USB: DeviceView[] = [
  { direction: "input", name: "Headset microphone (USB)", mode: "WASAPI exclusive", bufferMs: 10, latencyMs: 12, bluetooth: false },
  { direction: "output", name: "Headset (USB)", mode: "WASAPI exclusive", bufferMs: 10, latencyMs: 10, bluetooth: false },
];

const BLUETOOTH: DeviceView[] = [
  { direction: "input", name: "Wireless headset (Bluetooth)", mode: "WASAPI shared", bufferMs: 10, latencyMs: 28, bluetooth: true },
  { direction: "output", name: "Wireless headset (Bluetooth)", mode: "WASAPI shared", bufferMs: 10, latencyMs: 32, bluetooth: true },
];

/** Estimates per person, as the real app would show them. */
const ESTIMATES: Record<string, number> = { sam: 47, kim: 61, lee: 52, ana: 58, jo: 55, max: 64 };

/** Playback counters per person, so each card reads differently. */
const RECEIVED: Record<string, number> = { sam: 12480, kim: 11902, lee: 9310 };

/** The canvas's encryption scenes: noa just joined and her announcement
    has not arrived yet; lee's signature fails, so her audio is dropped. */
const NEWCOMER = "noa";
const FORGED = "lee";

/** What a bad link adds to every estimate. */
const POOR_EXTRA_MS = 168;

/** The default threshold until the person chooses one, as in the core. */
const DEFAULT_GATE = -50;

/** How long the simulated relay takes to answer. */
const BIND_S = 0.6;

/** How often a lost session tries again, as the countdown shows it. */
const RETRY_S = 5;

interface Call {
  space: SpaceView;
  roomId: string;
  since: number;
  muted: boolean;
}

const scene = sceneFromAddress();
const calibrated = !["firstRun", "alone"].includes(scene);

const state = {
  spaces: spaces(scene),
  settings: {
    gateOpenDbfs: calibrated ? -58 : null,
    volumes: { [keyOf("lee")]: 1.4 },
    muteShortcut: "Ctrl+Shift+M",
    soundVolume: 0.5,
    overlay: true,
    overlayCorner: "top-left",
  } as Settings,
  call: null as Call | null,
  launchAtLogin: false,
  recoveryCode: false,
  listeners: new Set<(notice: Notice) => void>(),
};

// The canvas's person is alex: the name an invitation would have saved.
if (typeof localStorage !== "undefined" && localStorage.getItem("drywire.displayName") === null) {
  localStorage.setItem("drywire.displayName", "alex");
}

// The in-call scenes open the window in Team's general room.
if (["call", "muted", "alone", "connecting", "reconnecting", "ended", "poor", "e2eePending", "e2eeFailed"].includes(scene)) {
  state.call = { space: state.spaces[0], roomId: "general", since: Date.now() - 5000, muted: scene === "muted" };
}

/** A slow, uneven wave between 0 and 1, so voices come and go. */
function wave(t: number, speed: number, phase: number): number {
  return (Math.sin(t * speed + phase) * 0.6 + Math.sin(t * speed * 3.7 + phase * 2) * 0.4 + 1) / 2;
}

function participants(call: Call, t: number, live: boolean, inputDbfs: number, gate: number): Participant[] {
  const seated = ["alone", "connecting"].includes(scene) ? [] : (OCCUPANTS[`${call.space.id}/${call.roomId}`] ?? []);
  const others = scene === "e2eePending" && seated.length > 0 ? [...seated, NEWCOMER] : seated;
  const me = person("alex", 0, true, live && !call.muted && inputDbfs > gate);
  // A rejected voice is dropped, never played: it cannot be heard speaking.
  const heard = (name: string) => !(scene === "e2eeFailed" && name === FORGED);
  return [
    me,
    ...others.map((name, i) =>
      person(name, i + 1, false, live && heard(name) && i < 2 && wave(t, 0.6 + i * 0.25, i * 2) > 0.55),
    ),
  ];
}

function person(name: string, session: number, me: boolean, speaking: boolean): Participant {
  const volume = state.settings.volumes[keyOf(name)] ?? 1;
  const received = RECEIVED[name] ?? 8000;
  const estimate = (ESTIMATES[name] ?? 55) + (scene === "poor" ? POOR_EXTRA_MS : 0);
  return {
    session,
    name,
    publicKey: keyOf(name),
    me,
    speaking,
    volume,
    bufferMs: me ? null : 20,
    lossPct: me ? null : 0,
    estimateMs: me ? null : estimate,
    counters: { received, played: received - 28, concealed: 18, late: 6, dropped: 4, depth: 2 },
  };
}

/** A stand-in public key per name, stable from one run to the next. */
function keyOf(name: string): string {
  return [...name].map((c) => c.charCodeAt(0).toString(16)).join("").padEnd(64, "0").slice(0, 64);
}

function view(call: Call): CallView {
  const t = (Date.now() - call.since) / 1000;
  const gate = state.settings.gateOpenDbfs ?? DEFAULT_GATE;
  const quiet = scene === "alone";
  const inputDbfs = quiet ? -66 + wave(t, 1.1, 0.5) * 10 : -62 + wave(t, 1.1, 0.5) * 34;
  const poor = scene === "poor";
  const bound = scene !== "connecting" && scene !== "reconnecting" && t > BIND_S;
  const ended = scene === "ended";
  const people = participants(call, t, bound && !ended, inputDbfs, gate);
  return {
    instance: call.space.instance,
    spaceId: call.space.id,
    spaceName: call.space.name,
    roomId: call.roomId,
    roomName: call.space.rooms.find((r) => r.id === call.roomId)?.name ?? call.roomId,
    ended: ended ? "the server ended this session: joined elsewhere, or removed" : null,
    reconnecting: scene === "reconnecting",
    retryInS: scene === "reconnecting" ? RETRY_S - (Math.floor(t) % RETRY_S) : null,
    bound: bound && !ended,
    rttMs: bound ? (poor ? 182 : 14) : null,
    minRttMs: bound ? (poor ? 96 : 11) : null,
    deviceMs: poor ? 231 : 88,
    deviceMeasured: true,
    muted: call.muted,
    inputDbfs,
    gateDbfs: gate,
    gateChosen: state.settings.gateOpenDbfs !== null,
    uplinkLossPct: poor ? 4 : 0,
    participants: people,
    devices: poor ? BLUETOOTH : USB,
    encryption: encryption(people, bound && !ended),
  };
}

function encryption(people: Participant[], bound: boolean): EncryptionView {
  const others = people.filter((p) => !p.me);
  const list = others.map((p) => ({
    publicKey: p.publicKey,
    verified: !(scene === "e2eePending" && p.name === NEWCOMER) && !(scene === "e2eeFailed" && p.name === FORGED),
    rejected: scene === "e2eeFailed" && p.name === FORGED,
  }));
  const state = list.some((p) => p.rejected)
    ? "failed"
    : !bound || others.length === 0 || list.some((p) => !p.verified)
      ? "pending"
      : "verified";
  return { state, people: list };
}

function emit(notice: Notice) {
  for (const f of state.listeners) f(notice);
}

const resolve = <T>(value: T) => Promise.resolve(value);

/** Codes, as people type them: groups of four, any case (protocol
    section 15.1). */
function looksLikeCode(text: string): boolean {
  return /^[0-9A-HJKMNP-TV-Z]{26}$/.test(text.toUpperCase().replace(/[-\s]/g, "").replace(/O/g, "0").replace(/[IL]/g, "1"));
}

export const simulated: Api = {
  publicKey: () => resolve("07e5c3a18f6d4b2907e5c3a18f6d4b2907e5c3a18f6d4b2907e5c3a18f6d4b29"),
  account: () => resolve({ recoveryCode: state.recoveryCode, spaces: state.spaces.length, portable: true }),
  newRecoveryCode: () => {
    if (state.spaces.length === 0) return Promise.reject({ code: "no_space", message: "join a space first" });
    state.recoveryCode = true;
    return resolve("HWMK-A9BQ-S8E2-ZCFM-PK19-NRRR-18");
  },
  newDeviceCode: () => {
    if (state.spaces.length === 0) return Promise.reject({ code: "no_space", message: "join a space first" });
    return resolve({ code: "7Q2M-KD4X-0RTA-9WEP-HJ3N-B5VC-6G", instance: state.spaces[0].instance, expiresInS: 600 });
  },
  restoreAccount: (_instance: string, code: string, replace: boolean) => {
    if (!looksLikeCode(code)) return Promise.reject({ code: "bad_code", message: "this is not a Drywire code" });
    // Any code of zeros stands for one that expired or was used.
    if (/^[0-]+$/.test(code.trim())) return Promise.reject({ code: "code_unknown", message: "unknown, used or expired" });
    if (state.spaces.length > 0 && !replace) {
      return Promise.reject({ code: "account_exists", message: `this device already has an account, with ${state.spaces.length} space(s)` });
    }
    state.call = null;
    state.spaces = spaces("idle");
    return resolve({ name: "alex", spaces: state.spaces.map((s) => ({ ...s })) });
  },
  acceptInvitation: (_link: string, _displayName: string) => {
    // Joining by invitation adds the canvas's spaces when there were none.
    if (state.spaces.length === 0) state.spaces = spaces("idle");
    return resolve(state.spaces[0]);
  },
  listSpaces: () => resolve(state.spaces.map((s) => ({ ...s }))),
  joinRoom: (instance: string, spaceId: string, roomId: string) => {
    const space = state.spaces.find((s) => s.instance === instance && s.id === spaceId);
    if (!space) return Promise.reject({ code: "not_found", message: "no such space" });
    state.call = { space, roomId, since: Date.now(), muted: false };
    const others = OCCUPANTS[`${spaceId}/${roomId}`] ?? [];
    if (others.length > 0) setTimeout(() => state.call && emit({ kind: "join", name: others[0] }), 1500);
    return resolve(view(state.call));
  },
  leaveRoom: () => {
    state.call = null;
    return resolve(undefined);
  },
  callStatus: () => resolve(state.call ? view(state.call) : null),
  setMuted: (muted: boolean) => {
    if (state.call) state.call.muted = muted;
    return resolve(undefined);
  },
  setGate: (openDbfs: number) => {
    state.settings.gateOpenDbfs = openDbfs;
    return resolve(undefined);
  },
  setVolume: (publicKey: string, volume: number) => {
    state.settings.volumes[publicKey] = volume;
    return resolve(undefined);
  },
  getSettings: () => resolve({ ...state.settings }),
  setPreferences: (preferences: Preferences) => {
    state.settings = { ...state.settings, ...preferences };
    return resolve({ ...state.settings });
  },
  setLanguage: (_language: string) => resolve(undefined),
  loopbackTest: () =>
    new Promise<LoopbackView>((done) =>
      setTimeout(
        () => done({ medianMs: 88, delaysMs: [86, 87, 88, 88, 88, 89, 89, 90, 91], clicks: 9, devices: USB }),
        1200,
      ),
    ),
  calibrateGate: () =>
    new Promise<CalibrationView>((done) =>
      setTimeout(() => done({ noiseDbfs: -67, peakDbfs: -61, gateDbfs: -58, tooLoud: false }), 1200),
    ),
  // Out of a call, someone testing their voice: words, then pauses.
  inputLevel: () => {
    if (state.call) return resolve(view(state.call).inputDbfs);
    return resolve(-62 + wave(Date.now() / 1000, 1.1, 0.5) * 34);
  },
  launchAtLogin: () => resolve(state.launchAtLogin),
  setLaunchAtLogin: (enabled: boolean) => {
    state.launchAtLogin = enabled;
    return resolve(enabled);
  },
  // The simulated rooms keep their people: nothing to push.
  onSpaces: (_f: (spaces: SpaceView[]) => void): Promise<UnlistenFn> => resolve(() => {}),
  onPresence: (f: (notice: Notice) => void): Promise<UnlistenFn> => {
    state.listeners.add(f);
    return resolve(() => {
      state.listeners.delete(f);
    });
  },
};
