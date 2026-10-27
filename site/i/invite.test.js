// Run with `bun test` in site/ (make test does when Bun is installed).
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { base64url, readInvitation } from "./invite.js";

const manifest = JSON.parse(
  readFileSync(new URL("../../core/drywire-proto/vectors/manifest.json", import.meta.url)),
);

test("the shared vector's link is read like the Rust and Go code read it", () => {
  const fragment = manifest.invitation.link.split("#")[1];
  const payload = base64url(fragment);
  expect(Buffer.from(payload).toString("hex")).toBe(manifest.invitation.payload);
  expect(readInvitation(fragment)).toEqual({ instance: manifest.invitation.instance });
});

test("damaged and newer links are told apart", () => {
  expect(readInvitation("A")).toEqual({ error: "damaged" });
  expect(readInvitation("AB")).toEqual({ error: "damaged" }); // spare bits
  expect(readInvitation("AQ")).toEqual({ error: "damaged" }); // version only
  expect(readInvitation("Ag")).toEqual({ error: "newer" });
  expect(readInvitation("a+b/")).toEqual({ error: "damaged" });
});

test("the page can make no request with the fragment", () => {
  const html = readFileSync(new URL("./index.html", import.meta.url), "utf8");
  expect(html).toContain("connect-src 'none'");
  expect(html).toContain("default-src 'none'");
  expect(html).toContain('content="no-referrer"');
  // Nothing loaded from elsewhere: every src and href is local.
  for (const [, url] of html.matchAll(/(?:src|href)="([^"]*)"/g)) {
    expect(url).not.toMatch(/^(https?:)?\/\//);
  }
});
