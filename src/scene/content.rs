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
use crate::text::{TEXT_RGB, build_text_gaussians};

const SIDE_SEP: f32 = 1.2; // half-spacing when a part places several splats side by side

#[derive(Clone)]
pub(crate) enum PartContent {
    Text(String),
    /// a PNG in the asset dir, rasterized to flat gaussians (a logo, etc.)
    Image(String),
    /// an SVG in the asset dir, rasterized (vector → pixels) then sampled to flat gaussians — any
    /// vector logo/art as a morph source, crisp at any size you raster it to.
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

    /// The asset filenames this content loads from the asset folder (empty for text/wall/shader,
    /// which carry no file). Used by the `intro` production-kind budget check (see `validate`).
    pub(crate) fn asset_files(&self) -> Vec<&str> {
        match self {
            PartContent::Image(n)
            | PartContent::Svg(n)
            | PartContent::Mesh(n)
            | PartContent::Model(n)
            | PartContent::GlMesh(n) => vec![n.as_str()],
            PartContent::Splats(list) => list.iter().map(|(n, _)| n.as_str()).collect(),
            PartContent::Text(_) | PartContent::Shader(_) => Vec::new(),
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn part_gaussians(
    content: &PartContent,
    state: &SeqState,
    assets: &Assets<PlanarGaussian3d>,
    root: &std::path::Path,
    disk: Option<f32>,
    aniso: Option<f32>,
    count: Option<usize>,
    font: Option<&str>,
) -> Vec<Gaussian3d> {
    match content {
        // `splat:` reads the already-loaded `.ply` asset — the only content that needs the Bevy world
        // (`&Assets`/`&SeqState`). Everything else is pure fs+compute → `sample_non_splat`, shared with
        // the off-thread reel build (which pre-extracts splats on the main thread, see `build.rs`).
        PartContent::Splats(list) => {
            let mut out = Vec::new();
            for (name, off) in list {
                let Some(idx) = state.load_names.iter().position(|x| x == name) else {
                    warn!("splat '{name}' not in load — check spelling / MARTIN_PLY paths");
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
        other => sample_non_splat(other, root, disk, aniso, count, font),
    }
}

/// The asset-free part of part sampling: every `PartContent` except `Splats` (pure fs+compute, no Bevy
/// world). Shared by [`part_gaussians`] (compose + the live path) and the off-thread reel build, so the
/// two can't drift. `Splats` is handled by the caller (it needs the loaded asset / a pre-extract).
pub(crate) fn sample_non_splat(
    content: &PartContent,
    root: &std::path::Path,
    disk: Option<f32>,
    aniso: Option<f32>,
    count: Option<usize>,
    font: Option<&str>,
) -> Vec<Gaussian3d> {
    match content {
        PartContent::Splats(_) => Vec::new(), // caller resolves splats (asset / pre-extract)
        PartContent::Text(s) => build_text_gaussians(s, TEXT_RGB, 3.0, 2, 0.012, font),
        PartContent::Image(name) => match std::fs::read(root.join(name)) {
            Ok(bytes) => build_image_gaussians(&bytes, 3.0, 0.5, 0.85),
            Err(e) => {
                warn!("image {name}: {e}");
                Vec::new()
            }
        },
        PartContent::Svg(name) => match std::fs::read(root.join(name)) {
            Ok(bytes) => build_svg_gaussians(&bytes, 3.0, 0.5, 0.85),
            Err(e) => {
                warn!("svg {name}: {e}");
                Vec::new()
            }
        },
        PartContent::Mesh(name) => {
            // Mesh → gaussians with fixed, well-tuned defaults. The one per-part knob is `disk:<f>` —
            // the splat-disk OVERLAP factor on the mean sample spacing (~1.2 = disks just overlap;
            // density-adaptive, right for any mesh size/poly count). Smaller = crisper edges on a sharp
            // graphic (the logo) without blurring another part's texture detail. thin 0.3 (flatness),
            // alpha 0.6 (blends front-to-back so grazing-edge "hairs" melt), colour from the mesh's own
            // material (glTF baseColorFactor / vertex colours).
            let splat = disk.unwrap_or(1.2);
            // `aniso:<f>` (>1) stretches each sample into an ellipsoid along the surface grain
            // (area-preserving) for a streaky/painterly read; 1.0 = round disks (the default).
            // Sample at the FINAL per-object count (not a fixed 60k) so the disk size — which is
            // computed from this count — matches the cloud's eventual density. Sampling high then
            // downsampling in compose left the disks too small for the thinned spacing → holes.
            mesh::build_mesh_gaussians(
                &root.join(name),
                count.unwrap_or(60_000),
                splat,
                0.3,
                0.6,
                aniso.unwrap_or(1.0),
                None,
            )
        }
        // A real glTF mesh isn't sampled to gaussians — build_composition spawns it as PBR geometry.
        PartContent::Model(_) => Vec::new(),
        // A glTF dissolve part: a transparent placeholder now; sample_gl_mesh fills it from the
        // loaded mesh once it's ready (invisible until then, and the rendered mesh covers it).
        PartContent::GlMesh(_) => mesh::transparent_placeholder(256),
        // A shader interlude: no splats — transparent placeholder so the morph chain stays valid
        // (the splats simply clear), while scene::shader_part plays the fullscreen effect over it.
        PartContent::Shader(_) => mesh::transparent_placeholder(256),
    }
}

/// Sample a placement's gaussians, applying the text-effect specials that need a *different* builder:
/// `~outline` traces filled-letter outlines and `~pen-write` builds single-stroke handwriting (both
/// drive the per-particle reveal shader). Everything else falls through to [`part_gaussians`]. Shared
/// by the reel (`build_sequence`) and the stage (`compose`) so the text-effect handling can't drift.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sample_content(
    content: &PartContent,
    entrance: Option<crate::scene::effects::Entrance>,
    state: &SeqState,
    assets: &Assets<PlanarGaussian3d>,
    root: &std::path::Path,
    disk: Option<f32>,
    aniso: Option<f32>,
    count: Option<usize>,
    font: Option<&str>,
) -> Vec<Gaussian3d> {
    text_effect_sample(content, entrance, font)
        .unwrap_or_else(|| part_gaussians(content, state, assets, root, disk, aniso, count, font))
}

/// Asset-free twin of [`sample_content`] for the off-thread reel build: `Splats` parts are pre-extracted
/// on the main thread (`presample`); everything else samples from fs+compute via [`sample_non_splat`].
/// Same text-effect head as `sample_content` (shared `text_effect_sample`) so the two can't drift.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sample_content_owned(
    content: &PartContent,
    entrance: Option<crate::scene::effects::Entrance>,
    presample: Option<Vec<Gaussian3d>>,
    root: &std::path::Path,
    disk: Option<f32>,
    aniso: Option<f32>,
    count: Option<usize>,
    font: Option<&str>,
) -> Vec<Gaussian3d> {
    text_effect_sample(content, entrance, font).unwrap_or_else(|| match content {
        PartContent::Splats(_) => presample.unwrap_or_default(),
        other => sample_non_splat(other, root, disk, aniso, count, font),
    })
}

/// The two text entrances that need a *different* builder than the filled-coverage default: `~outline`
/// traces filled-letter outlines and `~pen-write` builds single-stroke handwriting (both drive the
/// per-particle reveal shader). Returns `Some` only for those; `None` means "fall through to the normal
/// content sampler". Shared by [`sample_content`] + [`sample_content_owned`] so the head can't drift.
fn text_effect_sample(
    content: &PartContent,
    entrance: Option<crate::scene::effects::Entrance>,
    font: Option<&str>,
) -> Option<Vec<Gaussian3d>> {
    use crate::scene::effects::Entrance;
    use crate::text::{build_text_outline_gaussians, build_text_penwrite_gaussians};
    match (content, entrance) {
        (PartContent::Text(s), Some(Entrance::Outline)) => Some(build_text_outline_gaussians(
            s, TEXT_RGB, 3.0, 0.7, 0.012, font,
        )),
        (PartContent::Text(s), Some(Entrance::PenWrite)) => {
            // `~pen-write` traces the single-line STROKE_FONT (a `font:` choice doesn't apply — a normal
            // font has no centerline strokes), so font is intentionally ignored here.
            let pw_step = crate::envvar::or("MARTIN_PW_STEP", 0.5_f32);
            let pw_splat = crate::envvar::or("MARTIN_PW_SPLAT", 0.006_f32);
            Some(build_text_penwrite_gaussians(
                s, TEXT_RGB, 3.0, pw_step, pw_splat,
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_source_recognises_each_head() {
        assert!(matches!(parse_source("text:HELLO"), Some(PartContent::Text(s)) if s == "HELLO"));
        assert!(
            matches!(parse_source("svg:logo.svg"), Some(PartContent::Svg(s)) if s == "logo.svg")
        );
        assert!(matches!(
            parse_source("image:l.png"),
            Some(PartContent::Image(_))
        ));
        assert!(matches!(
            parse_source("mesh:m.dae"),
            Some(PartContent::Mesh(_))
        ));
        assert!(matches!(
            parse_source("glb:m.glb"),
            Some(PartContent::GlMesh(_))
        ));
        assert!(matches!(
            parse_source("gltf:m.gltf"),
            Some(PartContent::GlMesh(_))
        )); // alias
        assert!(matches!(
            parse_source("model:m.glb"),
            Some(PartContent::Model(_))
        ));
        assert!(matches!(parse_source("shader:warp"), Some(PartContent::Shader(s)) if s == "warp"));
        assert!(
            matches!(parse_source("shader:warp.wgsl"), Some(PartContent::Shader(s)) if s == "warp")
        );
        assert!(parse_source("bogus:x").is_none());
        assert!(parse_source("nonsense").is_none());
    }

    #[test]
    fn wall_splits_on_pipe() {
        let c = parse_source("wall:A|B|C").unwrap();
        assert!(matches!(c, PartContent::Text(s) if s == "A\nB\nC"));
    }

    #[test]
    fn splat_side_by_side_centres_and_strips_path() {
        // one splat → at the origin; a path is reduced to its file name.
        let one = side_by_side(["dir/dog.ply"].into_iter());
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].0, "dog.ply");
        assert_eq!(one[0].1, Vec3::ZERO);
        // several → symmetric about x=0.
        let many = side_by_side(["a.ply", "b.ply", "c.ply"].into_iter());
        assert_eq!(many.len(), 3);
        assert!(many[0].1.x < 0.0 && many[2].1.x > 0.0);
        assert!((many[1].1.x).abs() < 1e-6);
        assert!((many[0].1.x + many[2].1.x).abs() < 1e-6); // mirror
    }

    #[test]
    fn labels_are_descriptive() {
        assert_eq!(PartContent::Text("HI".into()).label(), "text \"HI\"");
        assert_eq!(PartContent::Svg("x.svg".into()).label(), "svg x.svg");
        assert_eq!(PartContent::Shader("warp".into()).label(), "shader warp");
    }
}
