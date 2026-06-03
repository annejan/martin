#!/usr/bin/env bash
# ============================================================================
# splat.sh  —  capture -> camera poses (CPU COLMAP) -> 3D Gaussian Splat (Brush)
#
# Usage:   ./splat.sh <video-file | image-dir> [workspace-dir]
# Example: ./splat.sh martin_gaus.mp4
#          ./splat.sh ./photos  ./mg_run
#
# Tunables (env vars):
#   FPS=2           frames/sec to sample from a video
#   MAX_SIZE=1600   cap longest image side (bounds CPU RAM — this box shares
#                   ~15 GB between CPU and the Radeon 860M iGPU)
#   EXPORT_EVERY=2000   how often Brush writes a .ply checkpoint
# ============================================================================
set -euo pipefail

INPUT="${1:?Usage: ./splat.sh <video-file | image-dir> [workspace-dir]}"
WORK="${2:-./splat_run}"
FPS="${FPS:-2}"
MAX_SIZE="${MAX_SIZE:-1600}"
EXPORT_EVERY="${EXPORT_EVERY:-2000}"

command -v colmap >/dev/null || { echo "colmap not found — run ./pipeline/splat-setup.sh first"; exit 1; }
command -v brush  >/dev/null || { echo "brush not found — run ./pipeline/splat-setup.sh (and put ~/.local/bin on PATH)"; exit 1; }

mkdir -p "$WORK/images"
SEQUENTIAL=0

if [ -d "$INPUT" ]; then
  echo "==> Using image directory: $INPUT"
  shopt -s nullglob nocaseglob
  cp "$INPUT"/*.{jpg,jpeg,png} "$WORK/images/" 2>/dev/null || true
  shopt -u nullglob nocaseglob
elif [ -f "$INPUT" ]; then
  echo "==> Extracting frames from video at ${FPS} fps"
  ffmpeg -hide_banner -loglevel warning -i "$INPUT" -vf "fps=${FPS}" -qscale:v 2 "$WORK/images/%05d.jpg"
  SEQUENTIAL=1     # video frames are ordered -> sequential matcher is faster
else
  echo "Input not found: $INPUT"; exit 1
fi

N=$(find "$WORK/images" -maxdepth 1 -type f | wc -l)
echo "==> $N images ready"
[ "$N" -ge 20 ] || echo "    WARNING: <20 images; SfM may fail. Aim for ~100-300."

DB="$WORK/database.db"

echo "==> [COLMAP 1/4] feature extraction (CPU, single camera)"
colmap feature_extractor \
  --database_path "$DB" --image_path "$WORK/images" \
  --ImageReader.single_camera 1 \
  --FeatureExtraction.use_gpu 0 \
  --FeatureExtraction.max_image_size "$MAX_SIZE"

echo "==> [COLMAP 2/4] feature matching (CPU)"
if [ "$SEQUENTIAL" -eq 1 ]; then
  colmap sequential_matcher --database_path "$DB" --FeatureMatching.use_gpu 0
else
  colmap exhaustive_matcher  --database_path "$DB" --FeatureMatching.use_gpu 0
fi

echo "==> [COLMAP 3/4] sparse mapping (Structure-from-Motion)"
mkdir -p "$WORK/sparse"
colmap mapper --database_path "$DB" --image_path "$WORK/images" --output_path "$WORK/sparse"
[ -d "$WORK/sparse/0" ] || { echo "ERROR: COLMAP found no model. Check overlap/sharpness of captures."; exit 1; }

echo "==> [COLMAP 4/4] undistort to a clean PINHOLE dataset"
colmap image_undistorter \
  --image_path "$WORK/images" --input_path "$WORK/sparse/0" \
  --output_path "$WORK/undistorted" --output_type COLMAP
# Brush / standard 3DGS expect the model under sparse/0/ :
mkdir -p "$WORK/undistorted/sparse/0"
mv "$WORK/undistorted/sparse/"*.bin "$WORK/undistorted/sparse/0/" 2>/dev/null || true

# Brush resolves a RELATIVE --export-path against the dataset's PARENT dir,
# so pass an absolute path to avoid surprises.
EXPORT_DIR="$(cd "$WORK" && pwd)/exports"
echo "==> [Brush] training on Vulkan (Radeon 860M / RADV)"
echo "    (.ply checkpoints every ${EXPORT_EVERY} steps -> $EXPORT_DIR/)"
VIEWER_ARGS=()
if [ "${VIEWER:-0}" = "1" ]; then VIEWER_ARGS+=(--with-viewer); echo "    VIEWER=1 -> opening live training window"; fi
brush "$WORK/undistorted" \
  --export-path "$EXPORT_DIR/" \
  --export-every "$EXPORT_EVERY" \
  "${VIEWER_ARGS[@]}"
# Length: --total-train-iters N (default 30000).  Cap splats: --max-splats N (default 10M).

echo
echo "============================================================"
echo "DONE.  Splat files: $EXPORT_DIR/*.ply"
echo "View / clean (crop floaters, compress) in your browser:"
echo "   https://superspl.at/editor   — just drag the .ply in"
echo "============================================================"
