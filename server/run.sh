#!/bin/sh
# Runs the server listening on the network (LAN play). The log shows here and is also kept in
# logs/server-<date>-<time>.log, so it can be read while (or after) the server runs.
cd "$(dirname "$0")" || exit 1
if [ ! -x target/release/fh2-server ]; then echo "Rode cargo build --release primeiro."; exit 1; fi
mkdir -p logs
log="logs/server-$(date +%Y%m%d-%H%M%S).log"
echo "log: $log"
# The game's car database, in this PC's layout (the game next to the repo's Forza Horizon 2/).
gamedb="../Forza Horizon 2/Anthem_1.0.0.64_x64__8wekyb3d8bbwe/Mount/media/stripped/gamedbRC.slt"
if [ -f "$gamedb" ]; then set -- --gamedb "$gamedb"; fi
./target/release/fh2-server --listen 0.0.0.0:8081 --root . --capture "$@" 2>&1 | tee -a "$log"
