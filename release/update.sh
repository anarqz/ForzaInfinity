#!/usr/bin/env bash
# ForzaInfinity updater for Linux and the Steam Deck: installs the latest release into an
# Azimuth folder, keeping its server and settings. The installer leaves a copy in the Azimuth
# folder as ForzaInfinity-Update.sh: close the game, then run that.
#   ./update.sh [<folder with azimuth.exe>]
set -euo pipefail
RELEASE='https://github.com/anarqz/ForzaInfinity/releases/latest/download'
HERE="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd || pwd)"
AZIMUTH="${1:-$HERE}"
[ -f "$AZIMUTH/azimuth.exe" ] || AZIMUTH="" # the installer then looks for it
SERVER=""
if [ -n "$AZIMUTH" ] && [ -f "$AZIMUTH/data/bin/FH2_ExtraOptions.ini" ]; then
    SERVER="$(tr -d '\r' < "$AZIMUTH/data/bin/FH2_ExtraOptions.ini" | sed -n 's/^[[:space:]]*Server[[:space:]]*=[[:space:]]*//p' | head -n 1)"
fi
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
curl -fsSL "$RELEASE/install.sh" -o "$TMP/install.sh"
bash "$TMP/install.sh" "$AZIMUTH" "$SERVER"
