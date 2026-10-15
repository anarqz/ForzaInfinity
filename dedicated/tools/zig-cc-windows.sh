#!/usr/bin/env bash
# Linker for x86_64-pc-windows-gnu on Linux: zig's clang + its own MinGW libs, so no MinGW
# toolchain is needed. Used by ../build-windows.sh (ZIG= overrides the zig path).
#
# rustc passes what a MinGW gcc wants; zig ships those pieces itself (or does not need them),
# so these are left out:
#   -lgcc_eh -lgcc -lgcc_s -l:libpthread.a  (zig links compiler-rt and winpthreads as needed)
#   rsbegin.o rsend.o crt2.o dllcrt2.o      (rust's self-contained MinGW startup objects;
#                                            zig adds its own crt)
set -euo pipefail
ZIG="${ZIG:-$HOME/tools/zig/zig}"
args=()
for a in "$@"; do
  case "$a" in
    -lgcc_eh) args+=("-lunwind") ;;
    -lgcc|-lgcc_s|-l:libpthread.a|-lpthread|-lmsvcrt|-lmingwex|-lmingw32) ;;
    */rsbegin.o|*/rsend.o|*/crt2.o|*/dllcrt2.o) ;;
    *) args+=("$a") ;;
  esac
done
exec "$ZIG" cc -target x86_64-windows-gnu "${args[@]}"
