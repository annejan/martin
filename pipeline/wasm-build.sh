#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
# SPDX-License-Identifier: MIT
#
# wasm-build.sh — bake ONE production into a static WebGPU site (wasm + js + assets + index.html),
# zipped, to drop on any static host. NOT generic: the baked-in show is web/show.show (main.rs
# include_str!s it) — edit that file (splat:/text: only; mesh:/image:/svg: load via std::fs → won't
# work in a browser) and re-run. Needs: nightly, the wasm32 target, wasm-bindgen-cli (cargo install
# wasm-bindgen-cli). Serve the result over HTTP (WebGPU + ES modules don't run from file://); Chrome/Edge.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
OUT="${1:-$ROOT/web/dist}"
SHOW="web/show.show"

echo "==> build wasm (release, webgpu)"
cargo +nightly build --target wasm32-unknown-unknown --release --no-default-features --features web

echo "==> wasm-bindgen (--target web)"
rm -rf "$OUT"; mkdir -p "$OUT/assets"
wasm-bindgen --target web --no-typescript --out-dir "$OUT" --out-name martin \
  target/wasm32-unknown-unknown/release/martin.wasm
if command -v wasm-opt >/dev/null 2>&1; then
  echo "==> wasm-opt -Oz"; wasm-opt -Oz -o "$OUT/martin_bg.wasm" "$OUT/martin_bg.wasm"
else
  echo "(wasm-opt not installed — skipping size optimization)"
fi

echo "==> page + assets"
cp web/index.html "$OUT/"
# pre-render the synth track (the browser build has no synth thread → loads this WAV). Trim to the
# demo's length so the WAV stays small (the full builtin score is ~160s; this reel is ~30s).
./target/release/martin "$SHOW" --synth-wav "$OUT/assets/_full.wav"
if command -v ffmpeg >/dev/null 2>&1; then
  ffmpeg -y -loglevel error -i "$OUT/assets/_full.wav" -t 32 "$OUT/assets/web_music.wav"
  rm -f "$OUT/assets/_full.wav"
else
  mv "$OUT/assets/_full.wav" "$OUT/assets/web_music.wav"
fi
# copy every splat .ply the show references (web-safe assets fetched by Bevy's web AssetReader)
for ply in $(grep -oE "splat:[A-Za-z0-9_]+\.ply" "$SHOW" | sed 's/splat://' | sort -u); do
  echo "    + assets/$ply"; cp "assets/$ply" "$OUT/assets/"
done

echo "==> zip"
( cd "$OUT" && zip -qr "$ROOT/web/martin-web.zip" . )
SZ=$(du -h "$ROOT/web/martin-web.zip" | cut -f1)
echo "DONE → web/martin-web.zip ($SZ).  Serve over HTTP (e.g. 'python3 -m http.server' in the unzip dir); open in Chrome/Edge."
