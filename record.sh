#!/usr/bin/env bash
# Render the martin timeline to an mp4 (headless deterministic frame capture + ffmpeg).
# Usage: ./record.sh [output.mp4]   (inherits any MARTIN_* env vars)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="${1:-$HERE/martin.mp4}"
# Frame scratch dir. mktemp honours $TMPDIR — and a full 60fps render dumps ~5300 PNGs (~10 GB),
# which OVERFLOWS a RAM-backed /tmp tmpfs ("Disk quota exceeded (os error 122)"). For full renders
# point it at a real disk: TMPDIR=/home/<you>/.cache/martin-render ./record.sh out.mp4
FR="$(mktemp -d)"
export DISPLAY="${DISPLAY:-:0}"

echo "==> building martin (release — debug can render the splats black, and release is far"
echo "    faster for big .ply clouds)"
cargo +nightly build --release --manifest-path "$HERE/Cargo.toml"
BIN="$(find "$HERE/target/release" -maxdepth 1 -type f -executable -name martin | head -n1)"

echo "==> recording the timeline -> $FR"
MARTIN_RECORD="$FR" BEVY_ASSET_ROOT="$HERE" "$BIN"

# Render the synth to a WAV and mux it in, so the .mp4 has the music (honours MARTIN_SCORE;
# skipped by MARTIN_MUTE). This invocation returns before the window — no GPU needed.
AUDIO=()
if [ -z "${MARTIN_MUTE:-}" ]; then
  WAV="$FR/track.wav"
  echo "==> rendering synth -> $WAV"
  MARTIN_SYNTH_WAV="$WAV" "$BIN"
  AUDIO=(-i "$WAV" -c:a aac -shortest)
fi

# Frame rate: honour MARTIN_PREVIEW_FPS (a fast low-fps preview render) so the mux matches the
# frames martin actually produced; default 60. Duration + audio sync stay correct at any fps.
FPS="${MARTIN_PREVIEW_FPS:-60}"
# Fade the video IN over the first ~1.5 s (from black), and OUT over the last ~2.6 s (to match the
# synth's own fade-out) — a clean open + close for the clip.
NF=$(find "$FR" -maxdepth 1 -name 'frame_*.png' | wc -l)
FADE=$(awk "BEGIN{d=$NF/$FPS-2.6; print (d>0)?d:0}")

echo "==> assembling $OUT (${FPS} fps)"
ffmpeg -y -framerate "$FPS" -start_number 0 -i "$FR/frame_%05d.png" "${AUDIO[@]}" \
  -vf "fade=t=in:st=0:d=1.5,fade=t=out:st=$FADE:d=2.6" \
  -c:v libx264 -pix_fmt yuv420p -crf 18 -movflags +faststart "$OUT"
rm -rf "$FR"
echo "==> wrote $OUT"
