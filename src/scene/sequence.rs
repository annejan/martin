//! The one morph-timeline engine: a show is a list of `Part`s that each assemble in from a
//! source cloud (ball/fade/explode/… or a per-particle shader transition) and then hold,
//! morphing into the next. Drives a single `GaussianInterpolate` entity retargeted per part.

use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::gltf::GltfAssetLabel;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy_gaussian_splatting::morph::interpolate::GaussianInterpolate;
use bevy_gaussian_splatting::sort::SortMode;
use bevy_gaussian_splatting::{
    CloudSettings, Gaussian3d, PlanarGaussian3d, PlanarGaussian3dHandle,
};

use crate::camera::{OrbitCam, DEFAULT_PITCH, FRONT_YAW};
use crate::morph::{
    ball_of, disperse_of, drop_of, evaporate_of, explode_of, fade_of, implode_of, rain_of,
    resample_morton, sink_of, swirl_of, wash_of,
};
use crate::scene::content::{parse_source, part_gaussians, side_by_side, PartContent};
use crate::scene::{cloud_base_rotation, file_name_of, parent_dir, AssetRoot, NORMALIZE_EXTENT};
use crate::score;
use crate::text::{build_text_outline_gaussians, build_text_penwrite_gaussians, TEXT_RGB};

const BALL_SHELL: f32 = 0.9; // intro ball-shell radius, in units of the framed radius
const FLASH_LEN: f32 = 0.18; // cut-flash decay time (s), MARTIN_FLASH strength
const DEFORM_SPEED: f32 = 2.0; // deform animation rate: deform_time = clock.t * this
const MODEL_FADE: f32 = 0.6; // splats→mesh materialize time (s), after the part's splat-assemble
const DISSOLVE_LEN: f32 = 1.2; // mesh→splats dissolve time (s) — its OWN step at the end of the hold
const DEPART_LEN: f32 = 1.5; // `out:` departure time (s) — carved from the end of a part's hold

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
    Rain,    // fall in from scattered high points (a shower), staggered
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
            "rain" => Transition::Rain,
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
    Wind,   // gusting sideways sway + spatial turbulence — flutters/streams in the wind
}

impl Deform {
    fn parse(s: &str) -> Option<Deform> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "wave" | "flag" => Deform::Wave,
            "cloth" | "billow" => Deform::Cloth,
            "ripple" => Deform::Ripple,
            "twist" | "curl" => Deform::Twist,
            "wind" | "gust" => Deform::Wind,
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
            Deform::Wind => (5, 0.15, 2.5),
        }
    }
}

/// How a part *leaves* (`out:name`). Where a `~transition` says how a part ARRIVES, this says how it
/// DEPARTS: it morphs to a faded "gone" cloud as a distinct step at the end of its hold (before the
/// next part arrives), so the object dissolves away instead of cross-morphing straight to the next.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Departure {
    Wash,      // flows off sideways and fades — washed away
    Disperse,  // scatters outward in all directions and fades — blown to dust
    Evaporate, // drifts upward and fades — rises away
    Sink,      // falls straight down and fades — drops out the bottom
}

impl Departure {
    fn parse(s: &str) -> Option<Departure> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "wash" | "washaway" | "wash-away" => Departure::Wash,
            "disperse" | "dust" | "dissolve" => Departure::Disperse,
            "evaporate" | "rise" => Departure::Evaporate,
            "sink" | "fall" => Departure::Sink,
            _ => return None,
        })
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
    pub out: Option<Departure>, // how the part LEAVES (`out:name`); None = cross-morph to the next
    pub rot: Option<Quat>,      // per-part orientation (`rot:rx,ry,rz` deg), baked into the shape
    pub cluster: Option<usize>, // `cluster:N` → N scattered, randomly-rotated copies (a "serving")
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
    pub out_clouds: Vec<Option<Handle<PlanarGaussian3d>>>, // per-part `out:` departure cloud (None = none)
    pub transitions: Vec<Transition>,                      // resolved transition per part
    pub deforms: Vec<Option<Deform>>,                      // resolved persistent deform per part
    pub starts: Vec<f32>,                                  // absolute start time (s) of each part
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
        // strip inline `# comments` (like parse_compose) so a trailing note can't eat the @timing.
        let s = line.split('#').next().unwrap_or("").trim();
        if s.is_empty() {
            continue;
        }
        // Pull the `~name` transition AND the `@@anchor` token (both single whitespace-delimited
        // tokens, position-independent); keep the rest of the line for the head + `@timing`.
        let mut transition = None;
        let mut anchor = None;
        let mut deform = None;
        let mut out = None;
        let mut rot = None;
        let mut cluster = None;
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
                if let Some(d) = tok.strip_prefix("out:").and_then(Departure::parse) {
                    out = Some(d);
                    return false;
                }
                if let Some(q) = tok.strip_prefix("rot:").and_then(parse_euler_deg) {
                    rot = Some(q);
                    return false;
                }
                if let Some(n) = tok.strip_prefix("cluster:").and_then(|s| s.parse().ok()) {
                    cluster = Some(n);
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
            out,
            rot,
            cluster,
        });
    }
    parts
}

/// Parse `rx,ry,rz` euler **degrees** into a quaternion (for a part's `rot:` token). Needs all three.
fn parse_euler_deg(s: &str) -> Option<Quat> {
    let n: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    (n.len() == 3).then(|| {
        Quat::from_euler(
            EulerRot::XYZ,
            n[0].to_radians(),
            n[1].to_radians(),
            n[2].to_radians(),
        )
    })
}

/// Build the show: `MARTIN_SEQ` if set, else a shorthand from `MARTIN_TEXT` /
/// `MARTIN_PLY(+_PLY2)(+_REFORM)`. Returns the sequence + the asset root (the .ply folder).
pub(crate) fn sequence_from_env(score: &score::Score) -> (Sequence, Option<String>) {
    // With NOTHING set, play the bundled default demo (assets/demo.seq) — a self-contained show from
    // the published, licence-cleared mesh/logo assets — so a fresh `cargo run` is a working demo.
    let nothing_set = [
        "MARTIN_SEQ",
        "MARTIN_COMPOSE",
        "MARTIN_TEXT",
        "MARTIN_PLY",
        "MARTIN_PLY2",
        "MARTIN_REFORM",
    ]
    .iter()
    .all(|k| std::env::var(k).is_err());
    if nothing_set {
        let count = std::env::var("MARTIN_MORPH_COUNT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(200_000);
        return (
            Sequence {
                parts: parse_seq("assets/demo.seq", score),
                count,
            },
            None, // → asset root defaults to `assets/`
        );
    }

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
            out: None,
            rot: None,
            cluster: None,
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
        out: None,
        rot: None,
        cluster: None,
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
            out: None,
            rot: None,
            cluster: None,
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
    // `cluster:N` → replicate a part into N scattered, randomly-rotated copies (a "serving", e.g. a
    // pile of bitterballen) BEFORE normalize, so the whole pile frames as one. Downsample per copy
    // to keep the total near the morph budget.
    let cluster_total = std::env::var("MARTIN_MORPH_COUNT")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(200_000);
    for (raw, part) in raws.iter_mut().zip(&seq.parts) {
        if let Some(copies) = part.cluster {
            let per = (cluster_total / copies.max(1)).max(2_000);
            let one = resample_morton(std::mem::take(raw), per);
            *raw = crate::morph::cluster_of(&one, copies);
        }
    }
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
            PartContent::GlMesh(name) => format!("gl-mesh {name}"),
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
        // a glb: part is a placeholder here (sample_gl_mesh fills + normalizes it from the loaded
        // mesh later) — don't normalize the placeholder (its zero extent would blow up the scale).
        // a `cluster:` part is already frame-sized by cluster_of (the whole serving ≈ NORMALIZE_EXTENT),
        // so don't re-normalize it (that would re-fit on the 90th-percentile and shrink the pile).
        if normalize && !matches!(part.content, PartContent::GlMesh(_)) && part.cluster.is_none() {
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
    let mut out_clouds: Vec<Option<Handle<PlanarGaussian3d>>> = Vec::new();
    for (idx, raw) in raws.into_iter().enumerate() {
        let mut shaped = resample_morton(raw, n);
        // bake the part's own `rot:` into its shape (so sources/out clouds inherit it, and the morph
        // between differently-oriented parts reorients smoothly). glb parts rotate in sample_gl_mesh.
        if let Some(q) = seq.parts[idx].rot {
            if !matches!(seq.parts[idx].content, PartContent::GlMesh(_)) {
                crate::morph::rotate_gaussians(&mut shaped, q);
            }
        }
        let tr = transitions[idx];
        let r = content_radius;
        // if the PREVIOUS part DEPARTS (washes/disperses away), there's no shape to flow from →
        // a Morph/Swarm part must assemble fresh from a ball instead.
        let prev_departs = idx > 0 && seq.parts[idx - 1].out.is_some();
        let src: Option<Vec<Gaussian3d>> = match tr {
            Transition::Morph | Transition::Swarm if prev_departs => {
                Some(ball_of(&shaped, r * BALL_SHELL))
            }
            Transition::Morph | Transition::Swarm => None, // both flow from the previous shape
            Transition::Ball => Some(ball_of(&shaped, r * BALL_SHELL)),
            Transition::Fade => Some(fade_of(&shaped)),
            Transition::Explode => Some(explode_of(&shaped, r * 1.6)),
            Transition::Implode => Some(implode_of(&shaped)),
            Transition::Drop => Some(drop_of(&shaped, r * 2.5)),
            Transition::Rain => Some(rain_of(&shaped, r * 3.0)),
            Transition::Swirl => Some(swirl_of(&shaped, 2.4, 1.5)),
            // Per-particle (shader) transitions: identity source — positions/opacity match the
            // target and the vendored shader staggers them per particle over the morph.
            _ if tr.shader_uniforms().is_some() => Some(shaped.clone()),
            _ => None,
        };
        // `out:` departure target cloud (faded + displaced) — the part morphs to this as it leaves.
        let out = seq.parts[idx].out.map(|d| match d {
            Departure::Wash => wash_of(&shaped, r * 2.5),
            Departure::Disperse => disperse_of(&shaped, r * 1.8),
            Departure::Evaporate => evaporate_of(&shaped, r * 3.0),
            Departure::Sink => sink_of(&shaped, r * 3.0),
        });
        sources.push(src.map(|s| assets.add(PlanarGaussian3d::from(s))));
        out_clouds.push(out.map(|o| assets.add(PlanarGaussian3d::from(o))));
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

    // `glb:` parts: render the real mesh AND sample its gaussians from that same mesh (filled by
    // sample_gl_mesh) so the mesh can dissolve into its own splats. Shares the cloud's base frame.
    for (idx, part) in seq.parts.iter().enumerate() {
        if let PartContent::GlMesh(name) = &part.content {
            spawn_gl_dissolve(
                &mut commands,
                &asset_server,
                name,
                idx,
                entity_rot,
                part.rot.unwrap_or(Quat::IDENTITY),
                shapes[idx].clone(),
                n,
            );
        }
    }

    state.shapes = shapes;
    state.sources = sources;
    state.out_clouds = out_clouds;
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
    // Phase: ARRIVING (source → shape), holding (shape), or DEPARTING (shape → its faded out-cloud
    // — a distinct step carved from the end of the hold, before the next part arrives; see `out:`).
    let next = idx + 1;
    let depart_at = if next < parts.len() && parts[idx].out.is_some() {
        (starts[next] - DEPART_LEN).max(starts[idx] + parts[idx].morph)
    } else {
        f32::MAX
    };
    let departing = t >= depart_at;
    let (want_lhs, want_rhs, factor, arriving) = if departing {
        let f = ((t - depart_at) / DEPART_LEN).clamp(0.0, 1.0);
        let out = state.out_clouds[idx].as_ref().unwrap_or(&state.shapes[idx]);
        (&state.shapes[idx], out, f, false)
    } else {
        let dt = t - starts[idx];
        let f = (dt / parts[idx].morph.max(1e-3)).clamp(0.0, 1.0);
        // lhs: the part's source cloud (ball/fade/explode/…), or — for a plain Morph — the prev shape.
        let lhs = match &state.sources[idx] {
            Some(h) => h,
            None => &state.shapes[idx - 1],
        };
        (lhs, &state.shapes[idx], f, dt < parts[idx].morph)
    };
    if interp.lhs.0.id() != want_lhs.id() {
        interp.lhs = PlanarGaussian3dHandle(want_lhs.clone());
    }
    if interp.rhs.0.id() != want_rhs.id() {
        interp.rhs = PlanarGaussian3dHandle(want_rhs.clone());
    }
    let morphing = arriving || departing;
    let eased = factor * factor * (3.0 - 2.0 * factor);
    cs.time = eased;
    // the ball-pulse shader effect belongs to the plain Morph transition (prev → next through a
    // ball); source-based transitions carry their own motion, so they don't pulse.
    cs.bulge = if arriving && state.transitions[idx] == Transition::Morph {
        parts[idx].bulge
    } else {
        0.0
    };
    // ~swarm: flock the particles along curled paths during the morph (the @_,_,N timing value is
    // the swarm strength); mutually exclusive with the ball-pulse above.
    cs.swarm = if arriving && state.transitions[idx] == Transition::Swarm {
        parts[idx].bulge
    } else {
        0.0
    };
    // per-particle shader transitions (typewriter/sparkle/…): drive the vendored uniforms only
    // while morphing in; otherwise mode 0 = off (held shape renders plain, fully sort-safe).
    let (mode, soft, axis) = arriving
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

    // glb: dissolve — the splats are the exact complement of the mesh (1 − its alpha): present
    // during the splat-assemble, hidden while the mesh is crisp (no poke-through), and back as it
    // dissolves — so mesh↔splats crossfade, and the dissolve completes before the next part morphs.
    if matches!(parts[idx].content, PartContent::GlMesh(_)) {
        cs.global_opacity *= 1.0 - gl_mesh_alpha(starts, parts, idx, t);
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
    splat: f32,     // disk size (fraction of the mesh's largest dim)
    thin: f32,      // disk thickness fraction
    sampled: bool,  // done once the mesh has loaded + been sampled
}

/// Spawn a `glb:` dissolve overlay: the rendered glTF mesh (hidden + identity until sampled, so
/// `sample_gl_mesh` can read its node-local geometry, then place it to coincide with the splats) +
/// a key/fill light (splats are unlit, the PBR mesh needs light).
fn spawn_gl_dissolve(
    commands: &mut Commands,
    assets: &AssetServer,
    name: &str,
    part: usize,
    base_rot: Quat,
    rot: Quat,
    shape: Handle<PlanarGaussian3d>,
    morph_n: usize,
) {
    let env_f = |k: &str, d: f32| {
        std::env::var(k)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(d)
    };
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
            sample_count: std::env::var("MARTIN_MESH_COUNT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(morph_n),
            splat: env_f("MARTIN_MESH_SPLAT", 0.006),
            thin: env_f("MARTIN_MESH_THIN", 0.2),
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
/// the exact complement (`1 - this`, see part_director), so mesh and splats crossfade cleanly:
/// 0 while the part assembles AS SPLATS → MATERIALIZE 0→1 over MODEL_FADE → 1 crisp hold → its OWN
/// DISSOLVE 1→0 over the last DISSOLVE_LEN of the hold (finishing BEFORE the next part's morph, so
/// the splats are fully back before they morph on — the dissolve is a distinct step, not overlapped).
fn gl_mesh_alpha(starts: &[f32], parts: &[Part], p: usize, t: f32) -> f32 {
    let assemble_end = starts[p] + parts[p].morph;
    let materialize_end = assemble_end + MODEL_FADE;
    // dissolve ends right as the next part starts morphing; carve DISSOLVE_LEN out of the hold for it.
    let (dissolve_start, dissolve_end) = match p + 1 {
        next if next < parts.len() => (
            (starts[next] - DISSOLVE_LEN).max(materialize_end),
            starts[next],
        ),
        _ => (f32::MAX, f32::MAX), // last part: never dissolves, just stays crisp
    };
    if t < assemble_end {
        0.0
    } else if t < materialize_end {
        (t - assemble_end) / MODEL_FADE
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
    let vis = gl_mesh_alpha(&state.starts, &seq.parts, m.part, clock.t);
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
