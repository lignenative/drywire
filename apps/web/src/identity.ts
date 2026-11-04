// The device's identity in the browser: an Ed25519 key pair made by
// WebCrypto with extractable: false, so no script, ours included, can read
// the private key (spec section 8, ADR 0040). It is kept in IndexedDB,
// which stores the key object itself, still non-extractable.

import type { Bytes } from "./protocol";

export type Identity = { keys: CryptoKeyPair; publicKey: Bytes };

const DB = "drywire";
const STORE = "identity";

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

/** Makes a new identity. The public key, which is no secret, is readable. */
export async function newIdentity(): Promise<Identity> {
  const keys = (await crypto.subtle.generateKey({ name: "Ed25519" }, false, [
    "sign",
    "verify",
  ])) as CryptoKeyPair;
  return fromKeys(keys);
}

export async function fromKeys(keys: CryptoKeyPair): Promise<Identity> {
  const publicKey = new Uint8Array(await crypto.subtle.exportKey("raw", keys.publicKey));
  return { keys, publicKey };
}

/** This browser's identity, made on first use. */
export async function loadIdentity(): Promise<Identity> {
  const db = await open();
  const stored = await request(db.transaction(STORE).objectStore(STORE).get("keys"));
  if (stored) return fromKeys(stored as CryptoKeyPair);
  const identity = await newIdentity();
  await request(db.transaction(STORE, "readwrite").objectStore(STORE).put(identity.keys, "keys"));
  return identity;
}

export async function sign(identity: Identity, message: Bytes): Promise<Bytes> {
  return new Uint8Array(await crypto.subtle.sign("Ed25519", identity.keys.privateKey, message));
}
