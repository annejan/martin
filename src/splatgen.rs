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
const ALPHA: f32 = 0.60; // per-splat opacity — soft. At high counts overlap still reads solid; at low
// counts (e.g. camp props ~12k via morph_count) it gives the airy, translucent splat look. See
// memory martin-diorama-safe-z / the density "diminishing returns" finding (140k was ~10x wasteful).

/// Every shape `gen_shape` knows how to synthesize (for the `splatgen` CLI's `list` + validation).
/// `allow(dead_code)`: used by the `splatgen` bin, unreferenced when this file is included by `build.rs`.
#[allow(dead_code)]
pub const SHAPES: &[&str] = &[
    "sphere",
    "cube",
    "torus",
    "ring",
    "helix",
    "galaxy",
    "star",
    "wave",
    "knot",
    "mobius",
    "supershape",
    "lsystem",
    "fern",
    "menger",
    "shell",
    "tent",
    "pine",
    "flame",
    "rocket",
    "terrain",
    "moon",
    "mountains",
];

/// For each referenced `*.ply` that's MISSING and is a shape we know how to synthesize, generate it
/// into `asset_dir`. Real captures (names we don't recognise) are left alone. Returns the names it
/// generated — the caller logs them (build.rs as a `cargo:warning`, the runtime as an `info!`); this
/// fn itself prints nothing, since it runs both at build time AND at startup (bundle/dev).
/// `allow(dead_code)`: unreferenced when this file is `#[path]`-included by the `splatgen` bin.
#[allow(dead_code)]
pub fn ensure_splats(asset_dir: &Path, names: &[String]) -> Vec<String> {
    let mut made = Vec::new();
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
        made.push(name.clone());
    }
    made
}

/// Whether `name` is a `.ply` we can synthesize at runtime — so the bundle can SKIP baking it (it's
/// regenerated on startup) and ship a leaner binary. Real captures (`austin_tight.ply`, …) return false.
#[allow(dead_code)]
pub fn is_procedural_ply(name: &str) -> bool {
    name.strip_suffix(".ply")
        .is_some_and(|stem| SHAPES.contains(&stem))
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
/// One L-system turtle frame saved on the branch stack: pos, heading, left, up, step, depth.
type TurtleState = ([f32; 3], [f32; 3], [f32; 3], [f32; 3], f32, u32);
/// A drawn L-system branch segment: start, end, branch depth (0 = trunk).
type Seg = ([f32; 3], [f32; 3], u32);

/// Synthesize a named shape (see [`SHAPES`]); unknown name → None.
pub fn gen_shape(stem: &str) -> Option<Cloud> {
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
        "menger" => {
            // Rejection-sample a depth-3 Menger sponge: a sub-cell is a hole when ≥2 of its three
            // ternary digits are the centre. Accept rate ≈ (20/27)^3 ≈ 0.4, so this fills quickly;
            // a generous attempt cap + a top-up keep the count at exactly N.
            let mut tries = 0u64;
            while pos.len() < N && tries < (N as u64) * 50 {
                tries += 1;
                let p = [
                    rng.uniform(-1.0, 1.0),
                    rng.uniform(-1.0, 1.0),
                    rng.uniform(-1.0, 1.0),
                ];
                if in_menger(p, 3) {
                    let s = (p[0] + p[1] + p[2]) * 0.18;
                    pos.push(p);
                    rgb.push(hsv(0.55 + 0.12 * s, 0.55, 0.92));
                }
            }
            while pos.len() < N {
                let i = pos.len() - 1; // defensive top-up (won't trigger at this accept rate)
                pos.push(pos[i]);
                rgb.push(rgb[i]);
            }
        }
        "shell" => {
            // A logarithmic-spiral nautilus: a tube of growing radius coiled ~3 turns in a near-plane.
            let turns = 3.0f32;
            let mut raw = Vec::with_capacity(N);
            let mut maxv = 1e-6f32;
            for _ in 0..N {
                let t = rng.uniform(0.0, turns * tau);
                let grow = (0.20 * t).exp();
                let rr = 0.05 * grow; // spiral radius
                let tube = 0.34 * rr; // cross-section thickens with the radius
                let a = rng.uniform(0.0, tau);
                let p = [
                    (rr + tube * a.cos()) * t.cos(),
                    tube * a.sin() + 0.06 * grow, // a gentle conic lift out of plane
                    (rr + tube * a.cos()) * t.sin(),
                ];
                maxv = maxv.max(p[0].abs()).max(p[1].abs()).max(p[2].abs());
                raw.push((p, t / (turns * tau)));
            }
            for (p, u) in raw {
                pos.push([p[0] / maxv, p[1] / maxv, p[2] / maxv]);
                rgb.push(hsv(0.02 + 0.5 * u, 0.85, 1.0)); // warm → cool along the coil
            }
        }
        "tent" => {
            // A ridge tent: two canvas panels sloping from a ridge (along X, height HH) down to the
            // ground at ±Z, plus two triangular end walls. ~80% of the splats on the panels (more area).
            let (hl, hw, hh) = (0.95f32, 0.7f32, 0.85f32);
            for _ in 0..N {
                // `seam` = this point sits on an EDGE (ridge line, ground hem, gable A-frame, corner
                // poles) → coloured as canvas trim, the rest as canvas. A cartoon-outlined tent.
                let (p, seam) = if rng.unit() < 0.8 {
                    let side = if rng.unit() < 0.5 { 1.0 } else { -1.0 };
                    let t = rng.unit(); // 0 = ridge, 1 = ground edge
                    let x = rng.uniform(-hl, hl);
                    let seam = !(0.04..=0.96).contains(&t) || x.abs() > hl * 0.96; // ridge, hem, gable ends
                    ([x, hh * (1.0 - t), side * hw * t], seam)
                } else {
                    let ex = if rng.unit() < 0.5 { hl } else { -hl };
                    let (mut a, mut b) = (rng.unit(), rng.unit());
                    if a + b > 1.0 {
                        a = 1.0 - a;
                        b = 1.0 - b;
                    } // uniform point in the end triangle (ridge-top, two base corners)
                    let seam = a < 0.04 || b < 0.04 || a + b > 0.96; // the gable's A-frame outline
                    ([ex, hh * (1.0 - a - b), (b - a) * hw], seam)
                };
                // Negate Y: martin flips Y-down .ply upright on load (capture convention), so build
                // these directional shapes inverted to render the right way up (ridge up). Same below.
                pos.push([p[0], 0.4 - p[1], p[2]]);
                rgb.push(if seam {
                    hsv(0.09, 0.55, 0.85) // warm canvas trim / poles — cartoon seams
                } else {
                    hsv(0.74, 0.55, 0.85) // violet canvas — pops against the orange fire + green pines
                });
            }
        }
        "pine" => {
            // A conifer: stacked shrinking cones (tiers of needles) on a short brown trunk.
            let tiers = 4u64;
            for _ in 0..N {
                if rng.unit() < 0.1 {
                    let a = rng.uniform(0.0, tau);
                    let r = 0.05 * rng.unit().sqrt();
                    pos.push([r * a.cos(), -rng.uniform(-1.0, -0.6), r * a.sin()]); // -Y (see tent)
                    rgb.push([0.35, 0.22, 0.12]); // trunk
                } else {
                    let tier = rng.int(tiers) as f32;
                    let t = rng.unit(); // 0 = tier base, 1 = tier tip
                    let r = 0.6 * (1.0 - tier / tiers as f32 * 0.7) * (1.0 - t);
                    let a = rng.uniform(0.0, tau);
                    pos.push([r * a.cos(), -(-0.6 + tier * 0.42 + t * 0.5), r * a.sin()]); // -Y
                    rgb.push(hsv(0.33, 0.7, 0.45 + 0.3 * t)); // green, lighter at the tips
                }
            }
        }
        "flame" => {
            // A campfire tongue: wide + hot at the base, tapering to a point, with angular flicker.
            // Pairs with MARTIN_PARTICLES=embers. Colour rides the temperature (white-yellow core → red).
            for _ in 0..N {
                let h = rng.unit().powf(0.7); // bias toward the base
                let a = rng.uniform(0.0, tau);
                let tongue = 1.0 + 0.25 * (3.0 * a).sin();
                let r = (1.0 - h) * 0.5 * tongue * (0.6 + 0.4 * rng.unit());
                pos.push([r * a.cos(), 1.0 - h * 2.0, r * a.sin() * 0.6]); // -Y: hot base renders low
                let heat = ((1.0 - h) * (1.0 - (r / 0.5).min(1.0))).clamp(0.0, 1.0);
                rgb.push([1.0, (0.3 + 0.7 * heat).min(1.0), 0.1 * heat]);
            }
        }
        "rocket" => {
            // A cartoon rocket pointing UP: white cylindrical body with red bands, a red nose cone,
            // three swept red tail fins, a cyan porthole, and a hot exhaust plume firing below. yu is
            // the up-axis (tail -1 → nose +1), stored NEGATED — martin flips Y-down .ply upright on
            // load, so build inverted to render nose-up (same trick as tent/pine/flame above).
            let body_r = 0.24f32;
            for _ in 0..N {
                let s = rng.unit();
                let p: [f32; 3];
                let col: [f32; 3];
                if s < 0.40 {
                    // body: a cylinder shell, yu in [-0.55, 0.45], with the odd red band for read.
                    let yu = rng.uniform(-0.55, 0.45);
                    let a = rng.uniform(0.0, tau);
                    let r = body_r * (1.0 + 0.02 * rng.normal(0.0, 1.0));
                    p = [r * a.cos(), yu, r * a.sin()];
                    col = if (yu * 6.0).sin() > 0.85 {
                        [0.85, 0.12, 0.10]
                    } else {
                        [0.88, 0.89, 0.93]
                    };
                } else if s < 0.58 {
                    // nose cone: yu in [0.45, 1.05], radius tapering to a point. Red.
                    let t = rng.unit();
                    let yu = 0.45 + t * 0.60;
                    let a = rng.uniform(0.0, tau);
                    let r = body_r * (1.0 - t) * (1.0 + 0.02 * rng.normal(0.0, 1.0));
                    p = [r * a.cos(), yu, r * a.sin()];
                    col = [0.85, 0.12, 0.10];
                } else if s < 0.73 {
                    // three fins at 120°: a triangle from the body root swept out to a tail tip.
                    let fin = (rng.int(3) as f32) * tau / 3.0;
                    let t = rng.unit(); // 0 = root (upper), 1 = tail tip
                    let yu = -0.15 - t * 0.45; // fin spans yu -0.15 → -0.60
                    let rad = body_r + t * 0.34 * rng.unit(); // fill the swept triangle
                    let thick = 0.02 * rng.normal(0.0, 1.0);
                    let (c, sn) = (fin.cos(), fin.sin());
                    p = [rad * c - thick * sn, yu, rad * sn + thick * c];
                    col = [0.80, 0.12, 0.10];
                } else if s < 0.77 {
                    // cyan porthole, a disc on the +Z body face around yu 0.12.
                    let a = rng.uniform(0.0, tau);
                    let rr = 0.07 * rng.unit().sqrt();
                    let yu = 0.12 + rr * a.sin();
                    let off = rr * a.cos();
                    p = [
                        off,
                        yu,
                        (body_r * body_r - off * off).max(0.0).sqrt() + 0.005,
                    ];
                    col = [0.30, 0.70, 0.95];
                } else {
                    // exhaust plume below the tail: yu -0.6 → -1.2, a hot tapering tongue (red=1.0
                    // exactly marks it as flame for opacity_of/size_of below).
                    let h = rng.unit().powf(0.7);
                    let a = rng.uniform(0.0, tau);
                    let tongue = 1.0 + 0.2 * (3.0 * a).sin();
                    let r = body_r * (1.0 - h) * tongue * (0.6 + 0.4 * rng.unit());
                    let yu = -0.6 - h * 0.6;
                    p = [r * a.cos(), yu, r * a.sin()];
                    let heat = (1.0 - h).clamp(0.0, 1.0);
                    col = [1.0, (0.3 + 0.7 * heat).min(1.0), 0.1 * heat];
                }
                pos.push([p[0], -p[1], p[2]]); // -Y: nose points up after martin's load-flip
                rgb.push(col);
            }
        }
        "terrain" => {
            // A ROUND field (disc) so scenes have a floor instead of floating in the void — reads as
            // a clean diorama base, not a square slab with corners. The show can stretch it to an
            // OVAL with a non-uniform scale (`*wx,wy,wz`). Gentle undulation only.
            for _ in 0..N {
                let ang = rng.uniform(0.0, tau);
                let r = rng.unit().sqrt(); // sqrt → uniform over the disc area (no centre clump)
                let (x, z) = (r * ang.cos(), r * ang.sin());
                // gentle undulation only — a near-flat FIELD, not mounds (tall hills loom over props
                // placed on it and read as a "second field" floating above the scene).
                let y = 0.06
                    * ((2.3 * x).sin() * (1.9 * z).cos()
                        + 0.5 * (5.1 * x + 1.0).sin() * (4.3 * z).cos())
                    - 0.55;
                pos.push([x, -y, z]); // -Y: hills bump up after martin's flip
                let g = ((y + 0.85) * 1.6).clamp(0.0, 1.0);
                rgb.push(hsv(0.28 - 0.07 * g, 0.5, 0.3 + 0.4 * g)); // grass-green → tan ridges
            }
        }
        "moon" => {
            // A grey cratered moon (vs the rainbow `sphere`): points on the unit sphere, darkened
            // inside a fixed set of craters (darkest at the centre, lighter to the rim).
            let craters: Vec<([f32; 3], f32)> = (0..18)
                .map(|_| (norm(rng.normal3()), rng.uniform(0.12, 0.34)))
                .collect();
            for _ in 0..N {
                let d = norm(rng.normal3());
                let mut shade = 1.0f32;
                for (c, rad) in &craters {
                    let ang = (d[0] * c[0] + d[1] * c[1] + d[2] * c[2])
                        .clamp(-1.0, 1.0)
                        .acos();
                    if ang < *rad {
                        shade = shade.min(0.45 + 0.55 * (ang / rad));
                    }
                }
                pos.push(d);
                let g = 0.72 * shade;
                rgb.push([g, g, (g * 1.04).min(1.0)]);
            }
        }
        "mountains" => {
            // A distant ridge that BOWS into a CRESCENT — the ends curve forward so it wraps the back
            // of a scene like surrounding peaks, not a flat wall. Place it back + wide; the per-axis
            // scale's Z controls how far the ends come forward. Hazy blue (atmospheric distance).
            for _ in 0..N {
                let x = rng.uniform(-1.0, 1.0);
                let h = (7.0 * x).sin().abs() * 0.5
                    + (15.0 * x + 0.7).sin().abs() * 0.3
                    + (29.0 * x).sin().abs() * 0.2; // many layered jagged peaks → a RIDGE, not 2 humps
                let fill = rng.unit(); // 0 = base, 1 = the ridge line → a solid silhouette
                pos.push([
                    x,
                    0.7 - h * fill * 1.4, // -Y: peaks point UP after martin's base flip (else upside-down)
                    // -Z bow: the load flip negates Z, so a NEGATIVE parabola here renders as ends
                    // curving FORWARD (toward the camera) → a crescent wrapping the back.
                    -0.85 * x * x + rng.uniform(-0.04, 0.04),
                ]);
                rgb.push(hsv(0.62, 0.3, 0.26 + 0.22 * fill)); // hazy blue-grey, lighter near the peaks
            }
        }
        "lsystem" => {
            return Some(turtle_plant(
                &mut rng,
                &expand("X", "F[&X]/[&X]/[&X]", "FF", 5),
                28.0,
                120.0,
            ));
        }
        "fern" => {
            // The canonical fractal-plant L-system (arching frond), rolled into 3D between branches.
            return Some(turtle_plant(
                &mut rng,
                &expand("X", "F-[[X]+X]/+F[+FX]-X", "FF", 4),
                22.0,
                90.0,
            ));
        }
        _ => return None,
    }
    Some((pos, rgb))
}

// ---- L-system plant -------------------------------------------------------------------------

/// Rotate the two frame vectors `a` and `b` about the unit `axis` by `ang` (Rodrigues). Used to
/// reorient the turtle's heading/left/up frame for yaw (about up), pitch (about left), roll (about
/// heading) — keeping the frame orthonormal as the plant branches in 3D.
fn rot(a: &mut [f32; 3], b: &mut [f32; 3], axis: [f32; 3], ang: f32) {
    let (c, s) = (ang.cos(), ang.sin());
    let rodr = |v: [f32; 3]| {
        let kv = axis[0] * v[0] + axis[1] * v[1] + axis[2] * v[2]; // axis·v
        let kx = [
            axis[1] * v[2] - axis[2] * v[1],
            axis[2] * v[0] - axis[0] * v[2],
            axis[0] * v[1] - axis[1] * v[0],
        ]; // axis×v
        [
            v[0] * c + kx[0] * s + axis[0] * kv * (1.0 - c),
            v[1] * c + kx[1] * s + axis[1] * kv * (1.0 - c),
            v[2] * c + kx[2] * s + axis[2] * kv * (1.0 - c),
        ]
    };
    *a = norm(rodr(*a));
    *b = norm(rodr(*b));
}

/// Expand an L-system: rewrite `X`→`rx` and `F`→`rf` for `iters` passes from `axiom`, leaving turtle
/// commands (`+-&^/\[]`) untouched. The string is capped (`MAX_LEN`) so the exponential growth of a
/// branching rule can't run away into a multi-GB allocation.
fn expand(axiom: &str, rx: &str, rf: &str, iters: usize) -> String {
    const MAX_LEN: usize = 400_000;
    let mut s = String::from(axiom);
    for _ in 0..iters {
        let mut next = String::with_capacity(s.len() * 3);
        for c in s.chars() {
            match c {
                'X' => next.push_str(rx),
                'F' => next.push_str(rf),
                other => next.push(other),
            }
        }
        s = next;
        if s.len() > MAX_LEN {
            break;
        }
    }
    s
}

/// Is `p` (in [-1,1]³) inside a depth-`levels` Menger sponge? At each level, map the point into its
/// 3×3×3 sub-cell; a cell is a "hole" (removed) when ≥2 of its three ternary digits are the centre (1).
fn in_menger(p: [f32; 3], levels: u32) -> bool {
    let mut c = [(p[0] + 1.0) * 0.5, (p[1] + 1.0) * 0.5, (p[2] + 1.0) * 0.5]; // → [0,1]
    for _ in 0..levels {
        let mut centres = 0;
        for ci in &mut c {
            let d = (*ci * 3.0).floor().clamp(0.0, 2.0);
            if d as i32 == 1 {
                centres += 1;
            }
            *ci = *ci * 3.0 - d;
        }
        if centres >= 2 {
            return false;
        }
    }
    true
}

/// Walk an expanded L-system string `s` with a 3D turtle (heading/left/up frame + a push/pop branch
/// stack; `ang_deg` yaw/pitch, `roll_deg` roll), collect the drawn segments, then scatter the splats
/// along them — thicker + browner at the base, thinner + greener toward the twigs. Deterministic.
fn turtle_plant(rng: &mut Rng, s: &str, ang_deg: f32, roll_deg: f32) -> Cloud {
    // Walk the string with a 3D turtle; collect drawn segments as (start, end, depth).
    let ang = ang_deg.to_radians();
    let roll = roll_deg.to_radians();
    let mut pos = [0.0f32, -1.0, 0.0];
    let mut h = [0.0f32, 1.0, 0.0]; // heading (grows up)
    let mut l = [1.0f32, 0.0, 0.0]; // left
    let mut u = [0.0f32, 0.0, 1.0]; // frame up
    let mut step = 0.14f32;
    let mut depth = 0u32;
    let mut stack: Vec<TurtleState> = Vec::new();
    let mut segs: Vec<Seg> = Vec::new();
    for c in s.chars() {
        match c {
            'F' => {
                let b = [
                    pos[0] + h[0] * step,
                    pos[1] + h[1] * step,
                    pos[2] + h[2] * step,
                ];
                segs.push((pos, b, depth));
                pos = b;
            }
            '+' => rot(&mut h, &mut l, u, ang), // yaw about up
            '-' => rot(&mut h, &mut l, u, -ang),
            '&' => rot(&mut h, &mut u, l, ang), // pitch about left
            '^' => rot(&mut h, &mut u, l, -ang),
            '/' => rot(&mut l, &mut u, h, roll), // roll about heading
            '\\' => rot(&mut l, &mut u, h, -roll),
            '[' => {
                stack.push((pos, h, l, u, step, depth));
                depth += 1;
                step *= 0.72; // children shorter → natural taper
            }
            ']' => {
                if let Some((p, hh, ll, uu, st, d)) = stack.pop() {
                    pos = p;
                    h = hh;
                    l = ll;
                    u = uu;
                    step = st;
                    depth = d;
                }
            }
            _ => {}
        }
    }
    if segs.is_empty() {
        segs.push(([0.0, -1.0, 0.0], [0.0, 1.0, 0.0], 0)); // degenerate guard: a single trunk
    }

    // Cumulative segment length → sample N points proportional to length (longer branches = more
    // splats). Each point: a random spot along a segment, jittered by a depth-tapered radius, coloured
    // brown (base) → green (canopy) by normalized height.
    let dist = |a: &[f32; 3], b: &[f32; 3]| {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    };
    let mut cum = Vec::with_capacity(segs.len());
    let mut acc = 0.0f32;
    for (a, b, _) in &segs {
        acc += dist(a, b);
        cum.push(acc);
    }
    let total = acc.max(1e-6);

    let mut raw: Vec<([f32; 3], u32)> = Vec::with_capacity(N);
    let mut maxv = 1e-6f32;
    for _ in 0..N {
        let r = rng.unit() * total;
        let si = cum.partition_point(|&c| c < r).min(segs.len() - 1);
        let (a, b, d) = segs[si];
        let t = rng.unit();
        let thick = 0.03 * 0.72f32.powi(d as i32); // twigs thinner than the trunk
        let p = [
            a[0] + (b[0] - a[0]) * t + rng.normal(0.0, thick),
            a[1] + (b[1] - a[1]) * t + rng.normal(0.0, thick),
            a[2] + (b[2] - a[2]) * t + rng.normal(0.0, thick),
        ];
        maxv = maxv.max(p[0].abs()).max(p[1].abs()).max(p[2].abs());
        raw.push((p, d));
    }

    let (mut pos_out, mut rgb_out) = (Vec::with_capacity(N), Vec::with_capacity(N));
    let brown = [0.40f32, 0.26, 0.13];
    let green = [0.22f32, 0.72, 0.20];
    for (p, _) in raw {
        let q = [p[0] / maxv, p[1] / maxv, p[2] / maxv];
        let hgt = ((q[1] + 1.0) * 0.5).clamp(0.0, 1.0); // base 0 → canopy 1
        pos_out.push(q);
        rgb_out.push([
            (brown[0] + (green[0] - brown[0]) * hgt).clamp(0.0, 1.0),
            (brown[1] + (green[1] - brown[1]) * hgt).clamp(0.0, 1.0),
            (brown[2] + (green[2] - brown[2]) * hgt).clamp(0.0, 1.0),
        ]);
    }
    (pos_out, rgb_out)
}

/// Per-object (and per-point) opacity — translucency is a property of WHAT it is, not one global value.
/// Fire glows (very translucent, wispy at the cool tips); tree FOLIAGE is airy but the TRUNK is solid;
/// distant mountains are hazy; tent canvas + moon are near-solid. Keyed by shape name + the point's
/// colour (so trunk-vs-foliage / hot-vs-cool splits fall out of the colour the generators already set).
fn opacity_of(name: &str, rgb: &[f32; 3]) -> f32 {
    match name {
        // fire: translucent glow, MORE see-through at the cooler (less-green) tips than the hot core.
        "flame" => 0.14 + 0.44 * ((rgb[1] - 0.3) / 0.7).clamp(0.0, 1.0),
        // pine: green foliage = airy translucent needles; brown trunk (r≥g) = solid.
        "pine" => {
            if rgb[1] > rgb[0] {
                0.42
            } else {
                0.85
            }
        }
        "tent" => 0.82, // canvas (near-solid)
        // rocket: the exhaust plume (red==1.0, low blue) glows translucent like fire; the hull is solid.
        "rocket" => {
            if rgb[0] > 0.97 && rgb[2] < 0.25 {
                0.14 + 0.44 * ((rgb[1] - 0.3) / 0.7).clamp(0.0, 1.0)
            } else {
                0.88
            }
        }
        "moon" => 0.92,      // solid rock
        "terrain" => 0.60,   // the ground reads, but a touch translucent helps it sit behind props
        "mountains" => 0.40, // hazy, distant
        _ => ALPHA,          // everything else: the soft default
    }
}

/// Per-object (and per-point) splat SIZE — the splat radius is part of the look, not one global value:
/// fire = big soft glow, tree FOLIAGE = fluffy (vs a crisp small TRUNK), distant mountains = big hazy
/// blobs, tent canvas / moon = small crisp. Keyed by shape name + the point's colour (same trunk/foliage
/// + hot/cool splits as the opacity).
fn size_of(name: &str, rgb: &[f32; 3]) -> f32 {
    match name {
        "flame" => 0.045 + 0.030 * ((rgb[1] - 0.3) / 0.7).clamp(0.0, 1.0), // big, biggest at the hot core
        "pine" => {
            if rgb[1] > rgb[0] {
                0.034 // fluffy needles
            } else {
                0.014 // crisp trunk
            }
        }
        "mountains" => 0.060, // big hazy blobs (distant)
        // rocket: big soft plume at the tail, crisp small splats on the hull.
        "rocket" => {
            if rgb[0] > 0.97 && rgb[2] < 0.25 {
                0.040 + 0.030 * ((rgb[1] - 0.3) / 0.7).clamp(0.0, 1.0)
            } else {
                0.016
            }
        }
        "terrain" => 0.030, // soft ground
        "tent" => 0.020,    // crisp canvas
        "moon" => 0.022,
        _ => SPLAT, // the default
    }
}

/// Write a cloud as martin's sh0 binary `.ply`.
pub fn write_ply(path: &Path, name: &str, pos: &[[f32; 3]], rgb: &[[f32; 3]]) {
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
        let scale = size_of(name, c).ln(); // per-point splat radius (size is part of the look)
        let op = opacity_of(name, c); // per-point opacity (translucency is part of the look)
        put(p[0]);
        put(p[1]);
        put(p[2]);
        put(scale);
        put(scale);
        put(scale);
        put((op / (1.0 - op)).ln()); // opacity → logit
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_shape_generates_a_full_finite_unit_cloud() {
        for &name in SHAPES {
            let (pos, rgb) = gen_shape(name).unwrap_or_else(|| panic!("{name} produced None"));
            assert_eq!(pos.len(), N, "{name}: point count");
            assert_eq!(rgb.len(), N, "{name}: colour count");
            for p in &pos {
                assert!(
                    p.iter().all(|c| c.is_finite()),
                    "{name}: non-finite position"
                );
                // shapes are authored to span ~±1; allow a little jitter slack
                assert!(
                    p.iter().all(|c| c.abs() <= 1.6),
                    "{name}: out of unit box: {p:?}"
                );
            }
            for c in &rgb {
                assert!(
                    c.iter().all(|&x| (0.0..=1.0).contains(&x)),
                    "{name}: colour out of [0,1]: {c:?}"
                );
            }
        }
    }

    #[test]
    fn unknown_shape_is_none() {
        assert!(gen_shape("definitely-not-a-shape").is_none());
    }

    #[test]
    fn directional_shapes_render_right_way_up_after_the_load_flip() {
        // martin flips every .ply 180° about X on load (`cloud_base_rotation`), which negates Y.
        // Directional shapes are authored inverted (the per-shape `-Y` negate) so they land
        // right-way-up in WORLD. Anchor each on an unmistakable coloured feature and assert it ends
        // up where it belongs — a forgotten negate (e.g. the upside-down `mountains` regression that
        // turned peaks into stalactites) flips these and fails here, in CI, not on screen.
        let mean_world_y = |name: &str, pick: &dyn Fn([f32; 3]) -> bool| -> f32 {
            let (pos, rgb) = gen_shape(name).unwrap();
            let ys: Vec<f32> = pos
                .iter()
                .zip(&rgb)
                .filter(|(_, c)| pick(**c))
                .map(|(p, _)| -p[1]) // -Y = the engine's load flip
                .collect();
            assert!(
                !ys.is_empty(),
                "{name}: no points matched the colour anchor"
            );
            ys.iter().sum::<f32>() / ys.len() as f32
        };
        // pine: the brown trunk (r>g>b) sits BELOW the green needles (g>r).
        let trunk = mean_world_y("pine", &|c| c[0] > c[1] && c[1] > c[2]);
        let needles = mean_world_y("pine", &|c| c[1] > c[0]);
        assert!(
            trunk < needles,
            "pine upside-down: trunk {trunk:.2} not below needles {needles:.2}"
        );
        // flame: a tongue tapering UP — its base is wider than its tip. (Colour rides radius too, so
        // it isn't a clean height anchor; geometry is.) Compare mean horizontal radius of the lowest
        // vs the highest world-Y band.
        {
            let (pos, _) = gen_shape("flame").unwrap();
            let w: Vec<[f32; 3]> = pos.iter().map(|p| [p[0], -p[1], p[2]]).collect();
            let (lo, hi) = w
                .iter()
                .fold((f32::MAX, f32::MIN), |(l, h), p| (l.min(p[1]), h.max(p[1])));
            let band = (hi - lo) * 0.2;
            let radius = |keep: &dyn Fn(f32) -> bool| {
                let r: Vec<f32> = w
                    .iter()
                    .filter(|p| keep(p[1]))
                    .map(|p| (p[0] * p[0] + p[2] * p[2]).sqrt())
                    .collect();
                r.iter().sum::<f32>() / r.len().max(1) as f32
            };
            let base = radius(&|y| y <= lo + band);
            let tip = radius(&|y| y >= hi - band);
            assert!(
                base > tip,
                "flame upside-down: base radius {base:.2} not wider than tip {tip:.2}"
            );
        }
        // mountains: the lighter peaks (high value) sit ABOVE the darker base.
        let value = |c: [f32; 3]| c[0].max(c[1]).max(c[2]);
        let peaks = mean_world_y("mountains", &|c| value(c) > 0.42);
        let base = mean_world_y("mountains", &|c| value(c) < 0.30);
        assert!(
            peaks > base,
            "mountains upside-down: peaks {peaks:.2} not above base {base:.2}"
        );
    }

    #[test]
    fn generation_is_deterministic() {
        // fixed-seed RNG → byte-identical clouds across runs (stable rebuilds, reproducible .ply).
        for &name in &["cube", "lsystem", "galaxy"] {
            assert_eq!(gen_shape(name), gen_shape(name), "{name} not deterministic");
        }
    }

    #[test]
    fn ply_blob_has_the_sh0_header_and_stride() {
        let (pos, rgb) = gen_shape("cube").unwrap();
        let dir = std::env::temp_dir().join(format!("splatgen_test_{}.ply", std::process::id()));
        write_ply(&dir, "cube", &pos, &rgb);
        let bytes = std::fs::read(&dir).unwrap();
        let _ = std::fs::remove_file(&dir);
        let header_end = b"end_header\n";
        let hpos = bytes
            .windows(header_end.len())
            .position(|w| w == header_end)
            .expect("end_header present")
            + header_end.len();
        assert!(bytes.starts_with(b"ply\n"));
        // 14 f32 per splat (xyz, scale0..2, opacity, rot wxyz, f_dc0..2)
        assert_eq!(bytes.len() - hpos, N * 14 * 4, "binary body stride");
    }
}
