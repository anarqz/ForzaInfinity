# ForzaInfinity

Forza Horizon 2's online services, back for the PC version running on **Azimuth 0.9.7**
(Windows, or Linux and the Steam Deck through Proton).

- **Friends**: add each other by `Name#12345` from the game, see who is online.
- **Car Clubs**: create, invite, roles, notice board.
- **Rivals and ghost races**: every friend's best time on each route is a time to beat, with
  their ghost.
- **Leaderboards**: speed traps, speed zones and skill chains among friends.
- **Drivatars**: your friends race you and park at car meets, in their cars' colours.
- **Storefront**: share designs, vinyl groups and tunes; browse, download, like; three
  strikes from different players take a file down.
- **Cloud save**: your profile starts from your offline progress and follows you to another
  PC or the Deck.

Live multiplayer (Online Road Trip, online free roam) is planned for a later version; see
[MILESTONES.md](MILESTONES.md) (M7).

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
[docs/INSTALL.md](docs/INSTALL.md).

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
