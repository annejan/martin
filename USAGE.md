# martin — usage & env vars

`martin` loads Gaussian splats and flies a camera around them as they **morph into one
another** — driven entirely by environment variables. There's no config file: you compose
the show by combining env vars on the command line.

```bash
cargo +nightly run --release        # nightly toolchain is pinned (rust-toolchain.toml)
```

It's **one sequence engine**: every run is a list of *parts* (splats or text) that each
assemble out of a ball cloud and morph into the next. With no env vars it assembles
`assets/aegg.ply` from a ball and holds it.

---

## The show is a sequence (shorthands build one)

Everything is one timeline. `MARTIN_SEQ` writes it explicitly; the other env vars are
**shorthands** that build a sequence for you (first match wins):

| If you set… | The sequence it builds |
|---|---|
| `MARTIN_SEQ` | exactly the parts you write (the full timeline) |
| `MARTIN_TEXT` | one part: that title, assembled from a ball |
| `MARTIN_PLY` (+ `_PLY2`) (+ `_REFORM`) | the splat(s) as part 1; the reform target (if any) as part 2 |
| *(nothing)* | one part: `assets/aegg.ply` |

Examples:

```bash
# Single splat (your own .ply)
MARTIN_PLY=assets/martin.ply cargo +nightly run --release

# Two Martins morph into a dog
MARTIN_PLY=assets/martin-peace.ply MARTIN_PLY2=martin.ply \
MARTIN_REFORM=doggo.ply cargo +nightly run --release

# A glowing title that assembles from particles
MARTIN_TEXT="MARTIN GAUS" cargo +nightly run --release

# A whole show (see "Sequences" below)
MARTIN_PLY=assets/doggo.ply \
MARTIN_SEQ="text:MARTIN GAUS; splat:doggo.ply; text:GREETINGS; text:CODE ANNEJAN" \
cargo +nightly run --release
```

---

## Where files are loaded from

The demo's splats live in **`assets/`** — the default asset root — so splat names
(`MARTIN_PLY2`, `MARTIN_REFORM`, `splat:` parts) resolve there with no extra setup. To
load splats from a **different folder**, point `MARTIN_PLY` at one of them; its **parent
folder becomes the asset root** and the other names resolve beside it:

```bash
MARTIN_PLY=/other/dir/martin.ply   # → asset root = /other/dir
MARTIN_PLY2=martin-peace.ply        # → /other/dir/martin-peace.ply
MARTIN_REFORM=doggo.ply             # → /other/dir/doggo.ply
```

(In a sequence, `MARTIN_PLY` itself need not appear in the parts — it just sets the root.)

> **Export uncompressed / standard PLY** (e.g. from [SuperSplat](https://superspl.at/editor)).
> The loader rejects SuperSplat's *compressed* format (`missing required properties`).

---

## Full env var reference

| Env var | Default | What it does |
|---|---|---|
| `MARTIN_PLY` | `assets/aegg.ply` | Primary splat / asset-folder override — its parent folder becomes the asset root. |
| `MARTIN_PLY2` | — | A second splat, placed beside the first. |
| `MARTIN_REFORM` | — | Morph target: the source splat(s) turn into this one. |
| `MARTIN_TEXT` | — | Splat-text: this string assembles out of a ball cloud (glowing). |
| `MARTIN_SEQ` | — | A timeline of parts (see [Sequences](#sequences)). Highest precedence. |
| `MARTIN_TRANSITION` | — | Default arrival transition for every part: `morph`/`ball`/`fade`/`explode`/`implode`/`drop`/`swirl` (data-only) or `typewriter`/`wipe`/`sparkle`/`slither`/`vortex`/`outline`/`pen-write` (per-particle shader; `outline`/`pen-write` are text-only). A per-part `~name` overrides it. See [Sequences](#sequences). |
| `MARTIN_FLASH` | `0` | Over-bright **bloom flash on each part cut** (0 = off; `~0.6` = punchy). Synced to the music when parts are `@@`-anchored to beats/bars. |
| `MARTIN_SYNTH_WAV` | — | Render the bundled deFEEST synth (Cinder) to a WAV at this path, then exit — for muxing audio onto a recording. See [Music](#music-the-synth). |
| `MARTIN_CAMERAS` | — | A 3DGS/COLMAP `cameras.json` (graphdeco format); parks the camera at a real capture pose (transformed through the same normalize + rotation as the splats). `MARTIN_CAM_INDEX` picks which shot (default 0). *Experimental:* helps cleanly-captured scenes; soft 360° photogrammetry dumps still render abstract (see the scene heads-up above). |
| `MARTIN_BULGE` | `0.9` | Ball-cloud size at a morph's midpoint, in object-radii. `0` = clean "puzzle-box" reorder (no explosion); `~0.9` = a ball roughly the object's size. (In sequences this is the per-part 3rd timing number instead.) |
| `MARTIN_MORPH_COUNT` | `0` (shorthand) / `200000` (`MARTIN_SEQ`) | Gaussian budget every part is resampled to. `0` = the largest part's natural count (~1.15M for the Martins; crisp, ~20 fps). Lower = faster: **250k ≈ 60 fps, 500k ≈ 40 fps.** |
| `MARTIN_YAW` | `1.4` (front) | Seed the orbit **yaw** in **radians** (e.g. `1.57` ≈ head-on). When set, a recording **holds** this yaw instead of swaying — bake a found scene viewpoint. |
| `MARTIN_PITCH` | `0.12` | Seed the orbit **pitch** in **radians** (0 = eye level, `+` looks down). |
| `MARTIN_WAYPOINTS` | `waypoints.json` | File the **M-key camera waypoints** are written to (and read from on startup). Each marker appends the live orbit pose (target/dist/yaw/pitch) so you can author a camera path while flying — see [live controls](#live-keyboard-controls). |
| `MARTIN_FLY` | — | `=<secs>` **flies the camera through the loaded waypoints** instead of free-orbiting. **Recording:** the path fills each part's on-screen time (so a longer part `hold` = a slower flyby), alternating direction so it flows through the morph. **Live:** `<secs>` = time per waypoint leg (default `2`) for a ping-pong preview loop. Needs ≥2 waypoints in `MARTIN_WAYPOINTS`. |
| `MARTIN_FPS` | off | `=1` logs smoothed FPS / frame-time + timeline clock every ~0.5 s. |
| `MARTIN_RECORD` | — | Directory to dump one PNG per frame into (the whole timeline; used by `record.sh`). |
| `MARTIN_SHOT` | — | Capture a single headless screenshot to this path, then exit ~2 s later. |
| `MARTIN_SHOT_AT` | `6.0` | When (seconds) to take the `MARTIN_SHOT`. |
| `MARTIN_FULLSCREEN` | off | `=1` starts borderless-fullscreen; toggle live with **F11 / F**. (Ignored while recording — that needs the fixed window.) |
| `MARTIN_NORMALIZE` | on | Each part is centred on its **centroid** and uniformly scaled (positions *and* gaussian sizes) so the bulk of its content (90th-percentile radius) ≈ 2 units. Using a percentile, not the bounding box, **ignores stray "floater" splats** that would otherwise shrink the scene to a distant dot — so a 200-unit COLMAP scene and a 1-unit TRELLIS object share one "normal" scale. `=0` keeps raw scales. |
| `MARTIN_ZOOM` | `1.0` | Camera closeness multiplier: **`>1` = closer / more zoomed in, `<1` = pull back**. The camera frames the normalized content up close by default; nudge this to taste. |
| `MARTIN_ROT` | — | `rx,ry,rz` euler **degrees** applied to the cloud — e.g. stand a COLMAP scene upright for a "normal" POV. Default = the portrait flip (gives scenes their abstract sideways look). |

---

## Live keyboard controls

When running in a window (not recording):

It's a **free-orbit inspection camera** (orbit `yaw`/`pitch` at `dist` around a look-at target):

| Key | Action |
|---|---|
| `←` / `→` | Orbit **yaw** (around the vertical axis) |
| `↑` / `↓` | Orbit **pitch** (look down / up) |
| `W` / `S` | Zoom **in / out** |
| `A` / `D` | Pan the target **left / right** |
| `Q` / `E` | Pan the target **down / up** |
| `M` | **Log a camera waypoint** → the waypoints file (`MARTIN_WAYPOINTS`) |
| `Space` | Restart the show (timeline back to t=0) |
| `F11` / `F` | Toggle borderless fullscreen |

Seed the framed angle with `MARTIN_YAW` / `MARTIN_PITCH` (radians) + `MARTIN_ZOOM`, then orbit
from there — handy for finding and baking a viewpoint. (Single-image splats from TRELLIS have a
**hollow back**, so don't orbit all the way around them; full multi-angle captures orbit freely.)

**Marking a camera path.** Fly to a pose you like and tap **M** — martin appends the live orbit
pose (`target` / `dist` / `yaw` / `pitch`) to the waypoints file (`MARTIN_WAYPOINTS`, default
`waypoints.json`) and logs the marker index to the console. Keep flying and dropping markers to
capture a whole path. The file is plain JSON — an array of poses — and is **read back on startup**,
so M *continues* an existing path across runs.

**Flying the path back.** With ≥2 markers, set **`MARTIN_FLY=<secs>`** and the camera flies the
path instead of free-orbiting (smoothstep easing through each marker, shortest-way yaw):

```bash
# preview the path live — ping-pongs there-and-back, ~<secs> between each marker
MARTIN_PLY=assets/train.ply MARTIN_FLY=2 cargo +nightly run --release

# bake it into a recording — the path fills the clip; a longer hold = a slower flyby
MARTIN_PLY=assets/train.ply MARTIN_SEQ="splat:train.ply @18,1" MARTIN_FLY=2 ./record.sh train_flyby.mp4
```

**Recording fills each *part's* on-screen window with one pass of the path**, **alternating
direction** — part 0 flies first marker → last, part 1 last → first, and so on. Two upshots: the
camera is *always moving* — it reaches the turn-marker exactly as the morph begins, so there's **no
dead pause before the transition** — and its position is *continuous* across the morph (the next
subject picks up from that marker and reverses — no jump). **A part's flyby lasts as long as its
`hold`** — want it slower, hold longer. Live, `<secs>` is the time per waypoint leg (default `2`)
and it ping-pongs the path on a loop, for judging the shape. Waypoints pin exact camera poses, so
replay them on `.ply`s that share a frame.

**Same flyby on two subjects, with a morph between** — when two splats normalize to the same spot
(the train + truck from one dataset do), one path frames both:

```bash
# train flies the path forward, then (continuous, no jump) the truck flies it back through the
# morph. Each subject's flyby lasts its hold (~14–15 s here). Bulge 0 = a straight morph.
MARTIN_PLY=assets/train.ply \
MARTIN_SEQ="splat:train.ply @13,1 ~fade; splat:truck.ply @12,3,0 ~morph" \
MARTIN_MORPH_COUNT=1000000 MARTIN_FLY=2 ./record.sh train_truck_flyby.mp4
```

(`@hold,morph,bulge` in seconds — the camera flies each subject for its full on-screen time, so
just set the holds to taste; no need to match a "pass length" — the flyby stretches to fit.)

> **Heads-up on raw scene `.ply`s.** A bare splat from a 360° capture (no camera poses) carries
> lots of under-constrained background "needle" splats and only blends coherently along its
> *capture trajectory* — so it may look streaky from an arbitrary orbit no matter the distance.
> Either **crop it to the subject** in SuperSplat (see `ART-DIRECTION.md`), or use a `.ply` whose
> capture cameras are available. Clean objects (TRELLIS, cropped captures) orbit fine.

---

## Sequences

`MARTIN_SEQ` is the composable mode: a list of **parts** that morph into one another,
each transition flowing through a ball cloud. It's either a `;`-separated string **or a
path to a file** with one part per line (`#` starts a comment, blank lines are skipped).

**Part grammar:**

```
text:STRING                      # splat-text (glowing)
image:logo.png                   # a PNG in the asset folder, rasterized to gaussians (a logo)
splat:name.ply                   # a splat (filename in the asset folder)
splat:a.ply+b.ply                # several splats, auto-arranged side by side
…any of the above… @hold,morph,bulge   ~transition   @@anchor
```

The optional trailing `@hold,morph,bulge` sets, in **seconds** (and ball amount):
- **hold** — how long to rest on this part once it arrives (default `1.5`)
- **morph** — how long the morph *into* this part takes (default `3.0`)
- **bulge** — ball-pulse explosiveness, `0`–`~1.4` (default `0.9`; **`morph` transition only**)

The optional trailing **`~transition`** picks *how* the part arrives (the ball is just one
of them). It can sit anywhere on the line, but reads best last:

**Data-only** (a built source cloud the morph flies in from):

| `~name` | How it arrives |
|---|---|
| `~morph` | flows from the **previous** part's shape (Morton-paired), with the ball-pulse `bulge` — the default after part 0 |
| `~ball` | assembles out of a fuzzy ball shell — the default for part 0 |
| `~fade` | fades up on the spot (opacity 0 → in) |
| `~explode` | gathers in from an outward burst |
| `~implode` | expands out from a dense point |
| `~drop` | falls straight down into place |
| `~swirl` | sweeps/spirals in around the vertical axis (cheap, straight-line) |

**Per-particle** (the vendored shader staggers each splat — great for text):

| `~name` | How it arrives |
|---|---|
| `~typewriter` (`~type`) | reveals left→right like a typewriter |
| `~wipe` | a hard slab edge sweeping across the x axis |
| `~sparkle` | random per-particle twinkle-in (HDR bloom makes it flash) |
| `~slither` | staggered lateral wobble that settles into place |
| `~vortex` | spins/unwinds into place (continuous, shader-driven) |
| `~outline` | **text only** — traces the *filled* font's letter outlines in pen order (a glowing neon draw-on) |
| `~pen-write` (`~pen`) | **text only** — real handwriting: traces a *single-stroke* font's centerline in pen order |

`MARTIN_TRANSITION=<name>` sets a default for **every** part (handy for trying one out); an
explicit per-part `~name` wins over it.

> **`~outline` vs `~pen-write` (both text-only).** Same shader mechanism (reveal along the pen
> path), different font. `~outline` traces the bundled *filled* font (DejaVu) → a glowing neon
> outline drawing itself on. `~pen-write` traces a bundled *single-stroke* font (Relief
> SingleLine CAD, OFL) via `ttf-parser`, keeping each stroke **open** (not closed back into a
> loop) and respecting pen-lifts between strokes → genuine centerline handwriting, upper- and
> lowercase. Tune stroke weight with `MARTIN_PW_SPLAT` (default `0.006`) / `MARTIN_PW_STEP`.

(The first part has nothing to morph *from*, so `~morph` there falls back to `~ball`.)

### Cue-anchoring: `@@anchor` (lock a part to the music)

By default parts are laid **end-to-end** (each starts when the previous finishes). A trailing
**`@@anchor`** token instead pins a part's *start* to an absolute time on the **music clock**
(the ported deFEEST score — 140 BPM; see [Music](#music-the-synth) below), so the show locks to
the track no matter how you retime the parts before it:

| `@@anchor` | Part starts at… |
|---|---|
| `@@drop` | a **section** boundary — `intro` / `build` / `drop` / `breakdown` / `climax` / `outro` |
| `@@bar32` (or `@@bar:32`) | **bar 32** (`32 × BAR`) |
| `@@beat:64` | **beat 64** (`64 × BEAT`) |
| `@@12.5` | **12.5 seconds** (raw) |

The part still uses its `@morph` to arrive and holds until the next part starts. Anchors should
increase down the show. Example — the title holds until the drop, then the dog hits *on* it:

```
text:MARTIN GAUS              @2,3
splat:doggo.ply  ~morph  @@drop @1,3,1.0     # morphs in exactly when the drop lands
image:defeest-logo.png ~ball  @@outro
```

`MARTIN_FLASH=<strength>` adds an over-bright **bloom flash on each part cut** (0 = off,
default; `~0.6` is punchy) — the demoscene scene-cut snap, synced to the music when parts are
anchored to beats/bars.

**Inline example — a full show:**

```bash
MARTIN_PLY=assets/doggo.ply \
MARTIN_SEQ="text:MARTIN GAUS @2,2.5,0; splat:doggo.ply @2,3,0.9; text:GREETINGS @1.5,2.5,0.9; text:DEFEEST CINDER @1.5,2.5,0.7; text:CODE ANNEJAN @2,2.5,0.6" \
cargo +nightly run --release
```

**File example** — put this in `show.seq`:

```
# Martin Gaus — title → two faces → dog → greetings → credits
text:MARTIN GAUS @2.5,3,0
splat:martin.ply+martin-peace.ply @2,3,0.6   # the two Martins, side by side
splat:doggo.ply @2,3.5,0.9                    # …become the dog
text:GREETINGS @1.5,2.5,0.9
text:CODE ANNEJAN @2.5,3,0.6
```

…and run it:

```bash
MARTIN_PLY=assets/doggo.ply MARTIN_SEQ=~/show.seq cargo +nightly run --release
```

All parts are resampled to one gaussian count (`MARTIN_MORPH_COUNT`, default 200k in
sequences) and the camera is framed once over everything, so it never pops between parts.

---

## Music (the synth)

martin carries a procedural synth + a **section/beat music clock**, ported (MIT) from Cinder's
(Kristian Vlaardingerbroek, deFEEST) `term-demo` — `src/audio.rs` + `src/score.rs`. The clock
is 140 BPM with a six-section arc (`intro → build → drop → breakdown → climax → outro`); those
section/bar/beat times are what `@@anchor` (above) pins parts to, so the visuals lock to the
track. There is no live audio yet — the synth renders **offline to a WAV** and ffmpeg muxes it
onto the recorded frames:

```bash
# 1. render the synth to a WAV (renders, then exits — no window)
MARTIN_SYNTH_WAV=/tmp/track.wav cargo +nightly run --release

# 2. record the (anchored) show to PNG frames
MARTIN_PLY=$PWD/assets/doggo.ply MARTIN_SEQ="…@@drop…@@outro…" MARTIN_RECORD=/tmp/frames \
  BEVY_ASSET_ROOT=$PWD cargo +nightly run --release

# 3. mux: video + audio (+ a fade to match the synth's own fade-out)
ffmpeg -framerate 60 -i /tmp/frames/frame_%05d.png -i /tmp/track.wav \
  -vf "fade=t=out:st=90:d=2.6" -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest out.mp4
```

The synth track is ~92.6 s (`DEMO_LEN`); anchor the final part near `@@outro` so the recording
covers the whole track.

---

## Recording to video

`record.sh` (in the repo root) builds the demo, renders frames headlessly, and runs
ffmpeg. It inherits all the `MARTIN_*` env vars:

```bash
# from the repo root
MARTIN_PLY=assets/doggo.ply \
MARTIN_SEQ="text:MARTIN GAUS; splat:doggo.ply; text:CODE ANNEJAN" \
./record.sh my_show.mp4
```

The clip length is computed automatically from the parts' `@hold,morph` timings.

To grab a single still instead:

```bash
MARTIN_TEXT="MARTIN GAUS" MARTIN_SHOT=/tmp/title.png MARTIN_SHOT_AT=6 \
cargo +nightly run --release
```

---

## Performance notes (Radeon 860M iGPU, Vulkan)

It's fill-rate bound and the depth sort scales with gaussian count:

| `MARTIN_MORPH_COUNT` | Frame rate |
|---|---|
| `250000` | locked 60 fps |
| `500000` | ~40 fps |
| `0` (max, ~1.15M) | ~20 fps — crisp, best for offline video / a beefier machine |

Use the lower counts for a smooth **live** demo and `0` for the final **rendered** video.
Run `--release`: the debug build is for fast iteration only.
