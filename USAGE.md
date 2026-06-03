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
| `MARTIN_BULGE` | `0.9` | Ball-cloud size at a morph's midpoint, in object-radii. `0` = clean "puzzle-box" reorder (no explosion); `~0.9` = a ball roughly the object's size. (In sequences this is the per-part 3rd timing number instead.) |
| `MARTIN_MORPH_COUNT` | `0` (shorthand) / `200000` (`MARTIN_SEQ`) | Gaussian budget every part is resampled to. `0` = the largest part's natural count (~1.15M for the Martins; crisp, ~20 fps). Lower = faster: **250k ≈ 60 fps, 500k ≈ 40 fps.** |
| `MARTIN_YAW` | — (gentle sway) | Pin the camera to a fixed orbit angle in **radians** (e.g. `1.57` ≈ head-on). Handy for inspecting a splat. |
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

| Key | Action |
|---|---|
| `Space` | Restart the show (timeline back to t=0) |
| `F11` / `F` | Toggle borderless fullscreen |
| `↑` / `↓` | Zoom in / out |
| `←` / `→` | Lower / raise the camera |

The camera only **sways across the front** of the subject — single-image splats (e.g.
from TRELLIS) have a hollow back, so a full 360° orbit would show the inside of the head.
Use `MARTIN_YAW` to inspect a fixed angle. Splats captured from all sides (COLMAP→Brush)
can be orbited freely.

---

## Sequences

`MARTIN_SEQ` is the composable mode: a list of **parts** that morph into one another,
each transition flowing through a ball cloud. It's either a `;`-separated string **or a
path to a file** with one part per line (`#` starts a comment, blank lines are skipped).

**Part grammar:**

```
text:STRING                      # splat-text (glowing)
splat:name.ply                   # a splat (filename in the asset folder)
splat:a.ply+b.ply                # several splats, auto-arranged side by side
…any of the above… @hold,morph,bulge   ~transition
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
> SingleLine CAD, OFL) → genuine centerline handwriting. The single-stroke font renders
> lowercase and most letters beautifully; a couple of its uppercase glyphs (`E`, `S`) come out
> boxy, so **prefer lowercase** for pen-write (or swap in another single-stroke font later).

(The first part has nothing to morph *from*, so `~morph` there falls back to `~ball`.)

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
