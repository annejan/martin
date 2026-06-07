//! The one morph-timeline engine: a show is a list of `Part`s that each assemble in from a
//! source cloud (ball/fade/explode/… or a per-particle shader transition) and then hold,
//! morphing into the next. Drives a single `GaussianInterpolate` entity retargeted per part.

use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::gltf::GltfAssetLabel;
use bevy::prelude::*;
use bevy_gaussian_splatting::morph::interpolate::GaussianInterpolate;
use bevy_gaussian_splatting::sort::SortMode;
use bevy_gaussian_splatting::{
    CloudSettings, Gaussian3d, PlanarGaussian3d, PlanarGaussian3dHandle,
};

use crate::camera::{OrbitCam, DEFAULT_PITCH, FRONT_YAW};
use crate::morph::{ball_of, drop_of, explode_of, fade_of, implode_of, resample_morton, swirl_of};
use crate::scene::content::{parse_source, part_gaussians, side_by_side, PartContent};
use crate::scene::{cloud_base_rotation, file_name_of, parent_dir, AssetRoot, NORMALIZE_EXTENT};
use crate::score;
use crate::text::{build_text_outline_gaussians, build_text_penwrite_gaussians, TEXT_RGB};

const BALL_SHELL: f32 = 0.9; // intro ball-shell radius, in units of the framed radius
const FLASH_LEN: f32 = 0.18; // cut-flash decay time (s), MARTIN_FLASH strength
const DEFORM_SPEED: f32 = 2.0; // deform animation rate: deform_time = clock.t * this
const MODEL_FADE: f32 = 0.6; // dissolve-model fade-in time (s) as its part finishes assembling

/// How a part *arrives*. `Morph` (the default after part 0) flows from the previous part's
/// shape, Morton-paired, with the optional ball-pulse `bulge`. The next group build a source
/// cloud from the part's own shape and morph in from that — the ball is just one of them. The
/// last group are *per-particle* transitions driven by the vendored shader (`transition_mode`
/// uniform): the source is an identity copy and the shader staggers opacity/position per
/// particle (see `SHADER-BLUEPRINT.md`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transition {
    Morph,   // prev shape → this shape (with bulge ball-pulse); the original behaviour
    Swarm,   // like Morph but particles flock/swarm along curled paths between the two scenes
    Ball,    // assemble out of a fuzzy ball shell (default for part 0)
    Fade,    // fade up on the spot (opacity 0 → in)
    Explode, // gather in from an outward burst
    Implode, // expand out from a dense point
    Drop,    // fall straight down into place
    Swirl,   // sweep/spiral in around the vertical axis
    // --- per-particle (shader transition_mode) ---
    Typewriter, // reveal left→right as a moving edge (great for text)
    Wipe,       // hard slab reveal across the x axis
    Sparkle,    // random per-particle twinkle-in (HDR bloom flashes)
    Slither,    // staggered lateral sine that settles
    Vortex,     // continuous unwind-rotation about the vertical axis
    Outline, // text traced in outline/pen order — a glowing neon draw-on (filled font); text only
    PenWrite, // text written in pen order on a single-stroke font — true handwriting; text only
}

impl Transition {
    fn parse(s: &str) -> Option<Transition> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "morph" => Transition::Morph,
            "swarm" => Transition::Swarm,
            "ball" => Transition::Ball,
            "fade" => Transition::Fade,
            "explode" => Transition::Explode,
            "implode" => Transition::Implode,
            "drop" => Transition::Drop,
            "swirl" => Transition::Swirl,
            "typewriter" | "type" => Transition::Typewriter,
            "wipe" => Transition::Wipe,
            "sparkle" => Transition::Sparkle,
            "slither" => Transition::Slither,
            "vortex" => Transition::Vortex,
            "outline" => Transition::Outline,
            "pen" | "penwrite" | "pen-write" | "write" => Transition::PenWrite,
            _ => return None,
        })
    }

    /// Per-particle shader transitions use an identity source cloud (same as the target);
    /// the vendored shader staggers opacity/position. Returns the `(mode, softness, axis)`
    /// uniform triple, or `None` for the data-only / Morph transitions.
    fn shader_uniforms(self) -> Option<(u32, f32, u32)> {
        match self {
            Transition::Typewriter => Some((1, 0.10, 0)),
            Transition::Slither => Some((2, 0.30, 0)),
            Transition::Sparkle => Some((3, 0.40, 0)),
            Transition::Vortex => Some((5, 0.35, 1)),
            Transition::Wipe => Some((6, 0.02, 0)),
            Transition::Outline => Some((7, 0.06, 0)), // filled font → traces outlines
            Transition::PenWrite => Some((7, 0.05, 0)), // single-stroke font → handwriting
            _ => None,
        }
    }
}

/// A *persistent* vertex deform (`^name` token / `MARTIN_DEFORM`). Unlike a `Transition` (which
/// plays once on arrival), this keeps running while the part is **held** — so a `wall:` of text
/// can ripple, billow or curl the whole time it's on screen. Drives the vendored shader's deform
/// uniforms (see SHADER-BLUEPRINT.md); default-off, so an unset part renders plain.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Deform {
    Wave,   // flag-like ripple travelling across x
    Cloth,  // 2D billow (x and y out of phase)
    Ripple, // concentric radial waves from the centre
    Twist,  // banner curl/uncurl
}

impl Deform {
    fn parse(s: &str) -> Option<Deform> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "wave" | "flag" => Deform::Wave,
            "cloth" | "billow" => Deform::Cloth,
            "ripple" => Deform::Ripple,
            "twist" | "curl" => Deform::Twist,
            _ => return None,
        })
    }

    /// The `(mode, amp, freq)` uniform triple for the vendored shader deform.
    fn uniforms(self) -> (u32, f32, f32) {
        match self {
            Deform::Wave => (1, 0.15, 4.0),
            Deform::Cloth => (2, 0.12, 3.5),
            Deform::Ripple => (3, 0.18, 6.0),
            Deform::Twist => (4, 0.5, 2.0), // amp is radians
        }
    }
}

/// One part morphs in from the previous (or, for part 0, from a ball), then holds.
#[derive(Clone)]
pub(crate) struct Part {
    pub content: PartContent,
    pub hold: f32,                      // seconds held after arriving
    pub morph: f32,                     // seconds to morph in
    pub bulge: f32,                     // ball-pulse explosiveness (Morph transition only)
    pub transition: Option<Transition>, // None = default (Ball for part 0, else Morph)
    pub anchor: Option<f32>,            // absolute start (s) on the music clock; None = relative
    pub deform: Option<Deform>, // persistent deform while held (None = none / MARTIN_DEFORM)
}

/// The whole show: a list of parts + the gaussian budget every part is resampled to.
#[derive(Resource)]
pub(crate) struct Sequence {
    pub parts: Vec<Part>,
    pub count: usize,
}

/// MARTIN_FLASH=<strength>: over-bright bloom pulse on each part cut (0 = off, the default).
#[derive(Resource)]
pub(crate) struct FlashStrength(pub f32);

/// Loaded splat handles + the per-part built shapes (all `count` gaussians) + each part's
/// morph-in source cloud + its resolved transition.
#[derive(Resource)]
pub(crate) struct SeqState {
    pub load_names: Vec<String>,
    pub loads: Vec<Handle<PlanarGaussian3d>>,
    pub shapes: Vec<Handle<PlanarGaussian3d>>,
    pub sources: Vec<Option<Handle<PlanarGaussian3d>>>, // per-part lhs source (None = morph from prev)
    pub transitions: Vec<Transition>,                   // resolved transition per part
    pub deforms: Vec<Option<Deform>>,                   // resolved persistent deform per part
    pub starts: Vec<f32>,                               // absolute start time (s) of each part
    pub built: bool,
    pub entity: Option<Entity>,
}

/// Load the capture-camera world positions from a 3DGS/COLMAP `cameras.json` (graphdeco format:
/// an array of objects each with a `"position": [x,y,z]`). These are in the same coordinates as
/// the scene's `.ply`, so applying the scene's normalize + cloud rotation places martin's camera
/// where the scene was actually shot — the only viewpoint a 360° capture renders coherently.
fn load_camera_positions(path: &str) -> Vec<Vec3> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    json.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|c| {
                    let p = c.get("position")?.as_array()?;
                    Some(Vec3::new(
                        p.first()?.as_f64()? as f32,
                        p.get(1)?.as_f64()? as f32,
                        p.get(2)?.as_f64()? as f32,
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse `MARTIN_SEQ`: a file path OR an inline string. Parts are `;`/newline-separated.
/// Each part: `text:STRING` or `splat:a.ply` (or `a.ply+b.ply` for side-by-side), optional
/// trailing `@hold,morph,bulge`. `#` comments and blank lines are skipped.
fn parse_seq(spec: &str, score: &score::Score) -> Vec<Part> {
    let raw = std::fs::read_to_string(spec).unwrap_or_else(|_| spec.to_string());
    let mut parts = Vec::new();
    for line in raw.split([';', '\n']) {
        let s = line.trim();
        if s.is_empty() || s.starts_with('#') {
            continue;
        }
        // Pull the `~name` transition AND the `@@anchor` token (both single whitespace-delimited
        // tokens, position-independent); keep the rest of the line for the head + `@timing`.
        let mut transition = None;
        let mut anchor = None;
        let mut deform = None;
        let s: String = s
            .split_whitespace()
            .filter(|tok| {
                if let Some(a) = tok.strip_prefix("@@").and_then(|a| score.anchor_seconds(a)) {
                    anchor = Some(a);
                    return false;
                }
                if let Some(d) = tok.strip_prefix('^').and_then(Deform::parse) {
                    deform = Some(d);
                    return false;
                }
                if let Some(tr) = tok.strip_prefix('~').and_then(Transition::parse) {
                    transition = Some(tr);
                    return false;
                }
                true
            })
            .collect::<Vec<_>>()
            .join(" ");
        let (head, timing) = match s.split_once('@') {
            Some((h, t)) => (h.trim(), Some(t.trim())),
            None => (s.as_str(), None),
        };
        let (mut hold, mut morph, mut bulge) = (1.5_f32, 3.0_f32, 0.9_f32);
        if let Some(t) = timing {
            let nums: Vec<f32> = t.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if let Some(v) = nums.first() {
                hold = *v;
            }
            if let Some(v) = nums.get(1) {
                morph = *v;
            }
            if let Some(v) = nums.get(2) {
                bulge = *v;
            }
        }
        let Some(content) = parse_source(head) else {
            continue;
        };
        parts.push(Part {
            content,
            hold,
            morph,
            bulge,
            transition,
            anchor,
            deform,
        });
    }
    parts
}

/// Build the show: `MARTIN_SEQ` if set, else a shorthand from `MARTIN_TEXT` /
/// `MARTIN_PLY(+_PLY2)(+_REFORM)`. Returns the sequence + the asset root (the .ply folder).
pub(crate) fn sequence_from_env(score: &score::Score) -> (Sequence, Option<String>) {
    let count_default = if std::env::var("MARTIN_SEQ").is_ok() {
        200_000
    } else {
        0
    };
    let count = std::env::var("MARTIN_MORPH_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(count_default);

    if let Ok(spec) = std::env::var("MARTIN_SEQ") {
        // asset root = the .ply folder (so `splat:` filenames resolve); MARTIN_PLY sets it.
        let root = std::env::var("MARTIN_PLY").ok().and_then(parent_dir);
        return (
            Sequence {
                parts: parse_seq(&spec, score),
                count,
            },
            root,
        );
    }

    if let Ok(text) = std::env::var("MARTIN_TEXT") {
        let part = Part {
            content: PartContent::Text(text),
            hold: 2.0,
            morph: 3.0,
            bulge: 0.0,
            transition: None,
            anchor: None,
            deform: None,
        };
        return (
            Sequence {
                parts: vec![part],
                count,
            },
            None,
        );
    }

    // splat shorthand: PLY (+ PLY2) as part 0; REFORM (if any) as part 1.
    let primary = std::env::var("MARTIN_PLY").ok();
    let root = primary.as_deref().and_then(|p| parent_dir(p.to_string()));
    let name1 = primary
        .as_deref()
        .map(file_name_of)
        .unwrap_or_else(|| "aegg.ply".into());
    let mut names = vec![name1];
    if let Ok(p2) = std::env::var("MARTIN_PLY2") {
        names.push(file_name_of(&p2));
    }
    let bulge = std::env::var("MARTIN_BULGE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.9);
    let mut parts = vec![Part {
        content: PartContent::Splats(side_by_side(names.iter().map(String::as_str))),
        hold: 2.0,
        morph: 3.0,
        bulge: 0.0,
        transition: None,
        anchor: None,
        deform: None,
    }];
    if let Ok(reform) = std::env::var("MARTIN_REFORM") {
        parts.push(Part {
            content: PartContent::Splats(vec![(file_name_of(&reform), Vec3::ZERO)]),
            hold: 2.0,
            morph: 3.5,
            bulge,
            transition: None,
            anchor: None,
            deform: None,
        });
    }
    (Sequence { parts, count }, root)
}

/// Once every referenced splat has loaded, build each part's shape (resampled to the fixed
/// count) + the intro ball, spawn the single interpolate entity, and frame the union once.
pub(crate) fn build_sequence(
    mut commands: Commands,
    mut assets: ResMut<Assets<PlanarGaussian3d>>,
    seq: Option<Res<Sequence>>,
    state: Option<ResMut<SeqState>>,
    root: Res<AssetRoot>,
    asset_server: Res<AssetServer>,
    mut cam: Query<&mut OrbitCam>,
) {
    let (Some(seq), Some(mut state)) = (seq, state) else {
        return;
    };
    if state.built || seq.parts.is_empty() {
        return;
    }
    if state.loads.iter().any(|h| assets.get(h).is_none()) {
        return; // wait for every referenced splat
    }

    // resolve each part's transition first (explicit ~name > MARTIN_TRANSITION > Ball for part
    // 0 / Morph after) — needed before building gaussians so a PenWrite text part is built as a
    // stroked outline (pen order baked into visibility) instead of filled coverage.
    let global_tr = std::env::var("MARTIN_TRANSITION")
        .ok()
        .and_then(|s| Transition::parse(&s));
    // persistent per-part deform: explicit `^name` > MARTIN_DEFORM > none. Runs while the part is
    // held (a waving wall of text etc.), independent of the arrival transition above.
    let global_deform = std::env::var("MARTIN_DEFORM")
        .ok()
        .and_then(|s| Deform::parse(&s));
    let deforms: Vec<Option<Deform>> = seq
        .parts
        .iter()
        .map(|p| p.deform.or(global_deform))
        .collect();
    let transitions: Vec<Transition> = seq
        .parts
        .iter()
        .enumerate()
        .map(|(idx, part)| {
            let tr = part.transition.or(global_tr).unwrap_or(if idx == 0 {
                Transition::Ball
            } else {
                Transition::Morph
            });
            // part 0 has nothing to morph from — fall back to a ball assemble.
            if idx == 0 && matches!(tr, Transition::Morph | Transition::Swarm) {
                Transition::Ball
            } else {
                tr
            }
        })
        .collect();

    // Absolute start time (s) of each part: its `@@anchor` (locked to the music clock), else
    // laid end-to-end after the previous part (start + morph + hold). This is the cue timeline.
    let mut starts: Vec<f32> = Vec::with_capacity(seq.parts.len());
    let mut cursor = 0.0_f32;
    for (i, part) in seq.parts.iter().enumerate() {
        let start = part.anchor.unwrap_or(if i == 0 { 0.0 } else { cursor });
        starts.push(start);
        cursor = start + part.morph + part.hold;
    }

    // read every part's gaussians once, so count==0 can mean "size N to the largest part"
    // (every part is then resampled to that single N — required by the shared morph output).
    // pen-write strokes are thin: MARTIN_PW_SPLAT (gaussian size) / MARTIN_PW_STEP (sample
    // spacing) tune stroke weight — a fat splat blooms the strokes into filled blobs.
    let pw_step = std::env::var("MARTIN_PW_STEP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.5_f32);
    let pw_splat = std::env::var("MARTIN_PW_SPLAT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.006_f32);
    let mut raws: Vec<Vec<Gaussian3d>> = seq
        .parts
        .iter()
        .zip(&transitions)
        .map(|(part, &tr)| match (&part.content, tr) {
            (PartContent::Text(s), Transition::Outline) => {
                build_text_outline_gaussians(s, TEXT_RGB, 3.0, 0.7, 0.012)
            }
            (PartContent::Text(s), Transition::PenWrite) => {
                build_text_penwrite_gaussians(s, TEXT_RGB, 3.0, pw_step, pw_splat)
            }
            _ => part_gaussians(&part.content, &state, &assets, &root.0),
        })
        .collect();
    // Normalize each part to a common "normal" size (MARTIN_NORMALIZE=0 to disable). Sources
    // vary wildly — a COLMAP scene spans hundreds of units, a TRELLIS object ~1 — so without
    // this they'd frame inconsistently and morph badly. We log the raw extent first.
    let normalize = std::env::var("MARTIN_NORMALIZE")
        .map(|v| v != "0")
        .unwrap_or(true);
    let mut scene_norm = (Vec3::ZERO, 1.0); // part 0's (center, scale) — to transform camera poses
    for (i, (raw, part)) in raws.iter_mut().zip(&seq.parts).enumerate() {
        let label = match &part.content {
            PartContent::Text(s) => format!("text \"{s}\""),
            PartContent::Image(name) => format!("image {name}"),
            PartContent::Mesh(name) => format!("mesh {name}"),
            PartContent::Model(name) => format!("model {name}"),
            PartContent::Splats(list) => list
                .iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>()
                .join("+"),
        };
        info!(
            "part {label}: raw extent {:.2} units ({} gaussians)",
            crate::morph::extent_of(raw),
            raw.len()
        );
        if normalize {
            let norm = crate::morph::normalize_to(raw, NORMALIZE_EXTENT);
            if i == 0 {
                scene_norm = norm;
            }
        }
    }
    let n = if seq.count > 0 {
        seq.count
    } else {
        raws.iter().map(Vec::len).max().unwrap_or(0).max(1)
    };

    let mut union_lo = Vec3::splat(f32::MAX);
    let mut union_hi = Vec3::splat(f32::MIN);
    for g in raws.iter().flatten() {
        let p = Vec3::from_array(g.position_visibility.position);
        union_lo = union_lo.min(p);
        union_hi = union_hi.max(p);
    }

    // Framing radius of the *content*: when normalized, every part is ~NORMALIZE_EXTENT across
    // centred on its centroid, so frame from that — robust to the floaters that still inflate
    // the raw union AABB and would otherwise shrink the scene to a distant dot. Raw mode (no
    // normalize) frames the union box instead. This radius also sizes each transition source.
    let (frame_center, content_radius, frame_factor) = if normalize {
        (Vec3::ZERO, NORMALIZE_EXTENT * 0.5, 2.5)
    } else {
        let c = (union_lo + union_hi) * 0.5;
        (c, ((union_hi - union_lo) * 0.5).length().max(0.1), 1.7)
    };

    // Each part is resampled to the shared count N, then gets the *source* cloud it morphs in
    // FROM, chosen by its transition (`~name` per part > MARTIN_TRANSITION default > Ball for
    // part 0 / Morph after). `Morph` has no source — it flows from the previous part's shape
    // (with the ball-pulse bulge); the others build a source from the part's own shape.
    let mut shapes = Vec::new();
    let mut sources: Vec<Option<Handle<PlanarGaussian3d>>> = Vec::new();
    for (idx, raw) in raws.into_iter().enumerate() {
        let shaped = resample_morton(raw, n);
        let tr = transitions[idx];
        let r = content_radius;
        let src: Option<Vec<Gaussian3d>> = match tr {
            Transition::Morph | Transition::Swarm => None, // both flow from the previous shape
            Transition::Ball => Some(ball_of(&shaped, r * BALL_SHELL)),
            Transition::Fade => Some(fade_of(&shaped)),
            Transition::Explode => Some(explode_of(&shaped, r * 1.6)),
            Transition::Implode => Some(implode_of(&shaped)),
            Transition::Drop => Some(drop_of(&shaped, r * 2.5)),
            Transition::Swirl => Some(swirl_of(&shaped, 2.4, 1.5)),
            // Per-particle (shader) transitions: identity source — positions/opacity match the
            // target and the vendored shader staggers them per particle over the morph.
            _ if tr.shader_uniforms().is_some() => Some(shaped.clone()),
            _ => None,
        };
        sources.push(src.map(|s| assets.add(PlanarGaussian3d::from(s))));
        shapes.push(assets.add(PlanarGaussian3d::from(shaped)));
    }
    let intro0 = sources[0]
        .clone()
        .expect("part 0 always builds a source cloud");

    // MARTIN_ROT="rx,ry,rz" (euler degrees) orients the cloud — e.g. to stand a COLMAP scene
    // upright for a "normal" POV. Default = cloud_base_rotation (flip-X, right for portrait
    // splats; gives scenes their abstract sideways look).
    let entity_rot = std::env::var("MARTIN_ROT")
        .ok()
        .and_then(|s| {
            let n: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            (n.len() == 3).then(|| {
                Quat::from_euler(
                    EulerRot::XYZ,
                    n[0].to_radians(),
                    n[1].to_radians(),
                    n[2].to_radians(),
                )
            })
        })
        .unwrap_or_else(cloud_base_rotation);

    let entity = commands
        .spawn((
            GaussianInterpolate::<Gaussian3d> {
                lhs: PlanarGaussian3dHandle(intro0.clone()),
                rhs: PlanarGaussian3dHandle(shapes[0].clone()),
            },
            CloudSettings {
                sort_mode: SortMode::Radix,
                time: 0.0, // start as the ball; part_director morphs it in
                time_start: 0.0,
                time_stop: 1.0,
                bulge: 0.0,
                ..default()
            },
            Transform::from_rotation(entity_rot),
        ))
        .id();

    // frame the union once (camera never pops between parts); apply the same rotation to the
    // centre so the camera looks at the post-transform world centre.
    // Seed the free-orbit camera. MARTIN_ZOOM scales distance (>1 = closer); MARTIN_YAW /
    // MARTIN_PITCH (radians) seed the orbit angle so you can bake a found viewpoint into a
    // render (and freely orbit live from there).
    let env_f = |k: &str| std::env::var(k).ok().and_then(|s| s.parse::<f32>().ok());
    let zoom = env_f("MARTIN_ZOOM").filter(|z| *z > 0.0).unwrap_or(1.0);
    let center = entity_rot * frame_center;
    let (mut yaw, mut pitch, mut dist) = (
        env_f("MARTIN_YAW").unwrap_or(FRONT_YAW),
        env_f("MARTIN_PITCH").unwrap_or(DEFAULT_PITCH),
        content_radius * frame_factor / zoom,
    );
    // MARTIN_CAMERAS=<cameras.json>: park the camera at a real capture pose (the only viewpoint
    // a raw 360° scene renders coherently). Transform the chosen capture position through the
    // SAME normalize (part 0) + cloud rotation as the gaussians, then read off yaw/pitch/dist
    // around the framed centre. MARTIN_CAM_INDEX picks which shot (default 0).
    if let Ok(cpath) = std::env::var("MARTIN_CAMERAS") {
        let positions = load_camera_positions(&cpath);
        if positions.is_empty() {
            warn!("MARTIN_CAMERAS: no camera positions in {cpath}");
        } else {
            let idx = std::env::var("MARTIN_CAM_INDEX")
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0)
                .min(positions.len() - 1);
            let (c0, s0) = scene_norm;
            let dir = entity_rot * ((positions[idx] - c0) * s0) - center;
            let len = dir.length().max(1e-4);
            yaw = dir.z.atan2(dir.x);
            pitch = (dir.y / len).asin();
            dist = len / zoom;
            info!(
                "camera: capture pose {idx}/{} from {cpath}",
                positions.len()
            );
        }
    }
    for mut c in &mut cam {
        c.target = center;
        c.dist = dist;
        c.yaw = yaw;
        c.pitch = pitch;
        c.framed = true;
    }

    // MARTIN_MODEL: overlay a real mesh on one part that dissolves into its generated splats.
    spawn_dissolve_model(&mut commands, &asset_server, seq.parts.len());

    state.shapes = shapes;
    state.sources = sources;
    state.transitions = transitions;
    state.deforms = deforms;
    state.starts = starts;
    state.entity = Some(entity);
    state.built = true;
    info!(
        "sequence built: {} parts × {n} gaussians",
        state.shapes.len()
    );
}

/// Index of the active part at time `t`: the last part whose absolute start (from the cue
/// timeline — `@@anchor` or laid end-to-end) has arrived. Shared by `part_director` and `flypath`.
pub(crate) fn active_part(starts: &[f32], t: f32) -> usize {
    let mut idx = 0;
    for (i, &start) in starts.iter().enumerate() {
        if t >= start {
            idx = i;
        } else {
            break;
        }
    }
    idx
}

/// End of the cue timeline: the latest part's `start + morph + hold` (anchors can push it past a
/// simple sum). The recorder uses this (+ a tail) for the clip length; `flypath` spreads the
/// camera path across it while recording.
pub(crate) fn show_end(parts: &[Part], starts: &[f32]) -> f32 {
    parts
        .iter()
        .zip(starts)
        .map(|(p, &start)| start + p.morph + p.hold)
        .fold(0.0_f32, f32::max)
}

/// Drive the show from `SeqClock.t`: find the active part, retarget the interpolate entity's
/// lhs/rhs (only on change), and set the blend factor + ball bulge. Part 0 morphs in from the
/// intro ball; every later part morphs in from the previous part's shape.
#[allow(clippy::type_complexity)]
pub(crate) fn part_director(
    seq: Option<Res<Sequence>>,
    state: Option<Res<SeqState>>,
    clock: Res<crate::scene::SeqClock>,
    flash: Res<FlashStrength>,
    beat: Res<crate::scene::beat::Beat>,
    // (amp_scale, speed) for the persistent deform — read from env once. MARTIN_DEFORM_AMP scales the
    // wobble strength (e.g. 0.3 = gentle on a whole scene), MARTIN_DEFORM_SPEED its rate.
    mut deform_tune: Local<Option<(f32, f32)>>,
    mut q: Query<(
        &mut GaussianInterpolate<Gaussian3d>,
        &mut CloudSettings,
        &mut Transform,
    )>,
) {
    let (Some(seq), Some(state)) = (seq, state) else {
        return;
    };
    if !state.built {
        return;
    }
    let Some(entity) = state.entity else { return };
    let Ok((mut interp, mut cs, mut tf)) = q.get_mut(entity) else {
        return;
    };
    let parts = &seq.parts;

    // The active part is the last one whose absolute start time has arrived (starts come from
    // the cue timeline — `@@anchor` or laid end-to-end). It morphs in over `morph`, then holds
    // until the next part starts. Before part 0's start, `factor` clamps to 0 (its source state).
    let t = clock.t;
    let starts = &state.starts;
    let idx = active_part(starts, t);
    let dt = t - starts[idx];
    let morphing = dt < parts[idx].morph;
    let factor = (dt / parts[idx].morph.max(1e-3)).clamp(0.0, 1.0);

    // lhs: the part's transition source cloud (ball/fade/explode/…), or — for a plain Morph —
    // the previous part's shape.
    let want_lhs = match &state.sources[idx] {
        Some(h) => h,
        None => &state.shapes[idx - 1],
    };
    if interp.lhs.0.id() != want_lhs.id() {
        interp.lhs = PlanarGaussian3dHandle(want_lhs.clone());
    }
    if interp.rhs.0.id() != state.shapes[idx].id() {
        interp.rhs = PlanarGaussian3dHandle(state.shapes[idx].clone());
    }
    let eased = factor * factor * (3.0 - 2.0 * factor);
    cs.time = eased;
    // the ball-pulse shader effect belongs to the plain Morph transition (prev → next through a
    // ball); source-based transitions carry their own motion, so they don't pulse.
    cs.bulge = if morphing && state.transitions[idx] == Transition::Morph {
        parts[idx].bulge
    } else {
        0.0
    };
    // ~swarm: flock the particles along curled paths during the morph (the @_,_,N timing value is
    // the swarm strength); mutually exclusive with the ball-pulse above.
    cs.swarm = if morphing && state.transitions[idx] == Transition::Swarm {
        parts[idx].bulge
    } else {
        0.0
    };
    // per-particle shader transitions (typewriter/sparkle/…): drive the vendored uniforms only
    // while morphing in; otherwise mode 0 = off (held shape renders plain, fully sort-safe).
    let (mode, soft, axis) = morphing
        .then(|| state.transitions[idx].shader_uniforms())
        .flatten()
        .unwrap_or((0, 0.0, 0));
    cs.transition_mode = mode;
    cs.transition_softness = soft;
    cs.transition_axis = axis;
    // Persistent deform (wave/cloth/ripple/twist): unlike the transition this runs the *whole*
    // time the part is up (not just while morphing), animated by the show clock. Mode 0 = off.
    let (amp_scale, speed) = *deform_tune.get_or_insert_with(|| {
        let f = |k: &str, d: f32| {
            std::env::var(k)
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(d)
        };
        (
            f("MARTIN_DEFORM_AMP", 1.0),
            f("MARTIN_DEFORM_SPEED", DEFORM_SPEED),
        )
    });
    let (dmode, damp, dfreq) = state.deforms[idx]
        .map(|d| d.uniforms())
        .unwrap_or((0, 0.0, 0.0));
    cs.deform_mode = dmode;
    cs.deform_amp = damp * amp_scale;
    cs.deform_freq = dfreq;
    cs.deform_time = t * speed;
    // Flash on each cut (term-demo's Director trick): a brief over-bright pulse at every part
    // start → the HDR bloom flares. MARTIN_FLASH=<strength> (0 = off, default); reuses
    // global_opacity, so off keeps every frame byte-identical.
    // MARTIN_FLASH defaults to 0 — skip the per-frame max-over-starts loop entirely in that case.
    let flash = if flash.0 <= 0.0 {
        0.0
    } else {
        flash.0
            * starts
                .iter()
                .map(|&s| {
                    let d = t - s;
                    if (0.0..FLASH_LEN).contains(&d) {
                        let a = 1.0 - d / FLASH_LEN;
                        a * a
                    } else {
                        0.0
                    }
                })
                .fold(0.0_f32, f32::max)
    };
    cs.global_opacity = 1.0 + flash;

    // Beat reactions (MARTIN_BEAT scale): the score's drum hits drive the look. A held part can't
    // use `bulge` (it's a mid-morph ball-pulse, zero at time==1), so the kick thump rides on the
    // cloud's scale; the snare flares the bloom; kick+snare swell any active deform so a ^wave /
    // ^ripple part pumps with the track. During a morph we add a little bulge punch too.
    let k = beat.intensity;
    if k > 0.0 {
        tf.scale = Vec3::splat(1.0 + beat.kick * 0.05 * k);
        cs.global_opacity += (beat.snare * 0.45 + beat.hat * 0.12) * k;
        if morphing {
            cs.bulge += beat.kick * 0.3 * k;
        }
        if cs.deform_mode != 0 {
            cs.deform_amp *= 1.0 + (beat.kick * 0.6 + beat.snare * 0.3) * k;
        }
    }
}

/// Add `NoFrustumCulling` to the sequence entity once its Aabb exists, so morph/ball
/// particles that briefly leave the framed view don't pop out.
#[allow(clippy::type_complexity)] // a Bevy query filter tuple — verbose by nature
pub(crate) fn seq_no_cull(
    mut commands: Commands,
    state: Option<Res<SeqState>>,
    q: Query<
        (),
        (
            With<GaussianInterpolate<Gaussian3d>>,
            With<Aabb>,
            Without<NoFrustumCulling>,
        ),
    >,
) {
    let Some(state) = state else { return };
    let Some(e) = state.entity else { return };
    if q.get(e).is_ok() {
        commands.entity(e).insert(NoFrustumCulling);
    }
}

/// A real PBR mesh overlaid on one sequence part, dissolving (material alpha) as that part morphs
/// out — so a solid mesh crumbles into its coincident generated splats. Carries which part it
/// shadows; `animate_seq_model` reads that part's cue times to drive the fade.
#[derive(Component)]
pub(crate) struct SeqModel {
    part: usize,
}

/// `MARTIN_MODEL=<file.glb>`: overlay a real lit mesh on sequence part `MARTIN_MODEL_PART`
/// (default 0). It fades in crisp as that part finishes assembling, holds, then DISSOLVES as the
/// part morphs out — revealing the coincident `mesh:`-sampled splats, which the morph swarms away.
/// `MARTIN_MODEL_SCALE` / `MARTIN_MODEL_ROT` (euler degrees) align it with those splats. Returns
/// whether a model was spawned (only the lights are needed if so). Splats are unlit, so we add a
/// key + fill light for the PBR mesh.
fn spawn_dissolve_model(commands: &mut Commands, assets: &AssetServer, part_count: usize) {
    let Ok(name) = std::env::var("MARTIN_MODEL") else {
        return;
    };
    let env_f = |k: &str| std::env::var(k).ok().and_then(|s| s.parse::<f32>().ok());
    let part = std::env::var("MARTIN_MODEL_PART")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0)
        .min(part_count.saturating_sub(1));
    let scale = env_f("MARTIN_MODEL_SCALE").unwrap_or(1.0);
    let rot = std::env::var("MARTIN_MODEL_ROT")
        .ok()
        .map(|s| {
            let n: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            Quat::from_euler(
                EulerRot::XYZ,
                n.first().copied().unwrap_or(0.0).to_radians(),
                n.get(1).copied().unwrap_or(0.0).to_radians(),
                n.get(2).copied().unwrap_or(0.0).to_radians(),
            )
        })
        .unwrap_or(Quat::IDENTITY);
    commands.spawn((
        SceneRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(name))),
        Transform {
            rotation: rot,
            scale: Vec3::splat(scale),
            ..default()
        },
        SeqModel { part },
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

/// Drive the dissolve-model's opacity: invisible until its part is nearly assembled, crisp (opaque)
/// while the part holds, then fading to transparent over the *next* part's morph — the moment the
/// generated splats flow away. The only PBR materials in a sequence are this overlay's, so we fade
/// every `StandardMaterial`.
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
    let (starts, parts) = (&state.starts, &seq.parts);
    let full_at = starts[m.part] + parts[m.part].morph;
    let (dissolve_start, dissolve_end) = match m.part + 1 {
        next if next < parts.len() => (starts[next], starts[next] + parts[next].morph),
        _ => (f32::MAX, f32::MAX), // last part: never dissolves, just stays
    };
    let t = clock.t;
    let vis = if t < full_at - MODEL_FADE {
        0.0
    } else if t < full_at {
        (t - (full_at - MODEL_FADE)) / MODEL_FADE
    } else if t < dissolve_start {
        1.0
    } else if t < dissolve_end {
        1.0 - (t - dissolve_start) / (dissolve_end - dissolve_start).max(1e-3)
    } else {
        0.0
    }
    .clamp(0.0, 1.0);
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
