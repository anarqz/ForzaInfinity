#!/bin/bash
# Builds the server here and installs it on the VM through IAP (no open ports needed):
# bin/fh2-server, responses/ and, when found, the game's car database (gamedbRC.slt, for car
# and route names and the integrity checks). The database on the VM is left alone.
#   ./deploy.sh [--gamedb PATH]
set -euo pipefail
cd "$(dirname "$0")"
ROOT=$(cd .. && pwd)
PROJECT=${PROJECT:-forza-infinity-online}
ZONE=${ZONE:-us-east1-b}
VM=infinity
GAMEDB="$ROOT/Forza Horizon 2/Anthem_1.0.0.64_x64__8wekyb3d8bbwe/Mount/media/stripped/gamedbRC.slt"
if [ "${1:-}" = "--gamedb" ]; then GAMEDB=$2; fi

(cd "$ROOT/server" && PATH="$HOME/.cargo/bin:$PATH" cargo build --release)

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
cp "$ROOT/server/target/release/fh2-server" "$STAGE/"
tar -C "$ROOT/server" -czf "$STAGE/responses.tgz" responses
[ -f "$GAMEDB" ] && cp "$GAMEDB" "$STAGE/gamedbRC.slt"

gcloud compute ssh "$VM" --project "$PROJECT" --zone "$ZONE" --tunnel-through-iap --command 'rm -rf /tmp/infinity-deploy && mkdir -p /tmp/infinity-deploy'
gcloud compute scp --project "$PROJECT" --zone "$ZONE" --tunnel-through-iap "$STAGE"/* "$VM:/tmp/infinity-deploy/"
gcloud compute ssh "$VM" --project "$PROJECT" --zone "$ZONE" --tunnel-through-iap --command '
  set -e
  cd /tmp/infinity-deploy
  sudo install -m 0755 fh2-server /opt/infinity/bin/fh2-server
  sudo rm -rf /opt/infinity/responses && sudo tar -C /opt/infinity -xzf responses.tgz
  [ -f gamedbRC.slt ] && sudo install -m 0644 gamedbRC.slt /opt/infinity/gamedbRC.slt
  # free roam (startup.sh has it from the next boot on; this is for the VM as it runs now)
  if ! grep -q -- "--freeroam" /etc/systemd/system/fh2-server.service; then
    sudo sed -i "s|^ExecStart=/opt/infinity/bin/fh2-server .*|& --freeroam|" /etc/systemd/system/fh2-server.service
    sudo systemctl daemon-reload
  fi
  sudo systemctl restart fh2-server
  sleep 2
  systemctl is-active fh2-server cloudflared-infinity
  curl -s http://127.0.0.1:8081/infinity/v1/info; echo
  rm -rf /tmp/infinity-deploy'
