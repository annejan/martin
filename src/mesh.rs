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

    // Flatten every mesh's faces into world-space triangles + (optional) per-vertex colours.
    let mut tris: Vec<[[f32; 3]; 3]> = Vec::new();
    let mut tri_cols: Vec<Option<[[f32; 3]; 3]>> = Vec::new();
    for mesh in &scene.meshes {
        let cols = &mesh.colors[0]; // colour set 0; empty if the mesh carries no vertex colours
        let has_cols = cols.len() == mesh.vertices.len() && !cols.is_empty();
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
        }
    }
    if tris.is_empty() {
        eprintln!("mesh {}: no triangles", path.display());
        return Vec::new();
    }

    let weights: Vec<f32> = tris.iter().map(double_area).collect();
    let total: f32 = weights.iter().sum::<f32>().max(1e-9);

    let mut flat = SphericalHarmonicCoefficients::default();
    flat.set(0, dc(rgb[0]));
    flat.set(1, dc(rgb[1]));
    flat.set(2, dc(rgb[2]));

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
                Some(c) => {
                    let col = lerp3(c);
                    let mut s = SphericalHarmonicCoefficients::default();
                    s.set(0, dc(col[0]));
                    s.set(1, dc(col[1]));
                    s.set(2, dc(col[2]));
                    s
                }
                None => flat,
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
}
