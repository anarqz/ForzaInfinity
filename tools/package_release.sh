#!/usr/bin/env bash
# Builds a ForzaInfinity player release into dist/:
#   dist/ForzaInfinity.zip    the package players download (hook + installers + INI template)
#   dist/<each file>          the same files loose, uploaded next to the zip so that
#                             install.sh / install.ps1 run straight from the release URL
# Usage: tools/package_release.sh [--zig <zig>] [--server <host:port>] [--dll <prebuilt dll>]
#   --server bakes a default server address into the installers (they still ask).
# Then attach every file in dist/ to a GitHub release (the landing links to releases/latest).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ZIG="${ZIG:-$HOME/tools/zig/zig}"
SERVER=""
DLL=""
while [ $# -gt 0 ]; do
    case "$1" in
        --zig) ZIG="$2"; shift 2 ;;
        --server) SERVER="$2"; shift 2 ;;
        --dll) DLL="$2"; shift 2 ;;
        *) echo "unknown option $1" >&2; exit 2 ;;
    esac
done
if [ -n "$SERVER" ] && ! [[ "$SERVER" =~ ^[A-Za-z0-9.-]+:[0-9]{1,5}$ ]]; then echo "--server must be host:port" >&2; exit 2; fi

DIST="$ROOT/dist"
STAGE="$DIST/ForzaInfinity"
rm -rf "$DIST"
mkdir -p "$STAGE"

# the hook: the same build as hook/BUILD.md
if [ -z "$DLL" ]; then
    (cd "$ROOT/hook" && python3 build.py "$ZIG" --azimuth --build-dir build --output build/azimuth-tls.dll >/dev/null)
    DLL="$ROOT/hook/build/azimuth-tls.dll"
fi
cp "$DLL" "$STAGE/azimuth-tls.dll"
VERSION="$(grep -o 'MOD_VERSION "[^"]*"' "$ROOT/hook/extra_options.h" | cut -d'"' -f2)"

for f in Install.cmd Uninstall.cmd install.ps1 uninstall.ps1 install.sh uninstall.sh FH2_ExtraOptions.ini; do
    cp "$ROOT/release/$f" "$STAGE/$f"
done
chmod +x "$STAGE/install.sh" "$STAGE/uninstall.sh"
if [ -n "$SERVER" ]; then
    sed -i "s/__DEFAULT_SERVER__/$SERVER/" "$STAGE/install.sh" "$STAGE/install.ps1"
fi
{
    printf 'ForzaInfinity %s\r\n\r\n' "$VERSION"
    printf 'Windows: double click Install.cmd (Uninstall.cmd puts the original file back).\r\n'
    printf 'Steam Deck / Linux: run ./install.sh (./uninstall.sh puts the original file back).\r\n\r\n'
    printf 'Then start the game and open Settings > INFINITY > Create profile.\r\n'
    printf 'Full guide: https://github.com/anarqz/ForzaInfinity/blob/main/docs/INSTALL.md\r\n'
} > "$STAGE/README.txt"
sha256sum "$STAGE/azimuth-tls.dll" | cut -d' ' -f1 > "$STAGE/azimuth-tls.dll.sha256"

(cd "$DIST" && python3 -c "
import os, sys, zipfile
with zipfile.ZipFile('ForzaInfinity.zip', 'w', zipfile.ZIP_DEFLATED) as z:
    for name in sorted(os.listdir('ForzaInfinity')):
        info = zipfile.ZipInfo('ForzaInfinity/' + name)
        mode = 0o755 if name.endswith('.sh') else 0o644
        info.external_attr = (0o100000 | mode) << 16
        info.compress_type = zipfile.ZIP_DEFLATED
        z.writestr(info, open(os.path.join('ForzaInfinity', name), 'rb').read())
")
cp "$STAGE"/* "$DIST/"
rm -rf "$STAGE"
echo "ForzaInfinity $VERSION -> $DIST"
ls -1 "$DIST"
