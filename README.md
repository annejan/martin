# martin — CUDA-free Gaussian-splat morphing on AMD

[![build](https://github.com/annejan/evoke-martin/actions/workflows/build.yml/badge.svg)](https://github.com/annejan/evoke-martin/actions/workflows/build.yml)

A standalone **Bevy + Vulkan** demo that morphs 3D Gaussian splats into one another —
a title, two faces, a dog — entirely **without CUDA or ROCm** (CPU + Vulkan / Mesa
RADV; built for an AMD Ryzen AI 7 PRO 350 / Radeon 860M on openSUSE Tumbleweed).

```bash
cargo +nightly run --release     # a splat assembles out of a ball cloud
```

See **[`USAGE.md`](USAGE.md)** for the full env-var reference and the `MARTIN_SEQ`
timeline. The repo also ships the CUDA-free **splat-creation pipeline** that produces
the `.ply` assets the demo renders:

## Splat-creation pipeline (`pipeline/`)

| Script | What it does |
|---|---|
| `pipeline/splat-setup.sh` | One-time: installs COLMAP build deps via `zypper`, builds **COLMAP** (CUDA off) and **Brush** (wgpu/Vulkan), symlinks `~/.local/bin/brush`. |
| `pipeline/splat.sh` | `video \| image-dir` → ffmpeg frames → COLMAP CPU SfM + undistort → **Brush** training → `.ply`. |

### Usage

```bash
./pipeline/splat-setup.sh                 # once
./pipeline/splat.sh my_video.mp4          # or:  ./pipeline/splat.sh ./photos/
VIEWER=1 ./pipeline/splat.sh ./photos/    # watch training live in Brush's window
```

Tunables (env): `FPS`, `MAX_SIZE`, `EXPORT_EVERY`, `VIEWER`.
View / clean / compress the resulting `.ply` at <https://superspl.at/editor>.

## Making your own splats — a capture recipe 🐕🌿

You don't need a fancy rig — a phone camera is enough. **Capture quality is 90% of
the result**, so the tips below matter more than any setting. (Share this part with
anyone who wants to splat their dog, their garden, or grandma's statue.)

### 1. Shoot the photos/video

The golden rule: **you move around the subject; the subject stays still.** You're
giving the computer many views of the same thing so it can work out the 3D shape.

- **Coverage.** Walk a full circle around the subject (or more than one — a low
  circle *and* a higher one looking slightly down). For an object you can also put
  it on a turntable/lazy-Susan and spin *it* while you hold still. Aim for **40–150
  photos** or a slow **30–60 s video** (the script pulls frames from video).
- **Overlap.** Each shot should overlap the last by ~70% — small steps, not big
  jumps. Think "shuffle sideways", not "teleport".
- **Light.** Flat, even light is your friend — an **overcast day outdoors is
  perfect**. Avoid hard shadows and harsh sun. Lock your exposure/focus if your
  phone lets you (tap-and-hold), so brightness doesn't jump between frames.
- **Hold still-ish.** Blurry frames hurt. Good light → fast shutter → sharp frames.
- **What breaks it:** anything that *moves or changes* between shots — wind in
  leaves, a wagging tail, ripples, passing people, your own shadow. Also **shiny,
  reflective, or transparent** things (glass, water, chrome, wet noses) and big
  blank surfaces (clear sky, a plain white wall) — the computer needs *texture* to
  lock onto. A cluttered, textured background actually *helps*.

- 🐕 **Dogs / animals** are hard because they move. Best options, in order:
  a **sleeping/very calm** dog; **burst mode** and shoot a full circle fast; or a
  **toy/figurine/statue** of the breed (trivial — put it on a turntable). Get down
  to their eye level, and grab a few from above and below.
- 🌿 **Nature / scenes** (a tree, a rock, a garden corner): pick a **calm, overcast,
  windless** moment, lock exposure, and arc around the subject. Avoid pointing at
  bright sky.

### 2. Turn the photos into a splat (all CUDA-free, on this machine)

```bash
./pipeline/splat-setup.sh                 # once: builds COLMAP + Brush
./pipeline/splat.sh my_dog_video.mp4      # or:  ./pipeline/splat.sh ./my_photos/
VIEWER=1 ./pipeline/splat.sh ./my_photos/ # watch it train live
```

Out comes a `.ply`. (No good photo set yet, or only **one** image? Drop it into
**[TRELLIS](https://huggingface.co/spaces/trellis-community/TRELLIS)** in your
browser for a quick single-image splat — see the aesthetic note below.)

### 3. Tidy it up (browser, free)

Open the `.ply` at **<https://superspl.at/editor>**: box-select and **delete stray
"floater" splats** and the background, **recenter** the subject at the origin, and
scale it to a sane size. **Export as uncompressed / standard PLY** (the demo's
loader rejects SuperSplat's *compressed* format).

### 4. Drop it in the demo

```bash
MARTIN_PLY=assets/your.ply cargo run --release
```

For the **morph** (sources → target), prep all the splats the *same way*: same
up-axis, centred, similar overall size, and a **consistent gaussian character**
(see below). Mismatched assets blend muddily.

### Two flavours of splat (and the "PS1" look)

- **TRELLIS / single-image (HF):** many **small, opaque** splats → a crisp, slightly
  *hard* surface that gives a charming **PS1 texture-warp** wobble when morphed. No
  data for unseen sides, though → a **hollow back** (which is why the demo's camera
  only sways across the front instead of orbiting 360°).
- **Brush / photo-capture (local):** **fewer, bigger, semi-transparent** splats that
  blend → a softer, more photographic, *volumetric* look that **dissolves** rather
  than warps. Full multi-angle capture = **full 360°**, so you can orbit all the way
  around.

Pick one vibe per scene and keep a morph set consistent. Brush also lets you tune
**densification** to hit a gaussian budget — ~250k–500k stays a smooth 60 fps on the
iGPU (the full ~1.15M runs ~20 fps).

## Running the demo

Loads Gaussian splats, flies a camera around them, and **morphs them into one another**
— each splat assembles out of a ball cloud, then the next morphs in (per-Gaussian, on the
GPU), with HDR bloom on black. It's all one **sequence engine**; the `MARTIN_*` env vars
compose the show. Built on Bevy 0.18 + `bevy_gaussian_splatting` 7.0.1 (vendored fork in
`vendor/`), wgpu → Vulkan (nightly toolchain, pinned).

```bash
cargo run                          # window: a splat assembles from a ball cloud
#   ↑/↓ zoom · ←/→ raise/lower · Space = restart · F11/F = fullscreen
./record.sh out.mp4                # render the whole timeline to ./out.mp4
```

By default the splat loads from `assets/aegg.ply`; point it at any file with
`MARTIN_PLY=assets/your.ply cargo run --release`. Add
`MARTIN_PLY2=second.ply` (same folder) for a **second splat beside it**, and
`MARTIN_REFORM=dog.ply` so the source splat(s) **morph into that one** — a
per-Gaussian `GaussianInterpolate` blend where each source is paired to the target by
**Morton (Z-order) spatial sort**, so particles *flow* into their nearest part of the
target (no teleporting) and colours/positions lerp together (e.g. two Martins → one
dog: each becomes a half of the dog). A front-facing camera sway keeps the hollow back
of single-image splats out of frame (`MARTIN_YAW=<rad>` pins the angle for inspection).
**Export
uncompressed/standard PLY from SuperSplat** — the loader rejects SuperSplat's
*compressed* format (`missing required properties`). Linux build deps:
`systemd-devel` (libudev) + alsa (and a Vulkan/RADV driver).

**Get a subject splat:** capture with `pipeline/splat.sh` (COLMAP→Brush), or generate one
from a single image with **[TRELLIS](https://huggingface.co/spaces/trellis-community/TRELLIS)**
(image → 3DGS `.ply`, runs in the browser) — drop it in `assets/` and
`MARTIN_PLY=assets/dog.ply cargo run --release`.

**Prebuilt binaries:** GitHub Actions builds release binaries for **Linux,
Windows, and macOS** on every push — grab them from the artifacts of the latest
[build run](https://github.com/annejan/evoke-martin/actions/workflows/build.yml).
Release binary is ~75 MB (`strip` + thin LTO); use `cargo run --release` to show
it off (the 1.8 GiB build is debug-only, for fast iteration).

### Effects & env vars — mix and match

Everything is driven by env vars; combine them to taste. All splat positions/scales
are particles in the *same* system, so any of these morphs into any other.

| Env var | Effect |
|---|---|
| `MARTIN_PLY=/abs/x.ply` | Load a splat (sets the asset folder for the others). |
| `MARTIN_PLY2=y.ply` | A second splat beside the first (the two morph together). |
| `MARTIN_REFORM=dog.ply` | The source(s) **morph** into this one (Morton-paired particle flow). |
| `MARTIN_TEXT="MARTIN GAUS"` | **Splat-text**: the title assembles out of a ball cloud (glowing). |
| `MARTIN_SEQ="…"` | **Timeline** — a chain of beats that morph into one another (see below). |
| `MARTIN_BULGE=0.9` | Ball-cloud explosiveness at a morph's midpoint (`0` = clean reorder). |
| `MARTIN_MORPH_COUNT=250000` | Gaussian budget (`0`=max ~1.15M ≈ 20 fps; 250k ≈ 60 fps on the iGPU). |
| `MARTIN_YAW=1.4` | Pin the camera angle (no sway). |
| `MARTIN_FPS=1` | Log frame time / FPS. |
| `MARTIN_RECORD=/dir` | Dump one PNG per frame (used by `record.sh`). |
| `MARTIN_SHOT=/x.png` `MARTIN_SHOT_AT=<s>` | Headless screenshot at time `s`, then exit. |
| `MARTIN_FULLSCREEN=1` | Start borderless-fullscreen; toggle live with **F11 / F**. |

**`MARTIN_SEQ`** is a `;`-separated list of *beats* (or a path to a file of them, one
per line; `#` comments allowed). Each beat morphs into the next, through a ball cloud:

```
text:STRING               # splat-text (glowing)
splat:a.ply               # a splat (filename in the MARTIN_PLY folder)
splat:a.ply+b.ply         # several splats, auto-arranged side by side
…any beat… @hold,morph,bulge   # optional per-beat timing (seconds) + ball amount
```

Example — the full show (title → dog → greetings → credits):

```bash
MARTIN_PLY=assets/doggo.ply \
MARTIN_SEQ="text:MARTIN GAUS @2,3,0; splat:doggo.ply @2,3,0.9; text:GREETINGS @1.5,3,0.9; text:CODE ANNEJAN @2,3,0.6" \
cargo +nightly run --release
#   ./record.sh out.mp4   renders the whole timeline to video
```

## Note on git

Splats, captures, run outputs, and the external COLMAP/Brush checkouts are
**git-ignored** (multi-GB binaries). Only source/tools are tracked.
