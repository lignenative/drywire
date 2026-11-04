// The account's key in the browser. It is an Ed25519 key made from a
// 32-byte seed, which this page keeps in IndexedDB so that it can seal the
// account in a vault and carry it to another device (ADR 0051). For
// signing, the seed is imported as a non-extractable WebCrypto key.
//
// Before ADR 0051 the key was made non-extractable from the start
// (ADR 0040): such a key keeps working, but it has no seed, so it cannot
// go in a vault. A browser that had one and no space yet gets a new one.

import type { Bytes } from "./protocol";

export type Identity = {
  keys: CryptoKeyPair;
  publicKey: Bytes;
  /** The seed, or null for a key from before ADR 0051. */
  seed: Bytes | null;
};

const DB = "drywire";
const STORE = "identity";
/** What PKCS #8 puts before an Ed25519 seed (RFC 8410). */
const PKCS8_PREFIX = [0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20];

function request<T>(r: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}

async function open(): Promise<IDBDatabase> {
  const r = indexedDB.open(DB, 1);
  r.onupgradeneeded = () => r.result.createObjectStore(STORE);
  return request(r);
}

function base64url(text: string): Bytes {
  const plain = atob(text.replace(/-/g, "+").replace(/_/g, "/"));
  return Uint8Array.from(plain, (c) => c.charCodeAt(0));
}

/** The identity of this seed. The public key comes from WebCrypto itself:
    an extractable import gives it as a JWK, then the private key is
    imported again, non-extractable, for use. */
export async function fromSeed(seed: Bytes): Promise<Identity> {
  const pkcs8 = new Uint8Array([...PKCS8_PREFIX, ...seed]);
  const once = await crypto.subtle.importKey("pkcs8", pkcs8, { name: "Ed25519" }, true, ["sign"]);
  const jwk = await crypto.subtle.exportKey("jwk", once);
  const publicKey = base64url(jwk.x ?? "");
  const privateKey = await crypto.subtle.importKey("pkcs8", pkcs8, { name: "Ed25519" }, false, ["sign"]);
  const verify = await crypto.subtle.importKey("raw", publicKey, { name: "Ed25519" }, true, ["verify"]);
  return { keys: { privateKey, publicKey: verify }, publicKey, seed: seed.slice() };
}

/** Makes a new identity. */
export function newIdentity(): Promise<Identity> {
  return fromSeed(crypto.getRandomValues(new Uint8Array(32)));
}

async function fromKeys(keys: CryptoKeyPair): Promise<Identity> {
  const publicKey = new Uint8Array(await crypto.subtle.exportKey("raw", keys.publicKey));
  return { keys, publicKey, seed: null };
}

/** Keeps this identity as the browser's. */
export async function saveIdentity(identity: Identity): Promise<void> {
  const db = await open();
  const record = identity.seed ? { seed: identity.seed } : identity.keys;
  await request(db.transaction(STORE, "readwrite").objectStore(STORE).put(record, "keys"));
}

/** This browser's identity, made on first use. `hasSpaces` says whether an
    old key, which cannot go in a vault, is worth keeping. */
export async function loadIdentity(hasSpaces: boolean): Promise<Identity> {
  const db = await open();
  const stored = await request(db.transaction(STORE).objectStore(STORE).get("keys"));
  if (stored && "seed" in stored) return fromSeed(new Uint8Array(stored.seed));
  if (stored && hasSpaces) return fromKeys(stored as CryptoKeyPair);
  const identity = await newIdentity();
  await saveIdentity(identity);
  return identity;
}

export async function sign(identity: Identity, message: Bytes): Promise<Bytes> {
  return new Uint8Array(await crypto.subtle.sign("Ed25519", identity.keys.privateKey, message));
}
