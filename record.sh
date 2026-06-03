#!/usr/bin/env bash
# Render the dogdemo timeline to an mp4 (headless deterministic frame capture + ffmpeg).
# Usage: ./record.sh [output.mp4]   (inherits any DOGDEMO_* env vars)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="${1:-$HERE/dogdemo.mp4}"
FR="$(mktemp -d)"
export DISPLAY="${DISPLAY:-:0}"

echo "==> building dogdemo"
cargo +nightly build --manifest-path "$HERE/Cargo.toml"
BIN="$(find "$HERE/target/debug" -maxdepth 1 -type f -executable -name dogdemo | head -n1)"

echo "==> recording the timeline -> $FR"
DOGDEMO_RECORD="$FR" BEVY_ASSET_ROOT="$HERE" "$BIN"

echo "==> assembling $OUT"
ffmpeg -y -framerate 60 -start_number 0 -i "$FR/frame_%05d.png" \
  -c:v libx264 -pix_fmt yuv420p -crf 18 -movflags +faststart "$OUT"
rm -rf "$FR"
echo "==> wrote $OUT"
