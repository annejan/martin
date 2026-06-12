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

/// Append one flat-disk gaussian at `pos`, lying in the surface plane (local +Z = `n_axis`).
fn push_disk(
    out: &mut Vec<Gaussian3d>,
    pos: [f32; 3],
    n_axis: [f32; 3],
    sh: SphericalHarmonicCoefficients,
    r_plane: f32,
    r_thin: f32,
    alpha: f32,
) {
    let (t, b) = tangent_basis(n_axis);
    out.push(Gaussian3d {
        position_visibility: [pos[0], pos[1], pos[2], 1.0].into(),
        spherical_harmonic: sh,
        rotation: quat_from_basis(t, b, n_axis).into(),
        scale_opacity: [r_plane, r_plane, r_thin, alpha].into(),
    });
}

/// Area-weighted surface sampling shared by the file loader and the glTF dissolve: scatter
/// `target_count` flat disks across the triangles (≥1 per triangle so thin features survive),
/// oriented to the interpolated vertex normal (face normal fallback), coloured by `color(tri, bw,
/// bu, bv)`. `flip_y` negates Y (the file path stores Y-down so `cloud_base_rotation` flips it
/// upright; the glTF path samples in the mesh's own frame so it coincides with the rendered mesh).
#[allow(clippy::too_many_arguments)] // geometry + 3 disk-style knobs (splat/thin/alpha) + colour closure
fn sample_surface_disks<F>(
    tris: &[[[f32; 3]; 3]],
    tri_norms: &[Option<[[f32; 3]; 3]>],
    target_count: usize,
    splat: f32,
    thin: f32,
    alpha: f32,
    flip_y: bool,
    mut color: F,
) -> Vec<Gaussian3d>
where
    F: FnMut(usize, f32, f32, f32) -> SphericalHarmonicCoefficients,
{
    let yd = |v: [f32; 3]| {
        if flip_y {
            [v[0], -v[1], v[2]]
        } else {
            v
        }
    };
    // Size every splat to the mean inter-sample SPACING (× `splat`, an overlap factor ≈1) so the disks
    // just cover the surface regardless of the mesh's size or polygon density. The old code used a
    // fixed fraction of the bbox's largest dimension — decoupled from the actual point spacing — so it
    // bloomed into a fuzzy haze where samples were dense and left gaps where they were sparse.
    let weights: Vec<f32> = tris.iter().map(double_area).collect();
    let total: f32 = weights.iter().sum::<f32>().max(1e-9);
    let spacing = (0.5 * total / target_count.max(1) as f32).sqrt(); // double_area = 2·area
    let r_plane = (splat * spacing).max(1e-6);
    let r_thin = (r_plane * thin).max(1e-7);
    let mut out: Vec<Gaussian3d> = Vec::with_capacity(target_count + tris.len());
    // R2 low-discrepancy sequence (plastic-number additive recurrence): evenly-spread barycentric
    // samples instead of random ones, which kills the clumpy/grainy look. The accumulators stay in
    // [0,1) so precision holds even at hundreds of thousands of points (a raw `frac(a·i)` would not).
    let (mut r2u, mut r2v) = (0.5f32, 0.5f32);
    for (ti, tri) in tris.iter().enumerate() {
        let face_n = norm_or(
            cross(sub(yd(tri[1]), yd(tri[0])), sub(yd(tri[2]), yd(tri[0]))),
            [0.0, 0.0, 1.0],
        );
        let n = ((target_count as f32 * weights[ti] / total).round() as usize).max(1);
        for _ in 0..n {
            const A1: f32 = 0.754_877_7; // 1/plastic
            const A2: f32 = 0.569_840_3; // 1/plastic²
            r2u = (r2u + A1).fract();
            r2v = (r2v + A2).fract();
            let (mut bu, mut bv) = (r2u, r2v);
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
            let sh = color(ti, bw, bu, bv);
            let n_axis = match tri_norms[ti] {
                Some(nn) => norm_or(yd(lerp3(nn)), face_n),
                None => face_n,
            };
            push_disk(&mut out, yd(p), n_axis, sh, r_plane, r_thin, alpha);
        }
    }
    out
}

/// N fully-transparent gaussians — a placeholder for a part with no splats of its own (a glTF
/// dissolve until `sample_gl_mesh` fills it, or a `shader:` interlude). Spread over a small sphere
/// (golden-angle spiral, deterministic), NOT stacked at the origin: a transparent cloud is sometimes
/// rendered for real (an interlude holds it for seconds), and 30k coincident splats on one pixel
/// TDRs the GPU. Spread + a valid scale keeps it invisible (opacity 0) but harmless to rasterize.
pub fn transparent_placeholder(n: usize) -> Vec<Gaussian3d> {
    let n = n.max(1);
    let ga = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt()); // golden angle
    (0..n)
        .map(|i| {
            let y = 1.0 - 2.0 * (i as f32 + 0.5) / n as f32; // -1..1
            let r = (1.0 - y * y).max(0.0).sqrt();
            let a = ga * i as f32;
            Gaussian3d {
                position_visibility: [r * a.cos() * 0.5, y * 0.5, r * a.sin() * 0.5, 0.0].into(),
                spherical_harmonic: sh_of([0.0, 0.0, 0.0]),
                rotation: [0.0, 0.0, 0.0, 1.0].into(),
                scale_opacity: [0.02, 0.02, 0.02, 0.0].into(),
            }
        })
        .collect()
}

/// Build flat-disk gaussians from triangles already in their final frame (positions + normals in
/// the SAME space the mesh renders), one flat colour per triangle. No Y-flip — the glTF dissolve
/// wants the splats to coincide with the rendered mesh, not the Y-down splat convention.
pub fn build_gaussians_from_tris(
    tris: &[[[f32; 3]; 3]],
    tri_norms: &[Option<[[f32; 3]; 3]>],
    tri_rgb: &[[f32; 3]],
    target_count: usize,
    splat: f32,
    thin: f32,
    alpha: f32,
) -> Vec<Gaussian3d> {
    sample_surface_disks(
        tris,
        tri_norms,
        target_count,
        splat,
        thin,
        alpha,
        false,
        |ti, _, _, _| sh_of(tri_rgb[ti]),
    )
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
/// glTF (.glb/.gltf) → gaussians. mesh-loader can't read glTF, so we parse it here: walk the node
/// graph accumulating world transforms, pull each primitive's positions/indices/normals/vertex
/// colours, and surface-sample the triangles exactly like the .obj/.dae path. No textures (vertex
/// colours when present, else the caller's flat `rgb`).
fn build_gltf_gaussians(
    path: &Path,
    target_count: usize,
    splat: f32,
    thin: f32,
    alpha: f32,
    rgb: [f32; 3],
) -> Vec<Gaussian3d> {
    let (doc, buffers, _) = match gltf::import(path) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("mesh {}: {e}", path.display());
            return Vec::new();
        }
    };
    type M = [[f32; 4]; 4]; // column-major (glTF convention)
    let mul = |a: &M, b: &M| -> M {
        let mut o = [[0.0f32; 4]; 4];
        for (c, oc) in o.iter_mut().enumerate() {
            for (r, ocr) in oc.iter_mut().enumerate() {
                *ocr = (0..4).map(|k| a[k][r] * b[c][k]).sum();
            }
        }
        o
    };
    let xf_p = |m: &M, p: [f32; 3]| {
        [
            m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0],
            m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1],
            m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2],
        ]
    };
    let xf_n = |m: &M, n: [f32; 3]| {
        let v = [
            m[0][0] * n[0] + m[1][0] * n[1] + m[2][0] * n[2],
            m[0][1] * n[0] + m[1][1] * n[1] + m[2][1] * n[2],
            m[0][2] * n[0] + m[1][2] * n[1] + m[2][2] * n[2],
        ];
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
        [v[0] / l, v[1] / l, v[2] / l]
    };

    let mut tris: Vec<[[f32; 3]; 3]> = Vec::new();
    let mut tri_norms: Vec<Option<[[f32; 3]; 3]>> = Vec::new();
    let mut tri_cols: Vec<Option<[[f32; 3]; 3]>> = Vec::new();
    let id: M = [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ];
    let scene = doc.default_scene().or_else(|| doc.scenes().next());
    let mut stack: Vec<(gltf::Node, M)> = Vec::new();
    if let Some(scene) = scene {
        stack.extend(scene.nodes().map(|n| (n, id)));
    }
    while let Some((node, parent)) = stack.pop() {
        let world = mul(&parent, &node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            for prim in mesh.primitives() {
                let reader = prim.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
                let Some(pos) = reader.read_positions() else {
                    continue;
                };
                let positions: Vec<[f32; 3]> = pos.collect();
                let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(|n| n.collect());
                let colors: Option<Vec<[f32; 3]>> =
                    reader.read_colors(0).map(|c| c.into_rgb_f32().collect());
                let indices: Vec<u32> = reader
                    .read_indices()
                    .map(|i| i.into_u32().collect())
                    .unwrap_or_else(|| (0..positions.len() as u32).collect());
                for t in indices.chunks_exact(3) {
                    let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
                    let (Some(&pa), Some(&pb), Some(&pc)) =
                        (positions.get(a), positions.get(b), positions.get(c))
                    else {
                        continue;
                    };
                    tris.push([xf_p(&world, pa), xf_p(&world, pb), xf_p(&world, pc)]);
                    tri_norms.push(normals.as_ref().map(|nn| {
                        [
                            xf_n(&world, nn[a]),
                            xf_n(&world, nn[b]),
                            xf_n(&world, nn[c]),
                        ]
                    }));
                    tri_cols.push(colors.as_ref().map(|cc| [cc[a], cc[b], cc[c]]));
                }
            }
        }
        stack.extend(node.children().map(|ch| (ch, world)));
    }
    if tris.is_empty() {
        eprintln!("mesh {}: no triangles (glTF)", path.display());
        return Vec::new();
    }
    let flat = sh_of(rgb);
    sample_surface_disks(
        &tris,
        &tri_norms,
        target_count,
        splat,
        thin,
        alpha,
        true,
        |ti, bw, bu, bv| match tri_cols[ti] {
            Some(c) => sh_of([
                c[0][0] * bw + c[1][0] * bu + c[2][0] * bv,
                c[0][1] * bw + c[1][1] * bu + c[2][1] * bv,
                c[0][2] * bw + c[1][2] * bu + c[2][2] * bv,
            ]),
            None => flat,
        },
    )
}

pub fn build_mesh_gaussians(
    path: &Path,
    target_count: usize,
    splat: f32,
    thin: f32,
    alpha: f32,
    rgb: [f32; 3],
) -> Vec<Gaussian3d> {
    // mesh-loader handles .obj/.stl/.dae/.ply but NOT glTF — route .glb/.gltf to a dedicated path.
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "glb" || ext == "gltf" {
        return build_gltf_gaussians(path, target_count, splat, thin, alpha, rgb);
    }
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

    // Surface-sample (Y-down), colour per sample: vertex colours > diffuse texture@UV > material/flat.
    sample_surface_disks(
        &tris,
        &tri_norms,
        target_count,
        splat,
        thin,
        alpha,
        true,
        |ti, bw, bu, bv| {
            let mi = tri_mesh[ti];
            let lerp3 = |f: [[f32; 3]; 3]| {
                [
                    f[0][0] * bw + f[1][0] * bu + f[2][0] * bv,
                    f[0][1] * bw + f[1][1] * bu + f[2][1] * bv,
                    f[0][2] * bw + f[1][2] * bu + f[2][2] * bv,
                ]
            };
            if let Some(c) = tri_cols[ti] {
                sh_of(lerp3(c))
            } else if let (Some(img), Some(uvs)) = (&mesh_tex[mi], tri_uvs[ti]) {
                let u = uvs[0][0] * bw + uvs[1][0] * bu + uvs[2][0] * bv;
                let v = uvs[0][1] * bw + uvs[1][1] * bu + uvs[2][1] * bv;
                sh_of(sample_tex(img, u, v))
            } else {
                mesh_sh[mi]
            }
        },
    )
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
        let g = build_mesh_gaussians(&p, 10_000, 1.2, 0.2, 1.0, [0.8, 0.85, 0.95]);
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
        let g = build_mesh_gaussians(&p, 60_000, 1.2, 0.2, 1.0, fallback);
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
