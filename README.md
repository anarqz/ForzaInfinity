# ForzaInfinity

Forza Horizon 2's online services, back for the PC version running on **Azimuth 0.9.10, 0.9.9 or 0.9.7**
(Windows, or Linux and the Steam Deck through Proton).

> **Requires Azimuth 0.9.10 (the latest), 0.9.9 or 0.9.7.** 0.9.8 is not supported: on it the game plays
> offline. The installers check the version. Updating Azimuth puts its own `azimuth-tls.dll`
> back, so run the installer again after each update.

- **Friends**: add each other by `Name#12345` from the game, or with ADD FRIEND in any player's
  PLAYER ACTIONS (leaderboards, club members); see who is online.
- **Car Clubs**: create, invite, roles, notice board.
- **Rivals and ghost races**: every friend's best time on each route is a time to beat, with
  their ghost; with no friend there, the closest time on the server. Every ghost on the
  leaderboards can be raced.
- **Leaderboards**: speed traps, speed zones, skill chains and race times against every player
  on the server, with real positions.
- **Drivatars**: every player's drivatar races you and fills the roads and car meets, in their
  cars' colours; friends first.
- **Storefront**: share designs, vinyl groups and tunes; browse, download, like; three
  strikes from different players take a file down.
- **Cloud save**: your profile starts a new career with your first car (every event, road, barn
  find and treasure there to win, as for every player) and follows you to another PC or the Deck.
- **In Portuguese** when the game runs in Portuguese (Brazil); English otherwise.
- **Updates itself**: a new release installs on its own (from this repository's GitHub
  releases, checked against their SHA-256) and the game asks you to restart.

**Online free roam** is back: players host worlds on their own PC (Settings › INFINITY › HOST WORLD)
and reach each other directly, through routers, with no port to open; see
[docs/FREEROAM.md](docs/FREEROAM.md). Online Road Trip and online races are planned
for a later version.

## Play

Download **ForzaInfinity.zip** from the [latest release](https://github.com/anarqz/ForzaInfinity/releases/latest)
and run `Install.cmd` (Windows) or `install.sh` (Linux / Steam Deck), or in one line:

```sh
curl -fsSL https://github.com/anarqz/ForzaInfinity/releases/latest/download/install.sh | bash   # Linux / Steam Deck
```
```powershell
irm https://github.com/anarqz/ForzaInfinity/releases/latest/download/install.ps1 | iex          # Windows
```

Then start the game and open **Settings › INFINITY › Create profile**. Full guide:
[docs/INSTALL.md](docs/INSTALL.md), which also says [where your save is](docs/INSTALL.md#where-your-save-is). Something wrong? Run **ForzaInfinity-Report** next to
`azimuth.exe` and send the `ForzaInfinity-Report.txt` it writes on your Desktop.

## Host a server

`cd server && cargo build --release && ./run.sh`, then open TCP 8081 to your players.
See [docs/SERVER.md](docs/SERVER.md).

## How it works

Azimuth runs the Xbox One version of the game with its online services switched off.
ForzaInfinity replaces Azimuth's `azimuth-tls.dll` with a hook (a fork of
[ExtraOptions](https://github.com/RokkuDayo/ForzaHorizon2_ExtraOptions), in `hook/`) that
sends the game's own requests to a ForzaInfinity server (`server/`, Rust) instead. The game
files are never modified.

| Folder | |
|---|---|
| `hook/` | The hook (C, submodule, branch `fh2online`). |
| `server/` | The server (Rust, SQLite). |
| `web/` | The server's local manager page. |
| `landing/` | The public page (branch `landing`). |
| `release/` | Player installers. |
| `docs/` | [Install](docs/INSTALL.md), [server](docs/SERVER.md), [development](docs/DEVELOPMENT.md). |

## Legal

ForzaInfinity is a fan project. It is not affiliated with, endorsed or sponsored by
Microsoft, Xbox, Playground Games or Turn 10 Studios. Forza Horizon and Xbox are trademarks of
their owners. You need your own copy of the game; nothing of the game is distributed here.
