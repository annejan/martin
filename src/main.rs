//! martin — fly a camera around Gaussian splats while they morph and reassemble.
//!
//! ONE engine: everything is a **sequence of parts** that morph into one another. A part
//! is splat-text or one-or-more splats; each part assembles in from a ball cloud, then the
//! next part morphs in (Morton-paired, with a `sin(pi*t)` ball pulse). The `MARTIN_PLY /
//! _PLY2 / _REFORM / _TEXT` env vars are just shorthands that build a sequence;
//! `MARTIN_SEQ` is the full timeline. See `USAGE.md` for the env reference.
//!
//! Rendering: one `GaussianInterpolate` entity (the crate's GPU blend), retargeted per
//! part; depth-sorted by GPU radix (reads live morphed positions → no holes); HDR `Bloom`
//! on black makes bright splats glow. The ball pulse is a shader edit in the vendored
//! crate (see vendor/.../CHANGES.md). Live: ↑/↓ zoom · ←/→ raise/lower · Space = restart ·
//! F11/F = fullscreen (or start fullscreen with MARTIN_FULLSCREEN=1).

use bevy::prelude::*;
use bevy::app::AppExit;
use bevy::asset::AssetPlugin;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::render::view::Hdr;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::window::{MonitorSelection, WindowMode};
use bevy_gaussian_splatting::morph::interpolate::GaussianInterpolate;
use bevy_gaussian_splatting::sort::SortMode;
use bevy_gaussian_splatting::{
    CloudSettings, Gaussian3d, GaussianCamera, GaussianSplattingPlugin, PlanarGaussian3d,
    PlanarGaussian3dHandle,
};
use std::f32::consts::PI;

mod morph;
mod text;
use crate::morph::{ball_of, drop_of, explode_of, fade_of, implode_of, resample_morton, swirl_of};
use crate::text::{build_text_gaussians, build_text_pen_gaussians, TEXT_RGB};

const FRONT_YAW: f32 = 1.4; // camera faces the subject head-on (single-image splats have no back)
const SWAY: f32 = 0.25; // gentle left-right sway amplitude — never reaches the hollow back
const SIDE_SEP: f32 = 1.2; // half-spacing when a part places several splats side by side
const BALL_SHELL: f32 = 0.9; // intro ball-shell radius, in units of the framed radius
const NORMALIZE_EXTENT: f32 = 2.0; // each part is centered + scaled so its largest dim = this

/// `.ply` splats are Y-down → rotate the cloud 180° about X for Y-up. Text is built Y-down
/// too (see `build_text_gaussians`), so one transform makes text *and* splats upright.
fn cloud_base_rotation() -> Quat {
    Quat::from_rotation_x(PI)
}

fn file_name_of(p: &str) -> String {
    std::path::Path::new(p)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "aegg.ply".into())
}

// ===========================================================================================
// Camera
// ===========================================================================================

#[derive(Component)]
struct OrbitCam {
    center: Vec3,
    radius: f32,
    elevation: f32,
    yaw: f32,
    framed: bool,
}

impl Default for OrbitCam {
    fn default() -> Self {
        Self { center: Vec3::ZERO, radius: 5.0, elevation: 1.5, yaw: FRONT_YAW, framed: false }
    }
}

/// MARTIN_YAW=<rad>: pin the camera to a fixed orbit angle (for inspecting a splat).
#[derive(Resource)]
struct CamOverride(Option<f32>);

/// Place the camera from its orbit state. Framing (center/radius/elevation) is set once by
/// `build_sequence`; `yaw` is the gentle front sway, driven live by `controls` or
/// deterministically by `record_driver`.
fn orbit_camera(cam_override: Res<CamOverride>, mut q: Query<(&mut Transform, &OrbitCam)>) {
    for (mut tf, cam) in &mut q {
        let yaw = cam_override.0.unwrap_or(cam.yaw);
        let offset = Vec3::new(cam.radius * yaw.cos(), cam.elevation, cam.radius * yaw.sin());
        tf.translation = cam.center + offset;
        tf.look_at(cam.center, Vec3::Y);
    }
}

fn controls(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    rec: Res<RecordState>,
    mut clock: ResMut<SeqClock>,
    mut q: Query<&mut OrbitCam>,
) {
    if rec.dir.is_some() {
        return; // record_driver drives the camera + clock deterministically while recording
    }
    let dt = time.delta_secs();
    for mut cam in &mut q {
        cam.yaw = FRONT_YAW + SWAY * (time.elapsed_secs() * 0.4).sin(); // gentle front sway
        let step = cam.radius.max(1.0);
        if keys.pressed(KeyCode::ArrowUp) {
            cam.radius = (cam.radius - dt * step).max(0.05);
        }
        if keys.pressed(KeyCode::ArrowDown) {
            cam.radius += dt * step;
        }
        if keys.pressed(KeyCode::ArrowLeft) {
            cam.elevation -= dt * step;
        }
        if keys.pressed(KeyCode::ArrowRight) {
            cam.elevation += dt * step;
        }
    }
    if keys.just_pressed(KeyCode::Space) {
        clock.t = 0.0; // restart the show
    }
}

/// F11 / F: toggle borderless fullscreen at runtime.
fn fullscreen_toggle(keys: Res<ButtonInput<KeyCode>>, mut windows: Query<&mut Window>) {
    if keys.just_pressed(KeyCode::F11) || keys.just_pressed(KeyCode::KeyF) {
        for mut w in &mut windows {
            w.mode = match w.mode {
                WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
                _ => WindowMode::Windowed,
            };
        }
    }
}

// ===========================================================================================
// Sequence — the one timeline engine
// ===========================================================================================

#[derive(Clone)]
enum PartContent {
    Text(String),
    /// one or more splats (filename in the asset dir, world offset) combined into one shape
    Splats(Vec<(String, Vec3)>),
}

/// How a part *arrives*. `Morph` (the default after part 0) flows from the previous part's
/// shape, Morton-paired, with the optional ball-pulse `bulge`. The next group build a source
/// cloud from the part's own shape and morph in from that — the ball is just one of them. The
/// last group are *per-particle* transitions driven by the vendored shader (`transition_mode`
/// uniform): the source is an identity copy and the shader staggers opacity/position per
/// particle (see `SHADER-BLUEPRINT.md`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Transition {
    Morph,   // prev shape → this shape (with bulge ball-pulse); the original behaviour
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
    PenWrite,   // text drawn in pen order (stroke reveal); only meaningful for text parts
}

impl Transition {
    fn parse(s: &str) -> Option<Transition> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "morph" => Transition::Morph,
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
            Transition::PenWrite => Some((7, 0.06, 0)),
            _ => None,
        }
    }
}

/// One part morphs in from the previous (or, for part 0, from a ball), then holds.
#[derive(Clone)]
struct Part {
    content: PartContent,
    hold: f32,                      // seconds held after arriving
    morph: f32,                     // seconds to morph in
    bulge: f32,                     // ball-pulse explosiveness (Morph transition only)
    transition: Option<Transition>, // None = default (Ball for part 0, else Morph)
}

/// The whole show: a list of parts + the gaussian budget every part is resampled to.
#[derive(Resource)]
struct Sequence {
    parts: Vec<Part>,
    count: usize,
}

/// Loaded splat handles + the per-part built shapes (all `count` gaussians) + each part's
/// morph-in source cloud + its resolved transition.
#[derive(Resource)]
struct SeqState {
    load_names: Vec<String>,
    loads: Vec<Handle<PlanarGaussian3d>>,
    shapes: Vec<Handle<PlanarGaussian3d>>,
    sources: Vec<Option<Handle<PlanarGaussian3d>>>, // per-part lhs source (None = morph from prev)
    transitions: Vec<Transition>,                   // resolved transition per part
    built: bool,
    entity: Option<Entity>,
}

/// Master timeline clock (seconds). Live: accumulates real time; record: frame×dt.
#[derive(Resource, Default)]
struct SeqClock {
    t: f32,
}

/// Parse `MARTIN_SEQ`: a file path OR an inline string. Parts are `;`/newline-separated.
/// Each part: `text:STRING` or `splat:a.ply` (or `a.ply+b.ply` for side-by-side), optional
/// trailing `@hold,morph,bulge`. `#` comments and blank lines are skipped.
fn parse_seq(spec: &str) -> Vec<Part> {
    let raw = std::fs::read_to_string(spec).unwrap_or_else(|_| spec.to_string());
    let mut parts = Vec::new();
    for line in raw.split([';', '\n']) {
        let s = line.trim();
        if s.is_empty() || s.starts_with('#') {
            continue;
        }
        // a `~name` transition token (e.g. `~fade`) anywhere on the line — it's a single
        // whitespace-delimited token, so pull it out and keep the rest. Position-independent,
        // so `splat:x.ply ~fade @2,3` and `splat:x.ply @2,3 ~fade` both work.
        let mut transition = None;
        let s: String = s
            .split_whitespace()
            .filter(|tok| match tok.strip_prefix('~').and_then(Transition::parse) {
                Some(tr) => {
                    transition = Some(tr);
                    false
                }
                None => true,
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
            if let Some(v) = nums.first() { hold = *v; }
            if let Some(v) = nums.get(1) { morph = *v; }
            if let Some(v) = nums.get(2) { bulge = *v; }
        }
        let content = if let Some(txt) = head.strip_prefix("text:") {
            PartContent::Text(txt.to_string())
        } else if let Some(p) = head.strip_prefix("splat:") {
            PartContent::Splats(side_by_side(p.split('+').map(str::trim).filter(|x| !x.is_empty())))
        } else {
            continue;
        };
        parts.push(Part { content, hold, morph, bulge, transition });
    }
    parts
}

/// Arrange splat filenames evenly along X, centered (one splat → at origin).
fn side_by_side<'a>(names: impl Iterator<Item = &'a str>) -> Vec<(String, Vec3)> {
    let names: Vec<&str> = names.collect();
    let n = names.len();
    names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let x = if n <= 1 { 0.0 } else { -SIDE_SEP + 2.0 * SIDE_SEP * (i as f32) / ((n - 1) as f32) };
            (file_name_of(name), Vec3::new(x, 0.0, 0.0))
        })
        .collect()
}

/// Read a part's gaussians (text rasterized, or splats loaded + offset + combined).
fn part_gaussians(
    content: &PartContent,
    state: &SeqState,
    assets: &Assets<PlanarGaussian3d>,
) -> Vec<Gaussian3d> {
    match content {
        PartContent::Text(s) => build_text_gaussians(s, TEXT_RGB, 3.0, 2, 0.012),
        PartContent::Splats(list) => {
            let mut out = Vec::new();
            for (name, off) in list {
                let Some(idx) = state.load_names.iter().position(|x| x == name) else { continue };
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

/// Once every referenced splat has loaded, build each part's shape (resampled to the fixed
/// count) + the intro ball, spawn the single interpolate entity, and frame the union once.
fn build_sequence(
    mut commands: Commands,
    mut assets: ResMut<Assets<PlanarGaussian3d>>,
    seq: Option<Res<Sequence>>,
    state: Option<ResMut<SeqState>>,
    mut cam: Query<&mut OrbitCam>,
) {
    let (Some(seq), Some(mut state)) = (seq, state) else { return };
    if state.built || seq.parts.is_empty() {
        return;
    }
    if state.loads.iter().any(|h| assets.get(h).is_none()) {
        return; // wait for every referenced splat
    }

    // resolve each part's transition first (explicit ~name > MARTIN_TRANSITION > Ball for part
    // 0 / Morph after) — needed before building gaussians so a PenWrite text part is built as a
    // stroked outline (pen order baked into visibility) instead of filled coverage.
    let global_tr = std::env::var("MARTIN_TRANSITION").ok().and_then(|s| Transition::parse(&s));
    let transitions: Vec<Transition> = seq
        .parts
        .iter()
        .enumerate()
        .map(|(idx, part)| {
            let tr = part
                .transition
                .or(global_tr)
                .unwrap_or(if idx == 0 { Transition::Ball } else { Transition::Morph });
            // part 0 has nothing to morph from — fall back to a ball assemble.
            if idx == 0 && tr == Transition::Morph { Transition::Ball } else { tr }
        })
        .collect();

    // read every part's gaussians once, so count==0 can mean "size N to the largest part"
    // (every part is then resampled to that single N — required by the shared morph output).
    let mut raws: Vec<Vec<Gaussian3d>> = seq
        .parts
        .iter()
        .zip(&transitions)
        .map(|(part, &tr)| match (&part.content, tr) {
            (PartContent::Text(s), Transition::PenWrite) => {
                build_text_pen_gaussians(s, TEXT_RGB, 3.0, 0.7, 0.012)
            }
            _ => part_gaussians(&part.content, &state, &assets),
        })
        .collect();
    // Normalize each part to a common "normal" size (MARTIN_NORMALIZE=0 to disable). Sources
    // vary wildly — a COLMAP scene spans hundreds of units, a TRELLIS object ~1 — so without
    // this they'd frame inconsistently and morph badly. We log the raw extent first.
    let normalize = std::env::var("MARTIN_NORMALIZE").map(|v| v != "0").unwrap_or(true);
    for (raw, part) in raws.iter_mut().zip(&seq.parts) {
        let label = match &part.content {
            PartContent::Text(s) => format!("text \"{s}\""),
            PartContent::Splats(list) => list.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join("+"),
        };
        info!("part {label}: raw extent {:.2} units ({} gaussians)", crate::morph::extent_of(raw), raw.len());
        if normalize {
            crate::morph::normalize_to(raw, NORMALIZE_EXTENT);
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
            Transition::Morph => None,
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
    let intro0 = sources[0].clone().expect("part 0 always builds a source cloud");

    // MARTIN_ROT="rx,ry,rz" (euler degrees) orients the cloud — e.g. to stand a COLMAP scene
    // upright for a "normal" POV. Default = cloud_base_rotation (flip-X, right for portrait
    // splats; gives scenes their abstract sideways look).
    let entity_rot = std::env::var("MARTIN_ROT")
        .ok()
        .and_then(|s| {
            let n: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            (n.len() == 3).then(|| {
                Quat::from_euler(EulerRot::XYZ, n[0].to_radians(), n[1].to_radians(), n[2].to_radians())
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
    // MARTIN_ZOOM scales how close the camera sits (>1 = closer / more zoomed in, <1 = back).
    let zoom = std::env::var("MARTIN_ZOOM")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .filter(|z| *z > 0.0)
        .unwrap_or(1.0);
    let center = entity_rot * frame_center;
    for mut c in &mut cam {
        c.center = center;
        c.radius = content_radius * frame_factor / zoom;
        c.elevation = c.radius * 0.12; // gentle downward tilt, held constant across zoom levels
        c.framed = true;
    }

    state.shapes = shapes;
    state.sources = sources;
    state.transitions = transitions;
    state.entity = Some(entity);
    state.built = true;
    info!("sequence built: {} parts × {n} gaussians", state.shapes.len());
}

/// Drive the show from `SeqClock.t`: find the active part, retarget the interpolate entity's
/// lhs/rhs (only on change), and set the blend factor + ball bulge. Part 0 morphs in from the
/// intro ball; every later part morphs in from the previous part's shape.
fn part_director(
    seq: Option<Res<Sequence>>,
    state: Option<Res<SeqState>>,
    clock: Res<SeqClock>,
    mut q: Query<(&mut GaussianInterpolate<Gaussian3d>, &mut CloudSettings)>,
) {
    let (Some(seq), Some(state)) = (seq, state) else { return };
    if !state.built {
        return;
    }
    let Some(entity) = state.entity else { return };
    let Ok((mut interp, mut cs)) = q.get_mut(entity) else { return };
    let parts = &seq.parts;

    // each part occupies [morph_i + hold_i); the first `morph_i` is the morph-in.
    let mut t = clock.t;
    let (mut idx, mut morphing, mut factor) = (parts.len() - 1, false, 1.0_f32);
    for (i, b) in parts.iter().enumerate() {
        let seg = b.morph + b.hold;
        if t < seg {
            idx = i;
            morphing = t < b.morph;
            factor = if morphing { (t / b.morph.max(1e-3)).clamp(0.0, 1.0) } else { 1.0 };
            break;
        }
        t -= seg;
    }

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
    cs.bulge = if morphing && state.transitions[idx] == Transition::Morph { parts[idx].bulge } else { 0.0 };
    // per-particle shader transitions (typewriter/sparkle/…): drive the vendored uniforms only
    // while morphing in; otherwise mode 0 = off (held shape renders plain, fully sort-safe).
    let (mode, soft, axis) = morphing
        .then(|| state.transitions[idx].shader_uniforms())
        .flatten()
        .unwrap_or((0, 0.0, 0));
    cs.transition_mode = mode;
    cs.transition_softness = soft;
    cs.transition_axis = axis;
}

/// Live clock advance (record mode drives `SeqClock` itself, deterministically).
fn advance_seq_clock(
    time: Res<Time>,
    rec: Res<RecordState>,
    state: Option<Res<SeqState>>,
    mut clock: ResMut<SeqClock>,
) {
    if rec.dir.is_some() {
        return;
    }
    if state.map(|s| s.built).unwrap_or(false) {
        clock.t += time.delta_secs();
    }
}

/// Add `NoFrustumCulling` to the sequence entity once its Aabb exists, so morph/ball
/// particles that briefly leave the framed view don't pop out.
#[allow(clippy::type_complexity)] // a Bevy query filter tuple — verbose by nature
fn seq_no_cull(
    mut commands: Commands,
    state: Option<Res<SeqState>>,
    q: Query<(), (With<GaussianInterpolate<Gaussian3d>>, With<Aabb>, Without<NoFrustumCulling>)>,
) {
    let Some(state) = state else { return };
    let Some(e) = state.entity else { return };
    if q.get(e).is_ok() {
        commands.entity(e).insert(NoFrustumCulling);
    }
}

// ===========================================================================================
// Headless capture: deterministic recorder + single screenshot
// ===========================================================================================

/// MARTIN_RECORD=<dir>: dump one PNG per frame across the whole timeline, then exit.
#[derive(Resource)]
struct RecordState {
    dir: Option<String>,
    dt: f32,       // timeline seconds advanced per frame
    yaw_step: f32, // camera sway radians per frame
    i: u32,
    grace: u32,
}

/// Deterministic recorder: total duration = Σ(morph + hold) + tail; set the clock per frame,
/// sway the camera, screenshot, then exit. Frame-indexed → smooth regardless of render speed.
fn record_driver(
    mut rec: ResMut<RecordState>,
    seq: Option<Res<Sequence>>,
    state: Option<Res<SeqState>>,
    mut clock: ResMut<SeqClock>,
    mut camq: Query<&mut OrbitCam>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(dir) = rec.dir.clone() else { return };
    let (Some(seq), Some(state)) = (seq, state) else { return };
    if !state.built || !camq.iter().any(|c| c.framed) {
        return; // wait until built + framed
    }
    let dur = seq.parts.iter().map(|b| b.morph + b.hold).sum::<f32>() + 1.0; // +tail
    let total = (dur / rec.dt).ceil() as u32;
    if rec.i >= total {
        // Wait for the async PNG writes to actually land before exiting — a fast (release)
        // build outruns the screenshot writer, so a fixed grace count would truncate the clip.
        // Poll the directory until every frame is on disk (with a ~20 s safety cap).
        rec.grace += 1;
        let written = std::fs::read_dir(&dir)
            .map(|d| d.filter_map(Result::ok).filter(|e| e.path().extension().is_some_and(|x| x == "png")).count())
            .unwrap_or(total as usize);
        if written >= total as usize || rec.grace > 1200 {
            info!("recording complete: {total} frames ({written} on disk) -> {dir}");
            exit.write(AppExit::Success);
        }
        return;
    }
    let i = rec.i;
    clock.t = i as f32 * rec.dt;
    let yaw = FRONT_YAW + SWAY * (i as f32 * rec.yaw_step).sin();
    for mut c in &mut camq {
        c.yaw = yaw;
    }
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(format!("{dir}/frame_{i:05}.png")));
    rec.i += 1;
}

/// MARTIN_SHOT=<path> [MARTIN_SHOT_AT=<s>]: one headless screenshot at time `s`, then exit.
#[derive(Resource)]
struct ShotConfig {
    path: Option<String>,
    at: f32,
    done: bool,
}

fn shot_driver(
    time: Res<Time>,
    mut shot: ResMut<ShotConfig>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = shot.path.clone() else { return };
    let el = time.elapsed_secs();
    if !shot.done && el >= shot.at {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path.clone()));
        shot.done = true;
        info!("auto-screenshot -> {path}");
    }
    if shot.done && el >= shot.at + 2.0 {
        exit.write(AppExit::Success);
    }
}

/// MARTIN_FPS=1: log smoothed FPS + frame-time + timeline clock every ~0.5s.
#[derive(Resource)]
struct FpsLog {
    enabled: bool,
    accum: f32,
    frames: u32,
}

fn fps_log(time: Res<Time>, clock: Res<SeqClock>, mut f: ResMut<FpsLog>) {
    if !f.enabled {
        return;
    }
    f.accum += time.delta_secs();
    f.frames += 1;
    if f.accum >= 0.5 {
        let ms = 1000.0 * f.accum / f.frames as f32;
        info!("FPS {:.1} ({ms:.1} ms/frame) t={:.2}", f.frames as f32 / f.accum, clock.t);
        f.accum = 0.0;
        f.frames = 0;
    }
}

// ===========================================================================================
// Wiring
// ===========================================================================================

/// Build the show: `MARTIN_SEQ` if set, else a shorthand from `MARTIN_TEXT` /
/// `MARTIN_PLY(+_PLY2)(+_REFORM)`. Returns the sequence + the asset root (the .ply folder).
fn sequence_from_env() -> (Sequence, Option<String>) {
    let count_default = if std::env::var("MARTIN_SEQ").is_ok() { 200_000 } else { 0 };
    let count = std::env::var("MARTIN_MORPH_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(count_default);

    if let Ok(spec) = std::env::var("MARTIN_SEQ") {
        // asset root = the .ply folder (so `splat:` filenames resolve); MARTIN_PLY sets it.
        let root = std::env::var("MARTIN_PLY").ok().and_then(parent_dir);
        return (Sequence { parts: parse_seq(&spec), count }, root);
    }

    if let Ok(text) = std::env::var("MARTIN_TEXT") {
        let part =
            Part { content: PartContent::Text(text), hold: 2.0, morph: 3.0, bulge: 0.0, transition: None };
        return (Sequence { parts: vec![part], count }, None);
    }

    // splat shorthand: PLY (+ PLY2) as part 0; REFORM (if any) as part 1.
    let primary = std::env::var("MARTIN_PLY").ok();
    let root = primary.as_deref().and_then(|p| parent_dir(p.to_string()));
    let name1 = primary.as_deref().map(file_name_of).unwrap_or_else(|| "aegg.ply".into());
    let mut names = vec![name1];
    if let Ok(p2) = std::env::var("MARTIN_PLY2") {
        names.push(file_name_of(&p2));
    }
    let bulge = std::env::var("MARTIN_BULGE").ok().and_then(|s| s.parse().ok()).unwrap_or(0.9);
    let mut parts = vec![Part {
        content: PartContent::Splats(side_by_side(names.iter().map(String::as_str))),
        hold: 2.0,
        morph: 3.0,
        bulge: 0.0,
        transition: None,
    }];
    if let Ok(reform) = std::env::var("MARTIN_REFORM") {
        parts.push(Part {
            content: PartContent::Splats(vec![(file_name_of(&reform), Vec3::ZERO)]),
            hold: 2.0,
            morph: 3.5,
            bulge,
            transition: None,
        });
    }
    (Sequence { parts, count }, root)
}

fn parent_dir(p: String) -> Option<String> {
    std::path::Path::new(&p)
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.to_string_lossy().into_owned())
}

fn main() {
    let (sequence, asset_root) = sequence_from_env();

    // MARTIN_FULLSCREEN=1 → start borderless-fullscreen (ignored while recording, which
    // needs the fixed 1280×720 window for uniform frames). Toggle live with F11 / F.
    let fullscreen = std::env::var("MARTIN_FULLSCREEN").is_ok() && std::env::var("MARTIN_RECORD").is_err();
    let mut plugins = DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "martin — splat fly-around".into(),
            resolution: (1280, 720).into(), // fixed size so recorded frames are uniform
            mode: if fullscreen {
                WindowMode::BorderlessFullscreen(MonitorSelection::Current)
            } else {
                WindowMode::Windowed
            },
            ..default()
        }),
        ..default()
    });
    if let Some(root) = asset_root {
        plugins = plugins.set(AssetPlugin { file_path: root, ..default() });
    }

    App::new()
        .add_plugins(plugins)
        .add_plugins(GaussianSplattingPlugin)
        .insert_resource(sequence)
        .init_resource::<SeqClock>()
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(CamOverride(std::env::var("MARTIN_YAW").ok().and_then(|s| s.parse().ok())))
        .insert_resource(FpsLog { enabled: std::env::var("MARTIN_FPS").is_ok(), accum: 0.0, frames: 0 })
        .insert_resource(ShotConfig {
            path: std::env::var("MARTIN_SHOT").ok(),
            at: std::env::var("MARTIN_SHOT_AT").ok().and_then(|s| s.parse().ok()).unwrap_or(6.0),
            done: false,
        })
        .insert_resource(RecordState {
            dir: std::env::var("MARTIN_RECORD").ok(),
            dt: 1.0 / 60.0,
            yaw_step: 2.0 * PI / 480.0, // ~8s gentle sway period
            i: 0,
            grace: 0,
        })
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                build_sequence,
                part_director,
                advance_seq_clock,
                seq_no_cull,
                record_driver,
                orbit_camera,
                controls,
                fullscreen_toggle,
                shot_driver,
                fps_log,
            ),
        )
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>, seq: Res<Sequence>) {
    // load every referenced splat (by filename in the asset folder); build_sequence
    // assembles the per-part shapes once they're all available.
    let mut names: Vec<String> = Vec::new();
    for b in &seq.parts {
        if let PartContent::Splats(list) = &b.content {
            for (n, _) in list {
                if !names.contains(n) {
                    names.push(n.clone());
                }
            }
        }
    }
    let loads = names.iter().map(|n| asset_server.load::<PlanarGaussian3d>(n.clone())).collect();
    commands.insert_resource(SeqState {
        load_names: names,
        loads,
        shapes: Vec::new(),
        sources: Vec::new(),
        transitions: Vec::new(),
        built: false,
        entity: None,
    });

    commands.spawn((
        GaussianCamera { warmup: true },
        Camera3d::default(),
        Hdr, // HDR target so bright splats bloom
        Tonemapping::None,
        Bloom::NATURAL,
        Transform::default(),
        OrbitCam::default(),
    ));
}
