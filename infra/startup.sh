#!/bin/bash
# Runs as root on every boot of the VM (Terraform template: only backup_bucket and token_secret
# are filled in; every other dollar sign is the shell's). Safe to run again.
set -euo pipefail

# --- packages: sqlite3 for backups, cloudflared for the tunnel
if ! command -v cloudflared >/dev/null; then
  install -d -m 0755 /usr/share/keyrings
  curl -fsSL https://pkg.cloudflare.com/cloudflare-main.gpg -o /usr/share/keyrings/cloudflare-main.gpg
  echo "deb [signed-by=/usr/share/keyrings/cloudflare-main.gpg] https://pkg.cloudflare.com/cloudflared bookworm main" \
    > /etc/apt/sources.list.d/cloudflared.list
  apt-get update -q
  DEBIAN_FRONTEND=noninteractive apt-get install -y -q cloudflared sqlite3 unattended-upgrades
fi

# --- 1 GB of swap: an e2-micro has 1 GB of memory
if [ ! -f /swapfile ]; then
  fallocate -l 1G /swapfile && chmod 600 /swapfile && mkswap /swapfile
  echo '/swapfile none swap sw 0 0' >> /etc/fstab
fi
swapon -a || true

# --- the server: deploy.sh puts bin/fh2-server, responses/ and gamedbRC.slt in /opt/infinity
id infinity >/dev/null 2>&1 || useradd --system --home /opt/infinity --shell /usr/sbin/nologin infinity
install -d -o root -g root -m 0755 /opt/infinity /opt/infinity/bin /opt/infinity/responses
install -d -m 0755 /opt/infinity/data
# the database disk (Terraform's infinity-data): formatted once, mounted on every boot
DATA_DEV=/dev/disk/by-id/google-infinity-data
if ! blkid "$DATA_DEV" >/dev/null 2>&1; then
  mkfs.ext4 -q -L infinity-data -m 0 "$DATA_DEV"
fi
grep -q 'LABEL=infinity-data' /etc/fstab || echo 'LABEL=infinity-data /opt/infinity/data ext4 defaults,nofail,discard 0 2' >> /etc/fstab
mountpoint -q /opt/infinity/data || mount /opt/infinity/data
chown infinity:infinity /opt/infinity/data
chmod 0750 /opt/infinity/data

cat > /etc/systemd/system/fh2-server.service <<'UNIT'
[Unit]
Description=ForzaInfinity server
After=network-online.target
Wants=network-online.target
ConditionPathExists=/opt/infinity/bin/fh2-server
RequiresMountsFor=/opt/infinity/data

[Service]
User=infinity
Group=infinity
WorkingDirectory=/opt/infinity
# Only cloudflared (on loopback) reaches it. --registration closed turns new profiles away.
ExecStart=/opt/infinity/bin/fh2-server --listen 127.0.0.1:8081 --root /opt/infinity --behind-proxy --gamedb /opt/infinity/gamedbRC.slt
Restart=always
RestartSec=3
LimitNOFILE=4096
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
ReadWritePaths=/opt/infinity/data

[Install]
WantedBy=multi-user.target
UNIT

# --- the tunnel: its token comes from Secret Manager, readable only by this VM's account
install -d -m 0700 /etc/cloudflared
TOKEN=$(gcloud secrets versions access latest --secret="${token_secret}" --quiet)
umask 077
printf 'TUNNEL_TOKEN=%s\n' "$TOKEN" > /etc/cloudflared/tunnel.env
unset TOKEN
umask 022

cat > /etc/systemd/system/cloudflared-infinity.service <<'UNIT'
[Unit]
Description=Cloudflare Tunnel for ForzaInfinity
After=network-online.target
Wants=network-online.target

[Service]
EnvironmentFile=/etc/cloudflared/tunnel.env
ExecStart=/usr/bin/cloudflared --no-autoupdate tunnel run
Restart=always
RestartSec=5
DynamicUser=yes
NoNewPrivileges=yes

[Install]
WantedBy=multi-user.target
UNIT

# --- a daily backup of the database to the bucket (kept there by its lifecycle rule)
cat > /usr/local/bin/infinity-backup <<'SCRIPT'
#!/bin/bash
set -euo pipefail
DB=/opt/infinity/data/fh2online.sqlite
[ -f "$DB" ] || exit 0
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT
NAME="fh2online-$(date -u +%Y%m%d-%H%M%S).sqlite"
sqlite3 "$DB" ".timeout 30000" ".backup '$OUT/$NAME'"
gzip -9 "$OUT/$NAME"
# a plain upload that never overwrites (ifGenerationMatch=0): the VM may create objects only,
# and gcloud storage cp wants to read them too
TOKEN=$(curl -fsS -H 'Metadata-Flavor: Google' \
  http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token |
  sed -E 's/.*"access_token":"([^"]+)".*/\1/')
curl -fsS -o /dev/null -X POST -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/gzip' \
  --data-binary @"$OUT/$NAME.gz" \
  "https://storage.googleapis.com/upload/storage/v1/b/BUCKET/o?uploadType=media&ifGenerationMatch=0&name=$NAME.gz"
SCRIPT
sed -i "s|BUCKET|${backup_bucket}|" /usr/local/bin/infinity-backup
chmod 0755 /usr/local/bin/infinity-backup

cat > /etc/systemd/system/infinity-backup.service <<'UNIT'
[Unit]
Description=Back up the ForzaInfinity database

[Service]
Type=oneshot
ExecStart=/usr/local/bin/infinity-backup
UNIT

cat > /etc/systemd/system/infinity-backup.timer <<'UNIT'
[Unit]
Description=Daily ForzaInfinity backup

[Timer]
OnCalendar=*-*-* 06:00:00 UTC
Persistent=true

[Install]
WantedBy=timers.target
UNIT

systemctl daemon-reload
systemctl enable --now cloudflared-infinity.service infinity-backup.timer
systemctl enable fh2-server.service
systemctl restart fh2-server.service || true # waits for deploy.sh on the first boot
