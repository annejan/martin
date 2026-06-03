//! Morton (Z-order) correspondence + the intro ball.
//!
//! Resampling beats to a shared count via a Morton sort keeps spatial neighbours adjacent,
//! so consecutive beats pair k-th↔k-th and *flow* into one another instead of teleporting.
//! Pure functions over `Gaussian3d` — no Bevy/ECS.

use bevy::math::Vec3;
use bevy_gaussian_splatting::Gaussian3d;

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
/// beats pair k-th↔k-th in spatial order (coherent flow). Up/down-samples as needed.
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
/// 1:1, so it flies from the ball to its slot as the morph runs). The intro: beat 0 morphs
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
