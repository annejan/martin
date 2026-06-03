#!/usr/bin/env bash
# ============================================================================
# splat-setup.sh  —  one-time toolchain build for 3D Gaussian Splatting
# Target: openSUSE Tumbleweed, AMD Ryzen AI 7 PRO 350 / Radeon 860M (gfx1152).
#
# Strategy (verified for this box): NO CUDA, NO ROCm.
#   * Poses  -> COLMAP built CPU-only (CUDA_ENABLED=OFF). 16 cores handle it.
#   * Train  -> Brush (Rust + wgpu -> Vulkan/RADV). Runs natively on the iGPU.
# All COLMAP deps (incl. Ceres = libceres-devel) are in the Tumbleweed repos.
# ============================================================================
set -euo pipefail

BASE="${BASE:-$HOME/Projects/martin/splat-tools}"
mkdir -p "$BASE" "$HOME/.local/bin"

echo "==> 1/4  Sanity check: Vulkan device (Brush needs this)"
if vulkaninfo --summary 2>/dev/null | grep -q RADV; then
  vulkaninfo --summary 2>/dev/null | grep -m1 deviceName | sed 's/^/    /'
else
  echo "    WARNING: no RADV Vulkan device seen."
  echo "    Fix: sudo zypper install libvulkan_radeon vulkan-tools && reboot"
fi

echo "==> 2/4  Installing COLMAP build dependencies (sudo zypper)"
sudo zypper install -y \
  git cmake ninja gcc-c++ \
  libboost_headers-devel libboost_program_options-devel libboost_filesystem-devel \
  libboost_graph-devel libboost_test-devel \
  eigen3-devel freeimage-devel OpenImageIO-devel libceres-devel glog-devel gflags-devel-static \
  sqlite3-devel glew-devel Mesa-libGL-devel \
  qt6-base-devel qt6-opengl-devel qt6-widgets-devel qt6-gui-devel \
  metis-devel suitesparse-devel cgal-devel flann-devel liblz4-devel

echo "==> 3/4  Building COLMAP (CUDA OFF) — latest release tag"
if command -v colmap >/dev/null 2>&1; then
  echo "    colmap already installed: $(command -v colmap)"
else
  cd "$BASE"
  [ -d colmap ] || git clone https://github.com/colmap/colmap.git
  cd colmap
  git fetch --tags --quiet || true
  latest_tag="$(git describe --tags "$(git rev-list --tags --max-count=1)" 2>/dev/null || true)"
  if [ -n "$latest_tag" ]; then echo "    checking out $latest_tag"; git checkout -q "$latest_tag"; fi
  cmake -B build -GNinja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCUDA_ENABLED=OFF \
    -DGUI_ENABLED=ON \
    -DTESTS_ENABLED=OFF
  ninja -C build
  sudo ninja -C build install
fi

echo "==> 4/4  Building Brush (Vulkan trainer; binary 'brush' lives in app crate brush-app)"
cd "$BASE"
[ -d brush ] || git clone https://github.com/ArthurBrussee/brush.git
cd brush
cargo build --release -p brush-app          # brush-cli is a lib; brush-app -> [[bin]] name="brush"
BRUSH_BIN="$(find "$PWD/target/release" -maxdepth 1 -type f -executable -name brush | head -n1)"
[ -n "$BRUSH_BIN" ] || { echo "ERROR: built brush binary not found"; exit 1; }
ln -sf "$BRUSH_BIN" "$HOME/.local/bin/brush"
echo "    linked $BRUSH_BIN -> ~/.local/bin/brush"

echo
echo "============================================================"
echo "DONE."
case ":$PATH:" in
  *":$HOME/.local/bin:"*) : ;;
  *) echo 'NOTE: add ~/.local/bin to PATH:  echo '\''export PATH="$HOME/.local/bin:$PATH"'\'' >> ~/.bashrc' ;;
esac
echo "Next:  ./splat.sh my_video.mp4        # or"
echo "       ./splat.sh path/to/images/"
echo "============================================================"
