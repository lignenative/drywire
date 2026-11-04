// A call that exists only in this browser tab: the interface's commands
// answered without Tauri, the Rust core or a server (ADR 0037). It lets
// the design be checked and iterated on in any browser (`make ui`).
//
// Nothing here is measured. The numbers are made up to look like a real
// call, so they must never be shown as Drywire's latency anywhere else.
import type { UnlistenFn } from "@tauri-apps/api/event";
import type {
  Api,
  CalibrationView,
  CallView,
  DeviceView,
  LoopbackView,
  Notice,
  Participant,
  Preferences,
  Settings,
  SpaceView,
} from "./api";

const INSTANCE = "voice.example.org";

/** The same spaces as the design canvas. */
const SPACES: SpaceView[] = [
  {
    instance: INSTANCE,
    id: "team",
    name: "Team",
    role: "member",
    rooms: [
      { id: "general", name: "general" },
      { id: "scrims", name: "scrims" },
      { id: "afk", name: "afk" },
    ],
    error: null,
  },
  {
    instance: "drywire.example.net",
    id: "friday",
    name: "Friday night",
    role: "member",
    rooms: [{ id: "general", name: "general" }],
    error: null,
  },
  {
    instance: "old.example.com",
    id: "crew",
    name: "Old crew",
    role: "member",
    rooms: [{ id: "general", name: "general" }],
    error: null,
  },
];

/** Who sits in each room before you join, by space and room. */
const OCCUPANTS: Record<string, string[]> = {
  "team/general": ["sam", "kim", "lee"],
  "team/afk": ["ana"],
  "crew/general": ["jo", "max"],
};

const DEVICES: DeviceView[] = [
  { direction: "input", name: "Headset microphone (USB)", mode: "WASAPI exclusive", bufferMs: 10, latencyMs: 12, bluetooth: false },
  { direction: "output", name: "Headset (USB)", mode: "WASAPI exclusive", bufferMs: 10, latencyMs: 10, bluetooth: false },
];

/** Estimates per person, as the real app would show them. */
const ESTIMATES: Record<string, number> = { sam: 47, kim: 61, lee: 52, ana: 58, jo: 55, max: 64 };

interface Call {
  space: SpaceView;
  roomId: string;
  since: number;
  muted: boolean;
}

const state = {
  settings: {
    gateOpenDbfs: null,
    volumes: {},
    muteShortcut: "Ctrl+Shift+M",
    soundVolume: 0.5,
    overlay: true,
    overlayCorner: "top-left",
  } as Settings,
  call: null as Call | null,
  launchAtLogin: false,
  listeners: new Set<(notice: Notice) => void>(),
};

/** The default threshold until the person chooses one (ADR 0034). */
const DEFAULT_GATE = -58;

/** A slow, uneven wave between 0 and 1, so voices come and go. */
function wave(t: number, speed: number, phase: number): number {
  return (Math.sin(t * speed + phase) * 0.6 + Math.sin(t * speed * 3.7 + phase * 2) * 0.4 + 1) / 2;
}

function participants(call: Call, t: number, inputDbfs: number, gate: number): Participant[] {
  const key = `${call.space.id}/${call.roomId}`;
  const others = OCCUPANTS[key] ?? [];
  const me: Participant = person("alex", 0, true, !call.muted && inputDbfs > gate);
  return [
    me,
    ...others.map((name, i) => person(name, i + 1, false, i < 2 && wave(t, 0.6 + i * 0.25, i * 2) > 0.55)),
  ];
}

function person(name: string, session: number, me: boolean, speaking: boolean): Participant {
  const volume = state.settings.volumes[keyOf(name)] ?? 1;
  return {
    session,
    name,
    publicKey: keyOf(name),
    me,
    speaking,
    volume,
    bufferMs: me ? null : 20,
    lossPct: me ? null : 0,
    estimateMs: me ? null : (ESTIMATES[name] ?? 55),
    counters: { received: 12480, played: 12452, concealed: 18, late: 6, dropped: 4, depth: 2 },
  };
}

/** A stand-in public key per name, stable from one run to the next. */
function keyOf(name: string): string {
  return [...name].map((c) => c.charCodeAt(0).toString(16)).join("").padEnd(64, "0").slice(0, 64);
}

function view(call: Call): CallView {
  const t = (Date.now() - call.since) / 1000;
  const gate = state.settings.gateOpenDbfs ?? DEFAULT_GATE;
  const inputDbfs = -62 + wave(t, 1.1, 0.5) * 34;
  return {
    instance: call.space.instance,
    spaceId: call.space.id,
    spaceName: call.space.name,
    roomId: call.roomId,
    roomName: call.space.rooms.find((r) => r.id === call.roomId)?.name ?? call.roomId,
    ended: null,
    reconnecting: false,
    bound: t > 0.6,
    rttMs: t > 0.6 ? 14 : null,
    minRttMs: t > 0.6 ? 11 : null,
    deviceMs: 88,
    deviceMeasured: true,
    muted: call.muted,
    inputDbfs,
    gateDbfs: gate,
    gateChosen: state.settings.gateOpenDbfs !== null,
    uplinkLossPct: 0,
    participants: participants(call, t, inputDbfs, gate),
    devices: DEVICES,
  };
}

function emit(notice: Notice) {
  for (const f of state.listeners) f(notice);
}

const resolve = <T>(value: T) => Promise.resolve(value);

export const simulated: Api = {
  publicKey: () => resolve("07e5c3a18f6d4b2907e5c3a18f6d4b2907e5c3a18f6d4b2907e5c3a18f6d4b29"),
  acceptInvitation: (_link: string, _displayName: string) => resolve(SPACES[0]),
  listSpaces: () => resolve(SPACES),
  joinRoom: (instance: string, spaceId: string, roomId: string) => {
    const space = SPACES.find((s) => s.instance === instance && s.id === spaceId);
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
        () => done({ medianMs: 88, delaysMs: [86, 87, 88, 88, 88, 89, 89, 90, 91], clicks: 9, devices: DEVICES }),
        1200,
      ),
    ),
  calibrateGate: () =>
    new Promise<CalibrationView>((done) =>
      setTimeout(() => done({ noiseDbfs: -67, peakDbfs: -61, gateDbfs: -58, tooLoud: false }), 1200),
    ),
  launchAtLogin: () => resolve(state.launchAtLogin),
  setLaunchAtLogin: (enabled: boolean) => {
    state.launchAtLogin = enabled;
    return resolve(enabled);
  },
  onPresence: (f: (notice: Notice) => void): Promise<UnlistenFn> => {
    state.listeners.add(f);
    return resolve(() => {
      state.listeners.delete(f);
    });
  },
};
