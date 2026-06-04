//! Mesh → gaussians: sample a triangle mesh's **surface** (COLLADA `.dae`, `.obj`, `.stl`, `.ply`
//! via `mesh-loader`) into gaussians, so a 3D model — the deFEEST logo mesh, a bitterbal — is just
//! another morph/deform source that flows through the exact same pipeline as text/image/splats.
//! Built Y-DOWN (negate Y) so the shared `cloud_base_rotation` flips it upright like the others.
//! Pure (no Bevy/ECS) apart from the gaussian type, matching `text.rs` / `splat_image.rs`.

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

/// Twice the triangle area (= |edge0 × edge1|), used as the sampling weight.
fn double_area(t: &[[f32; 3]; 3]) -> f32 {
    let u = sub(t[1], t[0]);
    let v = sub(t[2], t[0]);
    let cx = u[1] * v[2] - u[2] * v[1];
    let cy = u[2] * v[0] - u[0] * v[2];
    let cz = u[0] * v[1] - u[1] * v[0];
    (cx * cx + cy * cy + cz * cz).sqrt()
}

/// Sample `target_count` gaussians over the mesh surface (distributed by triangle area). Each
/// gaussian is a small flat-coloured (or vertex-coloured) splat of size `splat`. Missing / bad
/// files → empty (the part just renders nothing rather than crashing the show).
pub fn build_mesh_gaussians(
    path: &Path,
    target_count: usize,
    splat: f32,
    rgb: [f32; 3],
) -> Vec<Gaussian3d> {
    let scene = match mesh_loader::Loader::default().load(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("mesh {}: {e}", path.display());
            return Vec::new();
        }
    };

    // mesh-loader builds `meshes[i]` and `materials[i]` from the same geometry in lockstep — so a
    // mesh's colour is its material's diffuse (COLLADA logos store colour per-material, not per
    // vertex). Priority per gaussian: vertex colours if the mesh has them, else material diffuse,
    // else the caller's flat `rgb`.
    let mut tris: Vec<[[f32; 3]; 3]> = Vec::new();
    let mut tri_cols: Vec<Option<[[f32; 3]; 3]>> = Vec::new();
    let mut tri_mesh: Vec<usize> = Vec::new();
    for (mi, mesh) in scene.meshes.iter().enumerate() {
        let cols = &mesh.colors[0]; // colour set 0; empty unless the mesh carries vertex colours
        let has_cols = !cols.is_empty() && cols.len() == mesh.vertices.len();
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
                let rgb_of = |i: usize| [cols[i][0], cols[i][1], cols[i][2]];
                [rgb_of(a), rgb_of(b), rgb_of(c)]
            }));
            tri_mesh.push(mi);
        }
    }
    if tris.is_empty() {
        eprintln!("mesh {}: no triangles", path.display());
        return Vec::new();
    }

    // one SH per mesh for the flat (no-vertex-colour) case: its material diffuse, else `rgb`.
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
            let sh = match tri_cols[ti] {
                Some(c) => sh_of(lerp3(c)),
                None => mesh_sh[tri_mesh[ti]],
            };
            out.push(Gaussian3d {
                // Y-down to match text/splats; the shared cloud_base_rotation flips it upright.
                position_visibility: [p[0], -p[1], p[2], 1.0].into(),
                spherical_harmonic: sh,
                rotation: [0.0, 0.0, 0.0, 1.0].into(),
                scale_opacity: [splat, splat, splat, 1.0].into(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Verifies COLLADA `.dae` parsing + surface sampling without a GPU. Skips when the (gitignored)
    // asset isn't present, so CI without it still passes.
    #[test]
    fn dae_surface_samples_to_gaussians() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/defeest.dae");
        if !p.exists() {
            eprintln!("skip dae_surface_samples: {} not present", p.display());
            return;
        }
        let g = build_mesh_gaussians(&p, 10_000, 0.01, [0.8, 0.85, 0.95]);
        assert!(!g.is_empty(), "defeest.dae produced no gaussians");
        let finite = g.iter().all(|gg| {
            gg.position_visibility
                .position
                .iter()
                .all(|c| c.is_finite())
        });
        assert!(finite, "non-finite gaussian positions from the mesh");
        eprintln!("defeest.dae → {} gaussians", g.len());
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
        let g = build_mesh_gaussians(&p, 60_000, 0.01, fallback);
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
