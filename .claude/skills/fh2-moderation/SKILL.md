---
name: fh2-moderation
description: Review what ForzaInfinity players reported and decide each case with the fh2-server moderation CLI (infra/moderation.sh on the public server). Accounts flagged after 5 reports: investigate (reports, records, profile, garage, integrity strikes, log), then clear or reset their progress. Shared liveries, vinyl groups and tunes off the storefront after 3 reports: look at the preview image and details, then keep or delete. Always with a written reason. Use when Alex asks to check reports, review flagged players or reported liveries, investigate a cheater, or run a moderation round.
---

# ForzaInfinity moderation

Players report others from the game (PLAYER ACTIONS › REPORT PLAYER, e.g. on a leaderboard). A
reporter must be a real player (an account at least a day old that has played online), reports a
given player once, at most 20 a day. When five different players reported an account,
`accounts.flagged` is set. The moderator, you here, decides what happens. Code:
`server/src/db/moderation.rs`, `server/src/db/reset.rs`, `server/src/moderation_cli.rs`;
`docs/SERVER.md` › "Reports and moderation".

Talk to Alex in Portuguese (CLAUDE.md).

## There is no ban

The only punishment is a **reset**: the account's season starts again, its career stays.
- Its own cloud save is edited: XP, level, skill points, perks and credits to 0; the garage keeps
  only the first starter car it got (with its parts); track records, speed trap bests and extra
  badges go. Championships, unlocks, tutorials stay, so nobody replays the story.
- Its scores (leaderboards, speed traps), ghosts, drivatar, XP, transactions and synced
  profile/garage on the server are deleted.
- The player is told in their Message Center and in INFINITY.
- Its liveries, designs and shared files are **kept** (they have their own reports).
- The flag and its reports go; new reports start from zero.
- The game downloads the reset save at its next start and INFINITY tells the player; until then
  its uploads are refused.

A reset cannot be undone by the CLI (the server keeps the 3 previous cloud saves; restoring one is
a manual database job for Alex). So reset only with evidence, and clear when in doubt.

## Commands

Public server (from the repository root; Alex's gcloud login with IAP access, as for deploy.sh):

```sh
infra/moderation.sh list                 # flagged accounts, oldest flag first (JSON)
infra/moderation.sh show XUID            # everything about one account (JSON)
infra/moderation.sh clear XUID "why"     # nothing wrong: flag and reports go
infra/moderation.sh reset XUID "why"     # cheating: progress reset, liveries kept
```

Local or development server:

```sh
server/target/release/fh2-server moderation --root server list
server/target/release/fh2-server moderation --root server show XUID
server/target/release/fh2-server moderation --root server clear XUID --note-base64 "$(printf '%s' 'why' | base64 -w0)"
server/target/release/fh2-server moderation --root server reset XUID --note-base64 "$(printf '%s' 'why' | base64 -w0)"
```

(`--root` is the server folder: its `data/fh2online.sqlite` is the database; `--db FILE` names
another database. To try a reset without touching real players, copy the database and pass the
copy with `--db`.)

`season-reset` (every account) is not a moderation tool: never run it unless Alex asks for a new
season, and then dry-run it first (without `--yes`) and show him the count. `check-saves` is read
only: use it to see whether the save checks would refuse anyone's real save.

## A moderation round

1. `list`. For each flagged account, oldest first:
2. `show XUID` and read it all (below). Write down the evidence, with numbers.
3. Decide: **clear** (no evidence, or the reports look coordinated) or **reset** (clear evidence of
   cheating). Anything in between: clear, and say in the note what to watch.
4. Run `clear` or `reset` with a note that a person can check later: the evidence in one or two
   sentences, e.g. `reset 2535... "speed trap 13 at 138 m/s (~497 km/h) in car 1218, credits 9.9M at
   level 4 with 3 races; 2 integrity strikes (credits jump)"`.
5. Report to Alex in Portuguese: one line per account (gamertag, XUID, decision, the evidence),
   and anything that looks wrong with the system itself (a gate that should have caught it, a
   coordinated wave of reports).

## Reading `show`

- `reports`: who reported, when, why. Signs of a **coordinated** wave (clear instead): all within
  minutes, reporters who are all friends or club mates of each other, the reported player is a
  top player in a board they all race on, vague or copy-pasted reasons. Signs of a **real** case:
  reports spread over days from unrelated players, reasons that agree (same board, same car).
- `profile`: credits, level, XP, races, distance, winnings from the last accepted profile sync.
  Plausibility: credits come from races and winnings; a huge balance with few races, or level/XP
  far beyond the races and distance, is suspicious. (Storm Island and the auction house make
  money too; a big but consistent career is fine.)
- `garage` / `garageCars`: car ids. Many expensive or exclusive cars with little career is
  suspicious. The car names are in the game's `gamedbRC.slt` (Data_Car) if needed.
- `recentScores`: board (scoreboard_type; 9 = speed traps/zones), track, car, score. Speed traps
  are m/s: above ~120 m/s (430 km/h) is beyond any car in the game; race times far below the
  board's next best, or a burst of top scores within minutes, are suspicious.
- `integrity`: strikes and `last_reason` from the server's own gates (forza/gates.rs: refused
  impossible growth in credits, XP, level, top speed, race times). Strikes are strong evidence;
  none is not proof of innocence.
- `records`: how many scores, ghosts, drivatars and cloud saves the account has.
- `moderationLog`: earlier flags, clears and resets. A second offence after a reset is a reset again
  (still no ban); tell Alex about repeat offenders.

## Reported liveries, vinyl groups and tunes

The game's own Storefront › Report (Storefront.ReportContent) counts once per real player; after 3
the file leaves the storefront and waits for moderation (`removed = 3`). Its content cannot be
shared again while it waits.

```sh
infra/moderation.sh files                 # the queue: id, kind, title, description, owner, reports
infra/moderation.sh file ID               # details and who reported it when
infra/moderation.sh preview ID out.png    # its preview image (PNG): open it and look at it
infra/moderation.sh file-keep ID "why"    # fine: back on the storefront, its reports go
infra/moderation.sh file-delete ID "why"  # gone for good: never shared again
```

(Locally: `fh2-server moderation --root server files | file ID [--preview-only] | file-keep ID --note-base64 ... | file-delete ID --note-base64 ...`;
`--preview-only` prints the image as base64.)

For each file: read the title and description, and **look at the preview** (a car with the
livery). Delete it when it is hateful, sexual, harassing, a real person's private data, or a
scam/spam; keep it when it is merely ugly, a joke without a target, or a copy of a real-world
racing livery. Tunes have no picture: judge by title and description (offensive text), keep
otherwise. A deleted file does not touch its owner's account; tell Alex about an owner with
several deleted files.

## Never

- Never reset without evidence you can write in the note, and never because of the number of reports alone.
- Never touch an account's liveries or designs in a reset, and never delete an account (`clear`/`reset` only); shared files only through `file-keep`/`file-delete`.
- Never run anything but `fh2-server moderation` (and `news` with the fh2-news skill) against the
  public server's database, and never edit it by hand.
- Never post a player's data anywhere outside the conversation with Alex.
