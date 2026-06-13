# martin — deFEEST 3D splatting engine

[![build](https://github.com/annejan/evoke-martin/actions/workflows/build.yml/badge.svg)](https://github.com/annejan/evoke-martin/actions/workflows/build.yml)

A standalone **Bevy + Vulkan** demo that flies a camera around 3D Gaussian splats while they
**morph into one another** — a title, two faces, a dog — entirely **without CUDA or ROCm**.
Developed and previewed on a modest **AMD Ryzen AI 7 PRO 350 / Radeon 860M iGPU** (CPU + Vulkan /
Mesa RADV, openSUSE Tumbleweed); the released demo targets bigger metal at showtime, so it can afford
heavier clouds and the `sh3` view-dependent profile (see [build profiles](#build-profiles)). Demoscene
spirit, all-AMD dev box. 🪩

```bash
cargo +nightly run --release     # a splat assembles out of a ball cloud
#   free-orbit: ←/→ yaw · ↑/↓ pitch · W/S zoom · A/D & Q/E pan · M mark waypoint · Space restart · F11/F fs
./record.sh out.mp4              # render the whole timeline to ./out.mp4
```

It's **one sequence engine**: every run is a timeline of *parts* (splat-text or splats) that
each assemble out of a ball cloud, then morph into the next (per-Gaussian, on the GPU), with
HDR bloom on black. The `MARTIN_*` env vars compose the show — there's no config file. Built on
Bevy 0.18 + `bevy_gaussian_splatting` 7.0.2 (our fork — the `martin` branch of
[`annejan/bevy_gaussian_splatting`](https://github.com/annejan/bevy_gaussian_splatting),
a git dep), wgpu → Vulkan, nightly toolchain (the `nightly` channel, unpinned — we ride
current nightly).

- **[`USAGE.md`](USAGE.md)** — the full env-var reference and the `MARTIN_SEQ` timeline.
- **[`ART-DIRECTION.md`](ART-DIRECTION.md)** — how to **shoot and prep good splats** for the
  demo (capture recipe, lighting, the two splat "flavours", cleanup).
- **[`productions/`](productions/README.md)** — one folder per demo (showbook, show, bundle,
  captures); the engine stays theme-agnostic. Current: **camping** (in design — see its
  [SHOWBOOK](productions/camping/SHOWBOOK.md)). Shared building blocks: [`parts/`](parts/README.md).

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
| `pipeline/mesh-splat.sh` | A **mesh** (`.obj`/`.dae`/`.stl`/`.ply`/`.glb`) → Blender (EEVEE) orbital renders with *known* poses (no COLMAP) → **Brush** training → a "proper" `.ply`. The offline bake for when a mesh matters (vs the in-engine `mesh:` sampler). Needs Blender (`BLENDER=blender-5.0`). |

**Capture quality is 90% of the result** — see **[`ART-DIRECTION.md`](ART-DIRECTION.md)** for
the full recipe (and the single-image **[TRELLIS](https://huggingface.co/spaces/trellis-community/TRELLIS)**
shortcut). View / clean / compress any `.ply` at <https://superspl.at/editor>.

### Ready-made demo splats

Two reference clouds from Mitchell Mosure (the upstream author) — fetched on demand (they're >100 MB
so they're gitignored, not committed):

```bash
./pipeline/fetch-demo-assets.sh    # → assets/go_trimmed.ply + assets/trellis.glb
MARTIN_PLY=assets/go_trimmed.ply cargo r-sh3                     # multi-view photogrammetry (SH3 glints)
MARTIN_GLB=assets/trellis.glb MARTIN_GLB_DIST=2.6 cargo r-sh0    # a KHR_gaussian_splatting glTF scene
```

The go-board is a real multi-view capture (its `sh3` glint is visible — `cargo r-sh3`); the trellis is a
single-image TRELLIS export (flat SH, so `sh0` loses nothing). See [build profiles](#build-profiles).

## Running the demo

With nothing set, `cargo +nightly run --release` plays the **intro production**
([`productions/intro/intro.show`](productions/intro/intro.show)) — the very same showcase CI bakes
into the downloadable single binary, so a fresh `git clone` runs exactly what the download shows. Its
procedural splats are synthesized by `build.rs` on first build (no python/numpy), so the clone needs
no extra step. (The older effect-catalogue demo still lives at [`assets/demo.show`](assets/demo.show),
built only from shipped assets — `MARTIN_SHOW=assets/demo.show cargo run --release`.) Point it
at your own splat with `MARTIN_PLY=assets/your.ply cargo run --release`. Add `MARTIN_PLY2=second.ply` (same folder)
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
| `MARTIN_SHOW=show.show` | **Unified scene file** — one file with settings + a `[seq]` + a `[compose]` stage + a music-timed `[camera]` track (keyframes can anchor to a music section, `t=@@drop`). Expands into the env vars below (which still override it). The recommended way to author a whole show; see `assets/example.show`. |
| `MARTIN_VALIDATE=1` | **Dry-run** — parse the show, print the resolved timeline (part cue times, effects, compose, camera) and exit, no render. A fast authoring check. |
| `MARTIN_PLY=/abs/x.ply` | Load a splat (sets the asset folder for the others). |
| `MARTIN_PLY2=y.ply` | A second splat beside the first (the two morph together). |
| `MARTIN_REFORM=dog.ply` | The source(s) **morph** into this one (Morton-paired particle flow). |
| `MARTIN_TEXT="MARTIN GAUS"` | **Splat-text**: the title assembles out of a ball cloud (glowing). |
| `MARTIN_SEQ="…"` | **Timeline** — a chain of parts that morph into one another (see below). |
| `MARTIN_COMPOSE=stage.compose` | **Composition** — many objects on one stage at once, placed + spinning/bobbing/drifting, fading in on the music, camera auto-orbiting (vs the morph timeline). Example: `assets/examples/stage.show`. |
| `MARTIN_FPS=1` / **`I`** key | Log FPS + splat count (the `I` key toggles it live + logs a snapshot). |
| `MARTIN_BULGE=0.9` | Ball-cloud explosiveness at a morph's midpoint (`0` = clean reorder). |
| `MARTIN_TRANSITION=fade` | How each part **arrives**: `morph`/`swarm`/`ball`/`fade`/`explode`/`implode`/`drop`/`swirl`, or the shader ones `typewriter`/`wipe`/`sparkle`/`slither`/`vortex`/`outline`/`pen-write` (per-part `~name` wins). `swarm` = like `morph` but the splats flock along curled paths *between* the two scenes (the `@_,_,N` value tunes the swarm strength). |
| `MARTIN_DEFORM=wave` | A **scene-wide persistent deform** field held the whole part (`wave`/`cloth`/`ripple`/`twist`/`wind`/`turbulence`/`pulse`/`jitter`/`spiral`) — great on a `wall:` of text, or to **gently wobble a whole splat scene** while you fly around it; applies to compose objects too. Per-part `^name` wins. |
| `MARTIN_DEFORM_AMP=0.3` `MARTIN_DEFORM_SPEED=1` | Tune the deform: amplitude scale (`0.3` ≈ a gentle wobble on a big scene; `1` = default) and animation rate. |
| `MARTIN_MESH_COUNT=60000` | A `mesh:model.dae` part (`.dae`/`.obj`/`.stl`/`.ply`) is surface-sampled into this many **flat, normal-aligned** gaussians, coloured from the diffuse texture (sampled at the UV), else vertex/material colour, else `MARTIN_MESH_RGB`. `MARTIN_MESH_SPLAT` = in-plane disk size; `MARTIN_MESH_THIN` = thickness (default 0.2× the radius). |
| `MARTIN_MORPH_COUNT=250000` | Gaussian budget (`0`=max ~1.15M ≈ 20 fps; 250k ≈ 60 fps on the iGPU). |
| `MARTIN_NORMALIZE=0` | Disable per-part centring + robust scale-to-common-size (on by default). |
| `MARTIN_ZOOM=1.5` | Camera closeness (`>1` = closer / more zoomed in, `<1` = pull back). |
| `MARTIN_ROT=rx,ry,rz` | Orient the cloud (euler degrees) — e.g. stand a COLMAP scene upright. |
| `MARTIN_YAW=1.4` `MARTIN_PITCH=0.1` | Seed the free-orbit camera angle (radians); `MARTIN_YAW` also holds it (no sway) when recording. |
| `MARTIN_WAYPOINTS=path.json` | Where the **M-key** camera waypoints are logged / read (default `waypoints.json`) — fly + mark to author a camera path. |
| `MARTIN_FLY=2` | Fly the camera through the marked waypoints. Recording: the path fills each part (longer hold = slower flyby), flowing through the morph. Live: `<secs>` = pace. |
| `MARTIN_FPS=1` | Log frame time / FPS. |
| `MARTIN_RECORD=/dir` | Dump one PNG per frame (used by `record.sh`). |
| `MARTIN_PREVIEW_FPS=8` | Render the timeline at N fps instead of 60 — far fewer frames for a fast preview (rendering frames is the slow part). Timing + audio sync stay correct; `record.sh` muxes at the same fps. |
| `MARTIN_RASTER=position` | Debug-shading view for the whole show (`color`/`depth`/`normal`/`position`/`classification`/`flow`/`velocity`) — the fork's RasterizeMode. Per-part `raster:<mode>` token overrides it. `position` colours by XYZ (rainbow). |
| `MARTIN_SHOT=/x.png` `MARTIN_SHOT_AT=<s>` | Headless screenshot at time `s`, then exit. |
| `MARTIN_FULLSCREEN=1` | Start borderless-fullscreen; toggle live with **F11 / F**. |
| `MARTIN_FLASH=0.6` | Over-bright bloom flash on each part cut (0 = off). |
| `MARTIN_SYNTH_WAV=/x.wav` | Render the bundled deFEEST synth to a WAV and exit (mux onto a recording). |
| `MARTIN_MUTE=1` | Silence the live synth (it plays in the window by default; recordings still mux the WAV). |
| `MARTIN_SCORE=score.txt` | Load a tracker-DSL **score file** (tempo / sections / drum patterns / dynamics) — drives the synth *and* the `@@anchor`s. Editable default ships at `assets/score.txt`. |
| `MARTIN_SCORE_DUMP=score.txt` | Export the built-in score as an editable file and exit. |

**`MARTIN_SEQ`** is a `;`-separated list of *parts* (or a path to a file of them, one per line;
`#` comments allowed). Each part morphs into the next, through a ball cloud:

```
text:STRING               # splat-text (glowing)
image:logo.png            # a PNG (in the MARTIN_PLY folder), rasterized to gaussians
svg:logo.svg              # an SVG, rasterized (vector → pixels) into gaussians — any vector art
mesh:model.dae            # a 3D mesh (.dae/.obj/.stl/.ply), surface-sampled into gaussians
glb:badge.glb             # a real glTF mesh: rendered crisp, then DISSOLVES into its own splats
shader:warp               # a fullscreen-effect INTERLUDE (warp/plasma/tunnel/stars/rings/grid/kaleido/bolt); splats clear
splat:a.ply               # a splat (filename in the MARTIN_PLY folder)
splat:a.ply+b.ply         # several splats, auto-arranged side by side
…any part… @hold,morph,bulge ~transition ^deform out:departure @@anchor   # timing · arrival · deform · departure · cue
```

The trailing `~transition` picks how a part arrives — data-only `ball` (default), `fade`,
`explode`, `implode`, `drop`, `swirl`, `extrude`, `helix`, `fold`, `zoom`, `morph`, or the per-particle shader transitions
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

## Build profiles

The **spherical-harmonic degree** is a compile-time choice (one-hot in the splat crate), exposed as two
build profiles. Synthetic content (text/morph) looks identical in both — only real captures differ:

| profile | what | when |
| :-- | :-- | :-- |
| **`sh0`** (default) | one flat colour per splat — lean, fast | the synthetic demo; the AMD iGPU dev box |
| **`sh3`** | full degree-3 **view-dependent** colour (specular glint) | real captures (camping footage → Brush); the showtime machine |

```bash
cargo b-sh0      # = cargo build --release            → target/release/martin    (sh0, default)
cargo b-sh3      # sh3 build into target/sh3/release/martin (both binaries coexist; r-sh0 / r-sh3 to run)
```

`sh3` costs ~16× the per-splat colour data in VRAM **for captures** (synthetic clouds compress that away),
so the lean `sh0` stays the default; flip to `sh3` once camping captures land. The aliases live in
[`.cargo/config.toml`](.cargo/config.toml).

## Single-binary bundle

Ship a whole show as **one self-contained executable** — assets baked in, no files, no env vars:

```bash
./pipeline/bundle.sh            # or: cargo build --release --features bundle
./target/release/martin         # runs the baked-in show anywhere
```

`cargo build --release --features bundle` *is* the pipeline: `build.rs` reads **`bundle.toml`**
(the show — a `seq`/`compose` + optional `score`, `logo`, `morph_count`), auto-collects every
`.ply`/PNG the show references, lz4-compresses them into the binary, and bakes the show string in.
At startup the binary self-extracts the assets to a temp dir (reused across relaunches) and plays
the show, with a loader screen (logo + progress bar) while the splats decompress. Env vars still
override the baked-in defaults (e.g. `MARTIN_LOOP=1`). Fonts and the default score are already
compiled into martin, so only splats (and any logo PNG) ship. Edit `bundle.toml` to pick the show;
its `.ply` must be present locally at build time (they're git-ignored).

## Releasing

`./pipeline/release.sh` builds a **single self-contained binary** (the show + all its assets baked
in) from `bundle.toml`, then verifies it self-extracts and plays:

```bash
./pipeline/release.sh                 # → target/release/martin (one file, no assets, no env)
./target/release/martin               # plays the baked-in show anywhere
```

CI bakes the **intro** production into the download (`productions/intro/bundle.toml` → `intro.show`);
the root `bundle.toml` defaults to `assets/demo.show`. Either stays self-contained by using only
*light* assets — procedural demo shapes + a few tracked meshes + text, no 500 MB photogrammetry
scenes. A bundle lands around ~180 MB (Bevy base ~75 MB + lz4-compressed splats). To shrink it: ship
fewer / **downsampled** `.ply` (the real lever — splat floats dominate), or trim the show.

**Portability (run on other distros).** A binary linked against this dev box's glibc (openSUSE
Tumbleweed = bleeding-edge) fails on older distros with `version 'GLIBC_2.xx' not found`. So
`release.sh` links against an **old glibc** via [`cargo-zigbuild`](https://github.com/rust-cross/cargo-zigbuild)
when `zig` + `cargo-zigbuild` are on `PATH` — built here, runs on Ubuntu 20.04+/Debian 11+/Mint 20+
(glibc ≥ `TARGET_GLIBC`, default 2.31; the GPU/audio/window libs are dlopen'd at runtime and present
on any desktop). One-time setup: `cargo install cargo-zigbuild` and put `zig` on `PATH`. Cross-*OS*
(Windows/macOS): run `release.sh` on each, or use GitHub Actions, then `gh release upload`.

## Note on git

Splats, captures, run outputs, and the external COLMAP/Brush checkouts are **git-ignored**
(multi-GB binaries). Only source/tools are tracked.
