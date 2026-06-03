# dogdemo — usage & env vars

`dogdemo` loads Gaussian splats and flies a camera around them as they **morph into one
another** — driven entirely by environment variables. There's no config file: you compose
the show by combining env vars on the command line.

```bash
cargo +nightly run --release        # nightly toolchain is pinned (rust-toolchain.toml)
```

It's **one sequence engine**: every run is a list of *beats* (splats or text) that each
assemble out of a ball cloud and morph into the next. With no env vars it assembles
`assets/aegg.ply` from a ball and holds it.

---

## The show is a sequence (shorthands build one)

Everything is one timeline. `DOGDEMO_SEQ` writes it explicitly; the other env vars are
**shorthands** that build a sequence for you (first match wins):

| If you set… | The sequence it builds |
|---|---|
| `DOGDEMO_SEQ` | exactly the beats you write (the full timeline) |
| `DOGDEMO_TEXT` | one beat: that title, assembled from a ball |
| `DOGDEMO_PLY` (+ `_PLY2`) (+ `_REFORM`) | the splat(s) as beat 1; the reform target (if any) as beat 2 |
| *(nothing)* | one beat: `assets/aegg.ply` |

Examples:

```bash
# Single splat (your own .ply)
DOGDEMO_PLY=assets/martin.ply cargo +nightly run --release

# Two Martins morph into a dog
DOGDEMO_PLY=assets/martin-peace.ply DOGDEMO_PLY2=martin.ply \
DOGDEMO_REFORM=doggo.ply cargo +nightly run --release

# A glowing title that assembles from particles
DOGDEMO_TEXT="MARTIN GAUS" cargo +nightly run --release

# A whole show (see "Sequences" below)
DOGDEMO_PLY=assets/doggo.ply \
DOGDEMO_SEQ="text:MARTIN GAUS; splat:doggo.ply; text:GREETINGS; text:CODE ANNEJAN" \
cargo +nightly run --release
```

---

## Where files are loaded from

The demo's splats live in **`assets/`** — the default asset root — so splat names
(`DOGDEMO_PLY2`, `DOGDEMO_REFORM`, `splat:` beats) resolve there with no extra setup. To
load splats from a **different folder**, point `DOGDEMO_PLY` at one of them; its **parent
folder becomes the asset root** and the other names resolve beside it:

```bash
DOGDEMO_PLY=/other/dir/martin.ply   # → asset root = /other/dir
DOGDEMO_PLY2=martin-peace.ply        # → /other/dir/martin-peace.ply
DOGDEMO_REFORM=doggo.ply             # → /other/dir/doggo.ply
```

(In a sequence, `DOGDEMO_PLY` itself need not appear in the beats — it just sets the root.)

> **Export uncompressed / standard PLY** (e.g. from [SuperSplat](https://superspl.at/editor)).
> The loader rejects SuperSplat's *compressed* format (`missing required properties`).

---

## Full env var reference

| Env var | Default | What it does |
|---|---|---|
| `DOGDEMO_PLY` | `assets/aegg.ply` | Primary splat / asset-folder override — its parent folder becomes the asset root. |
| `DOGDEMO_PLY2` | — | A second splat, placed beside the first. |
| `DOGDEMO_REFORM` | — | Morph target: the source splat(s) turn into this one. |
| `DOGDEMO_TEXT` | — | Splat-text: this string assembles out of a ball cloud (glowing). |
| `DOGDEMO_SEQ` | — | A timeline of beats (see [Sequences](#sequences)). Highest precedence. |
| `DOGDEMO_BULGE` | `0.9` | Ball-cloud size at a morph's midpoint, in object-radii. `0` = clean "puzzle-box" reorder (no explosion); `~0.9` = a ball roughly the object's size. (In sequences this is the per-beat 3rd timing number instead.) |
| `DOGDEMO_MORPH_COUNT` | `0` (shorthand) / `200000` (`DOGDEMO_SEQ`) | Gaussian budget every beat is resampled to. `0` = the largest beat's natural count (~1.15M for the Martins; crisp, ~20 fps). Lower = faster: **250k ≈ 60 fps, 500k ≈ 40 fps.** |
| `DOGDEMO_YAW` | — (gentle sway) | Pin the camera to a fixed orbit angle in **radians** (e.g. `1.57` ≈ head-on). Handy for inspecting a splat. |
| `DOGDEMO_FPS` | off | `=1` logs smoothed FPS / frame-time + timeline clock every ~0.5 s. |
| `DOGDEMO_RECORD` | — | Directory to dump one PNG per frame into (the whole timeline; used by `record.sh`). |
| `DOGDEMO_SHOT` | — | Capture a single headless screenshot to this path, then exit ~2 s later. |
| `DOGDEMO_SHOT_AT` | `6.0` | When (seconds) to take the `DOGDEMO_SHOT`. |

---

## Live keyboard controls

When running in a window (not recording):

| Key | Action |
|---|---|
| `Space` | Restart the show (timeline back to t=0) |
| `↑` / `↓` | Zoom in / out |
| `←` / `→` | Lower / raise the camera |

The camera only **sways across the front** of the subject — single-image splats (e.g.
from TRELLIS) have a hollow back, so a full 360° orbit would show the inside of the head.
Use `DOGDEMO_YAW` to inspect a fixed angle. Splats captured from all sides (COLMAP→Brush)
can be orbited freely.

---

## Sequences

`DOGDEMO_SEQ` is the composable mode: a list of **beats** that morph into one another,
each transition flowing through a ball cloud. It's either a `;`-separated string **or a
path to a file** with one beat per line (`#` starts a comment, blank lines are skipped).

**Beat grammar:**

```
text:STRING                      # splat-text (glowing)
splat:name.ply                   # a splat (filename in the asset folder)
splat:a.ply+b.ply                # several splats, auto-arranged side by side
…any of the above… @hold,morph,bulge
```

The optional trailing `@hold,morph,bulge` sets, in **seconds** (and ball amount):
- **hold** — how long to rest on this beat once it arrives (default `1.5`)
- **morph** — how long the morph *into* this beat takes (default `3.0`)
- **bulge** — ball-cloud explosiveness of that morph, `0`–`~1.4` (default `0.9`)

(The first beat assembles in from a ball over its `morph` seconds; its `bulge` is ignored
— the ball already *is* its source.)

**Inline example — a full show:**

```bash
DOGDEMO_PLY=assets/doggo.ply \
DOGDEMO_SEQ="text:MARTIN GAUS @2,2.5,0; splat:doggo.ply @2,3,0.9; text:GREETINGS @1.5,2.5,0.9; text:DEFEEST CINDER @1.5,2.5,0.7; text:CODE ANNEJAN @2,2.5,0.6" \
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
DOGDEMO_PLY=assets/doggo.ply DOGDEMO_SEQ=~/show.seq cargo +nightly run --release
```

All beats are resampled to one gaussian count (`DOGDEMO_MORPH_COUNT`, default 200k in
sequences) and the camera is framed once over everything, so it never pops between beats.

---

## Recording to video

`record.sh` (in the repo root) builds the demo, renders frames headlessly, and runs
ffmpeg. It inherits all the `DOGDEMO_*` env vars:

```bash
# from the repo root
DOGDEMO_PLY=assets/doggo.ply \
DOGDEMO_SEQ="text:MARTIN GAUS; splat:doggo.ply; text:CODE ANNEJAN" \
./record.sh my_show.mp4
```

The clip length is computed automatically from the beats' `@hold,morph` timings.

To grab a single still instead:

```bash
DOGDEMO_TEXT="MARTIN GAUS" DOGDEMO_SHOT=/tmp/title.png DOGDEMO_SHOT_AT=6 \
cargo +nightly run --release
```

---

## Performance notes (Radeon 860M iGPU, Vulkan)

It's fill-rate bound and the depth sort scales with gaussian count:

| `DOGDEMO_MORPH_COUNT` | Frame rate |
|---|---|
| `250000` | locked 60 fps |
| `500000` | ~40 fps |
| `0` (max, ~1.15M) | ~20 fps — crisp, best for offline video / a beefier machine |

Use the lower counts for a smooth **live** demo and `0` for the final **rendered** video.
Run `--release`: the debug build is for fast iteration only.
