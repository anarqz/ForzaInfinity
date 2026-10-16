#!/usr/bin/env bash
# ForzaInfinity report for Linux and the Steam Deck: puts what the last launch of the game left
# (versions, settings without tokens, the hook's logs, the last requests) in one file,
# ForzaInfinity-Report.txt on the Desktop (or in home), to send to whoever helps you. Nothing
# is sent anywhere. The installer leaves a copy in the Azimuth folder as ForzaInfinity-Report.sh.
#   ./report.sh [<folder with azimuth.exe>]
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd || pwd)"
AZIMUTH="${1:-$HERE}"
if [ ! -f "$AZIMUTH/azimuth.exe" ] && [ -f "$(dirname "$AZIMUTH")/azimuth.exe" ]; then AZIMUTH="$(dirname "$AZIMUTH")"; fi
BIN="$AZIMUTH/data/bin"
OUT_DIR="$HOME/Desktop"; [ -d "$OUT_DIR" ] || OUT_DIR="$HOME"
OUT="$OUT_DIR/ForzaInfinity-Report.txt"

section() { printf '\n===== %s =====\n' "$1"; }
# a text file without its tokens, keys and passwords
clean() { if [ -f "$1" ]; then tr -d '\r' < "$1" | sed -E 's/^([[:space:]]*(Token|Key|WebKey|Password)[[:space:]]*=).*/\1 (removed)/I'; else echo "(missing)"; fi; }
version() {
    LC_ALL=C grep -aoP '(?<![0-9.]\x00)0\x00\.\x00[0-9]+\x00\.\x00[0-9]+\x00(?!\.)' "$1" 2>/dev/null | tr -d '\0' | sort | uniq -c | sort -rn | awk 'NR == 1 { print $2 }'
}

{
    echo "ForzaInfinity report"
    echo "Made:     $(date -u '+%Y-%m-%d %H:%M:%S UTC')"
    echo "System:   $(uname -srm) $( [ -f /etc/os-release ] && . /etc/os-release && echo "($PRETTY_NAME)")"
    echo "Folder:   $AZIMUTH"
    if [ ! -f "$AZIMUTH/azimuth.exe" ]; then echo "azimuth.exe was not found here: run this from the Azimuth folder, or give it as the first argument."; fi
    echo "Azimuth:  $(version "$AZIMUTH/azimuth.exe")"
    if [ -f "$BIN/azimuth-tls.dll" ]; then
        echo "Hook:     $(LC_ALL=C grep -aoE '0\.2\.[0-9]+\+fh2online\.[0-9]+' "$BIN/azimuth-tls.dll" | head -n 1) sha256 $(sha256sum "$BIN/azimuth-tls.dll" | cut -c1-64)"
    else
        echo "Hook:     (no data/bin/azimuth-tls.dll)"
    fi
    echo "Running:  $(pgrep -x -i 'azimuth.exe' >/dev/null && echo yes || echo no)"
    section "FH2_ExtraOptions.log (last launch, $(date -r "$BIN/FH2_ExtraOptions.log" '+%Y-%m-%d %H:%M' 2>/dev/null))"
    clean "$BIN/FH2_ExtraOptions.log" | tail -n 3000
    section "FH2_ExtraOptions.previous.log (the launch before, $(date -r "$BIN/FH2_ExtraOptions.previous.log" '+%Y-%m-%d %H:%M' 2>/dev/null))"
    clean "$BIN/FH2_ExtraOptions.previous.log" | tail -n 3000
    section "FH2_ExtraOptions.ini"
    clean "$BIN/FH2_ExtraOptions.ini"
    section "FH2Online_Accounts.ini"
    clean "$BIN/FH2Online_Accounts.ini"
    section "Free roam world: dedicated_server.net (what the last hosted world found about the network)"
    clean "$BIN/dedicated_server.net"
    section "Free roam world: server.cfg"
    clean "$BIN/server.cfg"
    section "Free roam world: dedicated_server.log (last 300 lines)"
    clean "$BIN/dedicated_server.log" | tail -n 300
    section "Last requests (FH2_Network.jsonl: method, address, result)"
    if [ -f "$BIN/FH2_Network.jsonl" ] && command -v python3 >/dev/null 2>&1; then
        tail -n 40 "$BIN/FH2_Network.jsonl" | python3 -c '
import json, sys
for line in sys.stdin:
    try:
        d = json.loads(line)
    except Exception:
        continue
    print(d.get("id"), d.get("tick_ms"), d.get("method"), d.get("url"), d.get("status"), d.get("outcome"))'
    else
        echo "(none)"
    fi
} > "$OUT" 2>&1

echo "Report written to $OUT"
echo "Send that file to whoever helps you (it has no tokens or keys)."
if [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ] && command -v zenity >/dev/null 2>&1 && [ ! -t 0 ]; then
    zenity --info --title=ForzaInfinity --width=420 --text="Report written to $OUT\n\nSend that file to whoever helps you (it has no tokens or keys)." 2>/dev/null || true
fi
