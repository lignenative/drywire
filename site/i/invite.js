// Reads a Drywire invitation link in the browser (docs/protocol.md,
// section 14), only to show where it leads and how to use it.
//
// Nothing here sends anything anywhere: the page's Content-Security-Policy
// forbids every connection (connect-src 'none'), and the fragment, which
// holds the invitation code and the space key, never leaves this page.

/** Decodes base64url without padding; null if it is not exactly that. */
export function base64url(text) {
  if (!/^[A-Za-z0-9_-]*$/.test(text) || text.length % 4 === 1) return null;
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
  const bytes = [];
  for (let i = 0; i < text.length; i += 4) {
    const chunk = text.slice(i, i + 4);
    let n = 0;
    for (let j = 0; j < 4; j++) {
      n = n * 64 + (j < chunk.length ? alphabet.indexOf(chunk[j]) : 0);
    }
    const three = [(n >> 16) & 255, (n >> 8) & 255, n & 255];
    const used = chunk.length - 1;
    // Spare bits must be zero, so a payload has one spelling.
    if (three.slice(used).some((b) => b !== 0)) return null;
    bytes.push(...three.slice(0, used));
  }
  return new Uint8Array(bytes);
}

const CODE_SIZE = 16;
const KEY_SIZE = 32;

/**
 * Reads the fragment of an invitation link (without the "#"). Returns
 * { instance } for a valid invitation, or { error } saying what is wrong:
 * "damaged" or "newer". The code and the key are not returned: the page
 * has no use for them.
 */
export function readInvitation(fragment) {
  const payload = base64url(fragment);
  if (payload === null || payload.length === 0) return { error: "damaged" };
  if (payload[0] !== 1) return { error: "newer" };
  if (payload.length <= 1 + CODE_SIZE + KEY_SIZE) return { error: "damaged" };
  let instance;
  try {
    instance = new TextDecoder("utf-8", { fatal: true }).decode(
      payload.slice(1 + CODE_SIZE + KEY_SIZE),
    );
  } catch {
    return { error: "damaged" };
  }
  if (!/^https?:\/\/[^\s/?#@]+$/.test(instance)) return { error: "damaged" };
  return { instance };
}
