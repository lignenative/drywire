// Run with `bun test` in site/, after `astro build`: the last tests read
// the built page, which is what browsers get. `make test` builds first.
import { expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import { base64url, readInvitation } from "./invite";

const manifest = JSON.parse(
  readFileSync(
    new URL("../../../core/drywire-proto/vectors/manifest.json", import.meta.url),
    "utf8",
  ),
);

const dist = new URL("../../dist/", import.meta.url);

/** The built page, or a failure that says how to build it. */
function built(path: string): string {
  const file = new URL(path, dist);
  if (!existsSync(file)) throw new Error(`${file.pathname} is missing: run make site-build`);
  return readFileSync(file, "utf8");
}

test("the shared vector's link is read like the Rust and Go code read it", () => {
  const fragment = manifest.invitation.link.split("#")[1];
  const payload = base64url(fragment);
  expect(Buffer.from(payload ?? []).toString("hex")).toBe(manifest.invitation.payload);
  expect(readInvitation(fragment)).toEqual({ instance: manifest.invitation.instance });
});

test("damaged and newer links are told apart", () => {
  expect(readInvitation("A")).toEqual({ error: "damaged" });
  expect(readInvitation("AB")).toEqual({ error: "damaged" }); // spare bits
  expect(readInvitation("AQ")).toEqual({ error: "damaged" }); // version only
  expect(readInvitation("Ag")).toEqual({ error: "newer" });
  expect(readInvitation("a+b/")).toEqual({ error: "damaged" });
});

test("the built page can make no request with the fragment", () => {
  const html = built("i/index.html");
  const policy = html.match(/<meta http-equiv="Content-Security-Policy" content="([^"]*)"/)?.[1];
  expect(policy).toBeDefined();
  expect(policy).toContain("default-src 'none'");
  expect(policy).toContain("connect-src 'none'");
  expect(policy).toContain("script-src 'self'");
  expect(policy).toContain("style-src 'self'");
  expect(policy).not.toContain("unsafe");
  expect(html).toContain('<meta name="referrer" content="no-referrer"');
  expect(html).toContain('<meta name="robots" content="noindex"');

  // A meta policy only governs what comes after it in the document.
  const head = html.slice(0, html.indexOf("</head>"));
  const firstLoad = head.search(/<(?:link|script|style)\b/);
  if (firstLoad !== -1) expect(head.indexOf("Content-Security-Policy")).toBeLessThan(firstLoad);
});

test("everything the built page loads comes from its own files", () => {
  const html = built("i/index.html");
  // Inline scripts and styles would be refused by the policy, and the
  // page would break silently: every script and style is a file.
  expect(html).not.toMatch(/<script(?![^>]*\bsrc=)[^>]*>/);
  expect(html).not.toMatch(/<style\b/);
  expect(html).not.toMatch(/\sstyle="/);

  const urls = [...html.matchAll(/(?:src|href)="([^"]*)"/g)].map(([, url]) => url);
  expect(urls.some((url) => url.endsWith(".js"))).toBe(true);
  expect(urls.some((url) => url.endsWith(".css"))).toBe(true);
  for (const url of urls) {
    // Nothing loaded from elsewhere, and only links the page sets itself.
    expect(url).not.toMatch(/^(https?:)?\/\//);
    if (url === "#") continue;
    expect(url).toStartWith("/");
    const asset = built(url.slice(1));
    // The bundled files fetch nothing either.
    expect(asset).not.toMatch(/\b(?:fetch|XMLHttpRequest|WebSocket|EventSource|sendBeacon)\b/);
    expect(asset).not.toMatch(/url\((?!["']?\/)/);
  }
});
