# martin Engine Audit

Codebase review across `src/` (~17 kLOC Rust).

**Date:** 2026-06-19
**Scope:** `src/` — every `.rs` file
**Method:** Per-subsystem agent sweeps, then **re-verified line-by-line against the actual code**
(2026-06-19). The first AI sweep over-reported badly — it hallucinated GPU internals, invented an
`extract_ui_camera_bind_group`, and built every magnitude on a false `res=128` premise. This revision
keeps only findings that match the code; the discredited ones are listed under **Retracted** so the
record is honest.

---

## Severity Key

| Tag | Meaning |
|-----|---------|
| 🔵 MEDIUM | Clear win, real, worth doing when touching the file |
| ⚪ LOW | Micro-opt / build-time / cold path — nice-to-have |

> There are **no CRITICAL or HIGH findings**. Both originally-flagged "🔴 CRITICAL" items were false
> positives (see Retracted). The genuine items are small and mostly **build-time** or **load-time**, not
> per-frame hot paths.

---

## The `res` premise (corrects the original's core error)

The original audit assumed `res=128` (a configurable default) and derived "2,097,152 cells / 48 MB churn
/ 2M workgroup invocations" from it. **That is wrong.** `res` is computed from the splat count:

```rust
// src/morph.rs:148
let res: i32 = (n as f64).cbrt().round().clamp(16.0, 128.0) as i32;
```

Default budget `n = 200_000` (`src/scene/sequence/parse.rs:241`, `build.rs:150`) → `res = round(cbrt(200_000)) = 58`
→ `58³ = 195_112` cells (~4.7 MB of empty `Vec`s), not 2M / 48 MB. `res=128` needs `n ≥ 2,097,152` splats —
over 10× the default. Every magnitude in the original GPU/morph section inherited this error.

---

## 🔵 MEDIUM (real, worth doing)

### Per-sample section lookups — `src/audio/stream.rs:223, 337` (+ `levels`) · per-sample
`SubAtmo::process` (223, `chord_at(t).root`) and `MasterChain::process` (337, `gain_at(t)`) run inside the
`for i in f0..f1` sample loop; `levels(t)` adds 2 more via `smooth`. Each walks the section list
(`score::section_index_at`, a linear scan over the sections) every sample → on the order of millions of
walks per render. The list is short (~6 sections) so it's not catastrophic, but caching the current
section index in `SubAtmo`/`MasterChain` and re-resolving only on a section boundary is a clean win and
the only finding on the per-sample path.

### Flat grid instead of `Vec<Vec<u32>>` — `src/morph.rs:161` · **build-time**
```rust
let mut grid: Vec<Vec<u32>> = vec![Vec::new(); (res * res * res) as usize];
```
At the default `res=58` that's ~195K empty inner `Vec`s (~4.7 MB), most never filled. A flat `Vec<u32>` +
prefix-sum offset array (count → prefix-sum → fill) removes the per-cell allocation and is the idiomatic
shape for this pure-function module. Runs once at **sequence build** (`build.rs:349`), not per frame —
a cleanliness/startup win, not a hot-loop fire.

---

## ⚪ LOW (micro-opt / cold / build-time)

| Site | Issue | Note |
|------|------|------|
| `morph.rs:165-170` | `cost` closure in the innermost cell scan | build-time; minor. The original's `&Vec→&[u32]` advice doesn't apply (closure takes `&Gaussian3d`). |
| `morph.rs:31-61 cluster_of` | per-`copy` loop is independent → parallelizable | `rayon` is only a *transitive* dep (no `par_iter` in `src/` today); output order + determinism are **required** for recordings, so any parallel version must collect deterministically. Build-time. |
| `audio/voices.rs` (323,374,461,507,552,750,786,906,1124) | LFO closures recompute `mult*freq` / `rate*TAU` per sample | hoist into the closure capture; a couple of flops/sample, genuinely tiny. |
| `audio/stream.rs:475` | `to_vec()` per streaming segment (`echo_buf[..].to_vec()`) | reuse a scratch `Vec` (`clear()`+`extend_from_slice`); per-segment, not per-sample. |
| `score/mod.rs` per-sample walkers | `section_index_at` reached redundantly via `chord_at`/`levels`/`gain_at` | a single-pass resolver at the current `t` would fold these together (same win as the stream cache above). |
| `scene/compose.rs:653-659` | `Quat::from_euler` built unconditionally per entity | skip to `Quat::IDENTITY` when `spin`/`sway` are zero — only helps *static* props (spinning ones need it). Marginal. |
| `scene/compose.rs:650-729` | `animate_composition` is a single-threaded `Query` iter | independent per entity → `Query::par_iter_mut()` (Bevy native; no manual collect). Entity counts are tens, not thousands — low priority. |
| `waypoints.rs` parse | authored anchors aren't sorted/asserted monotonic | `pose_at_time` is a **linear** scan that assumes monotonic `t`; sort after parse + assert in `validate()` for defensive correctness (latent, not currently observed). |
| `capture.rs:178` | `read_dir` poll during the record **drain** | bounded to the end-of-recording grace window (≤1200 polls counting PNGs as async writes land), not per render frame. |
| `mesh.rs:180` | one weights `Vec` per `sample_surface_disks` call | **load-time**, single alloc per mesh (not per-triangle); negligible. |
| `scene/gl_dissolve.rs:246` | iterates overlay `StandardMaterial`s each frame to fade alpha | cardinality is tiny (overlay-only); a marker component would be over-engineering. |

---

## Retracted (false positives from the first sweep)

These referenced code that does not exist as described, or were already handled. Removed, with the refutation:

| Original claim | Why it's false |
|---|---|
| 🔴 `effects.rs` NaN chord root → `.unwrap_or(261.63)` | wrong file/lines (164-169 are the kick pitch-fold loop); `Chord.root` is `f32` not `Option`, so `.unwrap_or` wouldn't compile; no NaN path. |
| 🔴 `effects.rs` subnormal FP from `mix *= 0.999` etc | those decay-multiplier patterns don't exist; envelopes are recomputed per-sample as `(-t*k).exp()` (normal-range); the original's own LOW table even says effects.rs is a "clean path". |
| 🟡 `post.rs` UI camera bind group every frame (`extract_ui_camera_bind_group`, `UiBindGroupCache`) | that function and struct do not exist. The real `post.rs:113` bind group is the post-process pass, which *must* rebuild each frame (`post_process_write()` ping-pongs the source texture). |
| 🟡 `post.rs:97` `world.resource::<PostPipeline>()` panics | line-off (it's ~100); `PostPipeline` is `FromWorld` and the node is only added in `PostPlugin::build` → unreachable. |
| 🟡 `gl_dissolve.rs` compute bind group rebuilt / "2M workgroups" | `gl_dissolve.rs` has **no** compute shader, bind group, or dispatch — it's a CPU `StandardMaterial` alpha-fade. Fabricated. |
| 🟡 `particles.rs` ~30K trig/frame (sin/cos/atan2/normalize/from_axis_angle per ember) | embers use **2** trig calls + `Quat::IDENTITY`; the expensive ops are only in confetti/sparks/fireworks, one branch per particle; count clamps to 5000 (default 200), not 30K. |
| 🔵 `score/mod.rs:83` `param_at` 2× lookup | one `section_index_at` walk + O(1) array index; and `param_at` isn't on the audio path. |
| 🔵 `waypoints.rs:252` NaN if `t1==t0` | the actual delta division is line 306 and is already guarded by `.max(1e-4)`. |
| 🔵 `sync.rs:94-103` clone + `contains_key` + `insert` | already uses `entry().or_default()`; the quoted `seen`/`contains_key` code doesn't exist. |

---

## Architecture / File Map

| File | ~Lines | Role | Hot path? |
|------|------|------|-----------|
| `src/audio/voices.rs` | 1345 | Voice synthesis — kick, snare, hat, lead, arp, bass, pad, stab | per-sample |
| `src/scene/compose.rs` | 872 | Per-frame composition animation loop | per-frame |
| `src/morph.rs` | 817 | Gaussian splat morphing (match_reorder, cluster_of) | **build-time** |
| `src/mesh.rs` | 769 | Mesh loading, surface sampling, skinning | load |
| `src/score/mod.rs` | 715 | Score DSL parsing, section lookup, note resolution | per-block / per-sample lookups |
| `src/particles.rs` | 487 | Ember/fire particle system (embers = 2 trig + identity) | per-frame |
| `src/waypoints.rs` | 474 | Camera waypoint paths (linear-scan sampler) | per-frame |
| `src/post.rs` | 238 | Post-process pass (bind group rebuilt per frame by design) | per-frame |
| `src/scene/gl_dissolve.rs` | 304 | **CPU** glTF mesh alpha-dissolve (no compute) | per-frame |
| `src/scene/caption.rs` | 217 | Captions (center/scroll) | per-frame |
| `src/audio/stream.rs` | 489 | Master audio stream, render-to-WAV | per-sample |
| `src/audio/effects.rs` | 385 | Kick rendering, reverb, limiter | per-sample |
| `src/camera.rs` | 135 | Orbit camera, pump, flypath | per-frame |
| `src/sync.rs` | 130 | Sync look-track | per-frame |
| `src/capture.rs` | 218 | PNG frame capture + ffmpeg recording | per-frame |

---

## GPU Budget Notes

Radeon 860M iGPU — the bottleneck. The fill-rate headroom is large; the real risk is the per-cloud
Gaussian sort, not a compute dispatch (there is no compute path in `src/`).

- Morph grid `res = cbrt(splat_count)`, clamped [16, 128]; default budget 200K → **res 58** (~195K cells).
  `res=128` only at ~2M+ splats.
- Grazing cameras through dense splat clouds can wedge RADV (the sort saturates) — see ART-DIRECTION § Z.
- One martin render at a time — concurrent instances wedge the GPU.

---

## Test Status

`cargo +nightly test --release` — green. Audio tests verify shape, not bit-exact output (no golden
capture). No code was changed by this audit revision (docs-only).
