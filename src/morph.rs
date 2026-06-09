//! Morton (Z-order) correspondence + the intro ball.
//!
//! Resampling parts to a shared count via a Morton sort keeps spatial neighbours adjacent,
//! so consecutive parts pair k-th↔k-th and *flow* into one another instead of teleporting.
//! Pure functions over `Gaussian3d` — no Bevy/ECS.

use bevy::math::{EulerRot, Quat, Vec3};
use bevy_gaussian_splatting::Gaussian3d;

/// Replicate `shape` into `copies` scattered, randomly-rotated instances — a "serving" (e.g. a pile
/// of bitterballen, never just one). Deterministic (index-hashed) so it's frame-stable for recording.
/// `spread` is the scatter radius as a multiple of the shape's own size; the whole pile is normalized
/// to frame afterwards, so each instance ends up small.
pub fn cluster_of(shape: &[Gaussian3d], copies: usize) -> Vec<Gaussian3d> {
    use std::f32::consts::TAU;
    const BALL: f32 = 0.58; // each instance's diameter, world units
    const DISC: f32 = 0.95; // serving radius (a plate of them) — the whole pile ≈ NORMALIZE_EXTENT
    let copies = copies.max(1);
    if shape.is_empty() {
        return Vec::new();
    }
    // ball centroid + a scale that makes each instance ~BALL across, and the whole serving fit the
    // frame — so a cluster part SKIPS the later normalize (build_sequence) and frames as one plate.
    let mut c = Vec3::ZERO;
    for g in shape {
        c += Vec3::from_array(g.position_visibility.position);
    }
    c /= shape.len() as f32;
    let scale = BALL / extent_of(shape).max(1e-3);
    let mut out = Vec::with_capacity(shape.len() * copies);
    for n in 0..copies {
        let k = n as u32 + 1;
        let q = Quat::from_euler(
            EulerRot::XYZ,
            hash01(k, 2_654_435_761) * TAU,
            hash01(k, 2_246_822_519) * TAU,
            hash01(k, 3_266_489_917) * TAU,
        );
        // uniform-ish in a flattened disc (a plate: wide, a little stacking in Y), random rotation.
        // (Distinct LARGE salts — small salts like 0x1111_1111·k are a near-linear, low-entropy ramp.)
        let ang = hash01(k, 668_265_263) * TAU;
        let rad = hash01(k, 374_761_393).sqrt() * DISC;
        let off = Vec3::new(
            rad * ang.cos(),
            (hash01(k, 1_274_126_177) - 0.5) * BALL * 1.5,
            rad * ang.sin(),
        );
        for g in shape {
            let mut s = *g;
            let local = (Vec3::from_array(s.position_visibility.position) - c) * scale;
            s.position_visibility.position = (q * local + off).to_array();
            let rr = s.rotation.rotation;
            let nq = (q * Quat::from_xyzw(rr[0], rr[1], rr[2], rr[3])).normalize();
            s.rotation = [nq.x, nq.y, nq.z, nq.w].into();
            // shrink the splat disks to match the shrunk ball (this cluster skips normalize).
            let sc = s.scale_opacity.scale;
            let op = s.scale_opacity.opacity;
            s.scale_opacity = [sc[0] * scale, sc[1] * scale, sc[2] * scale, op].into();
            out.push(s);
        }
    }
    out
}

/// Rotate a gaussian set in place by `q` (about the origin): both each splat's position AND its
/// orientation quaternion. Used to bake a per-part rotation into a shape so different parts of a
/// morph timeline can sit at different orientations (and the morph between them reorients smoothly).
pub fn rotate_gaussians(v: &mut [Gaussian3d], q: Quat) {
    if q == Quat::IDENTITY {
        return;
    }
    for g in v.iter_mut() {
        let p = g.position_visibility.position;
        g.position_visibility.position = (q * Vec3::from_array(p)).to_array();
        let r = g.rotation.rotation;
        let nq = (q * Quat::from_xyzw(r[0], r[1], r[2], r[3])).normalize();
        g.rotation = [nq.x, nq.y, nq.z, nq.w].into();
    }
}

/// Spread a 10-bit integer so its bits occupy every 3rd position (for Morton/Z-order).
fn part1by2(mut n: u32) -> u32 {
    n &= 0x3ff;
    n = (n | (n << 16)) & 0x030000ff;
    n = (n | (n << 8)) & 0x0300f00f;
    n = (n | (n << 4)) & 0x030c30c3;
    n = (n | (n << 2)) & 0x09249249;
    n
}

/// 30-bit Morton (Z-order) code of a position normalized into [lo, hi] per axis.
fn morton3(p: [f32; 3], lo: [f32; 3], inv: [f32; 3]) -> u32 {
    let q = |k: usize| -> u32 { (((p[k] - lo[k]) * inv[k]).clamp(0.0, 1.0) * 1023.0) as u32 };
    part1by2(q(0)) | (part1by2(q(1)) << 1) | (part1by2(q(2)) << 2)
}

/// Morton-sort a gaussian set over its own bounds and resample to exactly `n`, so consecutive
/// parts pair k-th↔k-th in spatial order (coherent flow). Up/down-samples as needed.
pub fn resample_morton(mut v: Vec<Gaussian3d>, n: usize) -> Vec<Gaussian3d> {
    if v.is_empty() || n == 0 {
        return Vec::new();
    }
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for g in &v {
        let p = g.position_visibility.position;
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let inv = [
        1.0 / (hi[0] - lo[0]).max(1e-6),
        1.0 / (hi[1] - lo[1]).max(1e-6),
        1.0 / (hi[2] - lo[2]).max(1e-6),
    ];
    v.sort_by_key(|g| morton3(g.position_visibility.position, lo, inv));
    let m = v.len();
    (0..n).map(|i| v[((i * m) / n).min(m - 1)]).collect()
}

/// Scatter each gaussian of `shape` onto a fuzzy sphere shell of radius `shell_r` (paired
/// 1:1, so it flies from the ball to its slot as the morph runs). The intro: part 0 morphs
/// from this ball into `shape`. No shader bulge needed — the ball IS the lhs.
pub fn ball_of(shape: &[Gaussian3d], shell_r: f32) -> Vec<Gaussian3d> {
    shape
        .iter()
        .enumerate()
        .map(|(idx, g)| {
            let k = idx as u32;
            let h = |s: u32| ((k.wrapping_mul(s) >> 8) & 0xffff) as f32 / 65535.0; // [0,1)
            let z = h(2_654_435_761) * 2.0 - 1.0;
            let a = h(40_503) * std::f32::consts::TAU;
            let rxy = (1.0 - z * z).max(0.0).sqrt();
            let r = shell_r * (0.45 + 0.55 * h(2_246_822_519));
            let p = Vec3::new(rxy * a.cos(), rxy * a.sin(), z) * r;
            let mut s = *g;
            s.position_visibility = [p.x, p.y, p.z, 1.0].into();
            s
        })
        .collect()
}

/// Per-particle deterministic pseudo-random in [0,1) from an index + salt — so transitions are
/// stable across runs (and identical frame-for-frame when recording).
fn hash01(k: u32, salt: u32) -> f32 {
    ((k.wrapping_mul(salt) >> 8) & 0xffff) as f32 / 65535.0
}

/// FADE source: the shape itself, opacity 0 — it simply fades up in place (no motion).
pub fn fade_of(shape: &[Gaussian3d]) -> Vec<Gaussian3d> {
    shape
        .iter()
        .map(|g| {
            let mut s = *g;
            let sc = s.scale_opacity.scale;
            s.scale_opacity = [sc[0], sc[1], sc[2], 0.0].into();
            s
        })
        .collect()
}

/// EXTRUDE source: the shape collapsed onto a single plane (its thinnest/depth axis flattened to
/// the mid-depth). The morph back to the shape then makes every particle *rise out of the flat
/// silhouette into 3D* — a flat logo extruding into its mesh. Index-paired with the target (each
/// particle keeps its in-plane position, only the depth grows), so it's a clean extrusion, not a
/// scatter. Great on a `mesh:`/`glb:` logo: svg-flat → mesh → splats in one move.
pub fn flatten_of(shape: &[Gaussian3d]) -> Vec<Gaussian3d> {
    // the depth axis = the one with the smallest extent (a logo is wide+tall, thin in depth).
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for g in shape {
        let p = g.position_visibility.position;
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let thin = (0..3)
        .min_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b])))
        .unwrap_or(2);
    let mid = (lo[thin] + hi[thin]) * 0.5;
    shape
        .iter()
        .map(|g| {
            let mut s = *g;
            let mut p = s.position_visibility.position;
            p[thin] = mid; // collapse depth → a flat silhouette
            s.position_visibility.position = p;
            s
        })
        .collect()
}

/// HELIX source: lay every (paired) particle on a tall vertical spiral column, so the morph reels
/// them in off the helix into the shape — a logo/word spiralling in from a DNA-like column. `height`
/// = column length, `turns` = how many full twists. Index-ordered along the column (like the ball:
/// an assemble-from-a-column, not a warp of the target's own positions).
pub fn helix_of(shape: &[Gaussian3d], height: f32, turns: f32) -> Vec<Gaussian3d> {
    use std::f32::consts::TAU;
    let n = shape.len().max(1) as f32;
    shape
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let f = i as f32 / n; // 0..1 along the column
            let a = f * turns * TAU + hash01(i as u32 + 1, 2_654_435_761) * 0.3;
            let r = 0.6;
            let mut s = *g;
            s.position_visibility = [r * a.cos(), (f - 0.5) * height, r * a.sin(), 1.0].into();
            s
        })
        .collect()
}

/// FOLD source: the shape collapsed onto a vertical seam (its width axis x→0), so the morph *unfolds*
/// it sideways out of a line — like opening a folded sheet. Index-paired (keeps each particle's y/z,
/// only x grows), so it's a clean unfold, not a scatter. Sibling of `flatten_of` on the other axis.
pub fn fold_of(shape: &[Gaussian3d]) -> Vec<Gaussian3d> {
    shape
        .iter()
        .map(|g| {
            let mut s = *g;
            let mut p = s.position_visibility.position;
            p[0] = 0.0; // collapse width → a vertical seam to unfold from
            s.position_visibility.position = p;
            s
        })
        .collect()
}

/// ZOOM source: every (paired) particle scaled `factor`× outward from the centroid, so the morph
/// *rushes it in* from far — a telescope / hyperspace zoom into place. Uniform scale (vs `explode`'s
/// random burst), so the shape stays readable as it screams in.
pub fn zoom_of(shape: &[Gaussian3d], factor: f32) -> Vec<Gaussian3d> {
    let mut c = Vec3::ZERO;
    for g in shape {
        c += Vec3::from_array(g.position_visibility.position);
    }
    c /= shape.len().max(1) as f32;
    shape
        .iter()
        .map(|g| {
            let mut s = *g;
            let p = Vec3::from_array(s.position_visibility.position);
            s.position_visibility.position = (c + (p - c) * factor).to_array();
            s
        })
        .collect()
}

/// EXPLODE source: each (paired) particle flung outward from the centre, so the morph *gathers*
/// the burst back into the shape. `spread` ≈ object radius.
pub fn explode_of(shape: &[Gaussian3d], spread: f32) -> Vec<Gaussian3d> {
    shape
        .iter()
        .enumerate()
        .map(|(idx, g)| {
            let k = idx as u32;
            let p = g.position_visibility.position;
            let len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-4);
            let m = spread * (0.6 + 1.4 * hash01(k, 2_654_435_761)); // outward distance
            let j = spread * 0.5;
            let mut s = *g;
            s.position_visibility.position = [
                p[0] + p[0] / len * m + (hash01(k, 40_503) - 0.5) * j,
                p[1] + p[1] / len * m + (hash01(k, 2_246_822_519) - 0.5) * j,
                p[2] + p[2] / len * m + (hash01(k, 3_266_489_917) - 0.5) * j,
            ];
            s
        })
        .collect()
}

/// IMPLODE source: particles collapsed to a dense speck at the centre, expanding out to place.
pub fn implode_of(shape: &[Gaussian3d]) -> Vec<Gaussian3d> {
    shape
        .iter()
        .enumerate()
        .map(|(idx, g)| {
            let k = idx as u32;
            let p = g.position_visibility.position;
            let j = 0.02;
            let mut s = *g;
            s.position_visibility.position = [
                p[0] * 0.03 + (hash01(k, 40_503) - 0.5) * j,
                p[1] * 0.03 + (hash01(k, 2_246_822_519) - 0.5) * j,
                p[2] * 0.03 + (hash01(k, 3_266_489_917) - 0.5) * j,
            ];
            s
        })
        .collect()
}

/// DROP source: particles lifted by ~`height` (staggered) and falling straight down into place.
pub fn drop_of(shape: &[Gaussian3d], height: f32) -> Vec<Gaussian3d> {
    shape
        .iter()
        .enumerate()
        .map(|(idx, g)| {
            let k = idx as u32;
            let p = g.position_visibility.position;
            let mut s = *g;
            s.position_visibility.position = [
                p[0],
                p[1] + height * (0.6 + 0.8 * hash01(k, 2_654_435_761)),
                p[2],
            ];
            s
        })
        .collect()
}

/// RAIN source: particles start high AND scattered sideways (staggered heights), so they fall
/// diagonally inward into place — a shower raining in, vs `drop`'s straight vertical fall.
pub fn rain_of(shape: &[Gaussian3d], height: f32) -> Vec<Gaussian3d> {
    shape
        .iter()
        .enumerate()
        .map(|(idx, g)| {
            let k = idx as u32;
            let p = g.position_visibility.position;
            let spread = height * 0.6;
            let mut s = *g;
            s.position_visibility.position = [
                p[0] + (hash01(k, 40_503) - 0.5) * spread,
                p[1] + height * (0.2 + 1.4 * hash01(k, 2_654_435_761)), // staggered fall heights
                p[2] + (hash01(k, 2_246_822_519) - 0.5) * spread,
            ];
            s
        })
        .collect()
}

/// FUNNEL source: particles start in a tall narrow column high above the centre, then fan out and
/// down into the shape — a pour / funnel (vs `drop`'s straight fall or `rain`'s wide scatter).
pub fn funnel_of(shape: &[Gaussian3d], height: f32) -> Vec<Gaussian3d> {
    use std::f32::consts::TAU;
    shape
        .iter()
        .enumerate()
        .map(|(idx, g)| {
            let k = idx as u32;
            let p = g.position_visibility.position;
            let lift = height * (0.5 + hash01(k, 2_654_435_761));
            let ang = hash01(k, 40_503) * TAU;
            let rad = 0.18 * height * hash01(k, 2_246_822_519); // narrow spout
            let mut s = *g;
            s.position_visibility.position = [rad * ang.cos(), p[1] + lift, rad * ang.sin()];
            s
        })
        .collect()
}

/// CONDENSE source: particles fill a wide diffuse ball, faded to nothing — the shape condenses out
/// of a haze (positions converge + opacity fades up), vs `ball`'s organized full-opacity shell.
pub fn condense_of(shape: &[Gaussian3d], spread: f32) -> Vec<Gaussian3d> {
    use std::f32::consts::TAU;
    shape
        .iter()
        .enumerate()
        .map(|(idx, g)| {
            let k = idx as u32;
            let z = hash01(k, 2_654_435_761) * 2.0 - 1.0;
            let ang = hash01(k, 40_503) * TAU;
            let rxy = (1.0 - z * z).max(0.0).sqrt();
            let r = spread * hash01(k, 2_246_822_519).cbrt(); // uniform throughout the ball (filled)
            let mut s = *g;
            let sc = s.scale_opacity.scale;
            s.position_visibility = [r * rxy * ang.cos(), r * rxy * ang.sin(), r * z, 1.0].into();
            s.scale_opacity = [sc[0], sc[1], sc[2], 0.0].into(); // start as haze → fades up
            s
        })
        .collect()
}

/// SHATTER source: the shape broken into ~8 chunks (contiguous Morton blocks ≈ spatial pieces),
/// each flung out + tumbled; the morph flies them back together — it re-assembles from shards.
pub fn shatter_of(shape: &[Gaussian3d], spread: f32) -> Vec<Gaussian3d> {
    use std::f32::consts::TAU;

    use bevy::math::EulerRot;
    let n = shape.len().max(1);
    let chunks = 8u32;
    shape
        .iter()
        .enumerate()
        .map(|(idx, g)| {
            let c = (idx as u64 * chunks as u64 / n as u64) as u32 + 1; // which shard
            let q = Quat::from_euler(
                EulerRot::XYZ,
                hash01(c, 2_654_435_761) * TAU,
                hash01(c, 2_246_822_519) * TAU,
                hash01(c, 3_266_489_917) * TAU,
            );
            let off = Vec3::new(
                (hash01(c, 668_265_263) - 0.5) * 2.0 * spread,
                (hash01(c, 374_761_393) - 0.5) * 2.0 * spread,
                (hash01(c, 1_274_126_177) - 0.5) * 2.0 * spread,
            );
            let mut s = *g;
            s.position_visibility.position =
                (q * Vec3::from_array(s.position_visibility.position) + off).to_array();
            s
        })
        .collect()
}

// ---- DEPARTURE target clouds: the shape morphs TO one of these as a part LEAVES. All end faded
// (opacity 0) + displaced, so the object dissolves away rather than cross-morphing to the next. ----

/// Fade a gaussian's opacity to 0 (it's leaving).
fn faded(mut g: Gaussian3d) -> Gaussian3d {
    let sc = g.scale_opacity.scale;
    g.scale_opacity = [sc[0], sc[1], sc[2], 0.0].into();
    g
}

/// WASH-AWAY: flows off along +X (downstream spread + a little settle) and fades — washed away.
pub fn wash_of(shape: &[Gaussian3d], dist: f32) -> Vec<Gaussian3d> {
    shape
        .iter()
        .enumerate()
        .map(|(idx, g)| {
            let k = idx as u32;
            let p = g.position_visibility.position;
            let mut s = faded(*g);
            s.position_visibility.position = [
                p[0] + dist * (0.6 + 1.0 * hash01(k, 2_654_435_761)),
                p[1] - dist * 0.15 * hash01(k, 40_503),
                p[2] + (hash01(k, 2_246_822_519) - 0.5) * dist * 0.4,
            ];
            s
        })
        .collect()
}

/// DISPERSE: scatters outward in every direction and fades — blown to dust.
pub fn disperse_of(shape: &[Gaussian3d], spread: f32) -> Vec<Gaussian3d> {
    shape
        .iter()
        .enumerate()
        .map(|(idx, g)| {
            let k = idx as u32;
            let p = g.position_visibility.position;
            let m = spread * (0.4 + 1.6 * hash01(k, 2_654_435_761));
            let len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-3);
            let mut s = faded(*g);
            s.position_visibility.position = [
                p[0] + p[0] / len * m + (hash01(k, 40_503) - 0.5) * spread,
                p[1] + p[1] / len * m + (hash01(k, 2_246_822_519) - 0.5) * spread,
                p[2] + p[2] / len * m + (hash01(k, 3_266_489_917) - 0.5) * spread,
            ];
            s
        })
        .collect()
}

/// EVAPORATE: drifts upward (staggered) with a little sideways waft, and fades — rises away.
pub fn evaporate_of(shape: &[Gaussian3d], height: f32) -> Vec<Gaussian3d> {
    shape
        .iter()
        .enumerate()
        .map(|(idx, g)| {
            let k = idx as u32;
            let p = g.position_visibility.position;
            let mut s = faded(*g);
            s.position_visibility.position = [
                p[0] + (hash01(k, 40_503) - 0.5) * height * 0.3,
                p[1] + height * (0.3 + 1.2 * hash01(k, 2_654_435_761)),
                p[2] + (hash01(k, 2_246_822_519) - 0.5) * height * 0.3,
            ];
            s
        })
        .collect()
}

/// SINK: falls straight down (staggered) and fades — drops out the bottom.
pub fn sink_of(shape: &[Gaussian3d], depth: f32) -> Vec<Gaussian3d> {
    shape
        .iter()
        .enumerate()
        .map(|(idx, g)| {
            let k = idx as u32;
            let p = g.position_visibility.position;
            let mut s = faded(*g);
            s.position_visibility.position = [
                p[0],
                p[1] - depth * (0.4 + 1.2 * hash01(k, 2_654_435_761)),
                p[2],
            ];
            s
        })
        .collect()
}

/// SWIRL source: shape rotated about the vertical (Y) axis and pushed out, so it sweeps in.
/// (Linear position lerp → an approximate spiral; a true arc would need the shader.)
pub fn swirl_of(shape: &[Gaussian3d], angle: f32, expand: f32) -> Vec<Gaussian3d> {
    let (sa, ca) = angle.sin_cos();
    shape
        .iter()
        .map(|g| {
            let p = g.position_visibility.position;
            let (x, z) = (p[0] * expand, p[2] * expand);
            let mut s = *g;
            s.position_visibility.position = [x * ca - z * sa, p[1], x * sa + z * ca];
            s
        })
        .collect()
}

/// Largest bounding-box dimension of a gaussian set (its "size" in world units).
pub fn extent_of(v: &[Gaussian3d]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for g in v {
        let p = g.position_visibility.position;
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f32::max)
}

/// Center a gaussian set on its centroid and uniformly scale it so the *bulk* of its content
/// — the 90th-percentile distance from the centroid — spans `target` units, scaling positions
/// AND each gaussian's size together. Using a percentile rather than the bounding box makes it
/// robust to the stray "floater" gaussians that 3DGS scenes always carry: floaters would
/// otherwise blow up the bbox and shrink the real scene to a distant dot. Brings wildly
/// different sources (a huge COLMAP scene vs a tiny TRELLIS object) to one consistent "normal"
/// scale, so they frame well and morph cleanly between each other. Returns the
/// `(center, scale)` it applied (`p' = (p - center) * scale`) so a camera pose in the same
/// source coordinates can be transformed to match (see `MARTIN_CAMERAS`).
pub fn normalize_to(v: &mut [Gaussian3d], target: f32) -> (Vec3, f32) {
    if v.is_empty() {
        return (Vec3::ZERO, 1.0);
    }
    // centroid (mean position) — the dense centre, not pulled around by bbox extremes
    let mut sum = [0f64; 3];
    for g in v.iter() {
        let p = g.position_visibility.position;
        for k in 0..3 {
            sum[k] += p[k] as f64;
        }
    }
    let nf = v.len() as f64;
    let center = [
        (sum[0] / nf) as f32,
        (sum[1] / nf) as f32,
        (sum[2] / nf) as f32,
    ];
    // 90th-percentile distance from the centroid → ignore the far ~10% (the floaters)
    let mut dists: Vec<f32> = v
        .iter()
        .map(|g| {
            let p = g.position_visibility.position;
            ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2) + (p[2] - center[2]).powi(2))
                .sqrt()
        })
        .collect();
    let k = ((dists.len() as f32 * 0.90) as usize).min(dists.len() - 1);
    dists.select_nth_unstable_by(k, f32::total_cmp);
    let s = (target * 0.5) / dists[k].max(1e-6); // 90% of content fits within target/2 of centre
    for g in v.iter_mut() {
        let p = g.position_visibility.position;
        let vis = g.position_visibility.visibility;
        g.position_visibility = [
            (p[0] - center[0]) * s,
            (p[1] - center[1]) * s,
            (p[2] - center[2]) * s,
            vis,
        ]
        .into();
        let sc = g.scale_opacity.scale;
        let op = g.scale_opacity.opacity;
        g.scale_opacity = [sc[0] * s, sc[1] * s, sc[2] * s, op].into();
    }
    (Vec3::from_array(center), s)
}

#[cfg(test)]
mod tests {
    use bevy_gaussian_splatting::Gaussian3d;

    use super::*;

    fn g(x: f32, y: f32, z: f32) -> Gaussian3d {
        Gaussian3d {
            position_visibility: [x, y, z, 1.0].into(),
            rotation: [0.0, 0.0, 0.0, 1.0].into(),
            scale_opacity: [0.01, 0.01, 0.01, 1.0].into(),
            ..Default::default()
        }
    }

    #[test]
    fn flatten_collapses_the_thinnest_axis_and_keeps_the_others() {
        // wide in x, tall in y, THIN in z → flatten should equalise z, preserve x/y, index-paired.
        let src = vec![g(-5.0, -3.0, 0.1), g(5.0, 3.0, -0.1), g(0.0, 0.0, 0.05)];
        let flat = flatten_of(&src);
        assert_eq!(flat.len(), src.len());
        let z0 = flat[0].position_visibility.position[2];
        for (i, f) in flat.iter().enumerate() {
            let p = f.position_visibility.position;
            let s = src[i].position_visibility.position;
            assert!((p[0] - s[0]).abs() < 1e-6, "x preserved"); // in-plane kept
            assert!((p[1] - s[1]).abs() < 1e-6, "y preserved");
            assert!((p[2] - z0).abs() < 1e-6, "z collapsed to one plane");
        }
        assert!(z0.abs() < 0.1); // ~mid-depth of [-0.1, 0.1]
    }

    #[test]
    fn fold_collapses_width_keeps_height_and_depth() {
        let src = vec![g(-4.0, 1.0, 2.0), g(4.0, -1.0, -2.0)];
        let f = fold_of(&src);
        for (i, p) in f.iter().enumerate() {
            let q = p.position_visibility.position;
            assert_eq!(q[0], 0.0); // width → seam
            assert_eq!(q[1], src[i].position_visibility.position[1]); // y kept
            assert_eq!(q[2], src[i].position_visibility.position[2]); // z kept
        }
    }

    #[test]
    fn zoom_scales_out_from_the_centroid_index_paired() {
        let src = vec![g(2.0, 0.0, 0.0), g(-2.0, 0.0, 0.0)]; // centroid at origin
        let z = zoom_of(&src, 5.0);
        assert_eq!(z[0].position_visibility.position[0], 10.0); // 2 * 5
        assert_eq!(z[1].position_visibility.position[0], -10.0);
    }

    #[test]
    fn helix_lays_points_on_a_column_of_the_right_count() {
        let src: Vec<Gaussian3d> = (0..100).map(|i| g(i as f32, 0.0, 0.0)).collect();
        let h = helix_of(&src, 4.0, 3.0);
        assert_eq!(h.len(), 100);
        // every point sits on the r=0.6 cylinder, spread over the column height.
        let ys: Vec<f32> = h
            .iter()
            .map(|p| p.position_visibility.position[1])
            .collect();
        assert!(ys.iter().cloned().fold(f32::MAX, f32::min) < -1.5);
        assert!(ys.iter().cloned().fold(f32::MIN, f32::max) > 1.5);
    }

    #[test]
    fn resample_morton_hits_the_target_count() {
        let src: Vec<Gaussian3d> = (0..50).map(|i| g(i as f32, 0.0, 0.0)).collect();
        assert_eq!(resample_morton(src.clone(), 200).len(), 200); // up-sample
        assert_eq!(resample_morton(src, 10).len(), 10); // down-sample
    }
}
