//! `tint:<mode>` — a CPU colour routine that recolours a sampled cloud in place. Demoscene candy:
//! we have the cores, so per-splat noise/HSV is free. Two looks (the user wanted both):
//! - **fry** — a real bitterbal: orangey-beige in the crevices → dark brown on the crust peaks, driven
//!   by 3D value noise over the surface (the deep-fried blotch).
//! - **rainbow** — psychedelic: hue wraps around the globule by direction, value by height.
//!
//! Colours are written to the degree-0 SH (the same `dc()` encode as text.rs / mesh.rs).

use bevy_gaussian_splatting::Gaussian3d;

/// 3DGS degree-0 encode: rendered colour ≈ 0.5 + 0.2820948·dc, so invert for a target sRGB.
fn dc(c: f32) -> f32 {
    (c - 0.5) / 0.282_094_79
}

/// Which colour routine to run (`tint:fry` / `tint:rainbow`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tint {
    /// Deep-fried bitterbal: beige crevices → brown crust peaks (noise-driven).
    Fry,
    /// Hue wraps around the cloud by direction; value by height.
    Rainbow,
}

impl Tint {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "fry" | "bitterbal" | "deepfry" | "deep-fry" | "brown" => Some(Self::Fry),
            "rainbow" | "candy" | "psych" | "psychedelic" => Some(Self::Rainbow),
            _ => None,
        }
    }
}

/// Recolour `splats` in place by `tint`, around their own centroid + extent (so it's scale-invariant).
pub fn apply(splats: &mut [Gaussian3d], tint: Tint) {
    if splats.is_empty() {
        return;
    }
    let n = splats.len() as f32;
    let mut c = [0.0f32; 3];
    for g in splats.iter() {
        let p = g.position_visibility.position;
        for k in 0..3 {
            c[k] += p[k];
        }
    }
    for v in &mut c {
        *v /= n;
    }
    // extent: mean distance from centroid → a stable unit so the noise frequency reads the same
    // whatever the object's size.
    let mut ext = 0.0f32;
    for g in splats.iter() {
        let p = g.position_visibility.position;
        ext += ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2)).sqrt();
    }
    let inv = (n / ext.max(1e-6)).max(1e-6); // ≈ 1/mean-radius
    for g in splats.iter_mut() {
        let p = g.position_visibility.position;
        let l = [
            (p[0] - c[0]) * inv,
            (p[1] - c[1]) * inv,
            (p[2] - c[2]) * inv,
        ];
        let rgb = match tint {
            Tint::Fry => fry(l),
            Tint::Rainbow => rainbow(l),
        };
        let sh = &mut g.spherical_harmonic.coefficients;
        sh[0] = dc(rgb[0]);
        sh[1] = dc(rgb[1]);
        sh[2] = dc(rgb[2]);
    }
}

/// Deep-fry: blend beige↔brown by a fractal value-noise field; the high-noise lumps read as the
/// browner crust peaks, the lows as the lighter crevices. A touch of radial darkening on the outside.
fn fry(l: [f32; 3]) -> [f32; 3] {
    let beige = [0.86, 0.58, 0.30];
    let brown = [0.28, 0.13, 0.04];
    // two octaves of value noise → blotchy crust
    let n = 0.65 * vnoise([l[0] * 2.6, l[1] * 2.6, l[2] * 2.6])
        + 0.35 * vnoise([l[0] * 6.1 + 11.0, l[1] * 6.1 + 7.0, l[2] * 6.1 + 3.0]);
    let r = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
    let t = (smoothstep(n) * 0.85 + (r * 0.25).min(0.15)).clamp(0.0, 1.0);
    [
        beige[0] + (brown[0] - beige[0]) * t,
        beige[1] + (brown[1] - beige[1]) * t,
        beige[2] + (brown[2] - beige[2]) * t,
    ]
}

/// Rainbow: hue wraps around the cloud by azimuth, lightly modulated by height; full sat, bright.
fn rainbow(l: [f32; 3]) -> [f32; 3] {
    let az = l[2].atan2(l[0]) / std::f32::consts::TAU + 0.5; // 0..1 around
    let h = (az + l[1] * 0.15).rem_euclid(1.0);
    hsv2rgb(h, 0.9, 1.0)
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Cheap deterministic 3D value noise: trilinear blend of hashed lattice corners → [0,1].
fn vnoise(p: [f32; 3]) -> f32 {
    let fl = [p[0].floor(), p[1].floor(), p[2].floor()];
    let f = [p[0] - fl[0], p[1] - fl[1], p[2] - fl[2]];
    let u = [smoothstep(f[0]), smoothstep(f[1]), smoothstep(f[2])];
    let i = [fl[0] as i32, fl[1] as i32, fl[2] as i32];
    let corner = |dx: i32, dy: i32, dz: i32| hash3(i[0] + dx, i[1] + dy, i[2] + dz);
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), u[0]);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), u[0]);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), u[0]);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), u[0]);
    lerp(lerp(x00, x10, u[1]), lerp(x01, x11, u[1]), u[2])
}

/// Integer hash → [0,1].
fn hash3(x: i32, y: i32, z: i32) -> f32 {
    let mut h = (x as u32)
        .wrapping_mul(374_761_393)
        .wrapping_add((y as u32).wrapping_mul(668_265_263))
        .wrapping_add((z as u32).wrapping_mul(2_246_822_519));
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0
}

fn hsv2rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = (h.rem_euclid(1.0)) * 6.0;
    let c = v * s;
    let x = c * (1.0 - (h6.rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h6 as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + m, g + m, b + m]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_modes() {
        assert_eq!(Tint::parse("fry"), Some(Tint::Fry));
        assert_eq!(Tint::parse("RAINBOW"), Some(Tint::Rainbow));
        assert_eq!(Tint::parse("bitterbal"), Some(Tint::Fry));
        assert_eq!(Tint::parse("nope"), None);
    }

    #[test]
    fn vnoise_in_range_and_deterministic() {
        let a = vnoise([1.3, 2.7, 0.4]);
        let b = vnoise([1.3, 2.7, 0.4]);
        assert_eq!(a, b);
        assert!((0.0..=1.0).contains(&a));
    }

    #[test]
    fn hsv_primaries() {
        assert_eq!(hsv2rgb(0.0, 1.0, 1.0), [1.0, 0.0, 0.0]); // red
        let g = hsv2rgb(1.0 / 3.0, 1.0, 1.0);
        assert!(g[1] > 0.99 && g[0] < 0.01 && g[2] < 0.01); // green
    }
}
