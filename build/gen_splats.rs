// SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
// SPDX-License-Identifier: MIT
//
//! Procedural demo splats — the Rust port (and replacement) of the old `pipeline/gen-demo-splats.py`.
//! `build.rs` calls [`ensure_splats`] to synthesize any referenced `.ply` that's missing, so a fresh
//! `git clone` builds + runs the default show with no python/numpy step. The eleven shapes all morph
//! cleanly into one another (similar counts, vivid colours).
//!
//! Not bit-exact with the python (a different RNG) — it doesn't need to be: martin morton-resamples
//! each cloud on load, so only the SHAPE + colour matter, not point order. Deterministic per shape
//! (fixed seed) so rebuilds are stable. sh0 `.ply` layout: x y z | scale_0..2 (log) |
//! opacity (logit) | rot wxyz (identity) | f_dc_0..2 (SH0). ~140k splats/shape.

use std::path::Path;

const N: usize = 140_000;
const SPLAT: f32 = 0.02; // splat radius (shapes span ~±1)
const ALPHA: f32 = 0.92; // opacity

/// For each referenced `*.ply` that's missing and is a shape we know how to synthesize, generate it.
/// Names we don't recognise (real captures) are left alone — they fail loudly downstream if absent.
pub fn ensure_splats(asset_dir: &Path, names: &[String]) {
    for name in names {
        let Some(stem) = name.strip_suffix(".ply") else {
            continue;
        };
        let path = asset_dir.join(name);
        if path.exists() {
            continue;
        }
        let Some((pos, rgb)) = gen_shape(stem) else {
            continue;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        write_ply(&path, stem, &pos, &rgb);
        println!(
            "cargo:warning=gen: synthesized {} ({N} splats)",
            path.display()
        );
    }
}

/// A tiny splitmix64 PRNG — enough for uniform/normal/integer draws, no `rand` crate in build deps.
struct Rng(u64);
impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// [0, 1)
    fn unit(&mut self) -> f32 {
        ((self.next_u64() >> 11) as f64 / (1u64 << 53) as f64) as f32
    }
    fn uniform(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.unit()
    }
    fn int(&mut self, k: u64) -> u64 {
        self.next_u64() % k
    }
    /// Box–Muller (one of the pair) — plenty for jitter / scattered clouds.
    fn normal(&mut self, mu: f32, sigma: f32) -> f32 {
        let u1 = self.unit().max(1e-7);
        let u2 = self.unit();
        let z = (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos();
        mu + sigma * z
    }
    fn normal3(&mut self) -> [f32; 3] {
        [
            self.normal(0.0, 1.0),
            self.normal(0.0, 1.0),
            self.normal(0.0, 1.0),
        ]
    }
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    [v[0] / l, v[1] / l, v[2] / l]
}

/// h,s,v in [0,1] → (r,g,b) in [0,1] (matches the python `hsv`).
fn hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let i = h6.floor() as i32;
    let f = h6 - i as f32;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i.rem_euclid(6) {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

type Cloud = (Vec<[f32; 3]>, Vec<[f32; 3]>);

/// Synthesize a named shape (all eleven the python tool produced); unknown name → None.
fn gen_shape(stem: &str) -> Option<Cloud> {
    let mut rng = Rng(0xDEFEE5);
    let pi = std::f32::consts::PI;
    let tau = std::f32::consts::TAU;
    let (mut pos, mut rgb) = (Vec::with_capacity(N), Vec::with_capacity(N));
    match stem {
        "sphere" => {
            for _ in 0..N {
                let d = norm(rng.normal3());
                let r = rng.unit().powf(1.0 / 3.0);
                pos.push([d[0] * r, d[1] * r, d[2] * r]);
                rgb.push(hsv(d[2].atan2(d[0]) / tau + 0.5, 0.85, 1.0));
            }
        }
        "cube" => {
            for _ in 0..N {
                let p = [
                    rng.uniform(-1.0, 1.0),
                    rng.uniform(-1.0, 1.0),
                    rng.uniform(-1.0, 1.0),
                ];
                pos.push(p);
                rgb.push([(p[0] + 1.0) / 2.0, (p[1] + 1.0) / 2.0, (p[2] + 1.0) / 2.0]);
            }
        }
        "torus" | "ring" => {
            let (rad, tube, sat) = if stem == "torus" {
                (0.72, 0.3, 0.9)
            } else {
                (0.92, 0.08, 1.0)
            };
            for _ in 0..N {
                let u = rng.uniform(0.0, tau);
                let v = rng.uniform(0.0, tau);
                pos.push([
                    (rad + tube * v.cos()) * u.cos(),
                    tube * v.sin(),
                    (rad + tube * v.cos()) * u.sin(),
                ]);
                rgb.push(hsv(u / tau, sat, 1.0));
            }
        }
        "helix" => {
            for _ in 0..N {
                let t = rng.uniform(0.0, 6.0 * pi);
                let strand = rng.int(2);
                let phase = strand as f32 * pi;
                let j = [
                    rng.normal(0.0, 0.03),
                    rng.normal(0.0, 0.03),
                    rng.normal(0.0, 0.03),
                ];
                pos.push([
                    0.45 * (t + phase).cos() + j[0],
                    t / (3.0 * pi) - 1.0 + j[1],
                    0.45 * (t + phase).sin() + j[2],
                ]);
                rgb.push(if strand == 0 {
                    [0.1, 0.9, 1.0]
                } else {
                    [1.0, 0.2, 0.8]
                });
            }
        }
        "galaxy" => {
            let arms = 3u64;
            for _ in 0..N {
                let r = rng.unit().powf(0.7);
                let arm = rng.int(arms) as f32;
                let theta = arm * (tau / arms as f32) + r * 5.0 + rng.normal(0.0, 0.25);
                let y = rng.normal(0.0, 0.04) * (1.2 - r);
                pos.push([r * theta.cos(), y, r * theta.sin()]);
                rgb.push(hsv(0.6 + r * 0.35, 0.8, 1.0));
            }
        }
        "star" => {
            let spikes: Vec<[f32; 3]> = (0..24).map(|_| norm(rng.normal3())).collect();
            for _ in 0..N {
                let s = spikes[rng.int(24) as usize];
                let r = rng.unit().powf(0.5);
                let k = 0.12 * (1.0 - r);
                let perp = rng.normal3();
                pos.push([
                    s[0] * r + perp[0] * k,
                    s[1] * r + perp[1] * k,
                    s[2] * r + perp[2] * k,
                ]);
                rgb.push(hsv(0.05 + r * 0.12, 0.95, 1.0));
            }
        }
        "wave" => {
            for _ in 0..N {
                let x = rng.uniform(-1.0, 1.0);
                let z = rng.uniform(-1.0, 1.0);
                let y = 0.35 * (3.2 * x).sin() * (3.2 * z).cos();
                pos.push([x, y, z]);
                rgb.push(hsv(0.55 + y, 0.85, 1.0));
            }
        }
        "knot" => {
            for _ in 0..N {
                let t = rng.uniform(0.0, tau);
                pos.push([
                    (t.sin() + 2.0 * (2.0 * t).sin()) / 3.2 + rng.normal(0.0, 0.035),
                    (t.cos() - 2.0 * (2.0 * t).cos()) / 3.2 + rng.normal(0.0, 0.035),
                    -(3.0 * t).sin() / 3.2 + rng.normal(0.0, 0.035),
                ]);
                rgb.push(hsv(t / tau, 0.9, 1.0));
            }
        }
        "mobius" => {
            for _ in 0..N {
                let u = rng.uniform(0.0, tau);
                let half = rng.uniform(-1.0, 1.0) * 0.4;
                pos.push([
                    (1.0 + half * (u / 2.0).cos()) * u.cos() * 0.82,
                    (1.0 + half * (u / 2.0).cos()) * u.sin() * 0.82,
                    half * (u / 2.0).sin() * 0.82,
                ]);
                rgb.push(hsv(u / tau, 0.85, 1.0));
            }
        }
        "supershape" => {
            // 3D superformula — one organic "bloom" of many lobes.
            let sf = |a: f32| {
                let t = 7.0 * a / 4.0;
                let r = (t.cos().abs().powf(1.7) + t.sin().abs().powf(1.7) + 1e-9).powf(-1.0 / 0.3);
                r.clamp(0.0, 3.0)
            };
            let mut raw = Vec::with_capacity(N);
            let mut maxv = 1e-6f32;
            for _ in 0..N {
                let th = rng.uniform(-pi / 2.0, pi / 2.0);
                let ph = rng.uniform(-pi, pi);
                let (r1, r2) = (sf(th), sf(ph));
                let p = [
                    r1 * th.cos() * r2 * ph.cos(),
                    r2 * th.sin(),
                    r1 * th.cos() * r2 * ph.sin(),
                ]
                .map(|c: f32| if c.is_finite() { c } else { 0.0 });
                maxv = maxv.max(p[0].abs()).max(p[1].abs()).max(p[2].abs());
                raw.push((p, ph));
            }
            for (p, ph) in raw {
                pos.push([p[0] / maxv, p[1] / maxv, p[2] / maxv]);
                rgb.push(hsv(0.55 + 0.4 * (2.0 * ph).sin(), 0.9, 1.0));
            }
        }
        _ => return None,
    }
    Some((pos, rgb))
}

/// Write a cloud as martin's sh0 binary `.ply`.
fn write_ply(path: &Path, name: &str, pos: &[[f32; 3]], rgb: &[[f32; 3]]) {
    let scale = SPLAT.ln();
    let opacity = (ALPHA / (1.0 - ALPHA)).ln();
    let header = format!(
        "ply\nformat binary_little_endian 1.0\ncomment martin demo splat: {name}\n\
         element vertex {}\n\
         property float x\nproperty float y\nproperty float z\n\
         property float scale_0\nproperty float scale_1\nproperty float scale_2\n\
         property float opacity\n\
         property float rot_0\nproperty float rot_1\nproperty float rot_2\nproperty float rot_3\n\
         property float f_dc_0\nproperty float f_dc_1\nproperty float f_dc_2\nend_header\n",
        pos.len()
    );
    let mut buf = Vec::with_capacity(header.len() + pos.len() * 56);
    buf.extend_from_slice(header.as_bytes());
    let mut put = |v: f32| buf.extend_from_slice(&v.to_le_bytes());
    for (p, c) in pos.iter().zip(rgb) {
        put(p[0]);
        put(p[1]);
        put(p[2]);
        put(scale);
        put(scale);
        put(scale);
        put(opacity);
        put(1.0);
        put(0.0);
        put(0.0);
        put(0.0); // identity rot wxyz
        put((c[0] - 0.5) / 0.282_094_8);
        put((c[1] - 0.5) / 0.282_094_8);
        put((c[2] - 0.5) / 0.282_094_8);
    }
    std::fs::write(path, &buf).unwrap_or_else(|e| panic!("gen: write {}: {e}", path.display()));
}
