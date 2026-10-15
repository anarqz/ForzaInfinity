#!/usr/bin/env bash
# Signs a hook DLL for the self-update: writes <dll>.sig, the 64-byte raw Ed25519 signature of the
# ASCII line "fh2online-update-v1 <the DLL's SHA-256 in lowercase hex>" (no newline), and checks it.
# Hooks with the signature check install an update only when azimuth-tls.dll.sig verifies with the public key
# built into them (hook/adapters/azimuth_sign.c).
# Usage: tools/sign_release.sh <azimuth-tls.dll>
# The private key: ~/.config/forzainfinity/release-ed25519.pem (FH2_SIGNING_KEY chooses another).
# It never goes into a repository or a release.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
KEY="${FH2_SIGNING_KEY:-$HOME/.config/forzainfinity/release-ed25519.pem}"
if [ $# -ne 1 ] || [ ! -f "$1" ]; then
    echo "usage: tools/sign_release.sh <azimuth-tls.dll>" >&2
    exit 2
fi
if [ ! -r "$KEY" ]; then
    echo "no release signing key at $KEY (FH2_SIGNING_KEY chooses another):" >&2
    echo "without azimuth-tls.dll.sig the players' hooks refuse the update" >&2
    exit 1
fi
DLL="$1"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# the key must be the one the hook trusts
PUBLIC="$(openssl pkey -in "$KEY" -pubout -outform DER | tail -c 32 | od -An -tx1 -v | tr -d ' \n')"
EMBEDDED="$(sed -n '/azimuth_sign_release_key\[32\] = {/,/};/p' "$ROOT/hook/adapters/azimuth_sign.c" |
    grep -o '0x[0-9a-f][0-9a-f]' | sed 's/^0x//' | tr -d '\n')"
if [ "$PUBLIC" != "$EMBEDDED" ]; then
    echo "$KEY is not the key built into hook/adapters/azimuth_sign.c ($EMBEDDED)" >&2
    exit 1
fi

DIGEST="$(sha256sum "$DLL" | cut -d' ' -f1)"
printf 'fh2online-update-v1 %s' "$DIGEST" > "$WORK/line"
openssl pkeyutl -sign -inkey "$KEY" -rawin -in "$WORK/line" -out "$DLL.sig"
openssl pkey -in "$KEY" -pubout -out "$WORK/public.pem"
if [ "$(wc -c < "$DLL.sig")" -ne 64 ] ||
    ! openssl pkeyutl -verify -pubin -inkey "$WORK/public.pem" -rawin -in "$WORK/line" -sigfile "$DLL.sig" >/dev/null; then
    rm -f "$DLL.sig"
    echo "the signature of $DLL did not verify" >&2
    exit 1
fi
echo "$DLL.sig (sha256 $DIGEST)"
