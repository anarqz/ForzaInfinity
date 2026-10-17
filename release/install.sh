#!/usr/bin/env bash
# ForzaInfinity installer for Linux and the Steam Deck (Azimuth under Proton): puts the
# ForzaInfinity azimuth-tls.dll into Azimuth (keeping the original as azimuth-tls.dll.stock)
# and points it at a server.
#   ./install.sh                         finds azimuth.exe and asks for the server
#   ./install.sh <azimuth folder> <host:port | https://host>
#   curl -fsSL https://github.com/anarqz/ForzaInfinity/releases/latest/download/install.sh | bash
# It needs an Azimuth version the hook was made for (SUPPORTED_AZIMUTH); FORCE=1 installs on
# another one anyway (the game then plays offline). Updating Azimuth puts its own
# azimuth-tls.dll back: run the installer again after each update.
set -euo pipefail
SUPPORTED_AZIMUTH='0.9.7 0.9.9 0.9.10'
SUPPORTED_TEXT='0.9.10 (the latest), 0.9.9 or 0.9.7'
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

# the Azimuth version: the UTF-16 "0.x.y" string in azimuth.exe (grep -P, else python3)
azimuth_version() {
    local exe="$1/azimuth.exe" found
    found="$(LC_ALL=C grep -aoP '(?<![0-9.]\x00)0\x00\.\x00(?:[0-9]\x00)+\.\x00(?:[0-9]\x00)+(?![0-9.]\x00)' "$exe" 2>/dev/null | tr -d '\0' | sort | uniq -c | sort -rn | awk 'NR == 1 { print $2 }')" || true
    if [ -z "$found" ] && command -v python3 >/dev/null 2>&1; then
        found="$(python3 -c 'import re, sys, collections; t = open(sys.argv[1], "rb").read().decode("utf-16-le", "ignore"); c = collections.Counter(re.findall(r"(?<![0-9.])0\.[0-9]+\.[0-9]+(?![0-9.])", t)); print(c.most_common(1)[0][0] if c else "")' "$exe" 2>/dev/null)" || true
    fi
    echo "$found"
}
VERSION="$(azimuth_version "$AZIMUTH")"
if [ -z "$VERSION" ]; then
    echo "ForzaInfinity: could not read the Azimuth version; it needs Azimuth $SUPPORTED_TEXT." >&2
elif ! [[ " $SUPPORTED_AZIMUTH " == *" $VERSION "* ]] && [ -z "${FORCE:-}" ]; then
    fail "This is Azimuth $VERSION. ForzaInfinity needs Azimuth $SUPPORTED_TEXT: on $VERSION the game would play offline. Update Azimuth to the latest version and run the installer again, or FORCE=1 to install anyway."
fi

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
# keep Azimuth's own (a few KB; ours is far bigger): the first time, and again after an Azimuth
# update put its own back
if [ -f "$BIN/azimuth-tls.dll" ] && { [ ! -f "$BIN/azimuth-tls.dll.stock" ] || [ "$(wc -c < "$BIN/azimuth-tls.dll")" -lt 65536 ]; }; then
    cp "$BIN/azimuth-tls.dll" "$BIN/azimuth-tls.dll.stock"
fi
cp "$DLL" "$BIN/azimuth-tls.dll.new" && mv "$BIN/azimuth-tls.dll.new" "$BIN/azimuth-tls.dll"
# the free roam world a player hosts (Settings > INFINITY > HOST WORLD): dedicated_server.exe,
# next to this script, else the latest release's
RELAY="$HERE/dedicated_server.exe"
if [ ! -f "$RELAY" ]; then RELAY="$TMP/dedicated_server.exe"; curl -fsSL "$RELEASE/dedicated_server.exe" -o "$RELAY" || RELAY=""; fi
if [ -n "$RELAY" ]; then cp "$RELAY" "$BIN/dedicated_server.exe.new" && mv -f "$BIN/dedicated_server.exe.new" "$BIN/dedicated_server.exe"; fi

# 4. the settings. FH2_ExtraOptions.ini is the player's: what is in it stays as it is, and only the
# options of our template it lacks are added (with their comments), at the end of their section.
# [Network] is ForzaInfinity's: Mode=Proxy and Server=<the server chosen above>. Before the first
# change, the file as it was is kept as FH2_ExtraOptions.ini.pre-infinity.
INI="$BIN/FH2_ExtraOptions.ini"
TEMPLATE="$HERE/FH2_ExtraOptions.template.ini"
if [ ! -f "$TEMPLATE" ]; then TEMPLATE="$TMP/template.ini"; curl -fsSL "$RELEASE/FH2_ExtraOptions.template.ini" -o "$TEMPLATE"; fi
if [ -f "$INI" ]; then
    LC_ALL=C awk -v server="$SERVER" '
        function trim(t) { gsub(/^[ \t]+|[ \t]+$/, "", t); return t }
        function strip(t) { sub(/[;#].*/, "", t); return trim(t) }
        function sec(t) { t = tolower(trim(substr(t, 2, index(t, "]") - 2))); return t == "general" ? "global" : t }
        function key(t) { return tolower(trim(substr(t, 1, index(t, "=") - 1))) }
        function ours(t, k) { return k == "mode" ? "Mode=Proxy" : k == "server" ? "Server=" server : t }
        # an option with its comments; blank line before: 1 always, 0 when it has comments, -1 never
        function put(b, blank) { if (blank > 0 || (!blank && index(b, "\n"))) print ""; print b }
        { sub(/\r$/, ""); if (FNR == 1) sub(/^\xef\xbb\xbf/, "") }
        # the template: its sections in order, and each option with the comments right above it
        FNR == NR {
            line = strip($0)
            if (line == "") { notes = ($0 ~ /^[ \t]*[;#]/) ? notes $0 "\n" : ""; next }
            if (line ~ /^\[/) { s = sec(line); order[++sections] = s; header[s] = line; notes = ""; next }
            k = key(line); keys[s] = keys[s] " " k; block[s, k] = notes ($0 ~ /^[ \t]*Server[ \t]*=/ ? "Server=" server : $0); notes = ""
            next
        }
        # the player'"'"'s INI: what it has, and where each of its sections ends
        FNR == 1 { s = "" }
        {
            lines[++count] = $0; line = strip($0)
            if (line ~ /^\[/) { s = sec(line); seen[s] = 1; last[s] = count; next }
            if (line == "") next
            if (s != "" && index(line, "=")) { k = key(line); has[s, k] = 1; if (s == "network" && (k == "mode" || k == "server")) lines[count] = ours($0, k) }
            if (s != "") last[s] = count
        }
        END {
            for (i = 1; i <= count; i++) {
                print lines[i]
                for (j = 1; j <= sections; j++) {
                    s = order[j]; if (!seen[s] || last[s] != i) continue
                    n = split(keys[s], list, " "); first = 1
                    for (m = 1; m <= n; m++) if (!has[s, list[m]]) { put(block[s, list[m]], first); first = 0 }
                }
            }
            for (j = 1; j <= sections; j++) {
                s = order[j]; if (seen[s]) continue
                printf "\n%s\n", header[s]
                n = split(keys[s], list, " ")
                for (m = 1; m <= n; m++) put(block[s, list[m]], m == 1 ? -1 : 0)
            }
        }' "$TEMPLATE" "$INI" | sed 's/$/\r/' > "$TMP/ini"
    if ! cmp -s "$TMP/ini" "$INI"; then
        [ -f "$INI.pre-infinity" ] || cp "$INI" "$INI.pre-infinity"
        cp "$TMP/ini" "$INI"
    fi
else
    sed "s|__SERVER__|$SERVER|" "$TEMPLATE" > "$INI"
fi

# 5. the updater, left in the Azimuth folder: ForzaInfinity-Update.sh installs the latest release
UPDATER="$HERE/update.sh"
if [ ! -f "$UPDATER" ]; then UPDATER="$TMP/update.sh"; curl -fsSL "$RELEASE/update.sh" -o "$UPDATER" || UPDATER=""; fi
if [ -n "$UPDATER" ]; then cp "$UPDATER" "$AZIMUTH/ForzaInfinity-Update.sh" && chmod +x "$AZIMUTH/ForzaInfinity-Update.sh"; fi
# 6. the report, left there too: ForzaInfinity-Report.sh writes the last launch to one file
REPORTER="$HERE/report.sh"
if [ ! -f "$REPORTER" ]; then REPORTER="$TMP/report.sh"; curl -fsSL "$RELEASE/report.sh" -o "$REPORTER" || REPORTER=""; fi
if [ -n "$REPORTER" ]; then cp "$REPORTER" "$AZIMUTH/ForzaInfinity-Report.sh" && chmod +x "$AZIMUTH/ForzaInfinity-Report.sh"; fi

say "ForzaInfinity is installed in $AZIMUTH.

Server: $SERVER

Start the game, then open Settings > INFINITY to create your profile."
