#!/usr/bin/env bash
# dedicated_server for any x86_64 Linux (Debian, SteamOS on the Steam Deck...): a static binary
# (musl), so it does not depend on the system's glibc.
# Needs: rustup target add x86_64-unknown-linux-musl (rust's own musl; no musl-gcc needed).
# Output: target/x86_64-unknown-linux-musl/release/dedicated_server
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
CARGO="${CARGO:-$HOME/.cargo/bin/cargo}"
cd "$here"
"$CARGO" build --release --target x86_64-unknown-linux-musl "$@"
bin="$here/target/x86_64-unknown-linux-musl/release/dedicated_server"
ls -l "$bin"
sha256sum "$bin"
