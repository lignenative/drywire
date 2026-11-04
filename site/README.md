# site

The project's static site, served from `https://drywire.gg`. For now it
holds the invitation page; the landing page, the pricing page and the
privacy policy come later (spec sections 9 and 14).

Built with [Astro](https://astro.build) into plain HTML, CSS and
JavaScript files (ADR 0042), with [Bun](https://bun.sh) only: no Node.js,
no npm. There is no server code, and the Go server never serves any of it:
any static file server can serve `dist/`.

## Run it

From the repository root:

| Command | What it does |
| --- | --- |
| `make site` | Dev server with live reload, on `http://127.0.0.1:10053` |
| `make site-build` | The static site into `site/dist/` |
| `make site-preview` | Build, then serve `dist/` on `http://127.0.0.1:10052` |
| `make site-check` | Type-check (`astro check`), then lint (Biome) |
| `make site-stop` | Stop whatever listens on 10053 and 10052 |
| `make fmt` | Format the site's TypeScript with Biome, among the rest |
| `make test` | Check, build, then test the built pages and check licenses |

`SITE_DEV_PORT` and `SITE_PORT` move the two ports for a one-off. A busy
port is an error, never a silent move to the next one. Astro's telemetry
is off in every script.

To try the invitation page, open `http://127.0.0.1:10052/i#` followed by
a link's fragment. Use `make site-preview` for that, not `make site`: only
the built pages carry the security policy (see below).

## Layout

```text
src/
  pages/i.astro       the invitation page and its security policy
  invite/invite.ts    decodes the payload; tested against the shared vector
  invite/page.ts      fills the page, in English or French
  invite/style.css    light and dark
  invite/invite.test.ts  the decoder, and the built page's guarantees
stubs/sharp/          an empty package in place of sharp (ADR 0042)
astro.config.mjs      static output, ports, nothing inlined
```

## The invitation page: `/i`

Every invitation link points here, whichever instance it is for:
`https://drywire.gg/i#<payload>` (protocol section 14). The page reads the
fragment in the browser, shows which instance the invitation is for, and
offers to open it in the Drywire app (`drywire://i#…`) or with
`drywire join --link`.

What it never does, and how that is enforced:

- **No request with the fragment.** Browsers never send a fragment to a
  server, and the page's Content-Security-Policy forbids every connection
  (`connect-src 'none'`), image and form; scripts and styles come from the
  page's own origin only. Astro is told never to inline a script, a
  stylesheet or an asset, which that policy would refuse.
  `invite.test.ts` reads the built page and checks the policy, that it
  comes before anything the page loads, that nothing is inline or loaded
  from elsewhere, and that the bundled files make no request.
- **Less trace in history.** Once read, the fragment is removed from the
  address bar (`history.replaceState`), so the page's history entry reads
  `/i`. A browser may still have recorded the full link when the page
  opened, and some sync their history to their vendor: the link stays a
  secret to share privately, and the page says so.
- **No cookie, no analytics, no external font**, and `noindex`.

The dev server leaves the policy out, because Vite injects styles inline
and reloads the page through a socket; the built pages always have it.

## Dependencies

Pinned to exact versions (`bunfig.toml`), and their licenses checked by
`scripts/js-licenses.ts site` in `make test`. Astro's optional image
library, sharp, brings libvips, which is LGPL: `package.json` replaces it
with the empty package in `stubs/sharp`, and Astro uses its pass-through
image service instead (ADR 0042).
