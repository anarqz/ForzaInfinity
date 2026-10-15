# infra

The public server, `https://infinity.anarqz.dev`, on GCP (`forza-infinity-online`, us-east1).

```
player ──HTTPS──▶ Cloudflare (DDoS, rate limit) ──tunnel──▶ cloudflared ─▶ 127.0.0.1:8081 fh2-server
                                                            └──────── e2-micro VM, no open ports ┘
```

- **VM** `infinity`: e2-micro, Debian 12, 20 GB boot disk. The firewall only lets SSH in, and only
  through IAP. `startup.sh` runs on every boot: cloudflared, the `fh2-server` unit
  (`--behind-proxy`, hardened), 1 GB swap and the backup timer.
- **Database**: SQLite on its own disk, `infinity-data` (10 GB, `prevent_destroy`), mounted
  at `/opt/infinity/data`. Rebooting, deploying or recreating the VM keeps it. Copies:
  - a daily disk snapshot, kept 7 days;
  - a daily `.backup` at 06:00 UTC to `gs://forza-infinity-online-backups`, kept 30 days.
    The VM can only create objects there, not read or delete them.
- **Tunnel token**: Secret Manager (`infinity-tunnel-token`), readable only by the VM's
  service account.
- **Cloudflare** (`anarqz.dev`): tunnel `forza-infinity`, proxied CNAME `infinity`, and one
  rate-limit rule: 20 requests per 10 s per IP on register, login, save and the legacy routes.

## Use

```sh
./tf.sh plan -out=infinity.tfplan     # credentials: the gcloud login + ../.env (Cloudflare)
./tf.sh apply infinity.tfplan
./deploy.sh                           # builds server/ here, installs it on the VM through IAP
gcloud compute ssh infinity --zone us-east1-b --tunnel-through-iap   # a shell on the VM
```

On the VM:
- `journalctl -u fh2-server -f` follows the server log;
- `sudo systemctl start infinity-backup` makes a backup now;
- `sudo systemctl edit fh2-server` changes the flags, for example `--registration closed`.

The Terraform state (`*.tfstate`, git-ignored) holds the tunnel token. Keep it private.

## Restore a backup

```sh
gcloud storage cp gs://forza-infinity-online-backups/<file>.sqlite.gz . && gunzip <file>.sqlite.gz
gcloud compute scp --tunnel-through-iap --zone us-east1-b <file>.sqlite infinity:/tmp/
# on the VM:
sudo systemctl stop fh2-server
sudo install -o infinity -g infinity -m 0640 /tmp/<file>.sqlite /opt/infinity/data/fh2online.sqlite
sudo rm -f /opt/infinity/data/fh2online.sqlite-wal /opt/infinity/data/fh2online.sqlite-shm
sudo systemctl start fh2-server
```

Reading the bucket needs your own account (the VM's cannot).

## Admin news

`./news.sh` puts a message from ForzaInfinity in every player's in-game Message Center (the
same Message notification the server sends when players become friends):

```sh
./news.sh send "Leaderboards are now open to every player!"   # shows text and player count, asks first
./news.sh send -f news.txt --to Name#12345                      # one player; text from a file
./news.sh list                                                   # past sends
./news.sh delete 3                                               # takes send #3 back
```

There is no HTTP route for it, so nobody can send news through the server. The script runs
`fh2-server news` on the VM over IAP SSH, as the `infinity` user, against the live database;
what lets you in is your own gcloud login with access to the project, and nothing secret is
in this repository. Players see a news on their next Message Center poll (within 5 minutes).
At most 512 characters; line breaks are real ones or `\n`.
