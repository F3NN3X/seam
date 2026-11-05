#!/usr/bin/env bash
# Cross-compile Seam for Windows from Linux (needs mingw-w64 and Rust library sources for -Zbuild-std).
# On Windows itself, just run: cargo build --release
set -euo pipefail
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C link-self-contained=no -C target-feature=+crt-static"
RUSTC_BOOTSTRAP=1 cargo build --release --target x86_64-pc-windows-gnu -Zbuild-std=std,panic_abort "$@"
