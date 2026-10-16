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
| `/infinity/v1/info` | anyone | `{ "name": "ForzaInfinity", "api": 1, "freeroam": true, … }` |
| `/infinity/v1/worlds`, `worlds/of/<xuid>`, `worlds/<id>/join` | the hook | Player-hosted free roam worlds (below), with free roam on. |
| `/infinity/v1/stats` | anyone (CORS open) | Server-wide totals for the landing page, cached 30 s. |
| `/ui/`, `/api/`, `/ws` | this machine only | The manager (web/): players, friends, clubs, live feed. |

Hooks before fh2online.33 call `/fh2online/v1/…`; the server answers both.

## Free roam online (F0)

Desligado por padrão: sem as opções abaixo, `gameserverds`, `gameserverms` e `sessiondirectory`
continuam respondendo pelos stubs de `responses/hosts/`. Ligado, o servidor responde
(`server/src/freeroam.rs`, portado do `fh2srv.py` do fh2-mods), só para quem tem conta:

| Chamada | Resposta |
|---|---|
| `POST gameserverds …/titles/1368890819/variants` | Uma variant `Release_6CC152FD.FreeRoam`, todos os campos como string JSON. |
| `GET gameserverds /qosservers` | Exatamente uma chave `qosServers`, com um servidor. |
| `POST gameserverms …/clusters` | `Fulfilled` na hora, com o endereço do mundo. |
| `sessiondirectory /serviceconfigs/…/sessions/<nome>` | MPSD (contrato 105) em memória: PUT mescla (201 ao criar, 200 depois), GET, DELETE (204). `me` vira o XUID da conta. |
| `GET sessiondirectory …/sessions` (listagem) | Sempre `{"results":[]}`: listar sessões velhas fazia o jogo tentar reentrar nelas. |

Quando a sessão tem `constants.custom.gameServerPlatform`, o servidor preenche
`servers.gameServerPlatform.properties.custom` com `status: "Active"`, `datacenter`,
`secureDeviceAddress` e `hostName`. O `secureDeviceAddress` é o base64 do texto
`fh2inf:<ip>:<porta>` (IPv6 entre colchetes: `fh2inf:[2001:db8::1]:5101`), que o hook decodifica.

```sh
fh2-server … --freeroam                            # liga: só mundos hospedados pelos jogadores
fh2-server … --freeroam-world 192.168.68.40:5101   # liga, e quem não tem mundo hospedado vai para esse relay
```

Sessões paradas por 30 min somem; no máximo 512 em memória e 128 KB cada. Nada vai para o banco.
Um ticket do SmartMatch (`momatch`) sem nenhum mundo para o jogador recebe 503 (o jogo mostra o erro
de conexão).

## Mundos hospedados pelos jogadores (F1)

Com o free roam ligado (`--freeroam` ou `--freeroam-world`), o `/infinity/v1/info` diz
`"freeroam": true` e o hook pode anunciar mundos (`server/src/worlds.rs`). Todas as rotas exigem o
`X-Fh2-Token` (401 sem conta, 403 conta suspensa, 404 com o free roam desligado) e respondem
também em `/fh2online/v1/`. O IP do jogador é o da conexão ou, atrás do proxy (`--behind-proxy`),
o `CF-Connecting-IP` / último `X-Forwarded-For`, como nos limites.

| Rota | Corpo | Resposta |
|---|---|---|
| `POST worlds` | `{"name" (1–40), "message" (≤120), "visibility": "public"\|"friends"\|"off", "password" (≤32, "" = sem), "maxPlayers": 1–12, "port", "lan": [≤4 IPs], "version" (≤40), "banned"?: [xuids]}` | `{"id": "<xuid do anfitrião>", "address": "<ip>:<porta>", "players", "expiresIn": 45}`; 400 se inválido. Inicia ou renova (heartbeat a cada 15 s); um mundo por conta. |
| `DELETE worlds` | — | 204. O mundo acaba e os jogadores dele são liberados. |
| `GET worlds` | — | `{"worlds": [{"id", "name", "message", "host", "players", "maxPlayers", "friendsOnly", "sameNetwork"}]}`: públicos, ou só-amigos de um amigo; sem senha, sem `off`, sem quem baniu o jogador. |
| `GET worlds/of/<xuid>` | — | `{"world": null}` ou `{"world": {"id", "name", "host", "players", "maxPlayers", "password": bool, "canJoin": bool, "why": ""\|"full"\|"friends"\|"off"\|"banned"}}`: o mundo que esse jogador hospeda ou em que está, avaliado para quem pergunta. |
| `POST worlds/<id>/join` | `{"password"}` | `{"ok": true}`: o próximo FREE ROAM ONLINE do jogador vai para esse mundo (vale 10 min). 404 mundo desconhecido; 403 `{"error", "why"}` com `why` como acima ou `"password"` (`"error": "wrong password"`); 429 depois de 5 senhas erradas em 10 min. |

Para onde cada jogador vai (`world_for`, usado na sessão, no SmartMatch e no cluster), nesta ordem:
o mundo que ele hospeda (`127.0.0.1:<porta>`, o relay está no mesmo PC); o que escolheu pelo
`join`; aquele em que já está (por 30 min desde o último ticket); o melhor mundo aberto para ele
(sem senha, com vaga, visível): primeiro o do mesmo IP público, depois o com mais jogadores,
depois o heartbeat mais recente; por fim o `--freeroam-world`. Quem tem o mesmo IP público do
anfitrião recebe o primeiro endereço de `lan`; os outros, o IP público do anfitrião.

Jogadores = anfitrião + quem foi mandado ao mundo por um ticket. Um mundo sem heartbeat por 45 s
some. Tudo fica em memória (no máximo 256 mundos); a senha só como SHA-1 com sal próprio do
mundo, e nunca vai para o log.

## Limits

- Any request body: 2 MB. Account requests: 4 KB.
- Cloud saves: 512 KB per upload (a real one is about 28 KB), 8 MB per file and 16 MB in all
  once unpacked, only the seven file names of an Azimuth save, 3 uploads kept per player, at
  most one upload every 30 s per address after a burst of 5.
- Rates per address (token buckets): game traffic, account calls, account creation (3 an
  hour), sign-ins (slow on purpose).
- Storefront: 100 shared files per player; after reports from 3 different real players a file
  leaves the storefront and waits for moderation (`fh2-server moderation files`).
- Profile syncs are checked for impossible growth (credits, XP, level, top speed, race times);
  a refused sync is not stored and counts as a strike on the profile.

## Only the game's own cars

Every request of the hook (fh2online.46 and later) carries `X-Fh2-CarDb`, the SHA-256 of the
player's `media/stripped/gamedbRC.slt`. An account whose header is missing, says `changed` (the
file changed while the game ran) or names another file is refused everything but `/info`,
`/stats` and deleting itself: the game plays offline and INFINITY says why. The accepted hash is
the server's own `--gamedb` file; `--cardb-allow <sha-256>` adds others, `--cardb-gate off` turns
the check off. The header is only as honest as the client: it keeps modified databases out of the
official hook, the profile gates and the players' reports are for the rest.

## Reports and moderation

PLAYER ACTIONS › REPORT PLAYER (on a leaderboard, a club or the friends list) sends
`POST /infinity/v1/report {xuid}`: a player reports another once, 20 a day; only an account at
least a day old that has played online (its profile synced) may report. When five
different players reported an account, `accounts.flagged` (and `flagged_at`) is set. Nobody is
banned by reports; the moderator (a person, or an agent they run) decides, on the server's
machine:

```sh
fh2-server moderation --root /opt/infinity list            # flagged accounts, oldest first (JSON)
fh2-server moderation --root /opt/infinity show XUID       # reports, records, profile, garage, gates, log
fh2-server moderation --root /opt/infinity clear XUID why  # nothing wrong: the flag and reports go
fh2-server moderation --root /opt/infinity reset XUID why  # cheating: the account's season starts again
```

A reset edits the player's own cloud save (`server/src/db/reset.rs`), so the career stays (story,
championships, events, unlocks, tutorials, barn finds, bucket list). XP (ProfileData `XP` and its
copy `WristbandLevel`), level, skill points, perks and credits go to 0; the garage keeps only the
first starter car the player got, with its parts; track records, speed trap/zone bests and badges
beyond the 51 every profile has go. On the server the scores, ghosts, XP, transactions and synced
profile and garage are deleted (a moderation reset also deletes the drivatar, the flag and the
reports). Liveries and shared creations stay. The gates start again from 0, uploads are refused
until the game has downloaded the reset save, and the player is told in their Message Center and
in INFINITY. Everything is in `moderation_log`.

## Seasons and the cloud save

- `fh2-server moderation season-reset [--yes] [--only XUID] [why]` resets every account like that
  (no drivatar deleted); without `--yes` it only counts; with it, the database is first copied to
  `<db>.before-season-<ms>` (`VACUUM INTO`).
- **A new profile starts a new career.** Its first save (the hook sends the offline one) is not
  kept: the server stores the starting save of the starter car it shows (`server/fixtures/
  starter-<car>.bundle`, built into the binary: the game just past its first championship) with
  XP, level and credits at 0, so every championship, event, road, barn find and treasure is there
  to win, with the XP and credits they give. `fh2-server moderation career-reset [--yes] [--only
  XUID] [why]` does the same to existing accounts (their starter car kept; records and server
  progress as in a season reset; liveries and creations stay; backup `<db>.before-career-<ms>`);
  the player is told in the Message Center and in INFINITY.
- **The server's save is the online profile.** At the start, a local save newer than the last one
  synced (`SaveCommit`), with the server still on that one, is the last session's own progress
  (revisions after the hook's last one-minute look, or written as the game closed): the hook
  uploads it, and the save gates measure it. Any other local save that differs (edited, played
  without the server, or one the server refused) is kept in `saves\fh2online-backup` and the
  server's is downloaded over it. A session that starts without the server forgets `SaveCommit`,
  so nothing it saves is uploaded, not even at the next start.
- **Save gates** (`PUT /infinity/v1/save`): only a real save's shape (per-file sizes of real
  saves, the files every save has, each file matching its commit line, PlayerDatabase a SQLite
  database, SaveState XML, at most 384 KB packed), and progress since the server's last save
  within the profile gates' allowances per hour (credits, XP, level, cars). A refused upload is a
  strike, and the server's save is restored at the next start (`GET save/info` says `restore`).
- `fh2-server moderation check-saves` runs the save checks over every account's save, read only.

Shared files (liveries, vinyl groups, tunes) reported with the game's own Storefront › Report:

```sh
fh2-server moderation --root /opt/infinity files                       # the queue
fh2-server moderation --root /opt/infinity file ID [--preview-only]    # details (the preview PNG as base64)
fh2-server moderation --root /opt/infinity file-keep ID why            # back on the storefront
fh2-server moderation --root /opt/infinity file-delete ID why          # gone for good
```

`infra/moderation.sh` runs all of these on the public server (`preview ID OUT` saves the image);
`.claude/skills/fh2-moderation` and `.claude/skills/fh2-news` teach Claude to use them.

## Privacy

The server is one open world: every player sees everyone's gamertag, times and positions,
ghosts, drivatar, car club and shared files (banned accounts are left out). Friends are the
social side: the friends list and presence, messages, challenges and Meus Rivais, and they come
first in drivatar grids and as rivals. Hidden clubs (privacy 3) are seen only by their members
and the members' friends. `/infinity/v1/stats` only gives totals. A player deleting their profile from the game removes
every record of them, including friendships and shared files.
