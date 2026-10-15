#!/usr/bin/env bash
# dedicated_server.exe from Linux: cargo for x86_64-pc-windows-gnu with zig as the linker.
# Needs: rustup target add x86_64-pc-windows-gnu, and zig (~/tools/zig/zig, or ZIG=...).
# The linker is set only here (an env var), so Windows builds of this crate are untouched.
# Output: target/x86_64-pc-windows-gnu/release/dedicated_server.exe
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
CARGO="${CARGO:-$HOME/.cargo/bin/cargo}"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$here/tools/zig-cc-windows.sh"
cd "$here"
"$CARGO" build --release --target x86_64-pc-windows-gnu "$@"
exe="$here/target/x86_64-pc-windows-gnu/release/dedicated_server.exe"
ls -l "$exe"
sha256sum "$exe"
