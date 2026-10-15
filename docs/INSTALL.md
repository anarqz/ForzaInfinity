# Installing ForzaInfinity

ForzaInfinity brings Forza Horizon 2's online services back for the PC version that runs on
**Azimuth 0.9.9 or 0.9.7**: friends, car clubs, rivals and ghost races, leaderboards, shared liveries
and tunes, and a cloud save. It installs one file into Azimuth (`data/bin/azimuth-tls.dll`)
and keeps the original as a backup; the game is never modified on disk.

## Before you start

- Your own copy of Forza Horizon 2 running on **Azimuth 0.9.9** (the latest) or **0.9.7**, on
  Windows or on Linux / Steam Deck through Proton. **0.9.8 is not supported**: the hook only goes
  online on the Azimuth builds it was made for, so on another one the game plays offline. The
  installers read the version from `azimuth.exe` and stop on another one (`FORCE=1 ./install.sh`
  or `install.ps1 -Force` installs anyway).
- **After updating Azimuth, run the installer again.** An Azimuth update puts its own
  `azimuth-tls.dll` back (the game then plays offline); the installer keeps that new original as
  the backup and puts ForzaInfinity back, with your server and settings as they were. Azimuth
  0.9.9 also moves the saves to `data/titles/1A96545B/saves` on its first start (keeping a copy
  in `data/saves-backup-…`); the cloud save follows them there.
- A ForzaInfinity server. The installer suggests the public one, `https://infinity.anarqz.dev`
  (150 players at a time, then a waiting line). For another server, use the address its owner
  gives you (`https://host` or `host:port`); to run one yourself, see [SERVER.md](SERVER.md).
- An **offline save**: play until the game has saved at least once. Your online profile is
  created from it, so your cars, credits and level come with you.
- The game closed while you install.

## Windows

1. Download **ForzaInfinity.zip** from the [latest release](https://github.com/anarqz/ForzaInfinity/releases/latest)
   and unzip it anywhere (next to `azimuth.exe` is easiest).
2. Double click **Install.cmd**. It finds Azimuth (next to the installer, in Downloads,
   Desktop, Documents or a `Games` folder), or asks you to pick the folder with `azimuth.exe`.
3. Type the server address when asked. Done.

Or, in PowerShell: `irm https://github.com/anarqz/ForzaInfinity/releases/latest/download/install.ps1 | iex`

## Steam Deck and Linux

Azimuth runs under Proton as a non-Steam game:

1. In Steam, *Add a Non-Steam Game* and pick `azimuth.exe`.
2. In its *Properties › Compatibility*, force **Proton 11.0**.
3. Desktop PCs only: Proton 11 needs a Vulkan driver with `VK_KHR_maintenance5`
   (NVIDIA 550 or newer, or a recent Mesa). With an older driver Azimuth stops at
   `CreateDXGIFactory2`.

Then install ForzaInfinity (on the Deck, from desktop mode):

```sh
curl -fsSL https://github.com/anarqz/ForzaInfinity/releases/latest/download/install.sh | bash
```

or unzip ForzaInfinity.zip and run `./install.sh`. It looks for `azimuth.exe` in your home
folder, SD cards and drives (`/run/media`, `/media`, `/mnt`), asks which one when there are
several, and asks for the server address. Both take arguments to skip the questions:
`./install.sh "<folder with azimuth.exe>" https://infinity.anarqz.dev`.

## What the installer changes

In `<Azimuth>/data/bin`:

| File | Change |
|---|---|
| `azimuth-tls.dll` | Replaced by ForzaInfinity's. The original is kept as `azimuth-tls.dll.stock` (only the first time). |
| `FH2_ExtraOptions.ini` | Created from the template, or, if you have one, only `[Network]` changes: `Mode=Proxy` and `Server=<your server>`. Everything else stays. |
| `forzainfinity-update.ps1` (Windows) | The updater that `ForzaInfinity-Update.cmd` runs. |

Next to `azimuth.exe` it leaves the updater: `ForzaInfinity-Update.cmd` (Windows) or
`ForzaInfinity-Update.sh` (Linux / Steam Deck). Running the installer again updates the DLL
and the server address.

## Updates

ForzaInfinity updates itself. The server says which version is the newest; when the game
starts with an older one, the hook downloads that release's `azimuth-tls.dll` from this
repository's GitHub releases (never from the game server), checks it against the release's
SHA-256, and puts it in place. The pause menu then reads *UPDATE INSTALLED · RESTART THE
GAME*, and **Settings › INFINITY** offers to restart. The **UPDATE** tile there shows the state
at any time.

By hand (or with `AutoUpdate=0` under `[Network]` in `FH2_ExtraOptions.ini`, which turns the
self-update off): close the game and run `ForzaInfinity-Update` next to `azimuth.exe`. It
installs the latest release and keeps your server and settings.

## Your profile

1. Start the game and open **Settings › INFINITY**.
2. **Create profile** and type a name (1 to 9 letters or digits). It reads `Name#12345`, the
   number being this PC's.
3. The game restarts on your online profile, which goes on from your newest offline save.

From the same screen: **Add friend** (`Name#12345`; you become friends once both added each
other), **Site key** (to sign in on another PC with *Switch profile*), **Test connection** and
**Leave Infinity** (back to your offline profile).

**Delete profile** (inside *Leave Infinity*) removes your profile from the server for good:
cloud save, stats, scores, ghosts, designs and shared files, club membership and all your
friends. Your offline save on the PC is not touched.

## Uninstall

Double click **Uninstall.cmd** (Windows) or run `./uninstall.sh` (Linux / Steam Deck). It puts
`azimuth-tls.dll.stock` back and removes the updater. Saves and `FH2_ExtraOptions.ini` are left
as they are.

## Troubleshooting

| Symptom | Look at |
|---|---|
| INFINITY says it cannot reach the server | The address in `FH2_ExtraOptions.ini` `[Network] Server=`, and that the server's port is open to you. |
| Nothing online happens | `data/bin/FH2_ExtraOptions.log`: it should start with the ForzaInfinity version and show `NETWORK:` and `ACCOUNT:` lines. |
| The game stops at start on Linux | `data/logs/azimuth.log`. `CreateDXGIFactory2` means the Vulkan driver is too old for Proton 11. |
| A controller does nothing on Linux | Azimuth reads the first controller only. Check that it shows up as a gamepad (`/dev/input/js0`). |

Not available yet: live multiplayer (Online Road Trip, online free roam). Layered designs of
other players show in their car's factory colour for now.
