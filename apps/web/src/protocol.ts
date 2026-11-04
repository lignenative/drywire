// The Drywire protocol in the browser: drywire-wasm (ADR 0040), the same
// Rust code as the native clients, behind a small typed wrapper.
//
// The module exports plain functions and one shared buffer: each call
// copies its input into the buffer, calls, and copies the output out.
// A negative result is an error, named by dw_reason.

type Exports = {
  memory: WebAssembly.Memory;
  dw_buffer(): number;
  dw_invitation(length: number): number;
  dw_login_message(audienceLength: number): number;
  dw_start(session: number): number;
  dw_hello(counter: bigint): number;
  dw_is_hello_ack(length: number, counter: bigint): number;
  dw_ping(number: number, clientTime: bigint): number;
  dw_announcement_message(): number;
  dw_announce(): number;
  dw_seal(length: number, timestamp: number, endOfSpurt: number): number;
  dw_add_peer(session: number): number;
  dw_remove_peer(session: number): number;
  dw_receive(length: number): number;
  dw_reason(code: number): number;
};

/** Packet types the page reads itself (protocol section 4). */
export const HELLO_ACK = 0x03;
export const PONG = 0x06;

/** The audio flag of a talk spurt's last frame (protocol section 5). */
const END_OF_SPURT = 0x02;

/** The sender's session, from a packet's clear-text header (section 3). */
export function sessionOf(packet: Uint8Array): number | null {
  if (packet.length < 8) return null;
  return new DataView(packet.buffer, packet.byteOffset, packet.byteLength).getUint32(4);
}

export type Invitation = { code: Bytes; spaceKey: Uint8Array; instance: string };

/** What a packet from the relay turned out to be. */
export type Received =
  | { kind: "audio"; session: number; sequence: number; timestamp: number; end: boolean; frame: Bytes }
  | { kind: "announced" }
  | { kind: "control"; type: number }
  | { kind: "dropped"; reason: string };

export class ProtocolError extends Error {}

/** Bytes in an ordinary ArrayBuffer, which WebCrypto and WebTransport take. */
export type Bytes = Uint8Array<ArrayBuffer>;

export class Protocol {
  private readonly x: Exports;

  private constructor(instance: WebAssembly.Instance) {
    this.x = instance.exports as unknown as Exports;
  }

  /** Instantiates the module from its compiled form or its bytes. */
  static async load(module: WebAssembly.Module | BufferSource): Promise<Protocol> {
    const compiled = module instanceof WebAssembly.Module ? module : await WebAssembly.compile(module);
    return new Protocol(await WebAssembly.instantiate(compiled, {}));
  }

  // The buffer's view is taken anew on each call: memory can grow, which
  // detaches older views.
  private buffer(): Uint8Array {
    return new Uint8Array(this.x.memory.buffer, this.x.dw_buffer(), 2048);
  }

  private put(...parts: Uint8Array[]): number {
    const b = this.buffer();
    let at = 0;
    for (const p of parts) {
      b.set(p, at);
      at += p.length;
    }
    return at;
  }

  private out(result: number): Bytes {
    if (result < 0) throw new ProtocolError(this.reason(result));
    return copy(this.buffer().subarray(0, result));
  }

  reason(code: number): string {
    const n = this.x.dw_reason(code);
    return new TextDecoder().decode(this.buffer().slice(0, n));
  }

  /** Reads an invitation link (protocol section 14). */
  invitation(link: string): Invitation {
    const n = this.put(new TextEncoder().encode(link.trim()));
    const b = this.out(this.x.dw_invitation(n));
    return {
      code: b.slice(0, 16),
      spaceKey: b.slice(16, 48),
      instance: new TextDecoder().decode(b.slice(48)),
    };
  }

  /** The message an identity signs to log in to an instance (section 13.2). */
  loginMessage(challenge: Uint8Array, audience: string): Bytes {
    const a = new TextEncoder().encode(audience);
    this.put(challenge, a);
    return this.out(this.x.dw_login_message(a.length));
  }

  /** Starts the client of a session; salt is fresh randomness for epoch 0. */
  start(s: {
    session: number;
    secret: Uint8Array;
    spaceKey: Uint8Array;
    roomId: Uint8Array;
    publicKey: Uint8Array;
    salt: Uint8Array;
  }): void {
    this.put(s.secret, s.spaceKey, s.roomId, s.publicKey, s.salt);
    if (this.x.dw_start(s.session >>> 0) !== 0) throw new ProtocolError("invalid public key");
  }

  hello(counter: bigint): Bytes {
    return this.out(this.x.dw_hello(counter));
  }

  isHelloAck(packet: Uint8Array, counter: bigint): boolean {
    return this.x.dw_is_hello_ack(this.put(packet), counter) === 0;
  }

  ping(number: number, clientTime: bigint): Bytes {
    return this.out(this.x.dw_ping(number >>> 0, clientTime));
  }

  announcementMessage(): Bytes {
    return this.out(this.x.dw_announcement_message());
  }

  announce(signature: Uint8Array): Bytes {
    this.put(signature);
    return this.out(this.x.dw_announce());
  }

  seal(frame: Uint8Array, timestamp: number, endOfSpurt = false): Bytes {
    return this.out(this.x.dw_seal(this.put(frame), timestamp >>> 0, endOfSpurt ? 1 : 0));
  }

  addPeer(session: number, publicKey: Uint8Array): void {
    this.put(publicKey);
    if (this.x.dw_add_peer(session >>> 0) !== 0) throw new ProtocolError("invalid public key");
  }

  removePeer(session: number): void {
    this.x.dw_remove_peer(session >>> 0);
  }

  receive(packet: Uint8Array): Received {
    const r = this.x.dw_receive(this.put(packet));
    if (r === 0) return { kind: "announced" };
    if (r <= -100) return { kind: "dropped", reason: this.reason(r) };
    if (r <= -10) return { kind: "control", type: -10 - r };
    if (r < 0) throw new ProtocolError(this.reason(r));
    const b = copy(this.buffer().subarray(0, r));
    const h = new DataView(b.buffer);
    return {
      kind: "audio",
      session: h.getUint32(4),
      sequence: h.getUint32(8),
      timestamp: h.getUint32(12),
      end: (b[2] & END_OF_SPURT) !== 0,
      frame: b.slice(16),
    };
  }
}

/** A copy of bytes from the module's memory, in a buffer of their own. */
function copy(view: Uint8Array): Bytes {
  const b = new Uint8Array(view.length);
  b.set(view);
  return b;
}

export function hex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

export function unhex(text: string): Bytes {
  if (!/^([0-9a-f]{2})*$/.test(text)) throw new ProtocolError(`not lowercase hex: ${text}`);
  return Uint8Array.from(text.match(/../g) ?? [], (h) => parseInt(h, 16));
}
