# Installing ForzaInfinity

ForzaInfinity brings Forza Horizon 2's online services back for the PC version that runs on
**Azimuth 0.9.10** (recommended) or **0.9.9**: online free roam and races, friends, car clubs,
rivals and ghost races, leaderboards, shared liveries and tunes, and a cloud save. It installs one file into Azimuth (`data/bin/azimuth-tls.dll`)
and keeps the original as a backup; the game is never modified on disk.

## Before you start

- Your own copy of Forza Horizon 2 running on **Azimuth 0.9.10** (recommended) or **0.9.9**, on
  Windows or on Linux / Steam Deck through Proton. The hook only goes online on the Azimuth
  builds it was made for, so on another one the game plays offline. The
  installers read the version from `azimuth.exe` and stop on another one (`FORCE=1 ./install.sh`
  or `install.ps1 -Force` installs anyway).
- **After updating Azimuth, run the installer again.** An Azimuth update puts its own
  `azimuth-tls.dll` back (the game then plays offline); the installer keeps that new original as
  the backup and puts ForzaInfinity back, with your server and settings as they were. Azimuth
  0.9.9 also moves the saves to `data/titles/1A96545B/saves` on its first start (keeping a copy
  in `data/saves-backup-…`); the cloud save follows them there. 0.9.10 keeps them where 0.9.9
  put them.
- A ForzaInfinity server. The installer suggests the public one, `https://infinity.anarqz.dev`
  (50 players at a time, then a waiting line). For another server, use the address its owner
  gives you (`https://host` or `host:port`); to run one yourself, see [SERVER.md](SERVER.md).
- An **offline save**: play until the game has saved at least once (past the opening drive, with
  your first car chosen). A HARDCORE profile starts a new career with that first car; a NOOB
  profile brings the whole save as it is (see [Your profile](#your-profile)). Your offline save is
  not touched.
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
| `FH2_ExtraOptions.ini` | Yours, kept as it is. If you have none, it is created from the template. If you have one, every option you set stays, and only the options of the template it lacks are added (with their comments). `[Network]` is ForzaInfinity's: `Mode=Proxy` and `Server=<your server>`. Before the first change, your file is kept as `FH2_ExtraOptions.ini.pre-infinity`. The game does the same at startup when the INI or its `[Network]` is missing (a DLL copied by hand). |
| `forzainfinity-update.ps1` (Windows) | The updater that `ForzaInfinity-Update.cmd` runs. |
| `forzainfinity-report.ps1` (Windows) | The report that `ForzaInfinity-Report.cmd` writes. |

Next to `azimuth.exe` it leaves the updater, `ForzaInfinity-Update.cmd` (Windows) or
`ForzaInfinity-Update.sh` (Linux / Steam Deck), and the report, `ForzaInfinity-Report`. Running the installer again updates the DLL
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
2. **Create profile** and choose its type:
   - **HARDCORE**: a new career with the first car of your offline save. Every championship,
     event, road, barn find and treasure is there to win again, and the server checks that
     progress comes from play.
   - **NOOB**: your offline save comes with you as it is (cars, credits, level, progress), with
     no progress checks.

   HARDCORE and NOOB players never meet: leaderboards, rivals, ghosts, drivatars, friends,
   clubs, free roam worlds and races are kept within each type; the storefront is shared. A
   HARDCORE profile can become NOOB later, never the other way round.
3. Type a name (1 to 20 letters or digits). It reads `Name#12345`, the number being this PC's.
   In the game, names longer than 15 characters with the number show as the name alone (the
   game's gamertags hold 15 characters).
4. The game restarts on your online profile.

The pause menu shows the profile at the top: `INFINITY | CONNECTED | HC | Name#12345` (HC
HARDCORE, NB NOOB), or DISCONNECTED when the server cannot be reached. If the server does not
accept some progress, the game pauses and says so in red: nothing more is saved online until you
restart, and the next start brings back your cloud save. The game is never closed for you.

From the same screen: **Add friend** (`Name#12345`; you become friends once both added each
other), **Site key** (to sign in on another PC with *Switch profile*), **Test connection** and
**Leave Infinity** (back to your offline profile).

**Delete profile** (inside *Leave Infinity*) removes your profile from the server for good:
cloud save, stats, scores, ghosts, designs and shared files, club membership and all your
friends. Your offline save on the PC is not touched.

## Where your save is

Azimuth keeps the saves in a folder under `<Azimuth>/data`, which depends on its version:

| Azimuth | Saves folder |
|---|---|
| 0.9.10 and 0.9.9 | `data/titles/1A96545B/saves/player-1` |

`data/saves` and `data/saves-backup-…` are the copies left from before 0.9.9 moved
the saves; the game no longer reads them.

In that folder each profile has its own folder:

- `User_9000000000000` is your **offline** profile (Azimuth's own player).
- `User_<number>` is your **online** profile. The number is the profile's XUID in hexadecimal
  (not your name), so look for the `User_` folder that is not `User_9000000000000`. It appears
  the first time the game starts on the online profile.

Next to them, `Livery_*`, `BaseLivery_*`, `GarageThumbnail_*`, `LayerGroup_*` and the tune
folders are your designs, vinyl groups and tunes. Azimuth keeps them for the whole PC, and the
hook also keeps them on the server with your profile: on another PC they come back by
themselves, and one you delete in the game is deleted there too.

`fh2online-backup` holds a local save the server's replaced (when you played the
same profile on two PCs without syncing in between).

The online profile is an ordinary Azimuth save that the hook also keeps on the server.
**Leave Infinity**, **Delete profile** and uninstalling never remove anything from these folders.
Without ForzaInfinity the game plays `User_9000000000000`, the offline save as it was when you
created the profile; the online progress stays in its `User_<number>` folder. Turning it into
your offline save is not supported yet.

## Uninstall

Double click **Uninstall.cmd** (Windows) or run `./uninstall.sh` (Linux / Steam Deck). It puts
`azimuth-tls.dll.stock` back and removes the updater. Saves and `FH2_ExtraOptions.ini` are left
as they are (see [Where your save is](#where-your-save-is)).

## Troubleshooting

Something wrong? Run **ForzaInfinity-Report** next to `azimuth.exe` (`.cmd` on Windows, `.sh`
on Linux / Steam Deck). It writes `ForzaInfinity-Report.txt` on your Desktop with the last
launch (Azimuth and hook versions, the hook's logs of the last launch and the one before, your
settings, the last requests), without your token or keys, to send to whoever helps you. It
sends nothing by itself.

| Symptom | Look at |
|---|---|
| INFINITY says it cannot reach the server | The address in `FH2_ExtraOptions.ini` `[Network] Server=`, and that the server's port is open to you. |
| Nothing online happens | `data/bin/FH2_ExtraOptions.log`: it should start with the ForzaInfinity version and show `NETWORK:` and `ACCOUNT:` lines. |
| The game stops at start on Linux | `data/logs/azimuth.log`. `CreateDXGIFactory2` means the Vulkan driver is too old for Proton 11. |
| `winmm.dll` and `FH2_ExtraOptions.ini` next to `azimuth.exe` | The standalone ExtraOptions (0.2.88 and later installs there). ForzaInfinity already includes ExtraOptions, so the installer and the game turn that copy off (`*.extraoptions-off`); your settings are in `data/bin/FH2_ExtraOptions.ini`. |
| A controller does nothing on Linux | Azimuth reads the first controller only. Check that it shows up as a gamepad (`/dev/input/js0`). |

**Online free roam and races** (worlds the players host, 8 drivers by default, up to 12): see
[FREEROAM.md](FREEROAM.md). Not available yet: Online Road Trip.
