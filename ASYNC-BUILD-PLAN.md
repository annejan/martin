<!--
SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
SPDX-License-Identifier: MIT
-->

# Plan — async / non-blocking reel build (startup-freeze)

> **STATUS: SHIPPED (2026-06-25).** Both steps below landed: the per-part sampling fans across rayon,
> and the whole heavy build (`build_cpu`) runs off-thread on `AsyncComputeTaskPool` for live/windowed
> runs, finalizing (`finalize_build`) the frame it completes. Deterministic capture modes (record/shot/
> bench, see `build_inline`) build inline before frame 0 — a record bakes byte-identically. Verified:
> the camping reel renders byte-for-byte identical inline-vs-async; all 42 shows pass the headless smoke
> test. This file is kept as the design record. See `src/scene/sequence/build.rs`.

A design note for a future focused session. The goal: a reel of big captures should not freeze the
frame for seconds at startup while it samples + resamples every shot. **High regression risk** — it
touches the core build path that *every* reel show (camping, d2t, skyline, ruimte, baby, …) runs
through, so it deserves its own session with full per-show regression testing, not a tail-end rush.

## What's already done (this is the remaining piece)

The "handle lots of data" work landed in pieces; only the build *compute* is still synchronous:

- **Asset loading is already async** — `build_sequence` returns early until every referenced `.ply` is
  loaded (`build.rs`, the `state.loads … any none` gate), so the AssetServer streams them off-thread.
  The freeze is **not** the load; it's the build compute that runs once they're all in.
- **GPU residency: done** — splat streaming uploads only the active window `{idx-1,idx,idx+1}`
  (`ensure_window`), so VRAM no longer scales with shot count (commit `227adbe`).
- **Source RAM: done** — the raw `.ply` clouds are freed after sampling (`state.loads.clear()`),
  so we don't hold the sources on top of the built shots (commit `ef016fe`).

## The problem

`build_sequence` (`src/scene/sequence/build.rs`) runs as ONE Bevy system on the main thread and, in a
single frame, does for ALL parts:

1. **Sample** each part → `Vec<Gaussian3d>` (`sample_content`): a splat part copies/extracts its loaded
   `.ply`; text/svg/image/mesh parts rasterize/sample. Heavy for captures (~1.5 M gaussians each).
2. **Normalize** each to the common extent (`morph::normalize_to`).
3. **Global passes** that need EVERY part at once: union framing (`union_bounds`/`frame_of`),
   auto-budget `N` (`= max part len` when `budget==0`).
4. **`build_shots`**: resample each to `N` (`resample_morton`, heavy), build the ball/source/exit
   clouds, and `pair=match` (which pairs each shot against the **previous** shaped cloud → sequential).

Steps 1+4 are the multi-second cost. Today they all happen in the one frame the gate opens → a freeze.

## Constraints (why it's not trivial)

- **Global info first.** Framing + auto-budget need every part's extent/len → at least a cheap full
  pass before any per-shot heavy work.
- **`pair=match` is sequential.** Shot *i* pairs against shot *i-1*'s shaped cloud, so shots can't be
  resampled fully independently when pairing is on (cities use it).
- **`sample_content` is entangled with Bevy.** It borrows `&Assets<PlanarGaussian3d>` + `&SeqState`
  (to find a splat's loaded handle) and touches `std::fs` (svg/image/mesh) — not trivially `Send` to a
  worker as-is.
- **Determinism / record-safety.** The build must stay a pure function of the inputs (no wall-clock,
  no RNG) so a recording bakes identically regardless of when the async build finishes.

## Proposed design — off-thread build, hand the CPU shots back

Keep the *result* identical; move the heavy CPU off the main thread.

1. **Extract once (main thread, cheap).** When the load-gate opens, pull each splat part's gaussians
   out of `Assets` into an owned `Vec<Gaussian3d>` (a memcpy), and gather the non-asset inputs
   (text strings, file paths, per-part tokens). Drop the `.ply` assets immediately (already do).
2. **Spawn one build task** (`bevy::tasks::AsyncComputeTaskPool`, or a `std::thread` + channel): it
   runs steps 1(non-splat sampling)→4 as **pure CPU** (no Bevy world) and produces the `Vec<BuiltShot>`
   CPU data (`shape_data`/`origin_data`/`exit_data`) + the framing geometry. Make the sampling/build
   helpers take owned inputs so they're `Send`.
3. **Poll for completion** in a small main-thread system. While pending, show the loader / hold black
   (live) or, for **record**, simply *block until ready before frame 0* (records don't care about a
   one-time wait — only that the frames themselves are right). Determinism preserved.
4. **Finalize (main thread).** Move the `BuiltShot`s into `SeqState`, `ensure_window({0,1})` (the only
   `assets.add` on the main thread), spawn the interpolate entity, seed the framing camera. From here
   the existing director + streaming take over unchanged.

### Optional, smaller, lower-risk first step

Parallelize just the **independent** parts of the build with `rayon` (already a dep): the per-part
sampling + (for non-`pair=match` shows) the per-shot resample. Cap it to the non-pair path so the
sequential pairing is untouched. This shrinks the freeze without the full off-thread rearchitecture —
a good "land something safe first" increment before the task-based version.

## Risks + testing

- **Every reel show** runs this path → regression-test the lot: `pipeline/smoke-shows.py` (all shows
  render rc=0), plus eyeball camping / d2t / skyline / ruimte / baby for identical framing + morphs.
- **`Send` boundaries** — `sample_content` + the morph helpers must take owned data; watch for hidden
  `&Assets`/`&SeqState` borrows.
- **Determinism** — keep the build a pure fn; a record must bake byte-identically whether the build
  finished on frame 0 or 3. Block-before-frame-0 in record mode is the safe default.
- **Framing camera** — `seed_orbit_framing` must run after the async result lands (it needs the union
  geometry), before the first rendered frame.

## Definition of done

- A 4-city skyline (or a many-capture demo) starts with **no multi-second frozen frame** live; records
  bake identically to today; `smoke-shows.py` stays green; framing/morphs unchanged across all shows.
