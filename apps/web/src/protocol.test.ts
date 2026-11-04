// The WebAssembly module against the shared test vectors
// (core/drywire-proto/vectors/): the browser runs the protocol exactly as
// the native clients and the relay do. Needs the module built:
// scripts/test.sh and scripts/web.sh build it into public/.

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { fromKeys, sign } from "./identity";
import { hex, Protocol, unhex } from "./protocol";

const dir = new URL("../../../core/drywire-proto/vectors/", import.meta.url);
const manifest = JSON.parse(readFileSync(new URL("manifest.json", dir), "utf8"));
const file = (name: string) => new Uint8Array(readFileSync(new URL(`${name}.bin`, dir)));
const vector = (name: string) => manifest.vectors.find((v: { name: string }) => v.name === name);
const wasm = readFileSync(new URL("../public/drywire.wasm", import.meta.url));
const inputs = manifest.inputs;
const epoch0 = manifest.epochs[0];

/** The vectors' sender, as a client of its own session. */
async function sender(): Promise<Protocol> {
  const p = await Protocol.load(wasm);
  p.start({
    session: inputs.session,
    secret: unhex(inputs.session_secret),
    spaceKey: unhex(inputs.space_key),
    roomId: unhex(inputs.room_id),
    publicKey: unhex(inputs.sender_public_key),
    salt: unhex(epoch0.sender_salt),
  });
  return p;
}

/** Another member of the room, who knows the sender's public key. */
async function listener(): Promise<Protocol> {
  const p = await Protocol.load(wasm);
  p.start({
    session: 7,
    secret: new Uint8Array(32),
    spaceKey: unhex(inputs.space_key),
    roomId: unhex(inputs.room_id),
    publicKey: unhex(inputs.sender_public_key),
    salt: new Uint8Array(32),
  });
  p.addPeer(inputs.session, unhex(inputs.sender_public_key));
  return p;
}

describe("the WebAssembly protocol module", () => {
  test("builds the hello and checks the hello_ack", async () => {
    const p = await sender();
    const counter = BigInt(vector("hello").hello.counter);
    expect(hex(p.hello(counter))).toBe(hex(file("hello")));
    expect(p.isHelloAck(file("hello-ack"), counter)).toBe(true);
    expect(p.isHelloAck(file("hello-ack"), counter - 1n)).toBe(false);
  });

  test("builds the announcement from the signature WebCrypto makes", async () => {
    const p = await sender();
    // The vectors' seed as a WebCrypto key: PKCS #8 wraps a raw Ed25519 seed.
    const pkcs8 = new Uint8Array([
      ...unhex("302e020100300506032b657004220420"),
      ...unhex(inputs.sender_seed),
    ]);
    const privateKey = await crypto.subtle.importKey("pkcs8", pkcs8, "Ed25519", false, ["sign"]);
    const publicKey = await crypto.subtle.importKey(
      "raw",
      unhex(inputs.sender_public_key),
      "Ed25519",
      true,
      ["verify"],
    );
    const identity = await fromKeys({ privateKey, publicKey });
    const announce = p.announce(await sign(identity, p.announcementMessage()));
    // Any valid signature will do; this engine's is RFC 8032's.
    expect(hex(announce)).toBe(hex(file("announce")));
    expect(hex(p.announce(unhex(epoch0.signature)))).toBe(hex(file("announce")));
  });

  test("opens audio once the announcement is in, and drops what it must", async () => {
    const p = await listener();
    expect(p.receive(file("audio"))).toEqual({ kind: "dropped", reason: "no_key" });
    expect(p.receive(file("announce"))).toEqual({ kind: "announced" });
    const audio = p.receive(file("audio"));
    expect(audio.kind).toBe("audio");
    if (audio.kind === "audio") {
      expect(hex(audio.frame)).toBe(vector("audio").audio.primary_frame);
      expect([audio.session, audio.sequence, audio.timestamp]).toEqual([inputs.session, 7, 76800]);
    }
    expect(p.receive(file("audio"))).toEqual({ kind: "dropped", reason: "duplicate" });
    for (const v of manifest.vectors.filter((v: { error?: string }) => v.error)) {
      if (v.error === "bad_mac") continue; // a hello: the relay's business
      expect([v.name, p.receive(file(v.name))]).toEqual([v.name, { kind: "dropped", reason: v.error }]);
    }
  });

  test("reads the invitation link and the login message", async () => {
    const p = await Protocol.load(wasm);
    const invitation = p.invitation(manifest.invitation.link);
    expect(invitation.instance).toBe(manifest.invitation.instance);
    expect(hex(invitation.code)).toBe(manifest.invitation.code);
    expect(hex(invitation.spaceKey)).toBe(inputs.space_key);
    const { challenge, audience, message } = manifest.login;
    expect(hex(p.loginMessage(unhex(challenge), audience))).toBe(message);
    expect(() => p.invitation("https://drywire.gg/i#nope")).toThrow("not_an_invitation");
  });
});
