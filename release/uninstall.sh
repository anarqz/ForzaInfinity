#!/usr/bin/env bash
# ForzaInfinity uninstaller for Linux and the Steam Deck: puts back Azimuth's original
# azimuth-tls.dll (the azimuth-tls.dll.stock the installer kept).
#   ./uninstall.sh [<azimuth folder>]
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
AZIMUTH="${1:-}"
if [ -z "$AZIMUTH" ]; then
    for dir in "$HERE" "$(dirname "$HERE")"; do [ -f "$dir/data/bin/azimuth-tls.dll.stock" ] && AZIMUTH="$dir" && break; done
fi
if [ -z "$AZIMUTH" ]; then
    AZIMUTH="$(find "$HOME" /run/media /media /mnt -maxdepth 7 -type f -name azimuth-tls.dll.stock -path '*/data/bin/*' 2>/dev/null | head -1 | sed 's|/data/bin/azimuth-tls.dll.stock$||')"
fi
STOCK="$AZIMUTH/data/bin/azimuth-tls.dll.stock"
[ -n "$AZIMUTH" ] && [ -f "$STOCK" ] || { echo "No backup (data/bin/azimuth-tls.dll.stock) found. Run: ./uninstall.sh <folder with azimuth.exe>" >&2; exit 1; }
if ps -eo args | grep -qi '[a]zimuth\.exe'; then echo "Close Forza Horizon 2 (Azimuth) first." >&2; exit 1; fi
cp "$STOCK" "$AZIMUTH/data/bin/azimuth-tls.dll"
echo "Azimuth's original azimuth-tls.dll is back in $AZIMUTH. Saves and FH2_ExtraOptions.ini were left as they are."
