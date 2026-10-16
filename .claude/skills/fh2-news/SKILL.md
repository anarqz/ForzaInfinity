---
name: fh2-news
description: Send, list or take back ForzaInfinity admin news, the messages every player (or one player) sees in Forza Horizon 2's in-game Message Center, through the fh2-server news CLI (infra/news.sh for the public server). Use when Alex asks to announce something to the players, write a release/maintenance note, message one player, or undo a message.
---

# ForzaInfinity news (in-game Message Center)

News are written straight into the server's database by `fh2-server news` (`server/src/news_cli.rs`,
`server/src/db/news.rs`). There is no HTTP route for them. On the public server they go through
`infra/news.sh`, which runs the CLI on the VM over IAP SSH with Alex's own gcloud login.

Talk to Alex in Portuguese (CLAUDE.md).

## Commands

Public server (from the repository root):

```sh
infra/news.sh send "Text" [--to Name#12345]   # dry run first: shows the text and how many players get it, then asks
infra/news.sh send -f message.txt [--to ...]  # the text from a file (line breaks kept)
infra/news.sh send "Text" --yes               # no question: only after Alex said to send this exact text
infra/news.sh list                            # past sends, with their ID
infra/news.sh delete ID                       # takes a send back from every Message Center
```

A local or development server (the database is the server's own file; the CLI never creates one):

```sh
server/target/release/fh2-server news --db server/data/fh2online.sqlite send --text "Text" [--to Name#12345] [--dry-run]
server/target/release/fh2-server news --db server/data/fh2online.sqlite list
server/target/release/fh2-server news --db server/data/fh2online.sqlite delete ID
```

## Rules

- **Sending is outward-facing and reaches every player.** Always run the dry run (plain
  `infra/news.sh send` without `--yes` shows it and stops at the question; or `--dry-run` on the
  CLI), show Alex the exact text and the player count, and send only once Alex approves that text.
  `--yes` is for that second step only.
- At most 512 characters. Line breaks are kept (real ones or `\n` typed).
- One text for everyone: the game does not pick a language. Most players are Brazilian; for
  announcements to everyone write Portuguese first, then English, separated by a blank line, unless
  Alex says otherwise.
- `--to` takes a gamertag `Name#12345` (letters/digits then `#` and 5 digits).
- No secrets, tokens, keys, IPs or other players' data in a news text.
- A wrong send is taken back with `delete ID` (from `list`); say so to Alex.

## Writing a good note

- What changed or what to do, in one or two short sentences; a link only to the project's own pages
  (`https://anarqz.github.io/ForzaInfinity/`, `https://github.com/anarqz/ForzaInfinity`).
- Releases: the version, the headline feature, "updates install by themselves; restart the game
  when it asks".
- Maintenance: when, how long, what players will see (offline play keeps working).
