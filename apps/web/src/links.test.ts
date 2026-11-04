// The web client's invitation links against the shared vector, which the
// Go server and the Rust clients are checked against too.
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { invitationLink } from "./links";
import { unhex } from "./protocol";

const manifest = JSON.parse(
  readFileSync(new URL("../../../core/drywire-proto/vectors/manifest.json", import.meta.url), "utf8"),
);

test("the link is the vector's, byte for byte", () => {
  const { link, payload, instance } = manifest.invitation;
  const bytes = unhex(payload);
  const code = bytes.subarray(1, 17);
  const key = bytes.subarray(17, 49);
  expect(new TextDecoder().decode(bytes.subarray(49))).toBe(instance);
  expect(invitationLink(code, key, instance)).toBe(link);
});
