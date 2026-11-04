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
  /** With each member's device keys (ADR 0055), when the space is read. */
  members?: { devices?: string[] }[];
};

/** A presence event (protocol section 13.6). */
export type PresenceEvent =
  | { type: "snapshot"; sessions: Session[] }
  | { type: "join"; room_id: string; session: number; public_key: string; name: string }
  | { type: "leave" | "disconnect"; room_id: string; session: number; public_key: string }
  | { type: "mls"; epoch: number };

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

/** A commit refused because the group moved on: catch up and retry. */
export class StaleEpoch extends Error {}

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
  async me(): Promise<{ name: string; username?: string; email?: string } | null> {
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

  /** Mails a sign-in code to the address (ADR 0055, protocol 13.8). */
  async emailCode(email: string, lang: string): Promise<void> {
    const r = await this.fetch("POST", "/v1/auth/email", { email, lang });
    if (!r.ok) {
      const error = await r.json().catch(() => ({ error: r.statusText }));
      throw new ControlError(codeOf(r.status), `POST /v1/auth/email: ${r.status} ${error.error}`);
    }
  }

  /** Signs this device in with the mailed code, alongside a signed
      challenge; the token is kept. A wrong code is `wrong_code`, with the
      tries left as the message. */
  async emailVerify(email: string, code: string): Promise<{ new_account: boolean; username: string }> {
    const ch = await this.fetch("POST", "/v1/auth/challenge");
    if (!ch.ok) throw new ControlError(codeOf(ch.status), `challenge: ${ch.status}`);
    const { challenge } = (await ch.json()) as { challenge: string };
    const signature = await sign(this.identity, this.protocol.loginMessage(unhex(challenge), this.audience));
    const r = await this.fetch("POST", "/v1/auth/email/verify", {
      email,
      code,
      public_key: hex(this.identity.publicKey),
      challenge,
      signature: hex(signature),
    });
    const answer = await r.json().catch(() => ({}));
    if (r.status === 401 && typeof answer.tries_left === "number") {
      throw new ControlError("wrong_code", String(answer.tries_left));
    }
    if (!r.ok) throw new ControlError(codeOf(r.status), `verify: ${r.status} ${answer.error ?? ""}`);
    this.token = answer.token;
    return answer;
  }

  /** Chooses the user name; a taken one is `username_taken`, with free
      ones, comma-separated, as the message. */
  async setUsername(username: string, displayName: string): Promise<{ username: string; name: string }> {
    try {
      return await this.call("PUT", "/v1/me/username", { username, display_name: displayName });
    } catch (e) {
      if (e instanceof ControlError && / 409 /.test(e.message)) {
        const r = await this.fetch("PUT", "/v1/me/username", { username, display_name: displayName });
        const answer = await r.json().catch(() => ({ suggestions: [] }));
        throw new ControlError("username_taken", (answer.suggestions ?? []).join(","));
      }
      throw e;
    }
  }

  devices(): Promise<{ public_key: string; this_device: boolean; last_seen_at: string }[]> {
    return this.call("GET", "/v1/me/devices");
  }

  removeDevice(key: string): Promise<void> {
    return this.call("DELETE", `/v1/me/devices/${key}`);
  }

  /** Forgets the token: the next request logs in again. */
  forgetToken(): void {
    this.token = "";
  }

  invitations(): Promise<{ space_id: string; space_name: string; invited_by: string }[]> {
    return this.call("GET", "/v1/me/invitations");
  }

  acceptInvitation(space: string): Promise<SpaceAnswer> {
    return this.call("POST", `/v1/me/invitations/${space}`);
  }

  declineInvitation(space: string): Promise<void> {
    return this.call("DELETE", `/v1/me/invitations/${space}`);
  }

  inviteUser(space: string, username: string): Promise<{ username: string; name: string }> {
    return this.call("POST", `/v1/spaces/${space}/invitations/users`, { username });
  }

  // --- The MLS delivery service (protocol section 16) -----------------------

  postKeyPackage(space: string, device: string, keyPackage: Bytes): Promise<void> {
    return this.call("POST", `/v1/spaces/${space}/mls/key-packages`, { device, key_package: hex(keyPackage) });
  }

  async keyPackages(space: string): Promise<{ device: Bytes; keyPackage: Bytes }[]> {
    const list: { device: string; key_package: string }[] = await this.call("GET", `/v1/spaces/${space}/mls/key-packages`);
    return list.map((p) => ({ device: unhex(p.device), keyPackage: unhex(p.key_package) }));
  }

  /** Posts a commit made in `epoch`; the epoch it started, or a
      StaleEpoch when another member's came first. */
  async postCommit(space: string, epoch: number, commit: Bytes, welcome: Bytes, welcomed: Bytes): Promise<number> {
    const devices: string[] = [];
    for (let i = 0; i + 32 <= welcomed.length; i += 32) devices.push(hex(welcomed.subarray(i, i + 32)));
    try {
      const answer: { epoch: number } = await this.call("POST", `/v1/spaces/${space}/mls/commits`, {
        epoch,
        commit: hex(commit),
        welcome: welcome.length > 0 ? hex(welcome) : "",
        welcomed: devices,
      });
      return answer.epoch;
    } catch (e) {
      if (e instanceof ControlError && / 409 /.test(e.message)) throw new StaleEpoch();
      throw e;
    }
  }

  async commits(space: string, after: number): Promise<{ epoch: number; commit: Bytes }[]> {
    const list: { epoch: number; commit: string }[] = await this.call("GET", `/v1/spaces/${space}/mls/commits?after=${after}`);
    return list.map((c) => ({ epoch: c.epoch, commit: unhex(c.commit) }));
  }

  /** The welcome kept for this device, if any. */
  async welcome(space: string, device: string): Promise<Bytes | null> {
    try {
      const answer: { welcome: string } = await this.call("GET", `/v1/spaces/${space}/mls/welcome?device=${device}`);
      return unhex(answer.welcome);
    } catch (e) {
      if (e instanceof ControlError && e.code === "not_found") return null;
      throw e;
    }
  }

  /** The spaces this account is a member of. */
  spaces(): Promise<{ space: { id: string; name: string }; role: string }[]> {
    return this.call("GET", "/v1/spaces");
  }

  /** Creates a space, with a creation code if this key needs one. */
  createSpace(name: string, displayName: string, creatorCode: string | null): Promise<SpaceAnswer> {
    const body: Record<string, string> = { name, room: "general", display_name: displayName };
    if (creatorCode) body.creator_code = creatorCode;
    return this.call("POST", "/v1/spaces", body);
  }

  /** A new invitation code to a space, for 10 people and 7 days. */
  createInvitation(space: string): Promise<{ code: string; invitation: { max_uses: number; expires_at: string } }> {
    return this.call("POST", `/v1/spaces/${space}/invitations`, {});
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
