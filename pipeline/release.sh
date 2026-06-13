#!/usr/bin/env bash
# ============================================================================
# release.sh — build martin as ONE self-contained release binary (the show +
# all its assets baked in via --features bundle) and verify it self-extracts +
# plays. Thin wrapper around the bundling pipeline (build.rs reads bundle.toml).
#
# Usage:   ./pipeline/release.sh [bundle.toml]
# Tunable: TARGET_GLIBC=2.31  (the oldest glibc the binary must run on)
#
# PORTABILITY: a binary linked against this machine's (Tumbleweed = bleeding-edge) glibc won't run
# on older distros ("GLIBC_2.43 not found" on Mint/Ubuntu). So if `cargo-zigbuild` + `zig` are on
# PATH we link against an OLD glibc (default 2.31 → Ubuntu 20.04+/Debian 11+/Mint 20+) via zig —
# built here, runs everywhere. Without them we fall back to a native build (only runs on glibc ≥
# this machine's) and warn. Install once: `cargo install cargo-zigbuild` + put `zig` on PATH.
#
# The .ply the manifest references must exist locally (git-ignored); the demo shapes are generated
# on demand. Cross-*OS* (Windows/macOS) binaries: run this on each OS, or use GitHub Actions.
# ============================================================================
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$HERE"
MANIFEST="${1:-bundle.toml}"
TARGET_GLIBC="${TARGET_GLIBC:-2.31}"

# The bundle's procedural splats are synthesized by build.rs (build/gen_splats.rs) during the cargo
# build below if missing — no separate generate step needed.

if command -v cargo-zigbuild >/dev/null && command -v zig >/dev/null; then
  echo "==> building PORTABLE bundled binary from $MANIFEST (zigbuild, glibc $TARGET_GLIBC)"
  MARTIN_BUNDLE="$MANIFEST" cargo +nightly zigbuild --release --features bundle \
    --target "x86_64-unknown-linux-gnu.$TARGET_GLIBC"
  BIN="$HERE/target/x86_64-unknown-linux-gnu/release/martin"
else
  echo "WARNING: cargo-zigbuild + zig not on PATH → NATIVE build (only runs on glibc >= this box's)."
  echo "         For a portable binary: cargo install cargo-zigbuild, and put zig on PATH."
  MARTIN_BUNDLE="$MANIFEST" cargo +nightly build --release --features bundle
  BIN="$HERE/target/release/martin"
fi
echo "==> max glibc required: $(objdump -T "$BIN" 2>/dev/null | grep -oE 'GLIBC_[0-9.]+' | sort -V | tail -1)"
echo "==> verifying it self-extracts + builds the baked-in show (headless)"
env -u DISPLAY -u WAYLAND_DISPLAY MARTIN_BENCH=90 timeout 180 "$BIN" 2>&1 \
  | grep -iE "bundle:|sequence built|bench:" | head

echo
echo "============================================================"
echo "RELEASE BINARY:  $BIN   ($(du -h "$BIN" | cut -f1))"
echo "Self-contained — run it anywhere (no assets, no env):  $BIN"
echo "Publish:  gh release create vX.Y -t 'martin vX.Y' && gh release upload vX.Y \"$BIN\""
echo "============================================================"
