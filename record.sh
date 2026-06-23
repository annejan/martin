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

# MARTIN_SS=<n>: supersample anti-aliasing — render at n× the target MARTIN_RES, then lanczos-downscale
# in the mux. Smooths the splat disk-edges + text without any in-engine AA (the iGPU has fill headroom
# offline). Default 1 = off. Costs ~n² fill, so use it for the final master, not fast previews.
SS="${MARTIN_SS:-1}"
RES="${MARTIN_RES:-1280x720}"
TW="${RES%x*}"; TH="${RES#*x}"
SCALE=""
if [ "${SS}" -gt 1 ]; then
  export MARTIN_RES="$((TW * SS))x$((TH * SS))"
  SCALE="scale=${TW}:${TH}:flags=lanczos,"
  echo "==> supersample ${SS}x: rendering ${MARTIN_RES} → ${RES}"
fi

# --- disk pre-flight (fail fast, before the slow build) ---------------------------------------------
# A full 60 fps PNG dump is many GB; a RAM-backed /tmp (or a near-full disk) overflows MID-render with
# a cryptic "Disk quota exceeded (os error 122)" that looks like the shell broke. Report the scratch
# space + per-frame size + how much fits, and abort early (with the TMPDIR hint) below a hard floor.
DUMP_W=$((TW * SS)); DUMP_H=$((TH * SS))   # PNGs are dumped at the (super-sampled) render res
AVAIL_KB=$(df -Pk "$FR" | awk 'NR==2{print $4}')
AVAIL_GB=$(awk "BEGIN{printf \"%.1f\", $AVAIL_KB/1048576}")
PERFRAME_MB=$(awk "BEGIN{printf \"%.2f\", $DUMP_W*$DUMP_H*1.5/1048576}")   # ~1.5 B/px PNG (splat content)
FIT_FRAMES=$(awk "BEGIN{printf \"%d\", ($AVAIL_KB/1024)/$PERFRAME_MB}")
FIT_SECS=$(awk "BEGIN{printf \"%d\", $FIT_FRAMES/${MARTIN_PREVIEW_FPS:-60}}")
echo "==> scratch $FR: ${AVAIL_GB} GB free · ~${PERFRAME_MB} MB/frame @ ${DUMP_W}x${DUMP_H} · fits ~${FIT_FRAMES} frames (~${FIT_SECS}s @ ${MARTIN_PREVIEW_FPS:-60}fps)"
FLOOR_GB="${MARTIN_DISK_FLOOR_GB:-3}"
if awk "BEGIN{exit !($AVAIL_GB < $FLOOR_GB)}"; then
  echo "==> ERROR: only ${AVAIL_GB} GB free in the scratch dir (< ${FLOOR_GB} GB floor)." >&2
  echo "    A full render dumps PNGs there first; point TMPDIR at a disk with room:" >&2
  echo "    TMPDIR=/path/with/space ./record.sh $OUT" >&2
  rm -rf "$FR"; exit 1
fi

echo "==> building martin (release — debug can render the splats black, and release is far"
echo "    faster for big .ply clouds)"
cargo +nightly build --release --manifest-path "$HERE/Cargo.toml"
BIN="$(find "$HERE/target/release" -maxdepth 1 -type f -executable -name martin | head -n1)"

# Render the synth to a WAV FIRST, then mux it in (honours MARTIN_SCORE; skipped by MARTIN_MUTE).
# Audio BEFORE the frame capture: a long 1.2M capture can wedge the RADV GPU, and the synth pass
# still boots Bevy (inits the renderer) — doing it first means it runs on a fresh, un-wedged GPU.
AUDIO=()
if [ -z "${MARTIN_MUTE:-}" ]; then
  WAV="$FR/track.wav"
  echo "==> rendering synth -> $WAV"
  # capture the synth log so we can read the show duration (it prints "(89.0s)") for the fit-check;
  # `|| true` so a shutdown-panic exit doesn't abort under `set -e`.
  SYNTH_LOG=$(MARTIN_SYNTH_WAV="$WAV" BEVY_ASSET_ROOT="$HERE" "$BIN" 2>&1 || true)
  echo "$SYNTH_LOG" | tail -3
  AUDIO=(-i "$WAV" -c:a aac -shortest)
  # Will the whole PNG dump fit? Now that we know the show length, abort BEFORE the long capture if it
  # won't (the case the GB floor misses: plenty free generally, but not enough for THIS long render).
  SHOW_SECS=$(echo "$SYNTH_LOG" | grep -oE '\([0-9]+\.[0-9]+s\)' | head -1 | tr -dc '0-9.')
  if [ -n "$SHOW_SECS" ] && awk "BEGIN{exit !($SHOW_SECS > $FIT_SECS)}"; then
    NEED_GB=$(awk "BEGIN{printf \"%.0f\", $SHOW_SECS*${MARTIN_PREVIEW_FPS:-60}*$PERFRAME_MB/1024}")
    echo "==> ERROR: this ${SHOW_SECS}s show needs ~${NEED_GB} GB of frames but only ${AVAIL_GB} GB is free (~${FIT_SECS}s fits)." >&2
    echo "    Point TMPDIR at a bigger disk, or lower MARTIN_PREVIEW_FPS / MARTIN_RES / MARTIN_SS." >&2
    rm -rf "$FR"; exit 1
  fi
fi

echo "==> recording the timeline -> $FR"
# A long 1.2M capture wedges the RADV GPU and martin then PANICS on shutdown — but only AFTER every
# frame is written. So don't let that exit code abort the mux (set -e); the frame-count check below
# is the real gate. (`|| true` keeps the panic from killing the script.)
MARTIN_RECORD="$FR" BEVY_ASSET_ROOT="$HERE" "$BIN" || echo "==> (capture process exited nonzero — frames are written; continuing to mux)"
NF_CHECK=$(find "$FR" -maxdepth 1 -name 'frame_*.png' | wc -l)
if [ "$NF_CHECK" -lt 2 ]; then
  echo "==> ERROR: no frames captured ($NF_CHECK) — aborting" >&2
  exit 1
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
  -vf "${SCALE}fade=t=in:st=0:d=1.5,fade=t=out:st=$FADE:d=2.6" \
  -c:v libx264 -pix_fmt yuv420p -crf 18 -movflags +faststart "$OUT"
rm -rf "$FR"
echo "==> wrote $OUT"
