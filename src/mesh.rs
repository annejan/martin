//! Mesh → gaussians: sample a triangle mesh's **surface** (COLLADA `.dae`, `.obj`, `.stl`, `.ply`
//! via `mesh-loader`) into gaussians, so a 3D model — the deFEEST logo mesh, the bornhack badge —
//! is just another morph/deform source that flows through the exact same pipeline as
//! text/image/splats. Built Y-DOWN (negate Y) so the shared `cloud_base_rotation` flips it upright
//! like the others. Pure (no Bevy/ECS) apart from the gaussian type, matching `text.rs`.
//!
//! Each sample is a **flat disk** lying in the surface (oriented to the interpolated vertex normal,
//! face normal as fallback) — far closer to "proper" splats than round blobs — and coloured from
//! the diffuse **texture** at its UV when the material has one, else vertex colours, else the
//! material diffuse, else the caller's flat `rgb`.

use std::path::Path;

use bevy_gaussian_splatting::{Gaussian3d, SphericalHarmonicCoefficients};

/// 3DGS degree-0 encode (same as text.rs): rendered colour ≈ 0.5 + 0.2820948·dc, so invert it.
fn dc(c: f32) -> f32 {
    (c - 0.5) / 0.282_094_79
}

/// A degree-0 SH from a target RGB.
fn sh_of(rgb: [f32; 3]) -> SphericalHarmonicCoefficients {
    let mut sh = SphericalHarmonicCoefficients::default();
    sh.set(0, dc(rgb[0]));
    sh.set(1, dc(rgb[1]));
    sh.set(2, dc(rgb[2]));
    sh
}

/// Deterministic hash → [0, 1) (no rng dependency; for area-weighted surface scatter).
fn hash01(k: u32) -> f32 {
    let mut n = k.wrapping_add(0x9E37_79B9).wrapping_mul(0x85EB_CA6B);
    n ^= n >> 13;
    n = n.wrapping_mul(0xC2B2_AE35);
    ((n >> 8) & 0x00FF_FFFF) as f32 / 16_777_216.0
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(u: [f32; 3], v: [f32; 3]) -> [f32; 3] {
    [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ]
}

/// Normalize, or a fallback axis for a degenerate (zero-length) vector.
fn norm_or(v: [f32; 3], fallback: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 1e-12 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        fallback
    }
}

/// Twice the triangle area (= |edge0 × edge1|), used as the sampling weight.
fn double_area(t: &[[f32; 3]; 3]) -> f32 {
    let c = cross(sub(t[1], t[0]), sub(t[2], t[0]));
    (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt()
}

/// Quaternion `[x, y, z, w]` from an orthonormal basis whose columns are the gaussian's local
/// x/y/z axes (Shepperd's method) — here `(t, b, n)`, so local +Z maps to the surface normal.
fn quat_from_basis(t: [f32; 3], b: [f32; 3], n: [f32; 3]) -> [f32; 4] {
    let (m00, m10, m20) = (t[0], t[1], t[2]);
    let (m01, m11, m21) = (b[0], b[1], b[2]);
    let (m02, m12, m22) = (n[0], n[1], n[2]);
    let trace = m00 + m11 + m22;
    if trace > 0.0 {
        let s = 0.5 / (trace + 1.0).sqrt();
        [(m21 - m12) * s, (m02 - m20) * s, (m10 - m01) * s, 0.25 / s]
    } else if m00 > m11 && m00 > m22 {
        let s = 2.0 * (1.0 + m00 - m11 - m22).sqrt();
        [0.25 * s, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s]
    } else if m11 > m22 {
        let s = 2.0 * (1.0 + m11 - m00 - m22).sqrt();
        [(m01 + m10) / s, 0.25 * s, (m12 + m21) / s, (m02 - m20) / s]
    } else {
        let s = 2.0 * (1.0 + m22 - m00 - m11).sqrt();
        [(m02 + m20) / s, (m12 + m21) / s, 0.25 * s, (m10 - m01) / s]
    }
}

/// An arbitrary tangent basis `(t, b)` perpendicular to `n` (the disk is rotationally symmetric in
/// plane, so any perpendicular pair will do).
fn tangent_basis(n: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let reference = if n[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let t = norm_or(cross(reference, n), [1.0, 0.0, 0.0]);
    let b = cross(n, t);
    (t, b)
}

/// Load a diffuse texture beside the mesh (path is relative to the mesh file). PNG + JPEG only;
/// anything else (or a missing file) → `None`, and the sampler falls back to material/vertex colour.
fn load_texture(mesh_dir: &Path, rel: &Path) -> Option<image::RgbaImage> {
    let path = mesh_dir.join(rel);
    match image::open(&path) {
        Ok(img) => Some(img.to_rgba8()),
        Err(e) => {
            eprintln!("mesh texture {}: {e}", path.display());
            None
        }
    }
}

/// Nearest-texel sample at `(u, v)` (tiled), returned as linear-ish RGB in `[0, 1]` — matching how
/// the material diffuse colour is fed straight into `sh_of`.
fn sample_tex(img: &image::RgbaImage, u: f32, v: f32) -> [f32; 3] {
    let (w, h) = img.dimensions();
    let wrap = |x: f32| {
        let f = x.fract();
        if f < 0.0 {
            f + 1.0
        } else {
            f
        }
    };
    let x = ((wrap(u) * w as f32) as u32).min(w - 1);
    // image rows are top-down; UV origin is bottom-left → flip v.
    let y = ((wrap(1.0 - v) * h as f32) as u32).min(h - 1);
    let p = img.get_pixel(x, y);
    [
        p[0] as f32 / 255.0,
        p[1] as f32 / 255.0,
        p[2] as f32 / 255.0,
    ]
}

/// Sample `target_count` gaussians over the mesh surface (distributed by triangle area). Each is a
/// flat disk in the surface plane: in-plane radius `splat` (a FRACTION of the model's largest
/// dimension), thinned to `thin`× that along the normal. Missing / bad files → empty (the part
/// just renders nothing rather than crashing the show).
pub fn build_mesh_gaussians(
    path: &Path,
    target_count: usize,
    splat: f32,
    thin: f32,
    rgb: [f32; 3],
) -> Vec<Gaussian3d> {
    let scene = match mesh_loader::Loader::default().load(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("mesh {}: {e}", path.display());
            return Vec::new();
        }
    };
    let mesh_dir = path.parent().unwrap_or_else(|| Path::new("."));

    // mesh-loader builds `meshes[i]` and `materials[i]` from the same geometry in lockstep — so a
    // mesh's colour is its material's texture/diffuse. Per gaussian, colour priority is: vertex
    // colours > the diffuse texture sampled at the UV > material diffuse > the caller's flat `rgb`.
    // Orientation: interpolated vertex normals, else the triangle's own face normal.
    let mut tris: Vec<[[f32; 3]; 3]> = Vec::new();
    let mut tri_cols: Vec<Option<[[f32; 3]; 3]>> = Vec::new();
    let mut tri_norms: Vec<Option<[[f32; 3]; 3]>> = Vec::new();
    let mut tri_uvs: Vec<Option<[[f32; 2]; 3]>> = Vec::new();
    let mut tri_mesh: Vec<usize> = Vec::new();
    for (mi, mesh) in scene.meshes.iter().enumerate() {
        let cols = &mesh.colors[0]; // colour set 0; empty unless the mesh carries vertex colours
        let has_cols = !cols.is_empty() && cols.len() == mesh.vertices.len();
        let uvs = &mesh.texcoords[0];
        let has_uvs = !uvs.is_empty() && uvs.len() == mesh.vertices.len();
        let has_norms = mesh.normals.len() == mesh.vertices.len();
        for &[a, b, c] in &mesh.faces {
            let (a, b, c) = (a as usize, b as usize, c as usize);
            let (Some(&va), Some(&vb), Some(&vc)) = (
                mesh.vertices.get(a),
                mesh.vertices.get(b),
                mesh.vertices.get(c),
            ) else {
                continue;
            };
            tris.push([va, vb, vc]);
            tri_cols.push(has_cols.then(|| {
                let at = |i: usize| [cols[i][0], cols[i][1], cols[i][2]];
                [at(a), at(b), at(c)]
            }));
            tri_norms.push(has_norms.then(|| [mesh.normals[a], mesh.normals[b], mesh.normals[c]]));
            tri_uvs.push(has_uvs.then(|| [uvs[a], uvs[b], uvs[c]]));
            tri_mesh.push(mi);
        }
    }
    if tris.is_empty() {
        eprintln!("mesh {}: no triangles", path.display());
        return Vec::new();
    }

    // Splat size proportional to the model: `splat` is a FRACTION of the largest dimension, so a
    // tiny badge (±0.05) and a unit-scale object both get sensibly-sized splats. (normalize_to later
    // scales gaussian sizes along with positions, so an *absolute* size blobs the small ones.)
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for t in &tris {
        for v in t {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
    }
    let extent = (0..3).map(|k| hi[k] - lo[k]).fold(0.0_f32, f32::max);
    let r_plane = (splat * extent).max(1e-6); // in-plane disk radius
    let r_thin = (r_plane * thin).max(1e-7); // thickness along the normal

    // per-mesh fallbacks: the diffuse texture (if any + loadable) and a flat SH (material diffuse,
    // else the caller's `rgb`).
    let mesh_tex: Vec<Option<image::RgbaImage>> = (0..scene.meshes.len())
        .map(|mi| {
            scene
                .materials
                .get(mi)
                .and_then(|m| m.texture.diffuse.as_deref())
                .and_then(|rel| load_texture(mesh_dir, rel))
        })
        .collect();
    let mesh_sh: Vec<SphericalHarmonicCoefficients> = (0..scene.meshes.len())
        .map(|mi| {
            let c = scene
                .materials
                .get(mi)
                .and_then(|m| m.color.diffuse)
                .map(|d| [d[0], d[1], d[2]])
                .unwrap_or(rgb);
            sh_of(c)
        })
        .collect();

    let weights: Vec<f32> = tris.iter().map(double_area).collect();
    let total: f32 = weights.iter().sum::<f32>().max(1e-9);

    let mut out: Vec<Gaussian3d> = Vec::with_capacity(target_count + tris.len());
    let mut idx: u32 = 0;
    for (ti, tri) in tris.iter().enumerate() {
        let mi = tri_mesh[ti];
        // face normal in Y-down space (fallback when the mesh has no vertex normals): the cloud
        // is stored Y-down, so build the basis from the Y-down vertices for consistency.
        let yd = |v: [f32; 3]| [v[0], -v[1], v[2]];
        let face_n = norm_or(
            cross(sub(yd(tri[1]), yd(tri[0])), sub(yd(tri[2]), yd(tri[0]))),
            [0.0, 0.0, 1.0],
        );
        // at least one gaussian per triangle so thin features survive; bigger faces get more.
        let n = ((target_count as f32 * weights[ti] / total).round() as usize).max(1);
        for _ in 0..n {
            // uniform barycentric sample (reflect the far corner so it stays inside the triangle)
            let (mut bu, mut bv) = (hash01(idx * 2), hash01(idx * 2 + 1));
            idx = idx.wrapping_add(1);
            if bu + bv > 1.0 {
                bu = 1.0 - bu;
                bv = 1.0 - bv;
            }
            let bw = 1.0 - bu - bv;
            let lerp3 = |f: [[f32; 3]; 3]| {
                [
                    f[0][0] * bw + f[1][0] * bu + f[2][0] * bv,
                    f[0][1] * bw + f[1][1] * bu + f[2][1] * bv,
                    f[0][2] * bw + f[1][2] * bu + f[2][2] * bv,
                ]
            };
            let p = lerp3(*tri);

            // colour: vertex > texture@UV > material/flat
            let sh = if let Some(c) = tri_cols[ti] {
                sh_of(lerp3(c))
            } else if let (Some(img), Some(uvs)) = (&mesh_tex[mi], tri_uvs[ti]) {
                let u = uvs[0][0] * bw + uvs[1][0] * bu + uvs[2][0] * bv;
                let v = uvs[0][1] * bw + uvs[1][1] * bu + uvs[2][1] * bv;
                sh_of(sample_tex(img, u, v))
            } else {
                mesh_sh[mi]
            };

            // orient the disk: interpolated vertex normal (Y-down), else the face normal.
            let n_axis = match tri_norms[ti] {
                Some(nn) => {
                    let m = lerp3(nn);
                    norm_or([m[0], -m[1], m[2]], face_n)
                }
                None => face_n,
            };
            let (t, b) = tangent_basis(n_axis);
            out.push(Gaussian3d {
                // Y-down to match text/splats; the shared cloud_base_rotation flips it upright.
                position_visibility: [p[0], -p[1], p[2], 1.0].into(),
                spherical_harmonic: sh,
                rotation: quat_from_basis(t, b, n_axis).into(),
                scale_opacity: [r_plane, r_plane, r_thin, 1.0].into(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Verifies COLLADA `.dae` parsing + surface sampling + disk orientation without a GPU. Skips
    // when the (gitignored) asset isn't present, so CI without it still passes.
    #[test]
    fn dae_surface_samples_to_gaussians() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/defeest.dae");
        if !p.exists() {
            eprintln!("skip dae_surface_samples: {} not present", p.display());
            return;
        }
        let g = build_mesh_gaussians(&p, 10_000, 0.01, 0.2, [0.8, 0.85, 0.95]);
        assert!(!g.is_empty(), "defeest.dae produced no gaussians");
        let finite = g.iter().all(|gg| {
            gg.position_visibility
                .position
                .iter()
                .all(|c| c.is_finite())
                && gg.rotation.rotation.iter().all(|c| c.is_finite())
        });
        assert!(
            finite,
            "non-finite gaussian positions/rotations from the mesh"
        );
        // disks are flat: the thin axis (z scale) is well below the in-plane radius...
        let s = g[0].scale_opacity.scale;
        assert!(s[2] < s[0] * 0.5, "expected a flattened splat, got {s:?}");
        // ...and oriented: not every disk shares the identity rotation.
        let oriented = g
            .iter()
            .any(|gg| (gg.rotation.rotation[3] - 1.0).abs() > 1e-3);
        assert!(
            oriented,
            "every disk is identity-rotated — normals ignored?"
        );
        eprintln!("defeest.dae → {} oriented gaussians", g.len());
    }

    // Diagnostic for the bornhack badge: count, how many use the grey fallback colour, and any
    // non-finite / extreme positions that could crash the GPU.
    #[test]
    fn bornhack_diag() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/bornhack2026-hardware.dae");
        if !p.exists() {
            eprintln!("skip bornhack_diag: not present");
            return;
        }
        let fallback = [0.8_f32, 0.85, 0.95];
        let g = build_mesh_gaussians(&p, 60_000, 0.01, 0.2, fallback);
        let grey = sh_of(fallback);
        let n_grey = g
            .iter()
            .filter(|gg| {
                (0..3).all(|i| {
                    (gg.spherical_harmonic.coefficients[i] - grey.coefficients[i]).abs() < 1e-4
                })
            })
            .count();
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        let mut nonfinite = 0;
        for gg in &g {
            for c in gg.position_visibility.position {
                if !c.is_finite() {
                    nonfinite += 1;
                } else {
                    lo = lo.min(c);
                    hi = hi.max(c);
                }
            }
        }
        eprintln!(
            "bornhack: {} gaussians, {n_grey} grey ({:.0}%), pos range [{lo:.4}, {hi:.4}], nonfinite {nonfinite}",
            g.len(),
            100.0 * n_grey as f32 / g.len().max(1) as f32
        );
        assert_eq!(nonfinite, 0, "non-finite mesh positions");
        assert!(
            hi > lo,
            "mesh collapsed to a point (node transforms not baked?)"
        );
        assert!(
            n_grey * 4 < g.len(),
            "mesh mostly fell back to grey — materials not read?"
        );
    }
}
