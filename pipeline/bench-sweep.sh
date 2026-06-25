#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
# SPDX-License-Identifier: MIT
#
# bench-sweep.sh — repeatable LIVE perf sweep for martin on the dev GPU.
#
# Why this exists: MARTIN_BENCH measures CPU frame-SUBMIT rate (wgpu queues async, nothing host-waits),
# so it over-reports ~20x and is useless for the real live GPU ceiling. The trustworthy live number is
# MARTIN_FPS + MARTIN_VSYNC=0 in a real window. But the live clock advances by wall time, so a faster
# GPU samples a different moment — confounding any count/res/scale comparison. MARTIN_HOLD_T pins the
# timeline so every cell renders byte-identical content. This driver sweeps the levers ONE GPU run at a
# time (concurrent renders wedge RADV), drops warm-up frames, medians the steady-state fps, and tabs CSV.
#
# Usage:
#   pipeline/bench-sweep.sh <show> [out.csv]
# Env (space-separated lists; defaults shown):
#   COUNTS="120000 250000 400000"   morph counts to sweep (MARTIN_MORPH_COUNT)
#   RESES="960x540 1280x720 1920x1080"   resolutions (MARTIN_WIDTH x MARTIN_HEIGHT)
#   SCALES="1.0 0.7 0.5"            splat-disk scales (MARTIN_SPLAT_SCALE)
#   HOLD_T=20                       timeline second to pin (a held, content-rich moment)
#   DUR=8                           seconds per cell
#   WARMUP=3                        leading metrics samples to discard (sort/pipeline settle)
#   BIN=target/release/martin
# Requires a display (DISPLAY=:0). Renders one window at a time — do not run other GPU jobs alongside.
set -euo pipefail

SHOW="${1:?usage: bench-sweep.sh <show> [out.csv]}"
OUT="${2:-bench-sweep.csv}"
COUNTS="${COUNTS:-120000 250000 400000}"
RESES="${RESES:-960x540 1280x720 1920x1080}"
SCALES="${SCALES:-1.0 0.7 0.5}"
HOLD_T="${HOLD_T:-20}"
DUR="${DUR:-8}"
WARMUP="${WARMUP:-3}"
BIN="${BIN:-target/release/martin}"

[ -x "$BIN" ] || { echo "no binary at $BIN — cargo build --release first" >&2; exit 1; }
[ -n "${DISPLAY:-}" ] || { echo "no DISPLAY — this sweep needs a window (set DISPLAY=:0)" >&2; exit 1; }

echo "count,width,height,scale,median_fps,median_ms,samples" > "$OUT"
echo "sweep: $SHOW  hold_t=$HOLD_T dur=${DUR}s warmup=$WARMUP  → $OUT" >&2

# median of stdin numbers (one per line)
median() { sort -n | awk '{a[NR]=$1} END{ if(NR==0){print "nan"} else if(NR%2){print a[(NR+1)/2]} else {printf "%.1f",(a[NR/2]+a[NR/2+1])/2} }'; }

for n in $COUNTS; do
  for res in $RESES; do
    w="${res%x*}"; h="${res#*x}"
    for s in $SCALES; do
      log="$(mktemp)"
      # ONE GPU process; MARTIN_HOLD_T pins content, VSYNC=0 uncaps, FPS=1 logs steady-state metrics.
      MARTIN_FPS=1 MARTIN_MUTE=1 MARTIN_VSYNC=0 MARTIN_HOLD_T="$HOLD_T" \
        MARTIN_MORPH_COUNT="$n" MARTIN_WIDTH="$w" MARTIN_HEIGHT="$h" MARTIN_SPLAT_SCALE="$s" \
        timeout "$DUR" "$BIN" "$SHOW" >"$log" 2>&1 || true
      # collect fps + ms, drop the first WARMUP samples (sort/pipeline settle)
      fps=$(grep -oE 'metrics: [0-9.]+ fps' "$log" | grep -oE '[0-9.]+' | tail -n +"$((WARMUP+1))")
      ms=$(grep -oE '\([0-9.]+ ms' "$log" | grep -oE '[0-9.]+' | tail -n +"$((WARMUP+1))")
      cnt=$(printf '%s\n' "$fps" | grep -c . || true)
      mfps=$(printf '%s\n' "$fps" | median)
      mms=$(printf '%s\n' "$ms" | median)
      echo "$n,$w,$h,$s,$mfps,$mms,$cnt" | tee -a "$OUT" >&2
      rm -f "$log"
    done
  done
done

echo "--- done. sorted by median_fps: ---" >&2
{ head -1 "$OUT"; tail -n +2 "$OUT" | sort -t, -k5 -n; } | column -t -s, >&2
