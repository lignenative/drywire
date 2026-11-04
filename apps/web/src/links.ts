// Builds an invitation link (protocol section 14) in the page: the code
// from the instance, the space key this browser holds, and the instance's
// origin, in the fragment the instance never sees.

import type { Bytes } from "./protocol";

function base64url(bytes: Bytes): string {
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** The link to share: https://drywire.gg/i#<payload>. */
export function invitationLink(code: Bytes, spaceKey: Bytes, instance: string): string {
  const origin = new TextEncoder().encode(instance);
  const payload = new Uint8Array(1 + code.length + spaceKey.length + origin.length);
  payload[0] = 1;
  payload.set(code, 1);
  payload.set(spaceKey, 1 + code.length);
  payload.set(origin, 1 + code.length + spaceKey.length);
  return `https://drywire.gg/i#${base64url(payload)}`;
}
