# 0042. Build the site with Astro

Status: Accepted · 2026-11-04

## Context

The project site (`site/`) held one page, the invitation page `/i`, in
plain HTML, CSS and JavaScript with no build step. That suited a single
page whose whole job is to read a link's fragment and send nothing.

The site is about to grow for the public launch: a landing page and a
pricing page (self-hosted for free, hosted plans per space), in English
and French, drawn on the design canvas with large panels and motion, and
later the privacy policy (spec section 9). Written by hand, every page
would repeat its head, its navigation, its footer and its translations,
and the canvas's theme would be copied into each stylesheet.

The maintainer builds his other static sites with Astro, Tailwind and
Bun, and the desktop app's interface already defines Drywire's theme as
Tailwind variables (ADR 0037).

## Decision

- **Astro, static output only.** `astro build` writes plain HTML, CSS and
  JavaScript files to `site/dist/`, served by any static file server. No
  server-side rendering, no adapter, and still no server code: the Go
  server never serves the site.
- **Bun only**, as for the desktop interface: package manager, script
  runner (`bun --bun astro …`) and test runner. No Node.js, no npm.
  Dependencies are pinned to exact versions (`site/bunfig.toml`).
- **Tailwind** with the app's theme for the new pages, Geist bundled from
  `@fontsource-variable`. shadcn components only once a page needs one.
- **The invitation page keeps every guarantee** listed in `site/README.md`.
  Its policy (`connect-src 'none'`, scripts and styles from its own origin
  only) is written in the built page; Astro is told never to inline a
  stylesheet, a script or an asset, which the policy would refuse. The
  tests read the built page, as browsers get it, rather than the source.
  The policy is left out of the dev server's pages only, because Vite
  injects styles inline and reloads through a socket; `make site-preview`
  serves the built pages to check it.
- **Ports** from Drywire's block (CLAUDE.md): the built site on 10052, as
  reserved, and the dev server on 10053. 10051 is the desktop app's Vite;
  10053 is the block's PostgreSQL slot, free because the control plane
  keeps its data in SQLite. `SITE_PORT` and `SITE_DEV_PORT` move them.
- **Telemetry off.** Astro reports anonymous usage by default; every
  script sets `ASTRO_TELEMETRY_DISABLED=1`.

## Consequences

- A build step: `make site-build` before deploying, and `make test` builds
  the site before testing it.
- Astro brings about 260 packages, all build-time tools: none of their
  code reaches the pages except what Astro bundles of ours. Three of them
  use licenses that were not on the list, and two join it, for JavaScript
  packages:
  - **BlueOak-1.0.0** (`lru-cache`, `sax`, `common-ancestor-path`), a
    permissive license that grants what MIT grants, plus an explicit patent
    license.
  - **CC0-1.0** (`mdn-data`, the CSS reference tables Astro's CSS tools
    read), a dedication to the public domain.
- **sharp is replaced by an empty package.** Astro lists sharp, its image
  library, as an optional dependency, and sharp brings libvips, which is
  LGPL and therefore refused. The site processes no image: Astro uses its
  pass-through image service, and `site/package.json` overrides sharp with
  `site/stubs/sharp`, so libvips is never installed. A page that needs
  resized images will need another decision.
- Biome lints and formats the site's TypeScript (`make site-check`,
  `make fmt`); it does not read `.astro` templates yet.
- The invitation page's assets now load from absolute paths
  (`/_astro/…`), so `/i` works with or without a trailing slash.
