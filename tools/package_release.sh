#!/usr/bin/env bash
# Builds a ForzaInfinity player release into dist/:
#   dist/ForzaInfinity.zip    the package players download (hook, dedicated_server.exe for
#                             hosting free roam worlds, installers, INI template)
#   dist/<each file>          the same files loose, uploaded next to the zip so that
#                             install.sh / install.ps1 run straight from the release URL
# Usage: tools/package_release.sh [--zig <zig>] [--server <host:port | https://host>] [--dll <prebuilt dll>]
#                                 [--release vX.Y.Z]
#   --server bakes a default server address into the installers (they still ask).
# Needs the release signing key (FH2_SIGNING_KEY, by default ~/.config/forzainfinity/release-ed25519.pem):
# tools/sign_release.sh writes azimuth-tls.dll.sig and dedicated_server.exe.sig, without which
# hooks refuse the self-update (the relay's SHA-256 is also built into the hook:
# tools/embed_relay_hash.py).
#   --release also writes server/responses/infinity/latest.json ({hook, release}): the server's
#   /info names it, and hooks older than it install it from that GitHub release by themselves.
# Then attach every file in dist/ to the GitHub release (the landing links to releases/latest),
# and only then deploy the server with the new latest.json (infra/deploy.sh).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ZIG="${ZIG:-$HOME/tools/zig/zig}"
SERVER=""
DLL=""
RELEASE=""
while [ $# -gt 0 ]; do
    case "$1" in
        --zig) ZIG="$2"; shift 2 ;;
        --server) SERVER="$2"; shift 2 ;;
        --dll) DLL="$2"; shift 2 ;;
        --release) RELEASE="$2"; shift 2 ;;
        *) echo "unknown option $1" >&2; exit 2 ;;
    esac
done
if [ -n "$SERVER" ] && ! [[ "$SERVER" =~ ^(https?://[A-Za-z0-9.-]+(:[0-9]{1,5})?|[A-Za-z0-9.-]+:[0-9]{1,5})$ ]]; then echo "--server must be host:port or https://host" >&2; exit 2; fi

SIGNING_KEY="${FH2_SIGNING_KEY:-$HOME/.config/forzainfinity/release-ed25519.pem}"
if [ ! -r "$SIGNING_KEY" ]; then
    echo "no release signing key at $SIGNING_KEY (FH2_SIGNING_KEY chooses another): a release must ship" >&2
    echo "azimuth-tls.dll.sig, or the players' hooks refuse to update to it. Not packaging." >&2
    exit 1
fi

DIST="$ROOT/dist"
STAGE="$DIST/ForzaInfinity"
rm -rf "$DIST"
mkdir -p "$STAGE"

# the free roam relay (dedicated/), built first: the hook carries its SHA-256
(cd "$ROOT/dedicated" && bash ./build-windows.sh >/dev/null)
RELAY="$ROOT/dedicated/target/x86_64-pc-windows-gnu/release/dedicated_server.exe"
python3 "$ROOT/tools/embed_relay_hash.py" "$RELAY" >/dev/null
if ! git -C "$ROOT/hook" diff --quiet -- adapters/azimuth_relay_hash.h; then
    echo "warning: hook/adapters/azimuth_relay_hash.h changed with the relay: commit it in the hook" >&2
fi
# the hook: the same build as hook/BUILD.md, with the INI template it writes for a player who has
# none taken from release/FH2_ExtraOptions.template.ini
python3 "$ROOT/tools/embed_ini_template.py" >/dev/null
if ! git -C "$ROOT/hook" diff --quiet -- adapters/azimuth_ini_template.h; then
    echo "warning: hook/adapters/azimuth_ini_template.h changed with the template: commit it in the hook" >&2
fi
if [ -z "$DLL" ]; then
    (cd "$ROOT/hook" && python3 build.py "$ZIG" --azimuth --build-dir build --output build/azimuth-tls.dll >/dev/null)
    DLL="$ROOT/hook/build/azimuth-tls.dll"
fi
cp "$DLL" "$STAGE/azimuth-tls.dll"
cp "$RELAY" "$STAGE/dedicated_server.exe"
VERSION="$(grep -o 'MOD_VERSION "[^"]*"' "$ROOT/hook/extra_options.h" | cut -d'"' -f2)"

for f in Install.cmd Uninstall.cmd Update.cmd Report.cmd install.ps1 uninstall.ps1 update.ps1 report.ps1 install.sh uninstall.sh update.sh report.sh \
    FH2_ExtraOptions.template.ini; do
    cp "$ROOT/release/$f" "$STAGE/$f"
done
chmod +x "$STAGE/install.sh" "$STAGE/uninstall.sh" "$STAGE/update.sh" "$STAGE/report.sh"
if [ -n "$SERVER" ]; then
    sed -i "s|__DEFAULT_SERVER__|$SERVER|" "$STAGE/install.sh" "$STAGE/install.ps1"
fi
{
    printf 'ForzaInfinity %s\r\n\r\n' "$VERSION"
    printf 'Windows: double click Install.cmd (Uninstall.cmd puts the original file back).\r\n'
    printf 'Steam Deck / Linux: run ./install.sh (./uninstall.sh puts the original file back).\r\n\r\n'
    printf 'Updates install by themselves (the game asks to restart). By hand: close the game and run\r\n'
    printf 'ForzaInfinity-Update in the Azimuth folder (Update.cmd / update.sh here do the same).\r\n\r\n'
    printf 'Something wrong? ForzaInfinity-Report in the Azimuth folder (Report.cmd / report.sh here) writes\r\n'
    printf 'ForzaInfinity-Report.txt, the last launch, to send to whoever helps you.\r\n\r\n'
    printf 'Then start the game and open Settings > INFINITY > Create profile.\r\n\r\n'
    printf 'Free roam online: Settings > INFINITY > HOST WORLD hosts a world on this PC\r\n'
    printf '(dedicated_server.exe; Windows asks once about the firewall: allow private networks).\r\n'
    printf 'FREE ROAM ONLINE joins your friends; FIND WORLD lists the others.\r\n'
    printf 'Guide: https://github.com/anarqz/ForzaInfinity/blob/main/docs/FREEROAM.md\r\n\r\n'
    printf 'Full guide: https://github.com/anarqz/ForzaInfinity/blob/main/docs/INSTALL.md\r\n'
} > "$STAGE/README.txt"
sha256sum "$STAGE/azimuth-tls.dll" | cut -d' ' -f1 > "$STAGE/azimuth-tls.dll.sha256"
FH2_SIGNING_KEY="$SIGNING_KEY" "$ROOT/tools/sign_release.sh" "$STAGE/azimuth-tls.dll" >/dev/null
sha256sum "$STAGE/dedicated_server.exe" | cut -d' ' -f1 > "$STAGE/dedicated_server.exe.sha256"
FH2_SIGNING_KEY="$SIGNING_KEY" "$ROOT/tools/sign_release.sh" "$STAGE/dedicated_server.exe" >/dev/null
if [ -n "$RELEASE" ]; then
    [[ "$RELEASE" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "--release must look like v1.2.3" >&2; exit 2; }
    mkdir -p "$ROOT/server/responses/infinity"
    printf '{ "hook": "%s", "release": "%s" }\n' "$VERSION" "$RELEASE" > "$ROOT/server/responses/infinity/latest.json"
fi

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
