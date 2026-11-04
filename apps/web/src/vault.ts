// Account vaults in the browser (protocol section 15, ADR 0051): the code
// a person types, the id and key it derives, and the sealed bytes an
// instance keeps without being able to open them.
//
// The same thing as drywire-proto's vault.rs, in WebCrypto: a vault is up
// to 24 KiB, more than the WebAssembly module's shared buffer holds, and
// HKDF and AES-GCM are in every browser the client runs in. The tests
// check it against the Rust implementation's vectors.

import type { Bytes } from "./protocol";

const ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const CODE_LENGTH = 26;
const VERSION = 1;
const encoder = new TextEncoder();

export class BadCode extends Error {}
export class WrongCode extends Error {}

/** A code's 16 secret bytes. */
export type Secret = Bytes;

/** Reads a code as a person typed it (section 15.1). */
export function parseCode(text: string): Secret {
  let bits = 0n;
  let count = 0;
  let last = 0;
  for (const raw of text.toUpperCase()) {
    if (raw === "-" || raw === " ") continue;
    const c = raw === "O" ? "0" : raw === "I" || raw === "L" ? "1" : raw;
    const value = ALPHABET.indexOf(c);
    if (value < 0 || c.length !== 1) throw new BadCode("not a Drywire code");
    count++;
    if (count > CODE_LENGTH) throw new BadCode("not a Drywire code");
    if (count < CODE_LENGTH) bits = (bits << 5n) | BigInt(value);
    else last = value;
  }
  if (count !== CODE_LENGTH || (last & 0b11) !== 0) throw new BadCode("not a Drywire code");
  // 25 characters gave 125 bits; the last gives 3 more.
  bits = (bits << 3n) | BigInt(last >> 2);
  const secret = new Uint8Array(16);
  for (let i = 15; i >= 0; i--) {
    secret[i] = Number(bits & 0xffn);
    bits >>= 8n;
  }
  return secret;
}

/** The code as shown to a person, in groups of four. */
export function formatCode(secret: Secret): string {
  let bits = 0n;
  for (const b of secret) bits = (bits << 8n) | BigInt(b);
  bits <<= 2n; // 130 bits: the last character carries two zero bits.
  let out = "";
  for (let i = 0; i < CODE_LENGTH; i++) {
    if (i > 0 && i % 4 === 0) out += "-";
    out += ALPHABET[Number((bits >> BigInt(5 * (CODE_LENGTH - 1 - i))) & 31n)];
  }
  return out;
}

/** A fresh code's secret. */
export function newSecret(): Secret {
  return crypto.getRandomValues(new Uint8Array(16));
}

async function derive(secret: Secret, info: string): Promise<Bytes> {
  const ikm = await crypto.subtle.importKey("raw", secret, "HKDF", false, ["deriveBits"]);
  const bits = await crypto.subtle.deriveBits(
    { name: "HKDF", hash: "SHA-256", salt: new Uint8Array(0), info: encoder.encode(info) },
    ikm,
    256,
  );
  return new Uint8Array(bits);
}

/** The id an instance keeps the vault under. */
export function vaultId(secret: Secret): Promise<Bytes> {
  return derive(secret, "drywire/vault/id/v1");
}

/** The vault's AES-256-GCM key, as raw bytes (for the tests). */
export function vaultKeyBytes(secret: Secret): Promise<Bytes> {
  return derive(secret, "drywire/vault/key/v1");
}

async function cipher(secret: Secret): Promise<{ key: CryptoKey; aad: Bytes }> {
  const key = await crypto.subtle.importKey("raw", await vaultKeyBytes(secret), "AES-GCM", false, [
    "encrypt",
    "decrypt",
  ]);
  const id = await vaultId(secret);
  const prefix = encoder.encode("drywire/vault/v1");
  const aad = new Uint8Array(prefix.length + id.length);
  aad.set(prefix);
  aad.set(id, prefix.length);
  return { key, aad };
}

/** Seals a vault's plaintext; the nonce is drawn here unless given. */
export async function seal(secret: Secret, plaintext: Bytes, nonce?: Bytes): Promise<Bytes> {
  const iv = nonce ?? crypto.getRandomValues(new Uint8Array(12));
  const { key, aad } = await cipher(secret);
  const sealed = new Uint8Array(
    await crypto.subtle.encrypt({ name: "AES-GCM", iv, additionalData: aad }, key, plaintext),
  );
  const out = new Uint8Array(1 + iv.length + sealed.length);
  out[0] = VERSION;
  out.set(iv, 1);
  out.set(sealed, 1 + iv.length);
  return out;
}

/** Opens sealed bytes made with this code. */
export async function open(secret: Secret, sealed: Bytes): Promise<Bytes> {
  if (sealed.length < 13 + 16) throw new WrongCode("this code does not open this vault");
  if (sealed[0] !== VERSION) throw new WrongCode("this vault needs a newer version of Drywire");
  const { key, aad } = await cipher(secret);
  try {
    return new Uint8Array(
      await crypto.subtle.decrypt(
        { name: "AES-GCM", iv: sealed.subarray(1, 13), additionalData: aad },
        key,
        sealed.subarray(13),
      ),
    );
  } catch {
    throw new WrongCode("this code does not open this vault");
  }
}

/** What a vault holds (section 15.2). */
export interface VaultContents {
  /** The account's Ed25519 seed, in hex. */
  account: string;
  name: string;
  spaces: { instance: string; id: string; name: string; key: string }[];
}
