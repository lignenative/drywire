# site

The project's static site, served from `https://drywire.gg`. For now it
holds the invitation page; the landing page and the privacy policy come
later (spec section 14). Plain HTML, CSS and JavaScript: no build step, no
framework, no server code. The Go server never serves any of it.

## The invitation page: `i/`

Every invitation link points here, whichever instance it is for:
`https://drywire.gg/i#<payload>` (protocol section 14). The page reads the
fragment in the browser, shows which instance the invitation is for, and
offers to open it in the Drywire app (`drywire://i#…`) or with
`drywire-cli join --link`.

What it never does, and how that is enforced:

- **No request with the fragment.** Browsers never send a fragment to a
  server, and the page's Content-Security-Policy forbids every connection
  (`connect-src 'none'`), image and form; scripts and styles come from the
  page's own origin only. `invite.test.js` checks the policy and that
  nothing is loaded from elsewhere.
- **Less trace in history.** Once read, the fragment is removed from the
  address bar (`history.replaceState`), so the page's history entry reads
  `/i`. A browser may still have recorded the full link when the page
  opened, and some sync their history to their vendor: the link stays a
  secret to share privately, and the page says so.
- **No cookie, no analytics, no external font**, and `noindex`.

| File | What it does |
| --- | --- |
| `i/index.html` | The page and its security policy |
| `i/invite.js` | Decodes the payload; tested against the shared vector |
| `i/page.js` | Fills the page, in English or French |
| `i/style.css` | Light and dark |

## Run and test it

```sh
cd site && bun test                       # the decoder and the policy
python3 -m http.server 10052 --bind 127.0.0.1 --directory site   # preview
```

Then open `http://127.0.0.1:10052/i/#` followed by a link's fragment.
Module scripts do not load from `file://`, so the page needs a server, even
locally.
