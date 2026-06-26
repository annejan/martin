<!--
SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
SPDX-License-Identifier: MIT
-->

# PERF.md — the measured performance playbook

What actually costs frames in martin, measured on the dev box (AMD Radeon 860M iGPU, RADV/Mesa, the
weak end — a party machine is the target). Numbers are **live windowed fps** (`MARTIN_FPS` +
`MARTIN_VSYNC=0`, the timeline pinned with `MARTIN_HOLD_T` so every run renders identical content),
medianed over the steady state after a warm-up. See *Measuring* at the bottom for the method + traps.

## TL;DR

- **fps is bound by ONE of two things, depending on the scene:**
  - **Fill / overdraw** — big splat disks covering many pixels (a dense single cloud: a procedural
    shape, a tight object). Lever: **`MARTIN_SPLAT_SCALE`** (disk shrink) + **resolution**. Huge.
  - **Vertex / count** — the sheer number of splats projected per frame (a compose diorama of many
    clouds, or a spread cloud). Lever: **total splat count**. `SPLAT_SCALE`/res/sort do nothing here.
- **The universal lever is COUNT** — it helps both regimes. `SPLAT_SCALE` *additionally* rescues the
  fill-bound case. `MARTIN_QUALITY` stacks count + res + scale + sort, so it covers everything.
- **Effects are essentially FREE.** Bloom, the fullscreen background shaders (stars/warp/kaleido),
  per-part `^deform`s, and the particle layer each cost **< 0.5 fps**. Keep the look — it isn't the cost.
- **`MARTIN_BENCH` lies** (CPU submit rate, ~20× high). Use the windowed `MARTIN_FPS`+`VSYNC=0` path.

## The two regimes — measured (250k gaussians, 720p, content-rich moment)

### Fill/overdraw-bound — a dense single cloud (procedural torus)

| lever | fps | vs base |
|---|---|---|
| baseline (scale 1.0, 720p) | 13.7 | — |
| + bloom | 13.5 | ~free |
| + backdrop (stars / warp / kaleido) | 13.3–13.5 | ~free |
| + `^deform turbulence` | 13.0 | ~free |
| **`SPLAT_SCALE=0.5`** | **36.9** | **2.7×** |
| **`SPLAT_SCALE=0.2`** | **77.2** | **5.6×** |
| **res 360p** | 37.4 | 2.7× |
| count 60k | 48.0 | 3.5× |
| count 500k | 6.9 | 0.5× |

Disk size dominates: the 250k splats overlap densely, so the fragment/fill stage is the wall. Shrink
the disks or the resolution and it flies.

### Vertex/count-bound — a compose diorama (PonyCamp / `campsite-max`, climax)

| lever | fps | vs base |
|---|---|---|
| baseline | 13.4 | — |
| `SPLAT_SCALE=0.5` | 14.0 | ~free |
| `SPLAT_SCALE=0.2` | 14.4 | ~free |
| res 360p | 14.1 | ~free |
| particles off | 13.1 | ~free |
| bloom off | 13.1 | ~free |
| `SORT_BITS=16` | 13.2 | ~free |
| **count 60k** | **35.1** | **2.6×** |

The exact opposite: disk size, resolution, sort precision, particles, bloom — *nothing* moves it. Only
cutting the splat count does. Many clouds × their splats = a per-splat vertex/projection cost that
scales with count alone.

### Why the same 250k runs 3× apart between scenes

| scene | structure | fps @250k/720p |
|---|---|---|
| `cities-defeest` (held) | one spread city cloud — low overdraw | **45.7** |
| `tj` | light reel | 30.5 |
| `d2t` | reel, shapes + text | 16.1 |
| `anthem-baby` | single procedural + pedestal | 13.7 |
| `campsite-max` (PonyCamp) | compose diorama, many clouds | 13.3 |
| `ruimte` | explosive morphs + space bg | 12.8 |
| `intro` | gl-mesh dissolve + flock + backdrops | 10.7 |

Same count, 4× spread. A **spread** cloud (city) is cheap (low overdraw, one cloud). A **dense** shape
(torus) is fill-bound. A **many-cloud** diorama (PonyCamp) is count/vertex-bound. The differentiator is
the scene's spatial structure, never the effects on top.

## `MARTIN_QUALITY` tiers (measured on PonyCamp, the hard case)

| tier | caps | PonyCamp fps |
|---|---|---|
| `potato` | 8k count · 640×360 · scale 0.7 · sort16 | **91** |
| `low` | 120k · 854×480 · scale 0.8 · sort16 | **29** |
| `med` | 250k · 1280×720 | 15 |
| `high` | 250k · 1920×1080 | 13 |

Stacks count + res + scale, so it rescues both regimes at once. `low` clears 30 fps on the dev iGPU's
worst scene; `potato` is the party-hardware floor.

## Practical guidance

- **Live / party hardware (Evoke):** ship on `--quality low` (or `potato` on a weak box). For a single
  scene, the cheapest hand-tune is `SPLAT_SCALE` (≈ free 2× on a fill-bound scene) before touching count.
- **Recorded video:** no realtime constraint (the offscreen render builds inline and isn't fps-capped),
  so render dense — push count/scale up for quality; only wall-clock grows.
- **Authoring for speed:** prefer **spread** compositions over dense overlapping stacks; a city-style
  cloud is far cheaper than a tight object or a piled diorama at the same count. Keep the effects — they
  don't cost frames.
- **GOTCHA — always set `count:` on a `[stage]`/compose prop.** A compose object *without* a `count:`
  token falls back to the default sample count (**120k splat / 60k mesh**) — fine for a hero, ruinous for
  a small decorative prop. PonyCamp's climax had ~600k hidden gaussians in count-less snacks/animals
  (five `*0.2` bitterballen at 60k each, etc.), dwarfing the ~90k of explicit counts → 13 fps. Adding
  `count:` to all 15 via the **count × size law** (count ∝ on-screen area: a `*0.16` prop wants ~2.5k, not
  60k) took the climax to **40 fps (3×)** with no visible change. Audit a show with: list every
  `[stage]` `splat:`/`mesh:` line with no `count:`. (`[reel]` parts correctly omit it — they share the
  morph budget.)
- **Cities specifically:** ~0.9–2.0M splats native; live wants ~250–300k/city (≈40 fps), but the
  recorded video can take 600–800k. The streaming window caps a *morph* reel at ~800k/city before the
  iGPU OOMs (~2.5M resident, window ≈ 3× count).

## Measuring

- **On-screen HUD:** `MARTIN_FPS_OVERLAY=1` (or the **`I`** key live) draws the engine's own fps +
  frame-time + count readout. Live windows only (never baked into a `--record`).
- **Console metric:** `MARTIN_FPS=1` logs `metrics: <fps> fps` every ~0.5 s (the `I` key toggles it).
- **Repeatable sweep:** `pipeline/bench-sweep.sh <show>` sweeps count × res × scale, one GPU run at a
  time (concurrent renders wedge RADV), drops warm-up samples, medians the steady state → CSV. Always
  muted. Pin a content-rich moment with `HOLD_T=`; **use a long `WARMUP`** (≥6) so the off-thread build
  has finished before the measured window — otherwise the median swings (empty-scene frames are fast).
- **In-process tiers:** `--benchmark` (`MARTIN_BENCHMARK`) spawns a windowed child per `QUALITY` tier,
  reads each one's steady-state fps, and prints the table + the recommended tier. Muted.
- **Trap:** `MARTIN_BENCH`/`--bench` measures CPU frame *submit* rate (wgpu queues async) — ~20× too
  high. It is **not** the GPU ceiling; only the windowed `MARTIN_FPS`+`VSYNC=0` path is.
