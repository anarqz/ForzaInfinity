#!/usr/bin/env bash
# Runs release/install.sh (or, with --ps, release/install.ps1 under pwsh) on fake Azimuth folders
# and checks what it does to FH2_ExtraOptions.ini: the player's settings stay, only the template's
# missing options are added, [Network] gets Mode=Proxy and the server, the file as it was is kept
# once as .pre-infinity, and a second run changes nothing.
#   tools/test_install_ini.sh [--ps [pwsh]]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PS=""
if [ "${1:-}" = "--ps" ]; then PS="${2:-$HOME/tools/pwsh/pwsh}"; fi
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
SERVER="https://test.example"

STAGE="$WORK/stage"; mkdir -p "$STAGE"
cp "$ROOT/release/install.sh" "$ROOT/release/install.ps1" "$ROOT/release/update.sh" "$ROOT/release/update.ps1" \
    "$ROOT/release/report.sh" "$ROOT/release/report.ps1" "$ROOT/release/FH2_ExtraOptions.template.ini" "$STAGE/"
printf 'MZ fake hook' > "$STAGE/azimuth-tls.dll"
printf 'MZ fake relay' > "$STAGE/dedicated_server.exe"

install_into() { # <azimuth folder>
    if [ -n "$PS" ]; then "$PS" -NoProfile -File "$STAGE/install.ps1" -Folder "$1" -Server "$SERVER" -Quiet >/dev/null
    else "$STAGE/install.sh" "$1" "$SERVER" >/dev/null; fi
}
fake_azimuth() { # <name> -> folder, with azimuth.exe saying 0.9.10 (UTF-16, two digits)
    local dir="$WORK/$1"; mkdir -p "$dir/data/bin"
    printf '0\x00.\x009\x00.\x001\x000\x00' > "$dir/azimuth.exe"; printf 'stock' > "$dir/data/bin/azimuth-tls.dll"
    echo "$dir"
}
value() { # <ini> <section> <key>: the key's values in that section, one per line
    tr -d '\r' < "$1" | awk -v s="$2" -v k="$3" '
        /^[ \t]*\[/ { cur = tolower($0); gsub(/[][ \t]|;.*/, "", cur); next }
        cur == tolower(s) { line = $0; sub(/[;#].*/, "", line); n = index(line, "=")
            if (n && tolower(substr(line, 1, n - 1)) ~ "^[ \t]*" tolower(k) "[ \t]*$") { v = substr(line, n + 1); gsub(/^[ \t]+|[ \t]+$/, "", v); print v } }'
}
expect() { # <ini> <section> <key> <value>
    local got; got="$(value "$1" "$2" "$3")"
    [ "$got" = "$4" ] || { echo "FAIL $(basename "$(dirname "$(dirname "$(dirname "$1")")")"): [$2] $3 = '$got', expected '$4'" >&2; exit 1; }
}
no_duplicates() { # <ini>: no key twice in one section
    local dup; dup="$(tr -d '\r' < "$1" | awk '/^[ \t]*\[/ { s = tolower($0); next } { l = $0; sub(/[;#].*/, "", l); n = index(l, "="); if (!n) next
        k = tolower(substr(l, 1, n - 1)); gsub(/[ \t]/, "", k); if (seen[s, k]++) print s " " k }')"
    [ -z "$dup" ] || { echo "FAIL duplicated: $dup" >&2; exit 1; }
}
crlf() { # <ini>: every line ends in CRLF
    [ "$(grep -c $'\r$' "$1")" = "$(wc -l < "$1")" ] || { echo "FAIL not CRLF: $1" >&2; exit 1; }
}

# 1. no INI: the template, with the server
dir="$(fake_azimuth none)"; install_into "$dir"; ini="$dir/data/bin/FH2_ExtraOptions.ini"
expect "$ini" Network Server "$SERVER"; expect "$ini" Network Mode Proxy; expect "$ini" Frontend GetInCarFromPauseMenu 0
[ ! -e "$ini.pre-infinity" ] || { echo "FAIL none: backup without an INI" >&2; exit 1; }
crlf "$ini"; echo "ok: no INI -> template"

# 2. an ExtraOptions INI of the player's (older, [General], own values, a BOM, no [Network])
dir="$(fake_azimuth player)"; ini="$dir/data/bin/FH2_ExtraOptions.ini"
printf '\xef\xbb\xbf; my settings\r\n[General]\r\nEnabled=1\r\n\r\n[Frontend]\r\nSkipIntro = 0 ; I like the intro\r\nGetInCarFromPauseMenu=1\r\n\r\n[Autodrive]\r\nEnabled=0\r\n\r\n[Livery]\r\nIntermediateMSAA=2\r\nEnable4K=0\r\n' > "$ini"
cp "$ini" "$WORK/player.orig"
install_into "$dir"
expect "$ini" General Enabled 1; expect "$ini" Frontend SkipIntro 0; expect "$ini" Frontend GetInCarFromPauseMenu 1
expect "$ini" Autodrive Enabled 0; expect "$ini" Livery IntermediateMSAA 2; expect "$ini" Livery Enable4K 0
expect "$ini" Autodrive InEvents 0; expect "$ini" Frontend EnableClubs 1; expect "$ini" Compatibility DisableAzimuthSaveChecksums 0
expect "$ini" Network Mode Proxy; expect "$ini" Network Server "$SERVER"
[ -z "$(value "$ini" Global Enabled)" ] || { echo "FAIL player: [Global] added beside [General]" >&2; exit 1; }
cmp -s "$ini.pre-infinity" "$WORK/player.orig" || { echo "FAIL player: backup differs" >&2; exit 1; }
no_duplicates "$ini"; crlf "$ini"
if [ -n "$PS" ]; then # install.ps1 and install.sh write the same file
    twin="$(fake_azimuth twin)"; cp "$WORK/player.orig" "$twin/data/bin/FH2_ExtraOptions.ini"
    "$STAGE/install.sh" "$twin" "$SERVER" >/dev/null
    cmp -s "$ini" "$twin/data/bin/FH2_ExtraOptions.ini" || { echo "FAIL player: install.ps1 and install.sh differ" >&2; diff "$twin/data/bin/FH2_ExtraOptions.ini" "$ini" >&2; exit 1; }
fi
cp "$ini" "$WORK/player.first"; install_into "$dir"
cmp -s "$ini" "$WORK/player.first" || { echo "FAIL player: second run changed the INI" >&2; diff "$WORK/player.first" "$ini" >&2; exit 1; }
cmp -s "$ini.pre-infinity" "$WORK/player.orig" || { echo "FAIL player: backup replaced" >&2; exit 1; }
echo "ok: player's INI kept, template's missing options added, second run unchanged"

# 3. an INI with [Network] Mode=Off and another server: ours now, the rest stays
dir="$(fake_azimuth network)"; ini="$dir/data/bin/FH2_ExtraOptions.ini"
tr -d '\r' < "$ROOT/release/FH2_ExtraOptions.template.ini" | sed 's/^Mode=Proxy/Mode=Off/; s/^Server=.*/Server=10.0.0.2:8081\r\nAutoUpdate=0/; s/^OfflineVIP=1/OfflineVIP=0/; s/$/\r/' > "$ini"
install_into "$dir"
expect "$ini" Network Mode Proxy; expect "$ini" Network Server "$SERVER"; expect "$ini" Network AutoUpdate 0
expect "$ini" Content OfflineVIP 0; no_duplicates "$ini"; crlf "$ini"
[ -e "$ini.pre-infinity" ] || { echo "FAIL network: no backup" >&2; exit 1; }
cmp -s "$dir/data/bin/dedicated_server.exe" "$STAGE/dedicated_server.exe" || { echo "FAIL network: dedicated_server.exe not installed" >&2; exit 1; }
echo "ok: [Network] set to the server, the player's options and AutoUpdate kept; dedicated_server.exe installed"

# 4. already complete: nothing written, no backup
dir="$(fake_azimuth complete)"; ini="$dir/data/bin/FH2_ExtraOptions.ini"
sed "s|__SERVER__|$SERVER|" "$ROOT/release/FH2_ExtraOptions.template.ini" > "$ini"; cp "$ini" "$WORK/complete.orig"
install_into "$dir"
cmp -s "$ini" "$WORK/complete.orig" || { echo "FAIL complete: changed" >&2; diff "$WORK/complete.orig" "$ini" >&2; exit 1; }
[ ! -e "$ini.pre-infinity" ] || { echo "FAIL complete: backup without a change" >&2; exit 1; }
echo "ok: a complete INI is left alone"
