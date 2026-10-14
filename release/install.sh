#!/usr/bin/env bash
# ForzaInfinity installer for Linux and the Steam Deck (Azimuth under Proton): puts the
# ForzaInfinity azimuth-tls.dll into Azimuth (keeping the original as azimuth-tls.dll.stock)
# and points it at a server.
#   ./install.sh                         finds azimuth.exe and asks for the server
#   ./install.sh <azimuth folder> <host:port | https://host>
#   curl -fsSL https://github.com/anarqz/ForzaInfinity/releases/latest/download/install.sh | bash
set -euo pipefail
DEFAULT_SERVER='__DEFAULT_SERVER__'
RELEASE='https://github.com/anarqz/ForzaInfinity/releases/latest/download'
HERE="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd || pwd)"
GUI=0
if [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ] && command -v zenity >/dev/null 2>&1 && [ ! -t 0 -o -n "${FORZAINFINITY_GUI:-}" ]; then GUI=1; fi

say() { if [ "$GUI" = 1 ]; then zenity --info --title=ForzaInfinity --width=420 --text="$1" 2>/dev/null || true; fi; echo "$1"; }
fail() { if [ "$GUI" = 1 ]; then zenity --error --title=ForzaInfinity --width=420 --text="$1" 2>/dev/null || true; fi; echo "ForzaInfinity: $1" >&2; exit 1; }
is_azimuth() { [ -n "$1" ] && [ -f "$1/azimuth.exe" ] && [ -d "$1/data/bin" ]; }

# 1. the Azimuth folder: given, next to this script, or found in home and on SD cards / drives
AZIMUTH="${1:-}"
if ! is_azimuth "$AZIMUTH"; then
    AZIMUTH=""
    for dir in "$HERE" "$(dirname "$HERE")"; do is_azimuth "$dir" && AZIMUTH="$dir" && break; done
fi
if [ -z "$AZIMUTH" ]; then
    mapfile -t FOUND < <(find "$HOME" /run/media /media /mnt -maxdepth 6 -type f -iname azimuth.exe \
        -not -path '*/compatdata/*' -not -path '*/.local/share/Trash/*' 2>/dev/null | while read -r exe; do
            d="$(dirname "$exe")"; is_azimuth "$d" && echo "$d"; done | sort -u)
    if [ "${#FOUND[@]}" -eq 1 ]; then
        AZIMUTH="${FOUND[0]}"
    elif [ "${#FOUND[@]}" -gt 1 ]; then
        if [ "$GUI" = 1 ]; then
            AZIMUTH="$(zenity --list --title=ForzaInfinity --text='Which Azimuth?' --column=Folder --width=640 --height=320 "${FOUND[@]}" 2>/dev/null || true)"
        else
            echo "Azimuth folders found:"; for i in "${!FOUND[@]}"; do echo "  $((i + 1))) ${FOUND[$i]}"; done
            read -r -p "Which one? [1] " pick </dev/tty; AZIMUTH="${FOUND[$(( ${pick:-1} - 1 ))]:-}"
        fi
    fi
fi
is_azimuth "$AZIMUTH" || fail "Azimuth was not found. Run: ./install.sh <folder with azimuth.exe> <host:port | https://host>"
BIN="$AZIMUTH/data/bin"

if ps -eo args | grep -qi '[a]zimuth\.exe'; then fail "Close Forza Horizon 2 (Azimuth) first, then run the installer again."; fi

# 2. the server
SERVER="${2:-}"
if [ -z "$SERVER" ]; then
    DEFAULT=""; case "$DEFAULT_SERVER" in __*) ;; *) DEFAULT="$DEFAULT_SERVER" ;; esac
    if [ "$GUI" = 1 ]; then
        SERVER="$(zenity --entry --title=ForzaInfinity --width=420 --text='ForzaInfinity server address (host:port or https://host), from whoever runs your server:' --entry-text="$DEFAULT" 2>/dev/null || true)"
    elif (exec </dev/tty) 2>/dev/null; then # a terminal we can really read (not just a node)
        read -r -p "ForzaInfinity server (host:port or https://host) [${DEFAULT}]: " SERVER </dev/tty; SERVER="${SERVER:-$DEFAULT}"
    else
        SERVER="$DEFAULT"
    fi
fi
SERVER="$(echo "$SERVER" | tr -d '[:space:]')"
[[ "$SERVER" =~ ^(https?://[A-Za-z0-9.-]+(:[0-9]{1,5})?|[A-Za-z0-9.-]+:[0-9]{1,5})$ ]] || fail "The server must look like host:port (for example 192.168.0.10:8081) or https://host, not '$SERVER'."

# 3. the DLL (next to this script, else the latest release)
DLL="$HERE/azimuth-tls.dll"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
if [ ! -f "$DLL" ]; then DLL="$TMP/azimuth-tls.dll"; curl -fsSL "$RELEASE/azimuth-tls.dll" -o "$DLL"; fi
if [ -f "$BIN/azimuth-tls.dll" ] && [ ! -f "$BIN/azimuth-tls.dll.stock" ]; then cp "$BIN/azimuth-tls.dll" "$BIN/azimuth-tls.dll.stock"; fi
cp "$DLL" "$BIN/azimuth-tls.dll.new" && mv "$BIN/azimuth-tls.dll.new" "$BIN/azimuth-tls.dll"

# 4. the settings: [Network] Mode=Proxy and Server=..., keeping everything else (CRLF file)
INI="$BIN/FH2_ExtraOptions.ini"
if [ -f "$INI" ]; then
    awk -v server="$SERVER" '
        { sub(/\r$/, "") }
        /^\[/ { if (net && !mode) print "Mode=Proxy"; if (net && !srv) print "Server=" server; net = ($0 == "[Network]"); seen = seen || net }
        net && /^[ \t]*Mode[ \t]*=/ { print "Mode=Proxy"; mode = 1; next }
        net && /^[ \t]*Server[ \t]*=/ { print "Server=" server; srv = 1; next }
        { print }
        END {
            if (net && !mode) print "Mode=Proxy"; if (net && !srv) print "Server=" server
            if (!seen) { print ""; print "[Network]"; print "Mode=Proxy"; print "Server=" server }
        }' "$INI" | sed 's/$/\r/' > "$TMP/ini" && cp "$TMP/ini" "$INI"
else
    TEMPLATE="$HERE/FH2_ExtraOptions.ini"
    if [ ! -f "$TEMPLATE" ]; then TEMPLATE="$TMP/template.ini"; curl -fsSL "$RELEASE/FH2_ExtraOptions.ini" -o "$TEMPLATE"; fi
    sed "s|__SERVER__|$SERVER|" "$TEMPLATE" > "$INI"
fi

say "ForzaInfinity is installed in $AZIMUTH.

Server: $SERVER

Start the game, then open Settings > INFINITY to create your profile."
