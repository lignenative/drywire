// The control plane, as the web client needs it (protocol section 13):
// log in with the identity, redeem an invitation, read a space, join a
// room, follow the space's presence.
//
// Requests go to `base`. In development that is the page's own origin,
// which Vite forwards to the server: the control plane does not answer
// cross-origin requests (yet), and the login is still signed for the
// instance's own origin, the audience.

import type { AppError } from "@/api";
import { type Bytes, hex, type Protocol, unhex } from "./protocol";
import { type Identity, sign } from "./identity";

export type Session = { session: number; room_id: string; public_key: string; name: string };

export type JoinAnswer = {
  session: number;
  session_secret: string;
  relay: string;
  web_relay?: { url: string; certificate_hashes: string[] };
  room: { id: string; space_id: string; name: string };
  sessions: Session[];
};

export type SpaceAnswer = {
  space: { id: string; name: string };
  role: string;
  rooms: { id: string; name: string }[];
};

/** A presence event (protocol section 13.6). */
export type PresenceEvent =
  | { type: "snapshot"; sessions: Session[] }
  | { type: "join"; room_id: string; session: number; public_key: string; name: string }
  | { type: "leave" | "disconnect"; room_id: string; session: number; public_key: string };

/** A failure the interface can word: the codes of the desktop app's
    errors (desktop/src/i18n.ts, "error.…"). */
export class ControlError extends Error implements AppError {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }

  /** Worth trying again later, as ADR 0035 says: the network, or a
      server busy for a moment. */
  get temporary(): boolean {
    return this.code === "network" || this.code === "unavailable" || this.code === "relay";
  }
}

function codeOf(status: number): string {
  if (status === 404) return "not_found";
  if (status === 429 || status >= 500) return "unavailable";
  return "refused";
}

export class ControlPlane {
  private token = "";
  private loggingIn: Promise<void> | null = null;

  constructor(
    private readonly base: string,
    private readonly audience: string,
    private readonly identity: Identity,
    private readonly protocol: Protocol,
  ) {}

  private async fetch(method: string, path: string, body?: unknown, signal?: AbortSignal): Promise<Response> {
    const headers: Record<string, string> = { "Content-Type": "application/json" };
    if (this.token) headers.Authorization = `Bearer ${this.token}`;
    try {
      return await fetch(this.base + path, {
        method,
        headers,
        body: body === undefined ? undefined : JSON.stringify(body),
        signal,
      });
    } catch (e) {
      if (signal?.aborted) throw e;
      throw new ControlError("network", `${method} ${path}: ${(e as Error).message}`);
    }
  }

  private async call<T>(method: string, path: string, body?: unknown): Promise<T> {
    if (!this.token) await this.login();
    let r = await this.fetch(method, path, body);
    // Tokens last an hour and die with a server restart: log in again once.
    if (r.status === 401) {
      await this.login();
      r = await this.fetch(method, path, body);
    }
    if (!r.ok) {
      const error = await r.json().catch(() => ({ error: r.statusText }));
      throw new ControlError(codeOf(r.status), `${method} ${path}: ${r.status} ${error.error}`);
    }
    return r.status === 204 ? (undefined as T) : r.json();
  }

  /** Logs in, once for every caller waiting on it. */
  login(): Promise<void> {
    this.loggingIn ??= this.signIn().finally(() => {
      this.loggingIn = null;
    });
    return this.loggingIn;
  }

  private async signIn(): Promise<void> {
    this.token = "";
    const answer = async <T>(r: Response): Promise<T> => {
      if (!r.ok) throw new ControlError(codeOf(r.status), `login: ${r.status}`);
      return r.json();
    };
    const { challenge } = await answer<{ challenge: string }>(await this.fetch("POST", "/v1/auth/challenge"));
    const message = this.protocol.loginMessage(unhex(challenge), this.audience);
    const signature = await sign(this.identity, message);
    const { token } = await answer<{ token: string }>(
      await this.fetch("POST", "/v1/auth/login", {
        public_key: hex(this.identity.publicKey),
        challenge,
        signature: hex(signature),
      }),
    );
    this.token = token;
  }

  /** The caller's identity here, or null if it has none yet. */
  async me(): Promise<{ name: string } | null> {
    try {
      return await this.call("GET", "/v1/me");
    } catch (e) {
      if (e instanceof ControlError && e.code === "not_found") return null;
      throw e;
    }
  }

  /** Keeps the account's recovery vault here (protocol section 15). */
  putRecoveryVault(id: Bytes, sealed: Bytes): Promise<void> {
    return this.call("PUT", "/v1/me/vault", { id: hex(id), sealed: hex(sealed) });
  }

  /** Leaves a vault here for one new device, for ten minutes. */
  putDeviceVault(id: Bytes, sealed: Bytes): Promise<{ expires_in: number }> {
    return this.call("POST", "/v1/me/device-vaults", { id: hex(id), sealed: hex(sealed) });
  }

  /** Opens a vault: no login, the device has no account yet. Every
      failure is a `not_found`. */
  async openVault(id: Bytes): Promise<{ kind: "recovery" | "device"; sealed: Bytes }> {
    const r = await this.fetch("POST", "/v1/vaults/open", { id: hex(id) });
    if (!r.ok) throw new ControlError(codeOf(r.status), `open vault: ${r.status}`);
    const answer: { kind: "recovery" | "device"; sealed: string } = await r.json();
    return { kind: answer.kind, sealed: unhex(answer.sealed) };
  }

  redeem(code: Bytes, displayName: string): Promise<SpaceAnswer> {
    return this.call("POST", "/v1/invitations/redeem", { code: hex(code), display_name: displayName });
  }

  space(id: string): Promise<SpaceAnswer> {
    return this.call("GET", `/v1/spaces/${id}`);
  }

  join(room: string): Promise<JoinAnswer> {
    return this.call("POST", `/v1/rooms/${room}/join`);
  }

  leave(): Promise<void> {
    return this.call("POST", "/v1/leave");
  }

  /** Follows a space's presence until `signal` aborts: a snapshot, then
      every change. The stream is a fetch, not an EventSource, which cannot
      send the token in a header. It ends when the server ends it; the
      caller starts it again and gets a fresh snapshot. */
  async events(space: string, onEvent: (e: PresenceEvent) => void, signal: AbortSignal): Promise<void> {
    if (!this.token) await this.login();
    let r = await this.fetch("GET", `/v1/spaces/${space}/events`, undefined, signal);
    if (r.status === 401) {
      await this.login();
      r = await this.fetch("GET", `/v1/spaces/${space}/events`, undefined, signal);
    }
    if (!r.ok || !r.body) throw new ControlError(codeOf(r.status), `events: ${r.status}`);
    const reader = r.body.pipeThrough(new TextDecoderStream()).getReader();
    let text = "";
    for (;;) {
      const { value, done } = await reader.read();
      if (done) return;
      text += value;
      let end: number;
      while ((end = text.indexOf("\n\n")) >= 0) {
        const block = text.slice(0, end);
        text = text.slice(end + 2);
        const event = parseEvent(block);
        if (event) onEvent(event);
      }
    }
  }
}

/** One Server-Sent Events block: `event:` and `data:` lines; a comment
    (the keepalive) has neither. */
export function parseEvent(block: string): PresenceEvent | null {
  let type = "";
  let data = "";
  for (const line of block.split("\n")) {
    if (line.startsWith("event:")) type = line.slice(6).trim();
    else if (line.startsWith("data:")) data += line.slice(5).trim();
  }
  if (!type || !data) return null;
  try {
    return { ...JSON.parse(data), type } as PresenceEvent;
  } catch {
    return null;
  }
}
