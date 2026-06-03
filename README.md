# martin — CUDA-free Gaussian-splat morphing on AMD

[![build](https://github.com/annejan/evoke-martin/actions/workflows/build.yml/badge.svg)](https://github.com/annejan/evoke-martin/actions/workflows/build.yml)

A standalone **Bevy + Vulkan** demo that flies a camera around 3D Gaussian splats while they
**morph into one another** — a title, two faces, a dog — entirely **without CUDA or ROCm**
(CPU + Vulkan / Mesa RADV; built for an AMD Ryzen AI 7 PRO 350 / Radeon 860M on openSUSE
Tumbleweed). Demoscene spirit, all-AMD metal. 🪩

```bash
cargo +nightly run --release     # a splat assembles out of a ball cloud
#   free-orbit: ←/→ yaw · ↑/↓ pitch · W/S zoom · A/D & Q/E pan · M mark waypoint · Space restart · F11/F fs
./record.sh out.mp4              # render the whole timeline to ./out.mp4
```

It's **one sequence engine**: every run is a timeline of *parts* (splat-text or splats) that
each assemble out of a ball cloud, then morph into the next (per-Gaussian, on the GPU), with
HDR bloom on black. The `MARTIN_*` env vars compose the show — there's no config file. Built on
Bevy 0.18 + `bevy_gaussian_splatting` 7.0.1 (vendored fork in `vendor/`), wgpu → Vulkan,
nightly toolchain (pinned).

- **[`USAGE.md`](USAGE.md)** — the full env-var reference and the `MARTIN_SEQ` timeline.
- **[`ART-DIRECTION.md`](ART-DIRECTION.md)** — how to **shoot and prep good splats** for the
  demo (capture recipe, lighting, the two splat "flavours", cleanup).

## Make your own splats

The repo ships a **CUDA-free splat-creation pipeline** (`pipeline/`) that turns a phone video
or a folder of photos into the `.ply` assets the demo renders:

```bash
./pipeline/splat-setup.sh                 # once: builds COLMAP (CPU) + Brush (Vulkan)
./pipeline/splat.sh my_video.mp4          # or:  ./pipeline/splat.sh ./photos/
VIEWER=1 ./pipeline/splat.sh ./photos/    # watch training live in Brush's window
```

| Script | What it does |
|---|---|
| `pipeline/splat-setup.sh` | One-time: installs COLMAP build deps via `zypper`, builds **COLMAP** (CUDA off) and **Brush** (wgpu/Vulkan), symlinks `~/.local/bin/brush`. |
| `pipeline/splat.sh` | `video \| image-dir` → ffmpeg frames → COLMAP CPU SfM + undistort → **Brush** training → `.ply`. |

**Capture quality is 90% of the result** — see **[`ART-DIRECTION.md`](ART-DIRECTION.md)** for
the full recipe (and the single-image **[TRELLIS](https://huggingface.co/spaces/trellis-community/TRELLIS)**
shortcut). View / clean / compress any `.ply` at <https://superspl.at/editor>.

## Running the demo

By default the splat loads from `assets/aegg.ply`; point it at any file with
`MARTIN_PLY=assets/your.ply cargo run --release`. Add `MARTIN_PLY2=second.ply` (same folder)
for a **second splat beside it**, and `MARTIN_REFORM=dog.ply` so the source splat(s) **morph
into that one** — a per-Gaussian `GaussianInterpolate` blend where each source is paired to the
target by **Morton (Z-order) spatial sort**, so particles *flow* into their nearest part of the
target (no teleporting) and colours/positions lerp together (e.g. two Martins → one dog: each
becomes a half of the dog). A front-facing camera sway keeps the hollow back of single-image
splats out of frame (`MARTIN_YAW=<rad>` pins the angle for inspection). **Export
uncompressed/standard PLY from SuperSplat** — the loader rejects the *compressed* format
(`missing required properties`). Linux build deps: `systemd-devel` (libudev) + alsa (and a
Vulkan/RADV driver).

**Prebuilt binaries:** GitHub Actions builds release binaries for **Linux, Windows, and macOS**
on every push — grab them from the artifacts of the latest
[build run](https://github.com/annejan/evoke-martin/actions/workflows/build.yml). The release
binary is ~75 MB (`strip` + thin LTO); use `cargo run --release` to show it off (the 1.8 GiB
build is debug-only, for fast iteration).

### Effects & env vars — mix and match

Everything is driven by env vars; combine them to taste. All splat positions/scales are
particles in the *same* system, so any of these morphs into any other. Full reference in
**[`USAGE.md`](USAGE.md)**.

| Env var | Effect |
|---|---|
| `MARTIN_PLY=/abs/x.ply` | Load a splat (sets the asset folder for the others). |
| `MARTIN_PLY2=y.ply` | A second splat beside the first (the two morph together). |
| `MARTIN_REFORM=dog.ply` | The source(s) **morph** into this one (Morton-paired particle flow). |
| `MARTIN_TEXT="MARTIN GAUS"` | **Splat-text**: the title assembles out of a ball cloud (glowing). |
| `MARTIN_SEQ="…"` | **Timeline** — a chain of parts that morph into one another (see below). |
| `MARTIN_BULGE=0.9` | Ball-cloud explosiveness at a morph's midpoint (`0` = clean reorder). |
| `MARTIN_TRANSITION=fade` | How each part **arrives**: `morph`/`ball`/`fade`/`explode`/`implode`/`drop`/`swirl`, or the shader ones `typewriter`/`wipe`/`sparkle`/`slither`/`vortex`/`outline`/`pen-write` (per-part `~name` wins). |
| `MARTIN_MORPH_COUNT=250000` | Gaussian budget (`0`=max ~1.15M ≈ 20 fps; 250k ≈ 60 fps on the iGPU). |
| `MARTIN_NORMALIZE=0` | Disable per-part centring + robust scale-to-common-size (on by default). |
| `MARTIN_ZOOM=1.5` | Camera closeness (`>1` = closer / more zoomed in, `<1` = pull back). |
| `MARTIN_ROT=rx,ry,rz` | Orient the cloud (euler degrees) — e.g. stand a COLMAP scene upright. |
| `MARTIN_YAW=1.4` `MARTIN_PITCH=0.1` | Seed the free-orbit camera angle (radians); `MARTIN_YAW` also holds it (no sway) when recording. |
| `MARTIN_WAYPOINTS=path.json` | Where the **M-key** camera waypoints are logged / read (default `waypoints.json`) — fly + mark to author a camera path. |
| `MARTIN_FLY=2` | Fly the camera through the marked waypoints. Recording: the path fills each part (longer hold = slower flyby), flowing through the morph. Live: `<secs>` = pace. |
| `MARTIN_FPS=1` | Log frame time / FPS. |
| `MARTIN_RECORD=/dir` | Dump one PNG per frame (used by `record.sh`). |
| `MARTIN_SHOT=/x.png` `MARTIN_SHOT_AT=<s>` | Headless screenshot at time `s`, then exit. |
| `MARTIN_FULLSCREEN=1` | Start borderless-fullscreen; toggle live with **F11 / F**. |
| `MARTIN_FLASH=0.6` | Over-bright bloom flash on each part cut (0 = off). |
| `MARTIN_SYNTH_WAV=/x.wav` | Render the bundled deFEEST synth to a WAV and exit (mux onto a recording). |

**`MARTIN_SEQ`** is a `;`-separated list of *parts* (or a path to a file of them, one per line;
`#` comments allowed). Each part morphs into the next, through a ball cloud:

```
text:STRING               # splat-text (glowing)
image:logo.png            # a PNG (in the MARTIN_PLY folder), rasterized to gaussians
splat:a.ply               # a splat (filename in the MARTIN_PLY folder)
splat:a.ply+b.ply         # several splats, auto-arranged side by side
…any part… @hold,morph,bulge ~transition @@anchor   # timing · arrival transition · music cue
```

The trailing `~transition` picks how a part arrives — data-only `ball` (default), `fade`,
`explode`, `implode`, `drop`, `swirl`, `morph`, or the per-particle shader transitions
`typewriter`, `wipe`, `sparkle`, `slither`, `vortex`, `outline`, `pen-write` (great for text). The ball is just one
of many; the design + the shader fork are in **[`DESIGN.md`](DESIGN.md)** / **[`SHADER-BLUEPRINT.md`](SHADER-BLUEPRINT.md)**.

The optional `@@anchor` pins a part's start to the **music clock** (Cinder's ported synth/score,
`src/{audio,score}.rs`): `@@drop` (a section), `@@bar32`, `@@beat:64`, or `@@12.5` seconds — so
the visuals lock to the track. `MARTIN_SYNTH_WAV` renders that synth to a WAV; mux it onto a
recording with ffmpeg for a video-with-sound (see **[`USAGE.md`](USAGE.md)** → Music).

Example — the full show (title → dog → greetings → credits):

```bash
MARTIN_PLY=assets/doggo.ply \
MARTIN_SEQ="text:MARTIN GAUS @2,3,0; splat:doggo.ply @2,3,0.9; text:GREETINGS @1.5,3,0.9; text:CODE ANNEJAN @2,3,0.6" \
cargo +nightly run --release
#   ./record.sh out.mp4   renders the whole timeline to video
```

## Note on git

Splats, captures, run outputs, and the external COLMAP/Brush checkouts are **git-ignored**
(multi-GB binaries). Only source/tools are tracked.
