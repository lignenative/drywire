// Safety codes (ADR 0053): what two people compare, by voice or in
// person, to know the server listed the right key for each, and the
// marks this device keeps of the people its person verified.

const PREFIX = new TextEncoder().encode("drywire/safety-code/v1");
const VERIFIED_KEY = "drywire.verified";

function bytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(2 * i, 2 * i + 2), 16);
  return out;
}

function compare(a: Uint8Array, b: Uint8Array): number {
  for (let i = 0; i < Math.min(a.length, b.length); i++) if (a[i] !== b[i]) return a[i] - b[i];
  return a.length - b.length;
}

/** The safety code of two public keys (hex), in four groups of five
    digits: the same whichever key comes first. */
export async function safetyCode(keyA: string, keyB: string): Promise<string> {
  const [lo, hi] = [bytes(keyA), bytes(keyB)].sort(compare);
  const input = new Uint8Array(PREFIX.length + lo.length + hi.length);
  input.set(PREFIX);
  input.set(lo, PREFIX.length);
  input.set(hi, PREFIX.length + lo.length);
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", input));
  let n = 0n;
  for (const b of digest.subarray(0, 9)) n = (n << 8n) | BigInt(b);
  const digits = (n % 10n ** 20n).toString().padStart(20, "0");
  return digits.match(/.{5}/g)!.join(" ");
}

/** The people this device's person verified: public key → the name it had. */
export type Verified = Record<string, string>;

export function loadVerified(): Verified {
  try {
    const raw = localStorage.getItem(VERIFIED_KEY);
    return raw ? (JSON.parse(raw) as Verified) : {};
  } catch {
    return {};
  }
}

export function saveVerified(verified: Verified): void {
  try {
    localStorage.setItem(VERIFIED_KEY, JSON.stringify(verified));
  } catch {
    // A mark that cannot be kept is lost, never wrong: the person is
    // simply shown as not verified next time.
  }
}

/** What the lock's panel says of a person: verified with this very key,
    seen verified under this name with another key, or neither. */
export type Verification = "verified" | "newKey" | "none";

export function verificationOf(verified: Verified, publicKey: string, name: string): Verification {
  if (publicKey in verified) return "verified";
  return Object.values(verified).includes(name) ? "newKey" : "none";
}
