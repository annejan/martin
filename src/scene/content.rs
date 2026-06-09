//! What a part/object *is* — its source content — and how to turn it into gaussians.
//!
//! Shared by the morph timeline (`sequence`) and the composition stage (`compose`): both parse
//! the same `text:`/`wall:`/`image:`/`mesh:`/`splat:` heads and sample them into gaussians the
//! same way.

use bevy::prelude::*;
use bevy_gaussian_splatting::{Gaussian3d, PlanarGaussian3d};

use crate::mesh;
use crate::scene::file_name_of;
use crate::scene::sequence::SeqState;
use crate::splat_image::{build_image_gaussians, build_svg_gaussians};
use crate::text::{build_text_gaussians, TEXT_RGB};

const SIDE_SEP: f32 = 1.2; // half-spacing when a part places several splats side by side

#[derive(Clone)]
pub(crate) enum PartContent {
    Text(String),
    /// a PNG in the asset dir, rasterized to flat gaussians (a logo, etc.)
    Image(String),
    /// an SVG in the asset dir, rasterized (vector → pixels) then sampled to flat gaussians — any
    /// vector logo/art as a morph source, crisp at any size you raster it to (`MARTIN_SVG_PX`).
    Svg(String),
    /// a mesh in the asset dir (`.dae`/`.obj`/`.stl`/`.ply`), surface-sampled into gaussians
    Mesh(String),
    /// one or more splats (filename in the asset dir, world offset) combined into one shape
    Splats(Vec<(String, Vec3)>),
    /// a **real glTF mesh** (`.glb`/`.gltf`) rendered as PBR geometry *alongside* the splats (not
    /// sampled to gaussians) — they share the camera + depth, so meshes and splats coexist.
    /// Compose-stage only (a rigid prop; it doesn't morph).
    Model(String),
    /// a **real glTF mesh** (`.glb`/`.gltf`) rendered crisp AND surface-sampled into gaussians from
    /// that *same* loaded mesh — so the mesh can DISSOLVE into its own splats (which then morph on).
    /// Sequence-only; the gaussians are filled at runtime by `sample_gl_mesh` (see sequence.rs).
    GlMesh(String),
    /// a **fullscreen WGSL effect** (`shader:warp`/`plasma`/`tunnel`/`stars`) as a timeline interlude:
    /// the splats clear (this part's gaussians are transparent) and the effect plays full-frame,
    /// fading in/out across the part. Sequence-only; rendered by `scene::shader_part`.
    Shader(String),
}

impl PartContent {
    /// A short human label for logs / the `MARTIN_VALIDATE` report (e.g. `text "HELLO"`, `svg x.svg`).
    pub(crate) fn label(&self) -> String {
        match self {
            PartContent::Text(s) => format!("text \"{s}\""),
            PartContent::Image(name) => format!("image {name}"),
            PartContent::Svg(name) => format!("svg {name}"),
            PartContent::Mesh(name) => format!("mesh {name}"),
            PartContent::Model(name) => format!("model {name}"),
            PartContent::GlMesh(name) => format!("gl-mesh {name}"),
            PartContent::Shader(name) => format!("shader {name}"),
            PartContent::Splats(list) => list
                .iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>()
                .join("+"),
        }
    }
}

/// Parse a source head (`text:` / `wall:` / `image:` / `mesh:` / `splat:`) into a `PartContent`.
/// Shared by the morph timeline (`parse_seq`) and the composition stage (`parse_compose`).
pub(crate) fn parse_source(head: &str) -> Option<PartContent> {
    Some(if let Some(txt) = head.strip_prefix("text:") {
        PartContent::Text(txt.to_string())
    } else if let Some(w) = head.strip_prefix("wall:") {
        // a wall of text: a multi-line block. `|` separates lines (build_text_gaussians lays out
        // `\n`), or point at a text file. Great with a `^deform` to make it ripple/billow.
        let w = w.trim();
        PartContent::Text(std::fs::read_to_string(w).unwrap_or_else(|_| w.replace('|', "\n")))
    } else if let Some(name) = head.strip_prefix("image:") {
        PartContent::Image(name.trim().to_string())
    } else if let Some(name) = head.strip_prefix("svg:") {
        PartContent::Svg(name.trim().to_string())
    } else if let Some(name) = head.strip_prefix("mesh:") {
        PartContent::Mesh(name.trim().to_string())
    } else if let Some(name) = head.strip_prefix("model:") {
        PartContent::Model(name.trim().to_string())
    } else if let Some(name) = head
        .strip_prefix("glb:")
        .or_else(|| head.strip_prefix("gltf:"))
    {
        PartContent::GlMesh(name.trim().to_string())
    } else if let Some(name) = head.strip_prefix("shader:") {
        // a fullscreen-effect interlude; `name` is an effect (warp/plasma/tunnel/stars), `.wgsl` optional.
        PartContent::Shader(name.trim().trim_end_matches(".wgsl").to_string())
    } else {
        let p = head.strip_prefix("splat:")?;
        PartContent::Splats(side_by_side(
            p.split('+').map(str::trim).filter(|x| !x.is_empty()),
        ))
    })
}

/// Arrange splat filenames evenly along X, centered (one splat → at origin).
pub(crate) fn side_by_side<'a>(names: impl Iterator<Item = &'a str>) -> Vec<(String, Vec3)> {
    let names: Vec<&str> = names.collect();
    let n = names.len();
    names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let x = if n <= 1 {
                0.0
            } else {
                -SIDE_SEP + 2.0 * SIDE_SEP * (i as f32) / ((n - 1) as f32)
            };
            (file_name_of(name), Vec3::new(x, 0.0, 0.0))
        })
        .collect()
}

/// Read a part's gaussians (text rasterized, a PNG logo rasterized, or splats loaded + offset
/// + combined). `root` is the asset folder PNG `image:` parts are read from.
pub(crate) fn part_gaussians(
    content: &PartContent,
    state: &SeqState,
    assets: &Assets<PlanarGaussian3d>,
    root: &std::path::Path,
) -> Vec<Gaussian3d> {
    match content {
        PartContent::Text(s) => build_text_gaussians(s, TEXT_RGB, 3.0, 2, 0.012),
        PartContent::Image(name) => match std::fs::read(root.join(name)) {
            Ok(bytes) => {
                // MARTIN_IMG_STRIDE (pixel subsample) / MARTIN_IMG_SPLAT (gaussian size) tune crispness.
                let stride = std::env::var("MARTIN_IMG_STRIDE")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(2);
                let splat = std::env::var("MARTIN_IMG_SPLAT")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.012);
                build_image_gaussians(&bytes, 3.0, stride, splat, 0.5, 0.85)
            }
            Err(e) => {
                warn!("image {name}: {e}");
                Vec::new()
            }
        },
        PartContent::Svg(name) => match std::fs::read(root.join(name)) {
            Ok(bytes) => {
                // shares the image knobs (MARTIN_IMG_STRIDE/_SPLAT); MARTIN_SVG_PX = raster width.
                let stride = std::env::var("MARTIN_IMG_STRIDE")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(2);
                let splat = std::env::var("MARTIN_IMG_SPLAT")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.012);
                let px = std::env::var("MARTIN_SVG_PX")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(512);
                build_svg_gaussians(&bytes, px, 3.0, stride, splat, 0.5, 0.85)
            }
            Err(e) => {
                warn!("svg {name}: {e}");
                Vec::new()
            }
        },
        PartContent::Mesh(name) => {
            // MARTIN_MESH_COUNT (target gaussian count), MARTIN_MESH_SPLAT (size), MARTIN_MESH_RGB
            // ("r,g,b" flat colour; vertex colours used when the mesh has them).
            let count = std::env::var("MARTIN_MESH_COUNT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(60_000);
            // MARTIN_MESH_SPLAT is the splat size as a FRACTION of the mesh's largest dimension
            // (scale-invariant — works for a tiny badge or a unit object alike).
            let splat = std::env::var("MARTIN_MESH_SPLAT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.006);
            let rgb = std::env::var("MARTIN_MESH_RGB")
                .ok()
                .and_then(|s| {
                    let n: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    (n.len() == 3).then(|| [n[0], n[1], n[2]])
                })
                .unwrap_or([0.80, 0.85, 0.95]);
            // MARTIN_MESH_THIN: disk thickness as a fraction of the in-plane radius (flatness).
            let thin = std::env::var("MARTIN_MESH_THIN")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.2);
            mesh::build_mesh_gaussians(&root.join(name), count, splat, thin, rgb)
        }
        // A real glTF mesh isn't sampled to gaussians — build_composition spawns it as PBR geometry.
        PartContent::Model(_) => Vec::new(),
        // A glTF dissolve part: a transparent placeholder now; sample_gl_mesh fills it from the
        // loaded mesh once it's ready (invisible until then, and the rendered mesh covers it).
        PartContent::GlMesh(_) => mesh::transparent_placeholder(256),
        // A shader interlude: no splats — transparent placeholder so the morph chain stays valid
        // (the splats simply clear), while scene::shader_part plays the fullscreen effect over it.
        PartContent::Shader(_) => mesh::transparent_placeholder(256),
        PartContent::Splats(list) => {
            let mut out = Vec::new();
            for (name, off) in list {
                let Some(idx) = state.load_names.iter().position(|x| x == name) else {
                    continue;
                };
                if let Some(cloud) = assets.get(&state.loads[idx]) {
                    for mut g in cloud.iter() {
                        let p = g.position_visibility.position;
                        g.position_visibility.position = [p[0] + off.x, p[1] + off.y, p[2] + off.z];
                        out.push(g);
                    }
                }
            }
            out
        }
    }
}
