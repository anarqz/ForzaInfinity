# Running a ForzaInfinity server

The server (`server/`) is one Rust program: std only for the network side, SQLite
(`rusqlite`, bundled) for storage. It answers the game's Forza web services and the Xbox Live
calls it makes, the hook's account endpoints, a local manager page and a public stats
endpoint.

## Build and run

```sh
cd server
cargo build --release
./run.sh          # Linux; run.bat on Windows (build.bat builds there)
```

`run.sh` listens on `0.0.0.0:8081`, records every request in `captures/` (development) and
mirrors its log to `logs/server-<date>-<time>.log`. If the game folder sits next to the repo
(`Forza Horizon 2/Anthem_…/Mount`), it also reads the game's car database for car and route
names and the integrity checks.

Players need TCP **8081** open to them. With nftables, for the local network:

```sh
sudo nft add rule inet filter input ip saddr 192.168.0.0/16 tcp dport 8081 accept
```

## Data

Everything lives in `server/data/fh2online.sqlite` (WAL): accounts and devices, profiles,
cloud saves, scores, ghosts, drivatars, liveries, tunes, the Storefront, friends, clubs and
notifications. Back it up with SQLite's online backup (`sqlite3 fh2online.sqlite ".backup
copy.sqlite"`) or with the server stopped.

## Endpoints

| Path | Who | What |
|---|---|---|
| `/<host>/<path>` + `X-Fh2-Original-Url` | the hook | The game's own requests (Forza `bin/xtsw` services, Xbox Live JSON). |
| `/infinity/v1/register`, `me`, `login`, `webkey`, `friends`, `delete` | the hook | Accounts: one per PC, identified by its token (`X-Fh2-Token`). |
| `/infinity/v1/save`, `save/info` | the hook | The cloud save (PUT/GET). |
| `/infinity/v1/info` | anyone | `{ "name": "ForzaInfinity", "api": 1 }` |
| `/infinity/v1/stats` | anyone (CORS open) | Server-wide totals for the landing page, cached 30 s. |
| `/ui/`, `/api/`, `/ws` | this machine only | The manager (web/): players, friends, clubs, live feed. |

Hooks before fh2online.33 call `/fh2online/v1/…`; the server answers both.

## Limits

- Any request body: 2 MB. Account requests: 4 KB.
- Cloud saves: 512 KB per upload (a real one is about 28 KB), 8 MB per file and 16 MB in all
  once unpacked, only the seven file names of an Azimuth save, 3 uploads kept per player, at
  most one upload every 30 s per address after a burst of 5.
- Rates per address (token buckets): game traffic, account calls, account creation (3 an
  hour), sign-ins (slow on purpose).
- Storefront: 100 shared files per player; a file goes after reports from 3 different players.
- Profile syncs are checked for impossible growth (credits, XP, level, top speed, race times);
  a refused sync is not stored and counts as a strike on the profile.

## Privacy

The server is one open world: every player sees everyone's gamertag, times and positions,
ghosts, drivatar, car club and shared files (banned accounts are left out). Friends are the
social side: the friends list and presence, messages, challenges and Meus Rivais, and they come
first in drivatar grids and as rivals. Hidden clubs (privacy 3) are seen only by their members
and the members' friends. `/infinity/v1/stats` only gives totals. A player deleting their profile from the game removes
every record of them, including friendships and shared files.
