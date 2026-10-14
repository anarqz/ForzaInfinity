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

## Deploy

`.github/workflows/landing.yml` builds and publishes it to GitHub Pages on every push to the
`landing` branch. Once: *Settings › Pages › Source: GitHub Actions*, and set the repository
variables `STATS_URL` and `SERVER_ADDRESS` (*Settings › Secrets and variables › Actions ›
Variables*). The stats URL must be reachable from the visitors' browsers (a public address,
over HTTPS, since Pages is HTTPS).

The look follows the Xbox One dashboard (flat tiles, one green accent, Segoe) without any
Microsoft logo or artwork; the footer says the project is not affiliated.
