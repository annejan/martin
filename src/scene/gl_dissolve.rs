//! The `glb:` dissolve feature: a real PBR glTF mesh rendered crisp on a sequence part, whose
//! gaussians are surface-sampled from that SAME loaded mesh so they coincide by construction. The
//! mesh materializes over `MODEL_FADE`, holds crisp, then dissolves (material alpha) over its own
//! `DISSOLVE_LEN` step at the end of the hold while the splats it crumbles into take over.

use bevy::gltf::GltfAssetLabel;
use bevy::math::EulerRot;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy_gaussian_splatting::PlanarGaussian3d;

use crate::morph::resample_morton;
use crate::scene::NORMALIZE_EXTENT;
use crate::scene::sequence::{SeqState, Sequence, Shot};

const MODEL_FADE: f32 = 0.6; // splats→mesh materialize time (s), after the part's splat-assemble
const DISSOLVE_LEN: f32 = 1.2; // mesh→splats dissolve time (s) — its OWN step at the end of the hold

/// The `glb:` dissolve: a real PBR glTF mesh rendered crisp on one sequence part, whose gaussians
/// are sampled from that SAME loaded mesh (`sample_gl_mesh`) so they coincide exactly. As the part
/// morphs out the mesh dissolves (material alpha) and the splats it crumbles into morph away.
#[derive(Component)]
pub(crate) struct SeqModel {
    part: usize,    // sequence part this mesh shadows (drives the cue timing)
    base_rot: Quat, // the cloud's global orientation; the mesh + its splats share it
    rot: Quat,      // this part's own `rot:` (baked into the sampled splats + the mesh transform)
    shape: Handle<PlanarGaussian3d>, // this part's shape asset, filled from the sampled mesh
    morph_n: usize, // gaussian budget the shape resamples to
    sample_count: usize, // disks to scatter over the mesh before resampling
    splat: f32, // disk size (overlap factor on mean inter-sample spacing; ~1 = disks just touch)
    thin: f32,  // disk thickness fraction
    alpha: f32, // per-splat opacity (1 = solid; <1 softens silhouette edges)
    sampled: bool, // done once the mesh has loaded + been sampled
}

/// Spawn a `glb:` dissolve overlay: the rendered glTF mesh (hidden + identity until sampled, so
/// `sample_gl_mesh` can read its node-local geometry, then place it to coincide with the splats) +
/// a key/fill light (splats are unlit, the PBR mesh needs light).
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_gl_dissolve(
    commands: &mut Commands,
    assets: &AssetServer,
    name: &str,
    part: usize,
    base_rot: Quat,
    rot: Quat,
    shape: Handle<PlanarGaussian3d>,
    morph_n: usize,
) {
    use crate::envvar::or as env;
    commands.spawn((
        SceneRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(name.to_string()))),
        Transform::IDENTITY,
        Visibility::Hidden,
        SeqModel {
            part,
            base_rot,
            rot,
            shape,
            morph_n,
            sample_count: env("MARTIN_MESH_COUNT", morph_n),
            splat: env("MARTIN_MESH_SPLAT", 1.2),
            thin: env("MARTIN_MESH_THIN", 0.3),
            alpha: env("MARTIN_MESH_OPACITY", 0.6),
            sampled: false,
        },
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 9000.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.6, 0.5, 0.0)),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 3500.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, 0.5, -0.8, 0.0)),
    ));
}

/// Once a `glb:` mesh has loaded, surface-sample it into gaussians IN ITS OWN FRAME and fill the
/// part's shape with them — then place the rendered mesh on the very same `(centroid, scale)` the
/// gaussians were normalized by, so mesh and splats coincide by construction (no alignment knobs).
/// The only glTF meshes in a sequence are the overlay's, so we read every `Mesh3d`.
#[allow(clippy::type_complexity)]
pub(crate) fn sample_gl_mesh(
    mut models: Query<(&mut SeqModel, &mut Transform, &mut Visibility)>,
    prims: Query<(&Mesh3d, &MeshMaterial3d<StandardMaterial>, &GlobalTransform)>,
    meshes: Res<Assets<Mesh>>,
    mats: Res<Assets<StandardMaterial>>,
    mut clouds: ResMut<Assets<PlanarGaussian3d>>,
) {
    for (mut model, mut tf, mut vis) in &mut models {
        if model.sampled {
            continue;
        }
        let prims: Vec<_> = prims.iter().collect();
        // wait until the scene has spawned its meshes and every mesh asset is loaded.
        if prims.is_empty() || prims.iter().any(|(m, _, _)| meshes.get(&m.0).is_none()) {
            continue;
        }
        // triangles in the scene's own frame (the entity is still identity → each child's
        // GlobalTransform is its node-local transform), one flat material colour per primitive.
        let mut tris: Vec<[[f32; 3]; 3]> = Vec::new();
        let mut tri_norms: Vec<Option<[[f32; 3]; 3]>> = Vec::new();
        let mut tri_rgb: Vec<[f32; 3]> = Vec::new();
        for (m, mat, gt) in &prims {
            let Some(mesh) = meshes.get(&m.0) else {
                continue;
            };
            let Some(pos) = mesh
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .and_then(|a| a.as_float3())
            else {
                continue;
            };
            let norms = mesh
                .attribute(Mesh::ATTRIBUTE_NORMAL)
                .and_then(|a| a.as_float3());
            let rgb = mats
                .get(&mat.0)
                .map(|sm| {
                    let c = sm.base_color.to_srgba();
                    [c.red, c.green, c.blue]
                })
                .unwrap_or([0.8, 0.8, 0.8]);
            let xf = |p: [f32; 3]| gt.transform_point(Vec3::from_array(p)).to_array();
            let xn = |n: [f32; 3]| {
                gt.affine()
                    .transform_vector3(Vec3::from_array(n))
                    .to_array()
            };
            let idxs: Vec<usize> = match mesh.indices() {
                Some(Indices::U16(v)) => v.iter().map(|&i| i as usize).collect(),
                Some(Indices::U32(v)) => v.iter().map(|&i| i as usize).collect(),
                None => (0..pos.len()).collect(),
            };
            for t in idxs.chunks_exact(3) {
                let (Some(&pa), Some(&pb), Some(&pc)) =
                    (pos.get(t[0]), pos.get(t[1]), pos.get(t[2]))
                else {
                    continue;
                };
                tris.push([xf(pa), xf(pb), xf(pc)]);
                tri_norms.push(norms.and_then(|nn| {
                    Some([xn(*nn.get(t[0])?), xn(*nn.get(t[1])?), xn(*nn.get(t[2])?)])
                }));
                tri_rgb.push(rgb);
            }
        }
        if tris.is_empty() {
            model.sampled = true;
            continue;
        }
        let mut raw = crate::mesh::build_gaussians_from_tris(
            &tris,
            &tri_norms,
            &tri_rgb,
            model.sample_count,
            model.splat,
            model.thin,
            model.alpha,
        );
        // normalize like every morph part — capture (centroid, scale) to place the mesh identically.
        let (c, k) = crate::morph::normalize_to(&mut raw, NORMALIZE_EXTENT);
        // bake the part's own `rot:` into the gaussians; the mesh transform gets the same below, so
        // mesh + splats stay coincident at any orientation.
        crate::morph::rotate_gaussians(&mut raw, model.rot);
        let shaped = resample_morton(raw, model.morph_n);
        if let Some(cloud) = clouds.get_mut(&model.shape) {
            *cloud = PlanarGaussian3d::from(shaped);
        }
        // gaussian world = base_rot · rot · (k·(p − c)); match it on the mesh transform.
        let br = model.base_rot * model.rot;
        *tf = Transform {
            translation: -(br * (c * k)),
            rotation: br,
            scale: Vec3::splat(k),
        };
        *vis = Visibility::Visible;
        model.sampled = true;
        info!(
            "gl dissolve: sampled {} triangles → {} gaussians",
            tris.len(),
            model.morph_n
        );
    }
}

/// The `glb:` mesh's opacity over its part's life — the choreography backbone. The splat-opacity is
/// the exact complement (`1 - this`, see shot_director), so mesh and splats crossfade cleanly:
/// 0 while the part assembles AS SPLATS → MATERIALIZE 0→1 over MODEL_FADE → 1 crisp hold → its OWN
/// DISSOLVE 1→0 over the last DISSOLVE_LEN of the hold (finishing BEFORE the next part's morph, so
/// the splats are fully back before they morph on — the dissolve is a distinct step, not overlapped).
pub(crate) fn gl_mesh_alpha(starts: &[f32], parts: &[Shot], p: usize, t: f32) -> f32 {
    // PART 0 is the OPENING: the mesh is crisp from the very start (no splat-assemble), so it picks
    // up exactly where the loader's logo left off — the show flows OUT of the logo (svg→mesh→splats)
    // rather than ball-assembling. Later parts assemble as splats first, then materialize.
    let (appear_end, crisp_at) = if p == 0 {
        (starts[0], starts[0])
    } else {
        let assemble_end = starts[p] + parts[p].morph;
        (assemble_end, assemble_end + MODEL_FADE)
    };
    // dissolve ends right as the next part starts morphing; carve DISSOLVE_LEN out of the hold for it.
    let (dissolve_start, dissolve_end) = match p + 1 {
        next if next < parts.len() => ((starts[next] - DISSOLVE_LEN).max(crisp_at), starts[next]),
        _ => (f32::MAX, f32::MAX), // last part: never dissolves, just stays crisp
    };
    if t < appear_end {
        0.0
    } else if t < crisp_at {
        (t - appear_end) / MODEL_FADE
    } else if t < dissolve_start {
        1.0
    } else if t < dissolve_end {
        1.0 - (t - dissolve_start) / (dissolve_end - dissolve_start).max(1e-3)
    } else {
        0.0
    }
    .clamp(0.0, 1.0)
}

/// Drive the dissolve-mesh's material opacity from `gl_mesh_alpha`. The only PBR materials in a
/// sequence are the overlay's, so we fade every `StandardMaterial`.
pub(crate) fn animate_seq_model(
    seq: Option<Res<Sequence>>,
    state: Option<Res<SeqState>>,
    clock: Res<crate::scene::SeqClock>,
    model: Query<&SeqModel>,
    handles: Query<&MeshMaterial3d<StandardMaterial>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let (Some(seq), Some(state), Ok(m)) = (seq, state, model.single()) else {
        return;
    };
    if !state.built {
        return;
    }
    let vis = gl_mesh_alpha(&state.starts(), &seq.parts, m.part, clock.t);
    for h in &handles {
        if let Some(mat) = mats.get_mut(&h.0) {
            mat.base_color.set_alpha(vis);
            // opaque while crisp (writes depth → occludes the splats behind); blend while dissolving
            // (no depth write → the splats show through as it fades).
            mat.alpha_mode = if vis >= 0.999 {
                AlphaMode::Opaque
            } else {
                AlphaMode::Blend
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::content::PartContent;

    fn part(hold: f32, morph: f32) -> Shot {
        Shot {
            content: PartContent::Text("x".into()),
            hold,
            morph,
            bulge: 0.0,
            transition: None,
            anchor: None,
            deform: None,
            out: None,
            rot: None,
            cluster: None,
            bg: None,
            raster: None,
            flash: None,
            deform_amp: None,
            beat: None,
        }
    }

    #[test]
    fn part0_logo_is_crisp_from_the_start() {
        // the opening glb: must be at full alpha at t=0 (the show flows OUT of the logo).
        let parts = vec![part(5.0, 3.0), part(3.0, 2.0)];
        let starts = vec![0.0, 8.0];
        assert_eq!(gl_mesh_alpha(&starts, &parts, 0, 0.0), 1.0);
    }

    #[test]
    fn later_part_assembles_holds_then_dissolves_before_the_next() {
        // part 1: starts 8, morph 2 → assembled at 10, crisp at 10+MODEL_FADE; next part at 16,
        // so it dissolves over [16-DISSOLVE_LEN, 16].
        let parts = vec![part(5.0, 3.0), part(4.0, 2.0), part(3.0, 2.0)];
        let starts = vec![0.0, 8.0, 16.0];
        assert_eq!(gl_mesh_alpha(&starts, &parts, 1, 8.5), 0.0); // still assembling as splats
        let crisp_at = 10.0 + MODEL_FADE;
        assert_eq!(gl_mesh_alpha(&starts, &parts, 1, crisp_at + 0.5), 1.0); // crisp hold
        assert_eq!(gl_mesh_alpha(&starts, &parts, 1, 16.0), 0.0); // fully dissolved by the next start
        let mid = 16.0 - DISSOLVE_LEN / 2.0; // halfway through the dissolve
        let a = gl_mesh_alpha(&starts, &parts, 1, mid);
        assert!(a > 0.2 && a < 0.8, "mid-dissolve alpha {a}");
    }

    #[test]
    fn last_part_never_dissolves() {
        let parts = vec![part(5.0, 3.0), part(3.0, 2.0)];
        let starts = vec![0.0, 8.0];
        let crisp = 8.0 + 2.0 + MODEL_FADE;
        assert_eq!(gl_mesh_alpha(&starts, &parts, 1, crisp + 100.0), 1.0); // stays crisp forever
    }
}
