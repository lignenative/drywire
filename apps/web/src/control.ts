// The control plane, as the web client needs it (protocol section 13):
// log in with the identity, redeem an invitation, join a room.
//
// Requests go to `base`. In development that is the page's own origin,
// which Vite forwards to the server: the control plane does not answer
// cross-origin requests (yet), and the login is still signed for the
// instance's own origin, the audience.

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

type SpaceView = { space: { id: string; name: string }; rooms: { id: string; name: string }[] };

export class ControlPlane {
  private token = "";

  constructor(
    private readonly base: string,
    private readonly audience: string,
    private readonly identity: Identity,
    private readonly protocol: Protocol,
  ) {}

  private async call<T>(method: string, path: string, body?: unknown): Promise<T> {
    const headers: Record<string, string> = { "Content-Type": "application/json" };
    if (this.token) headers.Authorization = `Bearer ${this.token}`;
    const r = await fetch(this.base + path, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!r.ok) {
      const error = await r.json().catch(() => ({ error: r.statusText }));
      throw new Error(`${method} ${path}: ${r.status} ${error.error}`);
    }
    return r.status === 204 ? (undefined as T) : r.json();
  }

  async login(): Promise<void> {
    const { challenge } = await this.call<{ challenge: string }>("POST", "/v1/auth/challenge");
    const message = this.protocol.loginMessage(unhex(challenge), this.audience);
    const signature = await sign(this.identity, message);
    const { token } = await this.call<{ token: string }>("POST", "/v1/auth/login", {
      public_key: hex(this.identity.publicKey),
      challenge,
      signature: hex(signature),
    });
    this.token = token;
  }

  redeem(code: Bytes, displayName: string): Promise<SpaceView> {
    return this.call("POST", "/v1/invitations/redeem", { code: hex(code), display_name: displayName });
  }

  join(room: string): Promise<JoinAnswer> {
    return this.call("POST", `/v1/rooms/${room}/join`);
  }

  leave(): Promise<void> {
    return this.call("POST", "/v1/leave");
  }
}
