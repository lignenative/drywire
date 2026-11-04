// A call in the browser: what the desktop app's session thread does
// (desktop/src-tauri/src/session.rs), with WebTransport for the datagrams,
// the protocol in WebAssembly, WebCodecs for Opus and two AudioWorklets
// for the sound (ADR 0038 to 0040, 0044).
//
// - The microphone's frames that the gate lets through are encoded, sealed
//   and sent at once; the sender's announcement goes before every talk
//   spurt and once a second during one (as sender.rs does).
// - What the relay sends is opened by the protocol, decoded by one Opus
//   decoder per person, and handed to the playback worklet's jitter
//   buffers.
// - A ping every second measures the round trip and keeps the session
//   alive (protocol section 6.4); without a pong for five seconds the
//   session says hello again.
// - The space's presence stream adds and removes people, plays the
//   arrival and departure sounds, and tells a lost session (a disconnect
//   for this device, or a fresh snapshot without it) from an ended one (a
//   leave): a lost session joins again, after 1, 2, 4, 8 then every 15
//   seconds (ADR 0035), or at once when the person asks (ADR 0050).
//
// The interface polls `view()` ten times a second; nothing here waits on
// it.

import type { CallView, EncryptionView, Notice, Participant } from "@/api";
import { AudioEngine, MicrophoneDenied } from "./audio/engine";
import { DEFAULT_GATE } from "./audio/gate";
import type { SenderStats } from "./audio/messages";
import { type EncoderSettings, OpusDecoder, OpusEncoder, SAMPLE_RATE } from "./codec";
import { ControlError, type ControlPlane, type JoinAnswer, type PresenceEvent } from "./control";
import { type Identity, sign } from "./identity";
import { type Bytes, HELLO_ACK, hex, PONG, type Protocol, sessionOf, unhex } from "./protocol";
import { RelayLink } from "./relay";
import { Retries } from "./retry";
import type { StoredSpace, WebSettings } from "./store";

const PING_MS = 1000;
/** Without a pong for this long, say hello again (the core's link does). */
const REBIND_MS = 5000;
const HELLO_TRIES = 10;
const HELLO_WAIT_MS = 300;
/** A rejected voice keeps the lock open this long (ADR 0041). */
const REJECTED_MS = 10_000;
/** Losses are counted over this many seconds. */
const LOSS_WINDOW_S = 5;

export interface CallSetup {
  protocol: Protocol;
  identity: Identity;
  control: ControlPlane;
  space: StoredSpace;
  roomId: string;
  roomName: string;
  codec: EncoderSettings;
  settings: () => WebSettings;
  onNotice: (notice: Notice) => void;
  /** For a space with an MLS group (ADR 0052): syncs it and gives the
      room's current key. A space key is used otherwise. */
  roomKey?: () => Promise<Uint8Array>;
}

interface Person {
  session: number;
  name: string;
  publicKey: string;
  /** Their signed announcement was verified with the key the server gave. */
  verified: boolean;
  /** Audio came from them, decrypted or not. */
  heard: boolean;
  rejectedAt: number;
  /** The last timestamp handed to the decoder, unwrapped. */
  lastTs: number | null;
  /** Frames that came after a newer one: too late for the decoder. */
  late: number;
  decoder: OpusDecoder<{ ts: number; end: boolean }> | null;
  stats: SenderStats | null;
  /** Counters a few seconds ago, for the loss rate. */
  past: { played: number; concealed: number }[];
}

/** One connection: a session on the relay, from one join. */
interface Link {
  answer: JoinAnswer;
  relay: RelayLink;
  announce: Bytes;
  helloCounter: bigint;
  bound: boolean;
  lastPong: number;
}

export class WebCall {
  private link: Link | null = null;
  private readonly people = new Map<number, Person>();
  /** Names by session, from the presence stream, for departures. */
  private readonly names = new Map<number, string>();
  private engine: AudioEngine;
  private encoder: OpusEncoder<{ ts: number; start: boolean; end: boolean }>;
  private readonly events = new AbortController();
  private timers: ReturnType<typeof setInterval>[] = [];
  private pingNumber = 0;
  private rtts: number[] = [];
  private pongs: { at: number; received: number; lost: number }[] = [];
  private announcedAt = -Infinity;
  private inputDbfs = -90;
  private gateOpen = false;
  private muted = false;
  private ended: string | null = null;
  private reconnecting = false;
  /** The room key the call runs with, when the space has an MLS group. */
  private key: Uint8Array | null = null;
  /** When to join again, while reconnecting. */
  private retries: Retries | null = null;
  private connecting = false;
  private left = false;
  /** This device's session, from the last join's answer. */
  private session: number | null = null;
  /** The server holds a session for this call: leaving must end it. */
  private joined = false;
  /** Presence events that came while joining, played once the new
      session's people are known: a join answered before `start` would
      otherwise go to the protocol client being replaced. */
  private queued: PresenceEvent[] = [];
  private snapshots = 0;
  private sent = 0;

  private constructor(
    private readonly setup: CallSetup,
    engine: AudioEngine,
  ) {
    this.engine = engine;
    this.encoder = new OpusEncoder(setup.codec, (packet, frame) => this.sendFrame(packet, frame));
    engine.onCapture((m) => {
      if (m.type === "frame") {
        if (this.link?.bound) this.encoder.encode(m.pcm as Float32Array<ArrayBuffer>, m.ts, m);
      } else if (m.type === "level") {
        this.inputDbfs = m.dbfs;
        this.gateOpen = m.open;
      }
    });
    engine.onStats((m) => {
      for (const s of m.senders) {
        const p = this.people.get(s.session);
        if (p) p.stats = s;
      }
    });
    engine.toCapture({ type: "gate", openDbfs: setup.settings().gateOpenDbfs ?? DEFAULT_GATE.openDbfs });
  }

  /** Opens the microphone, joins the room and binds to the relay. */
  static async start(setup: CallSetup): Promise<WebCall> {
    let engine: AudioEngine;
    try {
      engine = await AudioEngine.start((setup.codec.frameMs * SAMPLE_RATE) / 1000);
    } catch (e) {
      if (e instanceof MicrophoneDenied) throw new ControlError("microphone", e.message);
      throw new ControlError("audio", (e as Error).message);
    }
    const call = new WebCall(setup, engine);
    try {
      void call.follow();
      await call.connect();
    } catch (e) {
      await call.leave();
      throw e;
    }
    call.timers.push(setInterval(() => call.tick(), PING_MS));
    call.timers.push(setInterval(() => call.sampleLoss(), 1000));
    return call;
  }

  // --- Connecting ----------------------------------------------------------

  /** Joins the room and binds a new session to the relay. */
  private async connect(): Promise<void> {
    const { protocol, control, identity, space, roomId } = this.setup;
    this.connecting = true;
    let connected = false;
    try {
      const answer = await control.join(roomId);
      this.session = answer.session;
      this.joined = true;
      // Left while joining: end the session the server just made.
      if (this.left) return void (await control.leave().catch(() => {}));
      if (!answer.web_relay) throw new ControlError("refused", "this instance serves no browser");
      this.key = this.setup.roomKey ? await this.setup.roomKey() : null;
      if (this.left) return void (await control.leave().catch(() => {}));
      protocol.start({
        session: answer.session,
        secret: unhex(answer.session_secret),
        key: this.key ? { roomKey: this.key } : { spaceKey: unhex(space.spaceKey) },
        roomId: unhex(answer.room.id),
        publicKey: identity.publicKey,
        salt: crypto.getRandomValues(new Uint8Array(32)),
      });
      for (const s of answer.sessions) {
        this.names.set(s.session, s.name);
        if (s.session !== answer.session) this.addPerson(s.session, s.name, s.public_key);
      }
      // The announcement is signed once per key epoch and sent unchanged.
      const signature = await sign(identity, protocol.announcementMessage());
      // The protocol module holds one client: a call left meanwhile must
      // not touch it, another call may own it now.
      if (this.left) return;
      const announce = protocol.announce(signature);
      let relay: RelayLink;
      try {
        relay = await RelayLink.open(answer.web_relay.url, answer.web_relay.certificate_hashes);
      } catch (e) {
        throw new ControlError("relay", (e as Error).message);
      }
      if (this.left) return relay.close();
      const link: Link = { answer, relay, announce, helloCounter: 0n, bound: false, lastPong: performance.now() };
      this.link = link;
      void this.receive(link);
      await this.bind(link);
      this.announcedAt = -Infinity;
      connected = true;
    } finally {
      this.connecting = false;
      const queued = this.queued;
      this.queued = [];
      if (connected && this.link) for (const e of queued) this.presence(e);
    }
  }

  /** Says hello until the relay answers (protocol section 6.1). */
  private async bind(link: Link): Promise<void> {
    // The relay's counters start again with every accepted hello (6.5).
    this.pongs = [];
    for (let attempt = 0; attempt < HELLO_TRIES; attempt++) {
      // A new counter for every attempt.
      link.helloCounter = BigInt(Date.now()) * 16n + BigInt(attempt);
      link.relay.send(this.setup.protocol.hello(link.helloCounter));
      await sleep(HELLO_WAIT_MS);
      if (link.bound || this.link !== link) return;
    }
    throw new ControlError("relay", "the relay never answered the hello");
  }

  /** Everything the relay sends on this link, until it closes. */
  private async receive(link: Link): Promise<void> {
    const { protocol } = this.setup;
    for (;;) {
      let packet: Uint8Array;
      try {
        const { value, done } = await link.relay.reader.read();
        if (done) break;
        packet = value;
      } catch {
        break;
      }
      // The module holds one client: a packet from an older link is not
      // for it.
      if (this.link !== link) return;
      let r: ReturnType<Protocol["receive"]>;
      try {
        r = protocol.receive(packet);
      } catch {
        // Not a packet the module can read: drop it, as the relay would.
        continue;
      }
      if (r.kind === "audio") {
        this.heard(r.session, r.timestamp, r.end, r.frame);
      } else if (r.kind === "announced") {
        const p = this.people.get(sessionOf(packet) ?? 0);
        if (p) p.verified = true;
      } else if (r.kind === "control" && r.type === HELLO_ACK) {
        if (protocol.isHelloAck(packet, link.helloCounter)) {
          link.bound = true;
          link.lastPong = performance.now();
        }
      } else if (r.kind === "control" && r.type === PONG) {
        this.pong(link, packet);
      } else if (r.kind === "dropped") {
        const p = this.people.get(sessionOf(packet) ?? 0);
        if (!p) continue;
        if (r.reason === "bad_signature" || r.reason === "bad_tag") {
          p.rejectedAt = performance.now();
          p.verified = false;
        } else if (r.reason === "no_key") {
          // Their audio came before their announcement.
          p.heard = true;
        }
      }
    }
    if (this.link === link) this.lose("the relay closed the connection");
  }

  // --- Sending ---------------------------------------------------------------

  private sendFrame(packet: Uint8Array, frame: { ts: number; start: boolean; end: boolean }): void {
    const link = this.link;
    // While muted, the worklet sends only silence, to end the spurt.
    if (!link?.bound) return;
    // Before every spurt, then once a second (sender.rs).
    if (frame.start || frame.ts - this.announcedAt >= SAMPLE_RATE) {
      link.relay.send(link.announce);
      this.announcedAt = frame.ts;
    }
    link.relay.send(this.setup.protocol.seal(packet, frame.ts, frame.end));
    this.sent++;
  }

  private tick(): void {
    const link = this.link;
    if (!link || this.reconnecting) return;
    link.relay.send(this.setup.protocol.ping(this.pingNumber++, BigInt(Math.round(performance.now() * 1000))));
    if (link.bound && performance.now() - link.lastPong > REBIND_MS) {
      link.bound = false;
      this.bind(link).catch(() => this.lose("the relay stopped answering"));
    }
  }

  private pong(link: Link, packet: Uint8Array): void {
    const v = new DataView(packet.buffer, packet.byteOffset, packet.byteLength);
    const now = performance.now();
    link.lastPong = now;
    this.rtts.push(now - Number(v.getBigUint64(16)) / 1000);
    if (this.rtts.length > 60) this.rtts.shift();
    this.pongs.push({ at: now, received: v.getUint32(24), lost: v.getUint32(28) });
    while (this.pongs.length > 0 && now - this.pongs[0].at > LOSS_WINDOW_S * 1000) this.pongs.shift();
  }

  // --- Receiving -------------------------------------------------------------

  private heard(session: number, timestamp: number, end: boolean, frame: Bytes): void {
    const p = this.people.get(session);
    if (!p) return;
    p.heard = true;
    // Timestamps wrap at 2³²; the jitter buffer wants them on a line.
    const ts = p.lastTs === null ? timestamp : p.lastTs + ((timestamp - p.lastTs) | 0);
    if (p.lastTs !== null && ts <= p.lastTs) {
      // Older than a frame already decoded: Opus decodes in order.
      p.late++;
      return;
    }
    p.lastTs = ts;
    p.decoder ??= new OpusDecoder((pcm, f) => {
      if (this.people.get(session) === p) {
        this.engine.toPlayback({ type: "frame", session, ts: f.ts, end: f.end, pcm });
      }
    });
    p.decoder.decode(frame, ts, { ts, end });
  }

  private addPerson(session: number, name: string, publicKey: string): void {
    if (this.people.has(session)) return;
    try {
      this.setup.protocol.addPeer(session, unhex(publicKey));
    } catch {
      return;
    }
    this.people.set(session, {
      session,
      name,
      publicKey,
      verified: false,
      heard: false,
      rejectedAt: -Infinity,
      lastTs: null,
      late: 0,
      decoder: null,
      stats: null,
      past: [],
    });
    const gain = this.setup.settings().volumes[publicKey] ?? 1;
    this.engine.toPlayback({ type: "volume", session, gain });
  }

  private removePerson(session: number): void {
    const p = this.people.get(session);
    if (!p) return;
    this.people.delete(session);
    this.setup.protocol.removePeer(session);
    p.decoder?.close();
    this.engine.toPlayback({ type: "forget", session });
  }

  // --- Presence --------------------------------------------------------------

  /** Follows the space's presence for as long as the call lasts. */
  private async follow(): Promise<void> {
    const { control, space } = this.setup;
    let wait = 0;
    while (!this.left) {
      if (wait > 0) await sleep(wait * 1000);
      try {
        await control.events(space.id, (e) => this.presence(e), this.events.signal);
        wait = 1;
      } catch {
        if (this.left) return;
        wait = Math.min(Math.max(wait * 2, 1), 15);
      }
    }
  }

  private presence(e: PresenceEvent): void {
    if (this.connecting && e.type !== "snapshot") {
      this.queued.push(e);
      return;
    }
    const mine = this.session ?? undefined;
    const room = this.setup.roomId;
    if (e.type === "snapshot") {
      this.snapshots++;
      for (const s of e.sessions) this.names.set(s.session, s.name);
      // Only a stream that came back can say the session was lost: the
      // first snapshot may predate the join.
      if (this.snapshots > 1 && mine !== undefined && !this.connecting) {
        if (!e.sessions.some((s) => s.session === mine)) return this.lose("the server forgot this session");
        const here = new Set(e.sessions.filter((s) => s.room_id === room).map((s) => s.session));
        for (const s of e.sessions) {
          if (s.room_id === room && s.session !== mine) this.addPerson(s.session, s.name, s.public_key);
        }
        for (const session of [...this.people.keys()]) if (!here.has(session)) this.removePerson(session);
      }
      return;
    }
    if (e.type === "join") {
      this.names.set(e.session, e.name);
      if (e.room_id !== room || e.session === mine) return;
      this.addPerson(e.session, e.name, e.public_key);
      if (!this.reconnecting) this.setup.onNotice({ kind: "join", name: e.name });
      return;
    }
    if (e.type === "mls") {
      void this.rekey();
      return;
    }
    // A leave or a disconnect.
    if (e.session === mine) {
      if (this.connecting || this.reconnecting) return;
      if (e.type === "disconnect") return this.lose("the relay stopped hearing this device");
      // Joined from elsewhere, or removed: joining again would fight it.
      this.end("the server ended this session: joined elsewhere, or removed");
      return;
    }
    if (e.room_id !== room || !this.people.has(e.session)) return;
    const name = this.people.get(e.session)?.name ?? this.names.get(e.session) ?? "";
    this.removePerson(e.session);
    this.setup.onNotice({ kind: e.type, name });
  }

  /** The space's group moved: with a new room key, the call joins again
      under it. The browser's module holds one key per call, so it does
      not switch keys in flight as the apps do; the others keep the old
      key a while, and the gap is one join. */
  private async rekey(): Promise<void> {
    if (!this.setup.roomKey || this.reconnecting || this.connecting || this.left || this.ended) return;
    try {
      const key = await this.setup.roomKey();
      const same = this.key !== null && key.length === this.key.length && key.every((b, i) => b === this.key?.[i]);
      if (!same) this.lose("the space's group moved to a new room key");
    } catch {
      // The next event, or the next join, tries again.
    }
  }

  // --- Losing and ending -----------------------------------------------------

  /** The session is gone: join again until it works (ADR 0035). */
  private lose(_why: string): void {
    if (this.left || this.ended || this.reconnecting) return;
    this.reconnecting = true;
    this.dropLink();
    void this.rejoin();
  }

  private async rejoin(): Promise<void> {
    const retries = new Retries();
    this.retries = retries;
    while (!this.left && !this.ended) {
      await retries.wait();
      if (this.left || this.ended) return;
      try {
        await this.connect();
        this.reconnecting = false;
        this.retries = null;
        return;
      } catch (e) {
        this.dropLink();
        if (!(e instanceof ControlError && e.temporary)) {
          this.end((e as Error).message);
          return;
        }
      }
    }
  }

  /** While waiting for the next attempt, starts it now (ADR 0050). */
  retryNow(): void {
    this.retries?.now();
  }

  private end(reason: string): void {
    this.ended = reason;
    this.reconnecting = false;
    this.retries = null;
    this.dropLink();
  }

  /** Closes the relay link and forgets everyone on it. */
  private dropLink(): void {
    this.link?.relay.close();
    this.link = null;
    this.pongs = [];
    this.rtts = [];
    for (const session of [...this.people.keys()]) this.removePerson(session);
  }

  async leave(): Promise<void> {
    if (this.left) return;
    this.left = true;
    // A wait for the next attempt ends now: the loop sees the call left.
    this.retries?.now();
    this.events.abort();
    this.timers.forEach(clearInterval);
    this.dropLink();
    this.encoder.close();
    this.engine.close();
    if (this.joined && !this.ended) await this.setup.control.leave().catch(() => {});
  }

  // --- Controls --------------------------------------------------------------

  setMuted(muted: boolean): void {
    this.muted = muted;
    this.engine.toCapture({ type: "muted", muted });
  }

  setGate(openDbfs: number): void {
    this.engine.toCapture({ type: "gate", openDbfs });
  }

  setVolume(publicKey: string, gain: number): void {
    for (const p of this.people.values()) {
      if (p.publicKey === publicKey) this.engine.toPlayback({ type: "volume", session: p.session, gain });
    }
  }

  /** The microphone's level, from -90 dBFS up. */
  inputLevel(): number {
    return this.inputDbfs;
  }

  /** The output's glitches and buffers, for measuring (README). */
  playbackStats() {
    return this.engine.playbackStats();
  }

  /** The microphone's frame levels for `seconds`, for the calibration. */
  calibrate(seconds: number): Promise<number[]> {
    return recordLevels(this.engine, seconds);
  }

  // --- What the interface shows ----------------------------------------------

  private sampleLoss(): void {
    for (const p of this.people.values()) {
      if (!p.stats) continue;
      p.past.push({ played: p.stats.played, concealed: p.stats.concealed });
      if (p.past.length > LOSS_WINDOW_S) p.past.shift();
    }
  }

  view(): CallView {
    const { space, roomId, roomName, settings, identity, codec } = this.setup;
    const link = this.link;
    const bound = !!link?.bound && !this.reconnecting && !this.ended;
    const rttMs = this.rtts.length > 0 ? median(this.rtts.slice(-5)) : null;
    const deviceMs = this.engine.deviceMs();
    const gate = settings().gateOpenDbfs;
    const me: Participant = {
      session: link?.answer.session ?? 0,
      name: this.names.get(link?.answer.session ?? -1) ?? "",
      publicKey: hex(identity.publicKey),
      me: true,
      speaking: bound && this.gateOpen && !this.muted,
      volume: 1,
      bufferMs: null,
      lossPct: null,
      estimateMs: null,
      counters: { received: 0, played: this.sent, concealed: 0, late: 0, dropped: 0, depth: 0 },
    };
    const others = [...this.people.values()].map((p): Participant => {
      const s = p.stats;
      const rejected = performance.now() - p.rejectedAt < REJECTED_MS;
      const first = p.past[0];
      const played = s && first ? s.played - first.played : 0;
      const concealed = s && first ? s.concealed - first.concealed : 0;
      return {
        session: p.session,
        name: p.name,
        publicKey: p.publicKey,
        me: false,
        speaking: !!s?.active && !rejected,
        volume: settings().volumes[p.publicKey] ?? 1,
        bufferMs: s ? s.targetMs : null,
        lossPct: played + concealed > 0 ? (concealed * 100) / (played + concealed) : s ? 0 : null,
        // Their half of the path is taken to be ours (desktop latency.rs).
        estimateMs: s && rttMs !== null ? deviceMs + codec.frameMs + rttMs + s.targetMs : null,
        counters: {
          received: s?.received ?? 0,
          played: s?.played ?? 0,
          concealed: s?.concealed ?? 0,
          late: (s?.late ?? 0) + p.late,
          dropped: s?.dropped ?? 0,
          depth: s?.depth ?? 0,
        },
      };
    });
    const first = this.pongs[0];
    const last = this.pongs[this.pongs.length - 1];
    const received = first && last ? last.received - first.received : 0;
    const lost = first && last ? Math.max(0, last.lost - first.lost) : 0;
    return {
      instance: space.instance,
      spaceId: space.id,
      spaceName: space.name,
      roomId,
      roomName,
      ended: this.ended,
      reconnecting: this.reconnecting,
      retryInS: this.retries?.secondsLeft() ?? null,
      bound,
      rttMs: bound ? rttMs : null,
      minRttMs: bound && this.rtts.length > 0 ? Math.min(...this.rtts) : null,
      deviceMs,
      deviceMeasured: false,
      muted: this.muted,
      inputDbfs: this.inputDbfs,
      gateDbfs: gate ?? DEFAULT_GATE.openDbfs,
      gateChosen: gate !== null,
      uplinkLossPct: received + lost > 0 ? (lost * 100) / (received + lost) : 0,
      participants: [me, ...others],
      devices: this.engine.devices(),
      encryption: this.encryption(bound),
    };
  }

  /** The lock (ADR 0041): verified once everyone heard is verified. */
  private encryption(bound: boolean): EncryptionView {
    const now = performance.now();
    const people = [...this.people.values()].map((p) => ({
      publicKey: p.publicKey,
      verified: p.verified,
      rejected: now - p.rejectedAt < REJECTED_MS,
    }));
    const heard = [...this.people.values()].filter((p) => p.heard);
    const state = people.some((p) => p.rejected)
      ? "failed"
      : bound && heard.length > 0 && heard.every((p) => p.verified)
        ? "verified"
        : "pending";
    return { state, people };
  }
}

/** Records the microphone's frame levels for `seconds`; fails if the
    microphone closes first (the call was left). */
export function recordLevels(engine: AudioEngine, seconds: number): Promise<number[]> {
  return new Promise((resolve, reject) => {
    const stop = () => {
      stopLevels();
      stopClose();
    };
    const stopLevels = engine.onCapture((m) => {
      if (m.type !== "levels") return;
      stop();
      resolve(m.levels);
    });
    const stopClose = engine.onClose(() => {
      stop();
      reject(new ControlError("audio", "the microphone closed during the calibration"));
    });
    engine.toCapture({ type: "calibrate", seconds });
  });
}

function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.floor(sorted.length / 2)];
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
