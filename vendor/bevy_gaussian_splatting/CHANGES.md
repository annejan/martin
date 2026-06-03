# Local fork — change log

**Forked from:** `bevy_gaussian_splatting` **7.0.1** (crates.io), patched in via
`[patch.crates-io]` in `../../Cargo.toml`.

This file lists every edit we made to the upstream sources, so the fork can be
re-applied (or dropped) when bumping the crate. Keep it current when you touch the
vendor tree. To see the raw diff against a clean 7.0.1 checkout:

```bash
cargo package --no-verify  # or: download the 7.0.1 .crate and `diff -ru`
```

Feature selection (e.g. `sh0`) is **not** a fork — it's set in `../../Cargo.toml`
(`default-features = false, features = [...]`). The vendor `Cargo.toml` is upstream.

---

## 1. Custom render-shader effects  (`src/render/gaussian.wgsl`)

Injected after `var position = get_position(...)`:

- **Explode displacement** — closed-form ballistic per-gaussian offset driven by
  `gaussian_uniforms.time`, gated `time != 0 && !interp_active` (so it only fires for
  plain explode-mode clouds, never a morph output).
- **Morph "ball cloud" pulse** — when `interp_active` (an interpolation range is set) and
  `gaussian_uniforms.bulge > 0`, routes each gaussian onto a fuzzy sphere by
  `sin(pi*t)` (zero at t=0/1, max mid) so a morph disperses into a ball and reassembles.

Both rely on `explode_hash3(splat_index)` (also added here).

*Why a fork:* the crate loads shaders by hardcoded UUID (`load_internal_asset!`) — there
is no app-side hook to extend the WGSL. This is the core reason the fork exists.

## 2. New uniform field `bulge`  (4 spots — must stay in sync)

Drives the ball-pulse amplitude per cloud. `encase`/std140 layout couples these:

- `src/render/bindings.wgsl` — `bulge: f32` in `struct GaussianUniforms` (after `time_stop`).
- `src/render/mod.rs` — `bulge: f32` in `pub struct CloudUniform` (after `time_stop`) **and**
  `bulge: settings.bulge` in its construction.
- `src/gaussian/settings.rs` — `pub bulge: f32` in `CloudSettings` + `bulge: 0.0` in `Default`.

> Fragile on upgrade: if upstream changes `CloudUniform`'s field order, re-check alignment.

## 3. Sort optimizations  (`src/sort/radix.{rs,wgsl}`, `src/render/mod.rs`)

~2.4× faster radix sort on the iGPU, correctness preserved (LSD-stable, reads live GPU
positions → no holes). Candidates to **upstream as a PR**.

- `render/mod.rs` `ShaderDefines::default()` — `radix_digit_places = 2` (was `32/bits` = 4):
  16-bit depth key → halves the radix C-pass cost (65536 buckets is ample).
- `sort/radix.wgsl` `radix_sort_a` — store `key = key >> 16u` (sort the high 16 bits, to
  match the 2 passes).
- `sort/radix.wgsl` `radix_sort_c_scan_tiles` — `@workgroup_size(RADIX_BASE)`, lane = digit
  (was `@workgroup_size(1)` × RADIX_BASE single-lane workgroups, 1/64 wave occupancy).
- `sort/radix.wgsl` `radix_sort_c_scatter` — per-digit COUNT parallelized across all lanes
  (atomic into `tile_digit_counts`); the stable placement stays serial (LSD requires it).
- `sort/radix.rs` — `radix_sort_c_scan` dispatch `(1,1,1)` (was `(1, radix_base, 1)`);
  dropped the now-unused `radix_base` local.

---

## Not a fork (for reference)
- `sh0` vs `sh3`: feature selection in `../../Cargo.toml`.
- `assets/font.ttf`, `build_text_gaussians`, the `GaussianInterpolate` morph, the
  `DOGDEMO_SEQ` timeline: all live in the **app** (`../../src/main.rs`) and use the crate's
  public API — zero vendor changes.
