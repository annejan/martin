# SHADER-BLUEPRINT.md — per-particle transition phase (the one deliberate fork edit)

> **STATUS: BLUEPRINT / REFERENCE ONLY — DO NOT APPLY YET.**
>
> This is the companion reference that `DESIGN.md` §5 points to. It is martin's
> *single* deliberate fork-shader edit, and it is **gated behind
> `DESIGN.md` OQ#12** ("do the one deliberate per-particle-phase edit now, or stay
> data-only?" — i.e. *do step 8 at all*). That is a morning **co-design** decision,
> not a thing to arrive pre-written. Nothing here is to be committed until OQ#12
> says "yes, do step 8."
>
> **Hard guarantee this doc is built around:** with `transition_mode == 0` the
> shader must be **byte-identical to today** — every existing user, and every golden
> frame martin already records, is unchanged. The edit is *append-only* (new uniform
> fields after the true struct end) and *default-off*. If mode 0 ever differs by a
> bit, the edit is wrong.
>
> Co-written by annejan & Kloot, deFEEST. Made on AMD · Vulkan · Bevy.

Section list (this file):

1. Status & scope
2. The idea — one per-particle `phase`, one moving window
3. The WGSL diff — `transition_phase()` + the gated `vs_points` branch + the one-line opacity multiply
4. The uniform plumbing (4 spots) + the std140 alignment rule
5. The mode table
6. App-side hook (`part_director`) — describe only, do not edit app code
7. Determinism
8. Sort safety
9. Pen-write caveat (the hard one)
10. Upstream-PR shape + a ready-to-paste CHANGES.md entry
11. RADV note (switch-on-uniform-int)

> **Verified-against-live note.** Every path, line number, struct field order, and
> code fragment below was checked against the current tree on 2026-06. Where
> `DESIGN.md` is slightly off (it is, in two places — the `settings.rs` path and one
> line range), this doc **corrects it** and flags the discrepancy in §10's "rebase
> notes." The field order in `bindings.wgsl` was re-verified by reading the struct
> directly, not trusted from `DESIGN.md`.

---

## 1. Status & scope

**Scope of the edit (the whole thing):**

- **one** new helper fn `transition_phase(splat_index, position) -> f32` in
  `gaussian.wgsl`,
- **one** gated branch in `vs_points` (sitting next to the existing ball-pulse
  branch), producing a per-particle `tx_reveal ∈ [0,1]` and optionally a position
  nudge,
- **one** edit at the color/opacity finalize site: `opacity` → `opacity * tx_reveal`,
- **one** uniform group (`transition_mode: u32`, `transition_softness: f32`,
  `transition_axis: u32`, plus std140 pad) plumbed through the **same 4 spots** the
  landed `bulge` feature uses.

That is the entire footprint. It mirrors `bulge` almost exactly; the only thing
`bulge` did that this doesn't is touch position *unconditionally* — here position
is touched only for the motion modes (slither / vortex / pen lead-in), and opacity
only for the reveal modes.

**Out of scope (explicitly):** anything in `interpolate.wgsl` (see §8 — that buffer
is what the sort reads), any second uniform group (B1–B5 all route through the one
`transition_mode` — see `DESIGN.md` §4.2's canonical-design note), the
out-transition *timeline slot* (`DESIGN.md` §4.4 / OQ#16 — that's an app-side
timeline gap, not a shader gap), and the pen-write CPU outline-walk (§9 — that's a
`text.rs` change that this shader edit only *consumes*).

---

## 2. The idea — one per-particle `phase`, one moving window

Today `gaussian_uniforms.time` is **one** blend factor `t ∈ [0,1]`, applied
identically to every particle (hard constraint #4 in `DESIGN.md` §1.2). Every
per-particle transition martin wants — typewriter, slither, sparkle in/out, true
vortex, directional-wipe hard edge, pen-write — shares **one** structure:

> Each particle is assigned its own scalar **`phase ∈ [0,1]`** (a pure function of
> its `splat_index` and/or its position), and the global `time` sweeps a **moving
> window** across the phase axis.

The one load-bearing formula (carried verbatim from `DESIGN.md` §5):

```wgsl
let local = saturate((global_t * (1.0 + softness) - phase) / softness);
```

`local` ramps `0 → 1` as the moving front passes the particle's `phase`:

- `softness` is the width of the ramp. Small `softness` → a near-hard edge
  (typewriter / directional-wipe). Large `softness` (→ 1) → a gentle dissolve.
- The `(1 + softness)` term guarantees that at `global_t == 1` **every** particle
  has `local == 1` regardless of `phase` (the front has fully passed even the
  last-revealed particle), and at `global_t == 0` every particle has `local == 0`.
  So the window cleanly covers `[0,1]` over the morph and the endpoints are exact.

`local` then feeds one of three sinks:

- **opacity** (`tx_reveal = local`, or `1 - local` for spark-out): reveal /
  typewriter / sparkle. Multiplied into opacity at the finalize site (§3, the
  `opacity * tx_reveal` line). This is the fully sort-safe family.
- **the ball-pulse** (scale the existing `sin(pi*mt)*bulge` disperse by a
  per-particle staggered `local`): a *staggered* disperse instead of the current
  uniform one. Reuses the existing branch's machinery.
- **position** (displace by `(1 - local)` along a sine, or rotate by
  `(1 - local)*turns`): slither / true-vortex / pen lead-in. The displacement dies
  to zero as `local → 1`, so the particle lands exactly on its morph-target — same
  "exact reset at the endpoints" property the explode and ball-pulse branches
  already rely on.

**Mode 0 reproduces today bit-for-bit** by never computing `local` and never
touching opacity or position (early `return 1.0` from the helper, and the branch
guarded by `transition_mode != 0u`).

`global_t` here is the morph-local factor, computed exactly like the existing
ball-pulse branch does it (`gaussian.wgsl:246-247`):

```wgsl
let denom = max(gaussian_uniforms.time_stop - gaussian_uniforms.time_start, 1e-6);
let mt = clamp((gaussian_uniforms.time - gaussian_uniforms.time_start) / denom, 0.0, 1.0);
```

— so it is the *morph progress*, already eased and frame-indexed in record mode.

---

## 3. The WGSL diff (`src/render/gaussian.wgsl` in the fork)

> Paths below are **fork-relative** — `src/...` means the `annejan/bevy_gaussian_splatting`
> `martin` branch (consumed via `[patch.crates-io]`), not this repo. martin no longer keeps a
> `vendor/` copy.

All references below are to the **live** file. The relevant landmarks I verified:

- `explode_hash3(i: u32) -> vec3<f32>` helper: `gaussian.wgsl:186-195` (we reuse it
  for the sparkle/hash phase — no new RNG needed).
- `vs_points` vertex entry: `gaussian.wgsl:197-498`.
- the **explode** ballistic branch: `gaussian.wgsl:220-236` (matches `DESIGN.md`
  §4.2 B6's "~lines 220–236").
- the **ball-pulse "bulge" branch**: `gaussian.wgsl:245-260` (the
  `if (interp_active && gaussian_uniforms.bulge > 0.0)` block). We insert the new
  branch immediately after it.
- `var opacity = get_opacity(splat_index);` at `gaussian.wgsl:291`.
- the color/opacity finalize site: `gaussian.wgsl:480-483`:
  ```wgsl
  output.color = vec4<f32>(
      rgb,
      opacity * gaussian_uniforms.global_opacity,
  );
  ```
  (`DESIGN.md` §5 cites "`opacity * gaussian_uniforms.global_opacity` at
  `gaussian.wgsl:480-482`" — verified; the multiply is on line 482.)

### 3a. New helper (insert after `explode_hash3`, i.e. after `gaussian.wgsl:195`)

```wgsl
// --- per-particle transition phase ∈ [0,1] for staggered transitions (typewriter,
//     slither, sparkle, vortex, directional-wipe). A PURE function of splat_index +
//     position + uniforms — no wall-clock, no RNG state — so it is deterministic in
//     record mode (DESIGN.md constraint #5). Mode 0 is never reached here (the caller
//     guards on transition_mode != 0u), so mode 0 stays byte-identical to today. ---
fn transition_phase(index: u32, position: vec3<f32>) -> f32 {
    let mode = gaussian_uniforms.transition_mode;
    let center = (gaussian_uniforms.min.xyz + gaussian_uniforms.max.xyz) * 0.5;
    let extent = max(gaussian_uniforms.max.xyz - gaussian_uniforms.min.xyz, vec3<f32>(1e-6));

    // axis-normalized coordinate of THIS particle, 0 at min, 1 at max, on transition_axis.
    let axis = gaussian_uniforms.transition_axis;          // 0=x, 1=y, 2=z
    var norm_axis = (position.x - gaussian_uniforms.min.x) / extent.x;
    if (axis == 1u) { norm_axis = (position.y - gaussian_uniforms.min.y) / extent.y; }
    else if (axis == 2u) { norm_axis = (position.z - gaussian_uniforms.min.z) / extent.z; }

    // radial fraction from the object center, 0 at center, 1 at the corner.
    let radius = max(length(extent) * 0.5, 1e-4);
    let radial = clamp(length(position - center) / radius, 0.0, 1.0);

    // hashed per-particle phase (reuses explode_hash3 — no new RNG path).
    let hashed = explode_hash3(index).x;

    // NOTE: if/else-if chain, NOT a switch — see §11 (RADV switch-on-uniform-int).
    if (mode == 1u) { return clamp(norm_axis, 0.0, 1.0); }   // typewriter / directional-wipe (axis order)
    else if (mode == 2u) { return clamp(norm_axis, 0.0, 1.0); } // slither (staggered along axis)
    else if (mode == 3u) { return hashed; }                  // sparkle-in
    else if (mode == 4u) { return hashed; }                  // spark-out (reveal inverted at the sink)
    else if (mode == 5u) { return clamp(radial, 0.0, 1.0); } // vortex-true (unwind by radius)
    else if (mode == 6u) { return clamp(norm_axis, 0.0, 1.0); } // directional-wipe HARD (softness≈0)
    // mode 7 (pen-write) reads a baked phase from the unused z channel — see §9:
    else if (mode == 7u) { return clamp(position.z, 0.0, 1.0); }
    return 0.0; // unreached: caller guards transition_mode != 0u
}
```

> The `mode` integers here are *example* assignments; the canonical table is §5. Note
> 1/2/6 all share the `norm_axis` source but differ at the sink (opacity vs position)
> and in `softness`; 3/4 share the hash source.

### 3b. The gated branch in `vs_points` (insert after the ball-pulse branch, i.e. after `gaussian.wgsl:260`, before `var transformed_position = …` at line 262)

**Before** (lines 245-262, abridged — the existing ball-pulse branch then the
transform):

```wgsl
    if (interp_active && gaussian_uniforms.bulge > 0.0) {
        // … sin(pi*mt) ball-pulse, position = mix(position.xyz, ball_pos, pulse) …
    }

    var transformed_position = (gaussian_uniforms.transform * position).xyz;
```

**After** (new branch inserted between them):

```wgsl
    if (interp_active && gaussian_uniforms.bulge > 0.0) {
        // … existing ball-pulse, UNCHANGED …
    }

    // --- per-particle TRANSITION phase (DESIGN.md §5 / SHADER-BLUEPRINT.md). Off by
    //     default: transition_mode == 0u skips the whole block, so mode 0 is byte-
    //     identical to today. Only active during a morph (interp_active), exactly like
    //     the ball-pulse above. Produces tx_reveal for the opacity sink (read at the
    //     finalize site, line ~483) and, for the motion modes, nudges `position`. ---
    var tx_reveal = 1.0;
    if (interp_active && gaussian_uniforms.transition_mode != 0u) {
        let denom = max(gaussian_uniforms.time_stop - gaussian_uniforms.time_start, 1e-6);
        let gt = clamp((gaussian_uniforms.time - gaussian_uniforms.time_start) / denom, 0.0, 1.0);
        let softness = max(gaussian_uniforms.transition_softness, 1e-4);
        let phase = transition_phase(splat_index, position.xyz);
        let local = clamp((gt * (1.0 + softness) - phase) / softness, 0.0, 1.0);
        let mode = gaussian_uniforms.transition_mode;

        if (mode == 1u || mode == 6u) {
            // typewriter / directional-wipe HARD: reveal opacity as the front passes.
            tx_reveal = local;
        } else if (mode == 2u) {
            // slither: lateral sine that dies as the particle settles (local → 1).
            let amp = (1.0 - local) * length(gaussian_uniforms.max.xyz - gaussian_uniforms.min.xyz) * 0.04;
            let wobble = sin(phase * 18.0 + gt * 6.2831853);
            position = vec4<f32>(position.x, position.y + amp * wobble, position.z, 1.0);
        } else if (mode == 3u) {
            tx_reveal = local;                 // sparkle-in (hashed phase → twinkly reveal; HDR Bloom flashes)
        } else if (mode == 4u) {
            tx_reveal = 1.0 - local;           // spark-OUT (reversed reveal)
        } else if (mode == 5u) {
            // vortex-true: unwind rotation about transition_axis, angle → 0 as it lands.
            let center = (gaussian_uniforms.min.xyz + gaussian_uniforms.max.xyz) * 0.5;
            let turns = 2.5;
            let ang = (1.0 - local) * turns * 6.2831853 * (0.4 + 0.6 * phase);
            let c = cos(ang); let s = sin(ang);
            let p = position.xyz - center;
            // rotate in the plane perpendicular to transition_axis (default: about Y).
            let rp = vec3<f32>(c * p.x + s * p.z, p.y, -s * p.x + c * p.z);
            position = vec4<f32>(center + rp, 1.0);
        } else if (mode == 7u) {
            tx_reveal = local;                 // pen-write: phase is baked pen distance (§9)
        }
    }
```

### 3c. The one-line opacity multiply at the finalize site

**Before** (`gaussian.wgsl:480-483`):

```wgsl
    output.color = vec4<f32>(
        rgb,
        opacity * gaussian_uniforms.global_opacity,
    );
```

**After** (one factor added):

```wgsl
    output.color = vec4<f32>(
        rgb,
        opacity * gaussian_uniforms.global_opacity * tx_reveal,
    );
```

Because `tx_reveal` defaults to `1.0` and is only changed by the opacity modes when
`transition_mode != 0u`, this multiply is **algebraically identical to today** in
mode 0 (`* 1.0`) and for the pure-position modes (slither / vortex, which leave
`tx_reveal == 1.0`). `DESIGN.md` §5 states exactly this ("the `* tx_reveal` edit is
algebraically identical when `tx_reveal == 1.0`") — verified against the live
finalize site.

> Scoping note: `tx_reveal` is declared just after the ball-pulse branch (≈ line
> 261) so it is in scope at the finalize site (≈ line 483) — both are inside the
> `vs_points` function body, after the early `discard_quad` returns, so a single
> `var tx_reveal = 1.0;` at branch-insert time is visible at the multiply.

---

## 4. The uniform plumbing (4 spots) + the std140 alignment rule

This is the part that's easy to get subtly, silently wrong — so it's spelled out
exhaustively. The new uniform group is plumbed through the **same 4 spots** as the
landed `bulge` feature.

### The std140 / vec4-alignment rule (read this first)

`GaussianUniforms` is a `<uniform>` block, so it obeys **std140**: a `vec4` must
start at a 16-byte boundary. The current struct (verified, `bindings.wgsl:13-27`)
ends:

```
… time, time_start, time_stop, bulge, num_classes: u32, color_space: u32, min: vec4<f32>, max: vec4<f32>
```

`min` and `max` are the **last two members and they are `vec4`** — i.e. the true
struct end is `max: vec4<f32>`, 16-byte aligned, ending on a 16-byte boundary.

> **CRITICAL APPEND LOCATION (verified, matches `DESIGN.md` §5):** the new fields go
> **AFTER `max: vec4<f32>`** (the true struct end) — **NOT** after `bulge`. Appending
> after `bulge` would shift `num_classes`, `color_space`, `min`, and `max` to new
> offsets and corrupt every existing field for every existing user. This single
> location is the difference between a clean PR and silently garbage uniforms.

Three scalars are added: `transition_mode: u32`, `transition_softness: f32`,
`transition_axis: u32`. Three 4-byte scalars = 12 bytes; std140 will round the
struct size up to the next 16-byte boundary anyway, but **make the pad explicit** so
the WGSL and the Rust `ShaderType` mirror agree without relying on the encoder's
implicit tail padding. So we append **four** members: the three reals + one
`_transition_pad: u32` to complete a 16-byte (`vec4`-sized) tail block. (Bevy's
`encase`/`ShaderType` derive computes std140 layout, but an explicit pad keeps the
WGSL struct, the Rust struct, and anyone reading the diff in lock-step — and makes
the next person's append start cleanly on a 16-byte boundary.)

### Spot 1 — `src/render/bindings.wgsl` (`bindings.wgsl:13-27`)

**Before:**

```wgsl
struct GaussianUniforms {
    transform: mat4x4<f32>,
    global_opacity: f32,
    global_scale: f32,
    count: u32,
    count_root_ceil: u32,
    time: f32,
    time_start: f32,
    time_stop: f32,
    bulge: f32,
    num_classes: u32,
    color_space: u32,
    min: vec4<f32>,
    max: vec4<f32>,
};
```

**After** (append after `max`, NOT after `bulge`):

```wgsl
struct GaussianUniforms {
    transform: mat4x4<f32>,
    global_opacity: f32,
    global_scale: f32,
    count: u32,
    count_root_ceil: u32,
    time: f32,
    time_start: f32,
    time_stop: f32,
    bulge: f32,
    num_classes: u32,
    color_space: u32,
    min: vec4<f32>,
    max: vec4<f32>,
    transition_mode: u32,        // 0 = identity/off (byte-identical to today)
    transition_softness: f32,    // moving-window ramp width (small = hard edge)
    transition_axis: u32,        // 0 = x, 1 = y, 2 = z (for axis/wipe/vortex modes)
    _transition_pad: u32,        // std140: complete the 16-byte tail block
};
```

### Spot 2 — `CloudUniform` Rust mirror, `src/render/mod.rs` (`mod.rs:970-984`)

This struct `#[derive(ShaderType)]` and **must mirror the WGSL field order exactly**
(`bulge` at `mod.rs:979`, `min`/`max` at `mod.rs:982-983` — verified).

**Before** (tail of the struct):

```rust
    pub bulge: f32,
    pub num_classes: u32,
    pub color_space: u32,
    pub min: Vec4,
    pub max: Vec4,
}
```

**After:**

```rust
    pub bulge: f32,
    pub num_classes: u32,
    pub color_space: u32,
    pub min: Vec4,
    pub max: Vec4,
    pub transition_mode: u32,
    pub transition_softness: f32,
    pub transition_axis: u32,
    pub _transition_pad: u32,
}
```

### Spot 3 — `CloudUniform` construction from `CloudSettings`, `mod.rs:1031-1048`

The `settings_uniform = CloudUniform { … }` literal populates every field from
`settings`. Append the three new ones (mirroring how `bulge: settings.bulge` is set
at `mod.rs:1040`):

**Before** (tail of the literal):

```rust
            color_space: match settings.color_space {
                GaussianColorSpace::SrgbRec709Display => 0,
                GaussianColorSpace::LinRec709Display => 1,
            },
            min: aabb.min().extend(1.0),
            max: aabb.max().extend(1.0),
        };
```

**After:**

```rust
            color_space: match settings.color_space {
                GaussianColorSpace::SrgbRec709Display => 0,
                GaussianColorSpace::LinRec709Display => 1,
            },
            min: aabb.min().extend(1.0),
            max: aabb.max().extend(1.0),
            transition_mode: settings.transition_mode,
            transition_softness: settings.transition_softness,
            transition_axis: settings.transition_axis,
            _transition_pad: 0,
        };
```

### Spot 4 — `CloudSettings`, `src/gaussian/settings.rs`

> **DISCREPANCY (corrected).** `DESIGN.md` §5 / §1.2 cite this as `settings.rs:71-80`.
> The file is actually at **`src/gaussian/settings.rs`** (not `src/settings.rs`), and
> the `CloudSettings` struct spans **lines 60-81** with `bulge` at **line 80**; the
> `time / time_scale / num_classes / color_space` cluster is at lines 71-76. The
> "71-80" line range is roughly right but the path in `DESIGN.md` is wrong — see §10.

`CloudSettings` is *richer* than the uniform (it also carries `time_scale`,
`num_classes`, `color_space`, sort/draw/gaussian/playback/rasterize modes — verified
`settings.rs:60-81`), so "identical footprint as `bulge`" is true of the **uniform**,
not of the whole settings struct (`DESIGN.md` §5 makes this same caveat). Add the
three fields to the struct **and** to `impl Default` (so existing users default to
mode 0 = off).

**Struct, before** (tail, `settings.rs:77-81`):

```rust
    /// Midpoint explosive-bulge amplitude for GaussianInterpolate morphs (0 = none).
    /// The renderer scatters each gaussian radially by `sin(pi*t)*bulge` (peaks at the
    /// blend midpoint, zero at both ends) so a morph blows apart then reassembles.
    pub bulge: f32,
}
```

**Struct, after:**

```rust
    /// Midpoint explosive-bulge amplitude for GaussianInterpolate morphs (0 = none).
    /// The renderer scatters each gaussian radially by `sin(pi*t)*bulge` (peaks at the
    /// blend midpoint, zero at both ends) so a morph blows apart then reassembles.
    pub bulge: f32,
    /// Per-particle transition mode (0 = off / identity → byte-identical to no fork).
    /// Selects the staggered reveal/motion effect in vs_points. See SHADER-BLUEPRINT.md §5.
    pub transition_mode: u32,
    /// Moving-window ramp width for the transition. Small ≈ hard edge (typewriter/wipe),
    /// larger → soft dissolve. Ignored when transition_mode == 0.
    pub transition_softness: f32,
    /// Axis for axis/wipe/vortex modes: 0 = x, 1 = y, 2 = z.
    pub transition_axis: u32,
}
```

**`impl Default`, before** (`settings.rs:102-103`):

```rust
            bulge: 0.0,
        }
```

**`impl Default`, after:**

```rust
            bulge: 0.0,
            transition_mode: 0,        // off → mode 0 → byte-identical to today
            transition_softness: 0.15, // sensible default ramp; only used when mode != 0
            transition_axis: 0,        // x
        }
```

> `CloudSettings` derives `Serialize/Deserialize` with `#[serde(default)]`
> (`settings.rs:57-59`), so adding fields with `Default` is backward-compatible for
> any serialized settings — old data simply gets the off defaults.

---

## 5. The mode table

The canonical integer → effect map. `phase source` is what `transition_phase`
returns; `sink` is what `local` drives.

| mode | name | phase source | sink (target) | softness | notes |
|---|---|---|---|---|---|
| **0** | **identity / off** | (none) | (none) | (ignored) | **byte-identical to today.** Default. |
| 1 | typewriter | normalized-axis coord (`transition_axis`) | **opacity** (`tx_reveal=local`) | small (≈0.05–0.15) | hard-ish left→right reveal edge (set axis=0 = x) |
| 2 | slither | normalized-axis coord | **position** (lateral sine, amp `1-local`) | medium | letters slither in, wobble dies as each settles |
| 3 | sparkle-in | `explode_hash3(index).x` | **opacity** (`tx_reveal=local`) | medium | random per-particle reveal; HDR Bloom makes it twinkle |
| 4 | spark-out | `explode_hash3(index).x` | **opacity** (`tx_reveal=1-local`) | medium | reversed reveal (a *leaving* look; see §4-caveat below) |
| 5 | vortex-true | radial fraction from center | **position** (unwind rotation, angle `1-local`) | medium | continuous t-driven vortex; the data-only `Swirl` is the cheap approximation |
| 6 | directional-wipe HARD | normalized-axis coord | **opacity** (`tx_reveal=local`) | ≈0 (tiny) | a true hard slab edge (1 with softness pinned near 0) |
| 7 | pen-write | **baked pen distance in `position.z`** | **opacity** (`tx_reveal=local`) | small | requires the §9 outline-walk in `text.rs` first — **the hard one** |

> mode 6 vs mode 1 differ only by `softness` (6 ≈ 0 → hard edge; 1 ≈ 0.1 → slightly
> feathered). They share `norm_axis` and the opacity sink. Keeping them as separate
> mode integers (rather than one mode + a softness=0 convention) makes the
> `part_director` mapping in §6 read cleanly and keeps the door open for them to
> diverge.

**spark-out caveat (DESIGN.md §4.4 / OQ#16).** mode 4 produces the *visual* of
sparking out, but the **timeline** has no dedicated "leaving" phase today — a part
holds, then the *next* part's morph-in begins. So mode 4 only reads as "leaving" if
the app gives it a slot to run on (a synthetic empty target, or the out-transition
timeline slot of OQ#16). The shader half is here; the timeline slot is a separate,
app-side open item. Do not promise true spark-out from the shader edit alone.

---

## 6. App-side hook (`src/main.rs`) — DESCRIBE ONLY, do not edit app code

`part_director` already sets the per-frame, per-part shader knob `cs.bulge` at
**`main.rs:494`**:

```rust
cs.bulge = if morphing && state.transitions[idx] == Transition::Morph { parts[idx].bulge } else { 0.0 };
```

The transition fields would be set right next to it, keyed off the part's resolved
transition (`state.transitions[idx]`, populated at `main.rs:202` / `main.rs:366`).
**Sketch (not to be applied):**

```rust
// alongside cs.bulge = …  (main.rs:494) — illustrative only
let (mode, soft, axis): (u32, f32, u32) = if morphing {
    match state.transitions[idx] {
        Transition::Typewriter => (1, 0.10, 0), // x
        Transition::Slither    => (2, 0.30, 0),
        Transition::Sparkle    => (3, 0.40, 0),
        Transition::SparkOut    => (4, 0.40, 0),
        Transition::Vortex     => (5, 0.35, 1), // about Y
        Transition::Wipe       => (6, 0.02, 0), // hard slab
        Transition::PenWrite   => (7, 0.08, 0),
        _ => (0, 0.0, 0),                        // every shipped transition → off, mode 0
    }
} else {
    (0, 0.0, 0) // not morphing → off; the held shape is plain (sort-safe)
};
cs.transition_mode = mode;
cs.transition_softness = soft;
cs.transition_axis = axis;
```

**This needs new `Transition` variants** — the per-particle ones above
(`Typewriter`, `Slither`, `Sparkle`, `SparkOut`, `Vortex`, `Wipe`, `PenWrite`) do
**not** exist in the live enum, which today is exactly
`Morph, Ball, Fade, Explode, Implode, Drop, Swirl` (`main.rs:152-160`) with
`Transition::parse` at `main.rs:162-175`. Adding a per-particle transition is the
same one-variant-plus-one-parse-arm pattern `DESIGN.md` §0.1 describes for the
data-only ones, except the "fn in morph.rs" is replaced by "set these three uniform
fields." All shipped transitions keep mapping to mode 0 (off), so existing shows are
unaffected. **Again: describe only — no app edit until OQ#12 says yes.**

---

## 7. Determinism (constraint #5 holds by construction)

`DESIGN.md` constraint #5: record mode must stay deterministic; `record_driver` is
frame-indexed (`clock.t = i*dt`), and `controls` / `advance_seq_clock` bail when
recording (verified: `advance_seq_clock` returns early on `rec.dir.is_some()` at
`main.rs:504-506`; the live clock advance is `clock.t += time.delta_secs()` at
`main.rs:508`, used only live).

The transition phase is a **pure function of `splat_index` + `position` +
uniforms** — there is no wall-clock read, no persistent RNG state, no frame-to-frame
accumulator inside the shader. `explode_hash3` is a stateless integer hash of
`splat_index` (`gaussian.wgsl:186-195`), exactly like the existing explode/ball
branches use. The only time input is `gaussian_uniforms.time`, which in record mode
is set from the frame index. Therefore:

> frame `i` → `time = i*dt` → `gt = f(time)` → `local = f(gt, phase(index, pos))` →
> identical pixels every run.

This is the same determinism argument the landed `bulge` and explode features
already satisfy; the edit adds no new state. Golden-frame pixel-diff (the per-step
gate in `DESIGN.md` §9.2) must pass trivially for mode 0 (byte-identical) and be
reproducible bit-for-bit for any active mode.

---

## 8. Sort safety (why this is in `vs_points`, NOT `interpolate.wgsl`)

The radix sort runs as a **compute pass over the morph-output buffer**, and
`vs_points` runs **later**, as the vertex stage that consumes the already-sorted
entries (`gaussian.wgsl:204-205`: `let entry = get_entry(instance_index); let
splat_index = entry.value;`). Consequences:

- A **position nudge in `vs_points`** (slither / vortex / pen lead-in) happens
  *after* the sort has already chosen draw order from the morph-output positions, so
  it is **invisible to the sort** — exactly like the existing explode displacement
  (`gaussian.wgsl:220-236`) and ball-pulse (`gaussian.wgsl:245-260`), which are both
  in `vs_points` for precisely this reason. Small per-particle nudges can cause minor
  local depth-order imperfections (same as the existing bulge), acceptable for a
  transient transition.
- The **opacity multiply** (typewriter / sparkle / wipe / pen reveal) is fully
  sort-safe: it changes only the emitted alpha, not position or order.

> **DO NOT put any of this in `interpolate.wgsl`.** That compute shader writes the
> morph-output buffer (`interpolate.wgsl:69-119`: `interpolate_gaussians` →
> `set_output_position_visibility` / `set_output_transform`), and **that buffer is
> exactly what the radix sort reads.** A per-particle reveal or displacement written
> there *would* be seen by the sort — defeating the whole "sort doesn't see it"
> property — and would also bake the transition into the buffer instead of being a
> cheap, default-off vertex-stage effect. `interpolate.wgsl` is included in this
> blueprint solely to show *where the edit must NOT go.*

This is, per `DESIGN.md` §5, "the cheapest, most correct insight in the whole shader
story," and it carries over verbatim from the bulge work.

---

## 9. Pen-write caveat (mode 7 — the hard one)

Per `DESIGN.md` §4.2 **B2** and §7.3: a *true* pen-write reveal follows the **pen
path**, not a straight axis sweep — so its `phase` must be **cumulative pen distance
along the glyph outline**, not `norm_axis`.

The blocker is in `text.rs`, not the shader. The live builder
`build_text_gaussians` (`src/text.rs:22-93`) rasterizes glyph **coverage**: it calls
`font.outline_glyph(...)` then `o.draw(|dx, dy, c| …)` (`text.rs:60-69`) and samples
*filled pixels* on a grid (`text.rs:70-90`). It **never walks the outline contours**
in pen order — coverage pixels come out in raster (row-major) order, which has no
relation to how a pen would draw the stroke.

So mode 7 needs a **new outline-walk code path** in `text.rs`, separate from the
coverage sampler:

- walk the `ab_glyph` `OutlineCurve` segments (lines / quad / cubic Béziers) in
  contour order,
- accumulate arc length to get each emitted gaussian's **cumulative pen distance**,
  normalize to `[0,1]` across the whole string,
- **bake that scalar into the gaussian's `z` channel** — which is **unused today**:
  every text gaussian is emitted at `position_visibility: [wx, wy, 0.0, 1.0]`
  (`text.rs:84`), i.e. `z == 0`. `DESIGN.md` §4.2 B2 calls out this "unused `z`
  channel idea." The shader's mode 7 then reads `position.z` as the phase (see the
  `transition_phase` mode-7 branch in §3a).

> **Important coupling:** baking pen distance into `z` means the text is no longer
> strictly flat at `z=0` during a pen-write. That's fine for the reveal phase, but
> the morph target / final resting positions and the `cloud_base_rotation` flip
> assume `z=0` flat text — so the baked `z` must either be (a) used only as a phase
> channel that the *morph target* doesn't see (i.e. the pen-distance is stored in a
> parallel buffer, not the live position), or (b) zeroed at `local==1` so the letter
> lands flat. Option (a) is cleaner but needs a channel the morph buffer carries;
> option (b) reuses the unused `z` directly but must be reconciled with the morph.
> This reconciliation is precisely why `DESIGN.md` rates B2 **hard**, not a
> one-liner. Treat mode 7 as a stretch goal of the fork, gated behind both OQ#12 and
> the `text.rs` outline-walk work.

For everything short of true pen order, mode 1 (typewriter, `norm_axis`) is the
in-budget left→right reveal and needs no `text.rs` change.

---

## 10. Upstream-PR shape + ready-to-paste CHANGES.md entry

### Keeping the diff clean / backward-compatible

The whole point of mirroring `bulge` is that this becomes a **clean, mergeable PR**
to `bevy_gaussian_splatting`, not a divergent fork martin has to carry forever:

- **Append-only.** New uniform fields go *after* `max: vec4` (the true struct end,
  §4) — they shift nothing, so no existing offset moves.
- **Default-off.** `transition_mode: 0` in `impl Default for CloudSettings` ⇒ every
  existing user gets identity behaviour with zero source changes. `#[serde(default)]`
  covers serialized settings.
- **Single concept, one uniform group.** No `sparkle_mode` / `slither_mode` sprawl —
  every effect routes through the one `transition_mode` integer (this is `DESIGN.md`
  §4.2's canonical-design note, honored here).
- **One isolated commit** (`DESIGN.md` §9.2 step 7: "isolated commit for the upstream
  PR"): the 4 plumbing spots + the helper + the gated branch + the one-line multiply.
  Nothing else in the same commit.
- **Mode 0 golden-frame test.** Ship the PR with a note that mode 0 is byte-identical;
  the reviewer can verify with a render diff. martin's own per-step golden-frame gate
  is the proof.

### Ready-to-paste CHANGES.md entry

> **Do NOT edit `CHANGES.md` now** — this is the *proposed* text to paste into
> the fork's `CHANGES.md` (`annejan/bevy_gaussian_splatting`, branch `martin`) **if and
> when** OQ#12 says yes and the edit is applied. Reproduced here only.

```markdown
## Unreleased

### Added
- **Per-particle transition phase** (opt-in, default-off). New `CloudSettings`
  fields `transition_mode: u32`, `transition_softness: f32`, `transition_axis: u32`
  drive a per-particle staggered reveal/motion effect in `vs_points`, enabling
  transitions a single global `time` uniform cannot express (typewriter, slither,
  sparkle in/out, true vortex, hard directional-wipe). Each particle derives a
  `phase ∈ [0,1]` from its `splat_index`/position; `time` sweeps a moving window
  `local = saturate((t*(1+softness) - phase)/softness)` that gates opacity or
  position. `transition_mode == 0` is the default and is **byte-identical to prior
  behaviour**. Mirrors the existing `bulge` plumbing; uniform fields are appended
  after `max` (no offset changes); the effect lives in `vs_points` (after the sort),
  so it is invisible to the radix sort. Pure function of `splat_index` + position +
  uniforms → deterministic.
```

---

## 11. RADV note (switch-on-uniform-int)

A `switch` statement on a **uniform integer** has been observed to mis-compile on
some Mesa/RADV driver versions (the AMD path martin targets — `DESIGN.md` §1.2
constraint #1: wgpu → Vulkan / Mesa RADV, no CUDA/ROCm). The failure mode is silent
wrong-branch selection or a fallthrough that the WGSL didn't intend.

**Mitigation (already applied in §3's sketch):** use an **`if / else if` chain** on
`gaussian_uniforms.transition_mode` rather than `switch`. Both `transition_phase`
(§3a) and the `vs_points` branch (§3b) are written as `if (mode == 1u) { … } else if
(mode == 2u) { … } …`. This compiles reliably across RADV versions and costs nothing
at the handful of modes involved (the branch is uniform-control-flow within a
warp/subgroup, so divergence is not even a concern — every invocation in a draw
shares the same `transition_mode`). Do not "tidy" it into a `switch` later.

---

## Appendix: discrepancies found vs DESIGN.md (verified against live code)

1. **`settings.rs` path is wrong in DESIGN.md.** DESIGN.md (§5, §1.2) cites
   `settings.rs:71-80`. The file is actually at
   **`src/gaussian/settings.rs`** (under `gaussian/`).
   The `CloudSettings` struct spans lines **60-81** (`bulge` at line 80), with the
   `time/time_scale/num_classes/color_space` cluster at lines 71-76. Line range is
   approximately right; the path is wrong.

2. **`CloudUniform` struct line range.** DESIGN.md cites "`mod.rs:979-983`" for the
   mirror. Verified: the *fields* `bulge`/`num_classes`/`color_space`/`min`/`max` are
   indeed at lines 979-983, but the **struct definition starts at line 970**
   (`pub struct CloudUniform {`) and the **construction literal** is at lines
   1031-1048. So "979-983" points at the tail fields, not the whole struct — minor,
   but worth knowing when applying the edit.

3. **Everything else verified correct.** The `bindings.wgsl` field order
   (`time_start, time_stop, bulge, num_classes, color_space, min: vec4, max: vec4`)
   is **exactly** as DESIGN.md claims — struct at lines 13-27, re-read directly, not
   trusted. The `opacity * gaussian_uniforms.global_opacity` finalize site is at
   `gaussian.wgsl:480-483` (multiply on line 482) as cited. `cs.bulge` is at
   `main.rs:494` as cited. The explode branch is at `gaussian.wgsl:220-236` as cited
   (DESIGN.md §4.2 B6). The ball-pulse "bulge" branch is at `gaussian.wgsl:245-260`.
   The `Transition` enum (`Morph, Ball, Fade, Explode, Implode, Drop, Swirl`) and its
   `parse` are at `main.rs:152-175` as described in §0.1. The text builder samples
   coverage (not outline order) at `text.rs:60-90`, confirming the §9 pen-write
   caveat, and emits `z=0` at `text.rs:84`, confirming the "unused z channel"
   available for pen-distance baking.
