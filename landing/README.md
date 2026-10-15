# ForzaInfinity landing

The public page: what ForzaInfinity is, the server's live totals, the install steps and the
docs. React + Vite, a static site.

```sh
cd landing
npm install
npm run dev        # http://localhost:5174
npm run build      # dist/
```

Settings (`.env`, see `.env.example`):

| Variable | |
|---|---|
| `VITE_STATS_URL` | The server's `GET /infinity/v1/stats` (CORS is open). Default: the public server, `https://infinity.anarqz.dev/infinity/v1/stats` (`.env.example` points at a local one). |
| `VITE_SERVER_ADDRESS` | The `host:port` players connect to, shown in the install steps. Empty: "ask whoever runs your server". |
| `VITE_REPO` | `owner/name`; download links go to its latest release. Default `anarqz/ForzaInfinity`. |
| `VITE_BASE` | The path the site is served under (`/ForzaInfinity/` on GitHub Pages). |
| `VITE_SITE_URL` | The site's full address, ending in `/`, for the sharing tags. Default `https://anarqz.github.io/ForzaInfinity/`. |

## Deploy

`.github/workflows/landing.yml` builds and publishes it to GitHub Pages on every push to the
`landing` branch (or by hand: *Actions › landing › Run workflow*; a new branch identical to
`main` does not trigger it). Once: *Settings › Pages › Source: GitHub Actions*, with `landing`
allowed in the `github-pages` environment. The repository variables `STATS_URL` and
`SERVER_ADDRESS` are optional: without them the page uses the public server.

## Sharing

`index.html` has Open Graph and X/Twitter tags (a large image card) and schema.org data. The
card is `public/og.png` (1200×630), made from `og/card.html` by `og/render.sh` (headless
Firefox): change the card there and render it again. After publishing, social sites keep
their copy of a link's card for a while; to see a change at once, use the site's own tool
(Facebook's Sharing Debugger, LinkedIn's Post Inspector) or share the link with `?v=2`.

The look is a festival at dusk with the Xbox green, without any Microsoft logo or artwork;
the footer says the project is not affiliated.
