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
use crate::splat_image::build_image_gaussians;
use crate::text::{build_text_gaussians, TEXT_RGB};

const SIDE_SEP: f32 = 1.2; // half-spacing when a part places several splats side by side

#[derive(Clone)]
pub(crate) enum PartContent {
    Text(String),
    /// a PNG in the asset dir, rasterized to flat gaussians (a logo, etc.)
    Image(String),
    /// a mesh in the asset dir (`.dae`/`.obj`/`.stl`/`.ply`), surface-sampled into gaussians
    Mesh(String),
    /// one or more splats (filename in the asset dir, world offset) combined into one shape
    Splats(Vec<(String, Vec3)>),
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
    } else if let Some(name) = head.strip_prefix("mesh:") {
        PartContent::Mesh(name.trim().to_string())
    } else if let Some(p) = head.strip_prefix("splat:") {
        PartContent::Splats(side_by_side(
            p.split('+').map(str::trim).filter(|x| !x.is_empty()),
        ))
    } else {
        return None;
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
            mesh::build_mesh_gaussians(&root.join(name), count, splat, rgb)
        }
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
