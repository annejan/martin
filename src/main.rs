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
//! crate (see vendor/.../CHANGES.md). Live free-orbit: ←/→ yaw · ↑/↓ pitch · W/S zoom · A/D &
//! Q/E pan · M = mark camera waypoint · Space = restart · F11/F = fullscreen (MARTIN_FULLSCREEN=1).

use std::f32::consts::PI;

use bevy::app::AppExit;
use bevy::asset::AssetPlugin;
use bevy::audio::{AudioPlayer, AudioSource, PlaybackSettings};
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::render::view::Hdr;
use bevy::window::{MonitorSelection, WindowMode};
use bevy_gaussian_splatting::morph::interpolate::GaussianInterpolate;
use bevy_gaussian_splatting::sort::SortMode;
use bevy_gaussian_splatting::{
    CloudSettings, Gaussian3d, GaussianCamera, GaussianSplattingPlugin, PlanarGaussian3d,
    PlanarGaussian3dHandle,
};

mod audio;
mod mesh;
mod morph;
mod score;
mod splat_image;
mod text;
mod waypoints;
use crate::morph::{ball_of, drop_of, explode_of, fade_of, implode_of, resample_morton, swirl_of};
use crate::splat_image::build_image_gaussians;
use crate::text::{
    build_text_gaussians, build_text_outline_gaussians, build_text_penwrite_gaussians, TEXT_RGB,
};

const FRONT_YAW: f32 = 1.4; // camera faces the subject head-on (single-image splats have no back)
const SWAY: f32 = 0.25; // gentle left-right sway amplitude — never reaches the hollow back
const SIDE_SEP: f32 = 1.2; // half-spacing when a part places several splats side by side
const BALL_SHELL: f32 = 0.9; // intro ball-shell radius, in units of the framed radius
const NORMALIZE_EXTENT: f32 = 2.0; // each part is centered + scaled so its largest dim = this
const FLASH_LEN: f32 = 0.18; // cut-flash decay time (s), MARTIN_FLASH strength
const DEFAULT_PITCH: f32 = 0.12; // camera pitch above the horizon (rad) when framing
const DEFORM_SPEED: f32 = 2.0; // deform animation rate: deform_time = clock.t * this

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

/// Free-orbit inspection camera: orbit `yaw`/`pitch` at `dist` around a `target` look-at point.
/// `build_sequence` frames it (MARTIN_YAW/PITCH/ZOOM seed it); `controls` flies it live; the
/// recorder sways or holds it deterministically.
#[derive(Component)]
struct OrbitCam {
    target: Vec3, // look-at point
    dist: f32,    // distance from the target
    yaw: f32,     // orbit angle around the vertical (Y) axis
    pitch: f32,   // angle above the horizon (0 = eye level, +up looks down)
    framed: bool,
}

impl Default for OrbitCam {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            dist: 5.0,
            yaw: FRONT_YAW,
            pitch: DEFAULT_PITCH,
            framed: false,
        }
    }
}

/// Place the camera on a sphere around `target` from `yaw`/`pitch`/`dist`.
fn orbit_camera(mut q: Query<(&mut Transform, &OrbitCam)>) {
    for (mut tf, cam) in &mut q {
        let (sp, cp) = cam.pitch.sin_cos();
        let (sy, cy) = cam.yaw.sin_cos();
        tf.translation = cam.target + Vec3::new(cp * cy, sp, cp * sy) * cam.dist;
        tf.look_at(cam.target, Vec3::Y);
    }
}

/// Live free-orbit controls (ignored while recording): **arrows** orbit (←/→ yaw, ↑/↓ pitch),
/// **W/S** zoom in/out, **A/D** pan left/right, **Q/E** pan down/up, **M** logs a camera
/// waypoint (→ the waypoints file), **Space** restarts.
fn controls(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    rec: Res<RecordState>,
    mut clock: ResMut<SeqClock>,
    mut marks: ResMut<waypoints::Waypoints>,
    mut q: Query<&mut OrbitCam>,
) {
    if rec.dir.is_some() {
        return; // record_driver drives the camera + clock deterministically while recording
    }
    // path playback owns the camera: while flying a loaded path (MARTIN_FLY) skip live orbit +
    // marking, but keep Space (restart) working so you can re-watch the move.
    if marks.fly.is_some() && marks.list.len() >= 2 {
        if keys.just_pressed(KeyCode::Space) {
            clock.t = 0.0;
        }
        return;
    }
    let dt = time.delta_secs();
    for mut cam in &mut q {
        let orbit = 1.3 * dt; // rad/s
        if keys.pressed(KeyCode::ArrowLeft) {
            cam.yaw -= orbit;
        }
        if keys.pressed(KeyCode::ArrowRight) {
            cam.yaw += orbit;
        }
        if keys.pressed(KeyCode::ArrowUp) {
            cam.pitch = (cam.pitch + orbit).min(1.5);
        }
        if keys.pressed(KeyCode::ArrowDown) {
            cam.pitch = (cam.pitch - orbit).max(-1.5);
        }
        let step = cam.dist.max(0.1) * dt;
        if keys.pressed(KeyCode::KeyW) {
            cam.dist = (cam.dist - step).max(0.05); // zoom in
        }
        if keys.pressed(KeyCode::KeyS) {
            cam.dist += step; // zoom out
        }
        // pan the look-at target: A/D along the camera's horizontal right, Q/E along world up.
        let right = Vec3::new(cam.yaw.sin(), 0.0, -cam.yaw.cos());
        let pan = cam.dist.max(0.1) * 0.6 * dt;
        if keys.pressed(KeyCode::KeyA) {
            cam.target -= right * pan;
        }
        if keys.pressed(KeyCode::KeyD) {
            cam.target += right * pan;
        }
        if keys.pressed(KeyCode::KeyQ) {
            cam.target.y -= pan;
        }
        if keys.pressed(KeyCode::KeyE) {
            cam.target.y += pan;
        }
    }
    // M: drop a camera waypoint — log the live orbit pose into the waypoints file, accumulating a
    // camera path you can replay / author the demo's camera moves from later.
    if keys.just_pressed(KeyCode::KeyM) {
        if let Ok(cam) = q.single() {
            marks.list.push(waypoints::Waypoint {
                target: cam.target,
                dist: cam.dist,
                yaw: cam.yaw,
                pitch: cam.pitch,
            });
            match waypoints::save(&marks.list, &marks.path) {
                Ok(()) => info!(
                    "waypoint #{} → {} (yaw {:.3}, pitch {:.3}, dist {:.2}, target [{:.2}, {:.2}, {:.2}])",
                    marks.list.len(),
                    marks.path,
                    cam.yaw,
                    cam.pitch,
                    cam.dist,
                    cam.target.x,
                    cam.target.y,
                    cam.target.z,
                ),
                Err(e) => warn!("waypoint save failed: {e}"),
            }
        }
    }
    if keys.just_pressed(KeyCode::Space) {
        clock.t = 0.0; // restart the show
    }
}

/// Triangle wave 0→1→0 over the unit interval — a there-and-back ease for path playback.
fn pingpong(x: f32) -> f32 {
    if x < 0.5 {
        x * 2.0
    } else {
        2.0 - x * 2.0
    }
}

/// `MARTIN_FLY=<secs>`: fly the camera through the loaded waypoints (the M-key path). While
/// **recording**, the path **fills each part's on-screen window**, **alternating direction**
/// (part 0 first→last, part 1 last→first, …) — so the camera is always moving (it reaches the
/// turn-marker exactly as the morph begins: no dead hold before the transition) and its position
/// is *continuous* across the morph (the next subject reverses from there: no jump). A part's
/// flyby is therefore as long as its `hold`. **Live**, `secs` sets the pace (time per leg) and it
/// ping-pongs the path on a loop for preview. Owns the camera (`controls` + recorder sway stand down).
fn flypath(
    marks: Res<waypoints::Waypoints>,
    rec: Res<RecordState>,
    seq: Option<Res<Sequence>>,
    state: Option<Res<SeqState>>,
    clock: Res<SeqClock>,
    mut q: Query<&mut OrbitCam>,
) {
    let Some(secs) = marks.fly else { return };
    let n = marks.list.len();
    if n < 2 {
        return;
    }
    let legs = (n - 1) as f32;
    let p = if rec.dir.is_some() {
        let (Some(seq), Some(state)) = (&seq, &state) else {
            return;
        };
        if !state.built {
            return;
        }
        // recording = the demo: the path fills each part's on-screen window (its slice of the
        // timeline), ALTERNATING direction (even parts first→last, odd parts last→first). Filling
        // the window keeps the camera always moving — it reaches the turn-marker exactly as the
        // morph begins, then reverses through it: no dead hold before the transition, no jump.
        // (So a part's flyby lasts its hold; live still paces by `secs` per leg.)
        let starts = &state.starts;
        let idx = active_part(starts, clock.t);
        let part_end = starts
            .get(idx + 1)
            .copied()
            .unwrap_or_else(|| show_end(&seq.parts, starts));
        let local = ((clock.t - starts[idx]) / (part_end - starts[idx]).max(0.1)).clamp(0.0, 1.0);
        if idx % 2 == 0 {
            local
        } else {
            1.0 - local
        }
    } else {
        // live: ping-pong there-and-back at `secs` per leg, looping for preview.
        pingpong((clock.t / (2.0 * secs * legs)).fract())
    };
    let Some(w) = waypoints::pose_at(&marks.list, p) else {
        return;
    };
    for mut cam in &mut q {
        cam.target = w.target;
        cam.dist = w.dist;
        cam.yaw = w.yaw;
        cam.pitch = w.pitch;
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
    /// a PNG in the asset dir, rasterized to flat gaussians (a logo, etc.)
    Image(String),
    /// a mesh in the asset dir (`.dae`/`.obj`/`.stl`/`.ply`), surface-sampled into gaussians
    Mesh(String),
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
    Outline, // text traced in outline/pen order — a glowing neon draw-on (filled font); text only
    PenWrite, // text written in pen order on a single-stroke font — true handwriting; text only
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
enum Deform {
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
struct Part {
    content: PartContent,
    hold: f32,                      // seconds held after arriving
    morph: f32,                     // seconds to morph in
    bulge: f32,                     // ball-pulse explosiveness (Morph transition only)
    transition: Option<Transition>, // None = default (Ball for part 0, else Morph)
    anchor: Option<f32>,            // absolute start (s) on the music clock; None = relative
    deform: Option<Deform>,         // persistent deform while held (None = none / MARTIN_DEFORM)
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

/// The whole show: a list of parts + the gaussian budget every part is resampled to.
#[derive(Resource)]
struct Sequence {
    parts: Vec<Part>,
    count: usize,
}

/// Folder that `image:` parts (PNG logos) are read from — the `.ply` asset root (default `assets`).
#[derive(Resource)]
struct AssetRoot(std::path::PathBuf);

/// MARTIN_FLASH=<strength>: over-bright bloom pulse on each part cut (0 = off, the default).
#[derive(Resource)]
struct FlashStrength(f32);

/// Loaded splat handles + the per-part built shapes (all `count` gaussians) + each part's
/// morph-in source cloud + its resolved transition.
#[derive(Resource)]
struct SeqState {
    load_names: Vec<String>,
    loads: Vec<Handle<PlanarGaussian3d>>,
    shapes: Vec<Handle<PlanarGaussian3d>>,
    sources: Vec<Option<Handle<PlanarGaussian3d>>>, // per-part lhs source (None = morph from prev)
    transitions: Vec<Transition>,                   // resolved transition per part
    deforms: Vec<Option<Deform>>,                   // resolved persistent deform per part
    starts: Vec<f32>,                               // absolute start time (s) of each part
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
fn parse_seq(spec: &str, score: &score::Score) -> Vec<Part> {
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

/// Parse a source head (`text:` / `wall:` / `image:` / `mesh:` / `splat:`) into a `PartContent`.
/// Shared by the morph timeline (`parse_seq`) and the composition stage (`parse_compose`).
fn parse_source(head: &str) -> Option<PartContent> {
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
fn side_by_side<'a>(names: impl Iterator<Item = &'a str>) -> Vec<(String, Vec3)> {
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
fn part_gaussians(
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

/// Once every referenced splat has loaded, build each part's shape (resampled to the fixed
/// count) + the intro ball, spawn the single interpolate entity, and frame the union once.
fn build_sequence(
    mut commands: Commands,
    mut assets: ResMut<Assets<PlanarGaussian3d>>,
    seq: Option<Res<Sequence>>,
    state: Option<ResMut<SeqState>>,
    root: Res<AssetRoot>,
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
            if idx == 0 && tr == Transition::Morph {
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
fn active_part(starts: &[f32], t: f32) -> usize {
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

/// Drive the show from `SeqClock.t`: find the active part, retarget the interpolate entity's
/// lhs/rhs (only on change), and set the blend factor + ball bulge. Part 0 morphs in from the
/// intro ball; every later part morphs in from the previous part's shape.
fn part_director(
    seq: Option<Res<Sequence>>,
    state: Option<Res<SeqState>>,
    clock: Res<SeqClock>,
    flash: Res<FlashStrength>,
    mut q: Query<(&mut GaussianInterpolate<Gaussian3d>, &mut CloudSettings)>,
) {
    let (Some(seq), Some(state)) = (seq, state) else {
        return;
    };
    if !state.built {
        return;
    }
    let Some(entity) = state.entity else { return };
    let Ok((mut interp, mut cs)) = q.get_mut(entity) else {
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
    let (dmode, damp, dfreq) = state.deforms[idx]
        .map(|d| d.uniforms())
        .unwrap_or((0, 0.0, 0.0));
    cs.deform_mode = dmode;
    cs.deform_amp = damp;
    cs.deform_freq = dfreq;
    cs.deform_time = t * DEFORM_SPEED;
    // Flash on each cut (term-demo's Director trick): a brief over-bright pulse at every part
    // start → the HDR bloom flares. MARTIN_FLASH=<strength> (0 = off, default); reuses
    // global_opacity, so off keeps every frame byte-identical.
    let flash = flash.0
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
            .fold(0.0_f32, f32::max);
    cs.global_opacity = 1.0 + flash;
}

/// Live clock advance (record mode drives `SeqClock` itself, deterministically).
fn advance_seq_clock(
    time: Res<Time>,
    rec: Res<RecordState>,
    state: Option<Res<SeqState>>,
    comp: Option<Res<Composition>>,
    mut clock: ResMut<SeqClock>,
) {
    if rec.dir.is_some() {
        return;
    }
    // advance once the show is up — the morph sequence OR the composition stage.
    let built = state.map(|s| s.built).unwrap_or(false) || comp.map(|c| c.built).unwrap_or(false);
    if built {
        clock.t += time.delta_secs();
    }
}

/// Add `NoFrustumCulling` to the sequence entity once its Aabb exists, so morph/ball
/// particles that briefly leave the framed view don't pop out.
#[allow(clippy::type_complexity)] // a Bevy query filter tuple — verbose by nature
fn seq_no_cull(
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

// ===========================================================================================
// Headless capture: deterministic recorder + single screenshot
// ===========================================================================================

/// MARTIN_RECORD=<dir>: dump one PNG per frame across the whole timeline, then exit.
#[derive(Resource)]
struct RecordState {
    dir: Option<String>,
    dt: f32,       // timeline seconds advanced per frame
    yaw_step: f32, // camera sway radians per frame
    sway: bool,    // gentle front-sway (true) vs hold the framed/pinned yaw (MARTIN_YAW set)
    i: u32,
    grace: u32,
}

/// End of the cue timeline: the latest part's `start + morph + hold` (anchors can push it past a
/// simple sum). The recorder uses this (+ a tail) for the clip length; `flypath` spreads the
/// camera path across it while recording.
fn show_end(parts: &[Part], starts: &[f32]) -> f32 {
    parts
        .iter()
        .zip(starts)
        .map(|(p, &start)| start + p.morph + p.hold)
        .fold(0.0_f32, f32::max)
}

/// Deterministic recorder: total duration = the cue timeline's end (last part's
/// `start + morph + hold`) + tail; set the clock per frame, sway the camera, screenshot, then
/// exit. Frame-indexed → smooth regardless of render speed.
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
    let (Some(seq), Some(state)) = (seq, state) else {
        return;
    };
    if !state.built || !camq.iter().any(|c| c.framed) {
        return; // wait until built + framed
    }
    // end of the cue timeline (the latest part's start + morph + hold), plus a tail.
    let dur = show_end(&seq.parts, &state.starts) + 1.0;
    let total = (dur / rec.dt).ceil() as u32;
    if rec.i >= total {
        // Wait for the async PNG writes to actually land before exiting — a fast (release)
        // build outruns the screenshot writer, so a fixed grace count would truncate the clip.
        // Poll the directory until every frame is on disk (with a ~20 s safety cap).
        rec.grace += 1;
        let written = std::fs::read_dir(&dir)
            .map(|d| {
                d.filter_map(Result::ok)
                    .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
                    .count()
            })
            .unwrap_or(total as usize);
        if written >= total as usize || rec.grace > 1200 {
            info!("recording complete: {total} frames ({written} on disk) -> {dir}");
            exit.write(AppExit::Success);
        }
        return;
    }
    let i = rec.i;
    clock.t = i as f32 * rec.dt;
    // gentle front-sway for object showcases; hold the framed yaw when MARTIN_YAW pins a scene.
    if rec.sway {
        let yaw = FRONT_YAW + SWAY * (i as f32 * rec.yaw_step).sin();
        for mut c in &mut camq {
            c.yaw = yaw;
        }
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
    let Some(path) = shot.path.clone() else {
        return;
    };
    let el = time.elapsed_secs();
    if !shot.done && el >= shot.at {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
        shot.done = true;
        info!("auto-screenshot -> {path}");
    }
    if shot.done && el >= shot.at + 2.0 {
        exit.write(AppExit::Success);
    }
}

/// In a live window (not recording / screenshotting), **exit when the show is done** instead of
/// sitting on the last part forever. `Space` restarts; `MARTIN_LOOP=1` keeps it up (for tuning).
fn live_end(
    rec: Res<RecordState>,
    shot: Res<ShotConfig>,
    seq: Option<Res<Sequence>>,
    state: Option<Res<SeqState>>,
    clock: Res<SeqClock>,
    mut exit: MessageWriter<AppExit>,
) {
    if rec.dir.is_some() || shot.path.is_some() || std::env::var("MARTIN_LOOP").is_ok() {
        return; // the recorder/screenshot exit on their own; MARTIN_LOOP = stay up
    }
    let (Some(seq), Some(state)) = (seq, state) else {
        return;
    };
    if state.built && clock.t > show_end(&seq.parts, &state.starts) + 2.5 {
        exit.write(AppExit::Success);
    }
}

/// FPS + splat-count metrics. `MARTIN_FPS=1` logs every ~0.5 s; the **`I`** key toggles that live
/// and logs one snapshot immediately.
#[derive(Resource)]
struct FpsLog {
    enabled: bool,
    accum: f32,
    frames: u32,
}

fn fps_log(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    clock: Res<SeqClock>,
    seq: Option<Res<Sequence>>,
    mut f: ResMut<FpsLog>,
) {
    let snap = keys.just_pressed(KeyCode::KeyI); // `I` → toggle logging + log one snapshot now
    if snap {
        f.enabled = !f.enabled;
    }
    f.accum += time.delta_secs();
    f.frames += 1;
    if (f.enabled && f.accum >= 0.5) || snap {
        let fps = f.frames as f32 / f.accum.max(1e-6);
        let ms = 1000.0 * f.accum / f.frames.max(1) as f32;
        // gaussians rendered per part (the morph budget; 0 = each part's native count).
        let splats = seq.map(|s| s.count).unwrap_or(0);
        info!(
            "metrics: {fps:.1} fps ({ms:.1} ms/frame) · {splats} splats/part · t={:.2}",
            clock.t
        );
        f.accum = 0.0;
        f.frames = 0;
    }
}

// ===========================================================================================
// Live audio (monitor the synth while flying — the recorder muxes the WAV instead)
// ===========================================================================================

/// The loaded score (`MARTIN_SCORE` file or built-in), shared for live-audio rendering.
#[derive(Resource, Clone)]
struct ScoreRes(std::sync::Arc<score::Score>);

/// Cinder's synth, rendered on a **background thread** (the render takes seconds; blocking startup
/// stalls the first frame long enough to lose the Vulkan swapchain → crash). `music_director` picks
/// up the WAV bytes when the thread finishes and spawns the player in sync with the show.
#[derive(Resource)]
struct Music {
    // Mutex so the !Sync Receiver can live in a (Send+Sync) Bevy resource.
    rx: std::sync::Mutex<std::sync::mpsc::Receiver<Vec<u8>>>,
    handle: Option<Handle<AudioSource>>,
    entity: Option<Entity>,
    prev_t: f32,
}

/// Live playback: turn the background-rendered WAV bytes into an `AudioSource` when ready, spawn it
/// once the sequence is built (so it starts in time with the show), and restart it on a clock reset
/// (Space). Only present when windowed — recording / screenshot / mute don't insert `Music`.
fn music_director(
    mut commands: Commands,
    music: Option<ResMut<Music>>,
    state: Option<Res<SeqState>>,
    comp: Option<Res<Composition>>,
    clock: Res<SeqClock>,
    mut audio_assets: ResMut<Assets<AudioSource>>,
) {
    let Some(mut music) = music else { return };
    // background render finished → make an AudioSource from its WAV bytes (once).
    if music.handle.is_none() {
        let received = music.rx.lock().unwrap().try_recv().ok();
        if let Some(bytes) = received {
            music.handle = Some(audio_assets.add(AudioSource {
                bytes: bytes.into(),
            }));
            info!("live audio: synth ready");
        }
    }
    // clock jumped backwards (Space restart) → despawn so it respawns from the top, resynced.
    if clock.t + 0.05 < music.prev_t {
        if let Some(e) = music.entity.take() {
            commands.entity(e).despawn();
        }
    }
    music.prev_t = clock.t;
    let built = state.map(|s| s.built).unwrap_or(false) || comp.map(|c| c.built).unwrap_or(false);
    if built && music.entity.is_none() {
        if let Some(h) = music.handle.clone() {
            music.entity = Some(
                commands
                    .spawn((AudioPlayer(h), PlaybackSettings::ONCE))
                    .id(),
            );
        }
    }
}

// ===========================================================================================
// Composition — many objects on one stage at once (MARTIN_COMPOSE), vs the single-morph timeline
// ===========================================================================================

/// One object placed on the composition stage: a source + where it sits + how it moves.
#[derive(Clone)]
struct Composed {
    content: PartContent,
    pos: Vec3,
    scale: f32,
    rot: Vec3,   // static orientation, euler degrees
    spin: Vec3,  // auto-rotation, degrees/sec
    sway: Vec3,  // oscillating rotation amplitude, degrees (swings front-on; for hollow-back splats)
    bob: f32,    // vertical bob amplitude (units)
    drift: Vec3, // translation velocity (units/sec)
    appear: f32, // fade-in start (s on the show clock)
    out: f32,    // fade-out start (s); f32::MAX = stays to the end
    fade: f32,   // fade in/out duration (s)
}

/// `MARTIN_COMPOSE=<file>`: a stage of objects, all on screen together.
#[derive(Resource)]
struct Composition {
    objects: Vec<Composed>,
    built: bool,
}

/// Per-object animation state, carried on each spawned cloud entity.
#[derive(Component)]
struct ComposeAnim {
    base_pos: Vec3,
    base_rot: Quat,
    spin: Vec3, // rad/sec
    sway: Vec3, // rad amplitude, oscillating
    bob: f32,
    drift: Vec3,
    appear: f32,
    out: f32,
    fade: f32,
}

/// Parse `MARTIN_COMPOSE` (a file path or inline string). Each line: a `<source>` head (text/splat/
/// mesh/image) then placement tokens — `@x,y,z` position, `*scale`, `rot a,b,c`, `spin a,b,c`
/// (deg/s), `bob amp`, `drift dx,dy,dz`, `in <anchor>`, `out <anchor>` (section/bar/beat/seconds).
fn parse_compose(spec: &str, score: &score::Score) -> Vec<Composed> {
    let raw = std::fs::read_to_string(spec).unwrap_or_else(|_| spec.to_string());
    let kw = |t: &str| matches!(t, "rot" | "spin" | "sway" | "bob" | "drift" | "in" | "out");
    let mut out = Vec::new();
    for line in raw.split([';', '\n']) {
        let s = line.split('#').next().unwrap_or("").trim();
        if s.is_empty() {
            continue;
        }
        let toks: Vec<&str> = s.split_whitespace().collect();
        // source = the leading tokens up to the first placement token (so `text:HELLO WORLD` works).
        let split = toks
            .iter()
            .position(|t| t.starts_with('@') || t.starts_with('*') || kw(t))
            .unwrap_or(toks.len());
        let Some(content) = parse_source(&toks[..split].join(" ")) else {
            continue;
        };
        let rest = &toks[split..];
        let (mut pos, mut scale, mut rot) = (Vec3::ZERO, 1.0_f32, Vec3::ZERO);
        let (mut spin, mut sway, mut bob, mut drift) = (Vec3::ZERO, Vec3::ZERO, 0.0_f32, Vec3::ZERO);
        // appear < 0 = no fade-in (visible from the start); `in <anchor>` sets it to a real time.
        let (mut appear, mut out_t, fade) = (-1.0_f32, f32::MAX, 0.8_f32);
        let mut i = 0;
        while i < rest.len() {
            let t = rest[i];
            if let Some(v) = t.strip_prefix('@') {
                pos = vec3_csv(v);
                i += 1;
            } else if let Some(v) = t.strip_prefix('*') {
                scale = v.parse().unwrap_or(1.0);
                i += 1;
            } else {
                let val = rest.get(i + 1).copied().unwrap_or("");
                match t {
                    "rot" => rot = vec3_csv(val),
                    "spin" => spin = vec3_csv(val),
                    "sway" => sway = vec3_csv(val),
                    "drift" => drift = vec3_csv(val),
                    "bob" => bob = val.parse().unwrap_or(0.0),
                    "in" => appear = score.anchor_seconds(val).unwrap_or(0.0),
                    "out" => out_t = score.anchor_seconds(val).unwrap_or(f32::MAX),
                    _ => {}
                }
                i += 2;
            }
        }
        out.push(Composed {
            content,
            pos,
            scale,
            rot,
            spin,
            sway,
            bob,
            drift,
            appear,
            out: out_t,
            fade,
        });
    }
    out
}

fn vec3_csv(s: &str) -> Vec3 {
    let n: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    Vec3::new(
        n.first().copied().unwrap_or(0.0),
        n.get(1).copied().unwrap_or(0.0),
        n.get(2).copied().unwrap_or(0.0),
    )
}

/// Build the stage once every referenced splat has loaded: each object → its own gaussian cloud
/// entity placed at its transform with a `ComposeAnim` for motion. Frames the camera on the union.
fn build_composition(
    mut commands: Commands,
    mut assets: ResMut<Assets<PlanarGaussian3d>>,
    comp: Option<ResMut<Composition>>,
    state: Option<Res<SeqState>>,
    root: Res<AssetRoot>,
    mut cam: Query<&mut OrbitCam>,
) {
    let (Some(mut comp), Some(state)) = (comp, state) else {
        return;
    };
    if comp.built || comp.objects.is_empty() {
        return;
    }
    if state.loads.iter().any(|h| assets.get(h).is_none()) {
        return; // wait for every referenced splat
    }
    let base = cloud_base_rotation();
    // cap each object's splats so a stage of big splats stays performant on the iGPU.
    let count = std::env::var("MARTIN_MORPH_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120_000);
    let mut placed: Vec<(Vec3, f32)> = Vec::new(); // (centre, radius) per object, for framing
    for obj in &comp.objects {
        let mut raw = part_gaussians(&obj.content, &state, &assets, &root.0);
        if raw.is_empty() {
            continue;
        }
        crate::morph::normalize_to(&mut raw, NORMALIZE_EXTENT); // centre + ~2 units across
        let raw = resample_morton(raw, count);
        let rot = Quat::from_euler(
            EulerRot::XYZ,
            obj.rot.x.to_radians(),
            obj.rot.y.to_radians(),
            obj.rot.z.to_radians(),
        ) * base;
        let handle = assets.add(PlanarGaussian3d::from(raw));
        commands.spawn((
            // a static GaussianInterpolate (lhs == rhs) — the same render path the morph engine
            // uses (a plain PlanarGaussian3dHandle isn't picked up by martin's pipeline).
            // NB: do NOT add NoFrustumCulling here — `calculate_bounds` skips culling-exempt
            // entities, so they'd never get an Aabb and `extract_gaussians` would drop them
            // (black screen). Static stage clouds want frustum culling anyway.
            GaussianInterpolate::<Gaussian3d> {
                lhs: PlanarGaussian3dHandle(handle.clone()),
                rhs: PlanarGaussian3dHandle(handle),
            },
            CloudSettings {
                sort_mode: SortMode::Radix,
                time: 0.0,
                time_start: 0.0,
                time_stop: 1.0,
                bulge: 0.0,
                global_opacity: 0.0, // animate_composition fades it in
                ..default()
            },
            Transform {
                translation: obj.pos,
                rotation: rot,
                scale: Vec3::splat(obj.scale),
            },
            ComposeAnim {
                base_pos: obj.pos,
                base_rot: rot,
                spin: obj.spin * (PI / 180.0),
                sway: obj.sway * (PI / 180.0),
                bob: obj.bob,
                drift: obj.drift,
                appear: obj.appear,
                out: obj.out,
                fade: obj.fade,
            },
        ));
        placed.push((obj.pos, NORMALIZE_EXTENT * 0.5 * obj.scale));
    }
    comp.built = true;
    if placed.is_empty() {
        return;
    }
    let center = placed.iter().map(|(p, _)| *p).sum::<Vec3>() / placed.len() as f32;
    let radius = placed
        .iter()
        .map(|(p, r)| (*p - center).length() + r)
        .fold(0.1_f32, f32::max);
    let zoom = std::env::var("MARTIN_ZOOM")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .filter(|z| *z > 0.0)
        .unwrap_or(1.0);
    let dist = radius * 2.5 / zoom;
    for mut c in &mut cam {
        c.target = center;
        c.dist = dist;
        c.yaw = FRONT_YAW;
        c.pitch = DEFAULT_PITCH;
        c.framed = true;
    }
    info!(
        "composition: {} objects on stage, centre [{:.2},{:.2},{:.2}], dist {:.2}",
        placed.len(),
        center.x,
        center.y,
        center.z,
        dist
    );
}

/// Animate the stage from the show clock: spin + bob + drift each object, fade it in (and out, if
/// it has an `out` time) via `global_opacity`.
fn animate_composition(
    clock: Res<SeqClock>,
    mut q: Query<(&ComposeAnim, &mut Transform, &mut CloudSettings)>,
) {
    let t = clock.t;
    for (a, mut tf, mut cs) in &mut q {
        // spin = continuous rotation; sway = a gentle oscillation around the base orientation
        // (swings a hollow-back single-image splat left/right without ever facing away).
        let osc = (t * 0.6).sin();
        tf.rotation = a.base_rot
            * Quat::from_euler(
                EulerRot::XYZ,
                a.spin.x * t + a.sway.x * osc,
                a.spin.y * t + a.sway.y * osc,
                a.spin.z * t + a.sway.z * osc,
            );
        let bob = if a.bob != 0.0 {
            a.bob * (t * 1.5).sin()
        } else {
            0.0
        };
        tf.translation = a.base_pos + a.drift * t + Vec3::Y * bob;
        let fin = if a.appear < 0.0 {
            1.0 // no `in` → visible from the start (robust even if the clock hasn't advanced)
        } else {
            ((t - a.appear) / a.fade.max(1e-3)).clamp(0.0, 1.0)
        };
        let fout = if a.out < f32::MAX {
            ((a.out + a.fade - t) / a.fade.max(1e-3)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        cs.global_opacity = fin.min(fout);
    }
}

/// Slowly orbit the camera around the stage (the "flow") — additive with the live arrow keys.
fn compose_camera(
    comp: Option<Res<Composition>>,
    rec: Res<RecordState>,
    time: Res<Time>,
    mut cam: Query<&mut OrbitCam>,
) {
    if comp.map(|c| c.built).unwrap_or(false) {
        let dt = if rec.dir.is_some() {
            1.0 / 60.0
        } else {
            time.delta_secs()
        };
        for mut c in &mut cam {
            c.yaw += 0.12 * dt;
        }
    }
}

// ===========================================================================================
// Wiring
// ===========================================================================================

/// Build the show: `MARTIN_SEQ` if set, else a shorthand from `MARTIN_TEXT` /
/// `MARTIN_PLY(+_PLY2)(+_REFORM)`. Returns the sequence + the asset root (the .ply folder).
fn sequence_from_env(score: &score::Score) -> (Sequence, Option<String>) {
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

fn parent_dir(p: String) -> Option<String> {
    std::path::Path::new(&p)
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.to_string_lossy().into_owned())
}

fn main() {
    // MARTIN_SCORE_DUMP=path: export the built-in score as an editable tracker file, then exit —
    // a ready-to-edit starting point (round-trips through MARTIN_SCORE).
    if let Ok(path) = std::env::var("MARTIN_SCORE_DUMP") {
        match std::fs::write(&path, score::Score::builtin().to_dsl()) {
            Ok(()) => eprintln!("score: built-in written to {path}"),
            Err(e) => eprintln!("score dump error: {e}"),
        }
        return;
    }

    // The score (MARTIN_SCORE file, else built-in) drives both the synth AND the @@anchor times.
    let score = score::Score::from_env();

    // MARTIN_SYNTH_WAV=path: render the synth to a WAV and exit (record.sh muxes it onto the
    // frames). Done before the Bevy app so it needs no window/GPU.
    if let Ok(path) = std::env::var("MARTIN_SYNTH_WAV") {
        let track = audio::synth_track(&score);
        match audio::write_wav(&track, &path) {
            Ok(()) => eprintln!(
                "synth: {} samples ({:.1}s) -> {path}",
                track.len(),
                track.len() as f32 / audio::SAMPLE_RATE as f32
            ),
            Err(e) => eprintln!("synth wav error: {e}"),
        }
        return;
    }

    // MARTIN_COMPOSE: the composition stage (many objects at once). When set it IS the show — the
    // morph timeline is left empty (build_sequence no-ops) and build_composition drives everything.
    let composition = std::env::var("MARTIN_COMPOSE")
        .ok()
        .map(|spec| parse_compose(&spec, &score));
    let (sequence, asset_root) = if composition.is_some() {
        let root = std::env::var("MARTIN_PLY").ok().and_then(parent_dir);
        (
            Sequence {
                parts: Vec::new(),
                count: 0,
            },
            root,
        )
    } else {
        sequence_from_env(&score)
    };
    // where `image:` PNG parts are read from — the .ply folder, or `assets` by default.
    let asset_root_path =
        std::path::PathBuf::from(asset_root.clone().unwrap_or_else(|| "assets".to_string()));

    // MARTIN_FULLSCREEN=1 → start borderless-fullscreen (ignored while recording, which
    // needs the fixed 1280×720 window for uniform frames). Toggle live with F11 / F.
    let fullscreen =
        std::env::var("MARTIN_FULLSCREEN").is_ok() && std::env::var("MARTIN_RECORD").is_err();
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
        plugins = plugins.set(AssetPlugin {
            file_path: root,
            ..default()
        });
    }

    App::new()
        .add_plugins(plugins)
        .add_plugins(GaussianSplattingPlugin)
        .insert_resource(sequence)
        .insert_resource(AssetRoot(asset_root_path))
        .insert_resource(FlashStrength(
            std::env::var("MARTIN_FLASH")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0),
        ))
        .insert_resource(waypoints::Waypoints::from_env())
        .insert_resource(ScoreRes(std::sync::Arc::new(score)))
        .insert_resource(Composition {
            objects: composition.unwrap_or_default(),
            built: false,
        })
        .init_resource::<SeqClock>()
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(FpsLog {
            enabled: std::env::var("MARTIN_FPS").is_ok(),
            accum: 0.0,
            frames: 0,
        })
        .insert_resource(ShotConfig {
            path: std::env::var("MARTIN_SHOT").ok(),
            at: std::env::var("MARTIN_SHOT_AT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(6.0),
            done: false,
        })
        .insert_resource(RecordState {
            dir: std::env::var("MARTIN_RECORD").ok(),
            dt: 1.0 / 60.0,
            yaw_step: 2.0 * PI / 480.0, // ~8s gentle sway period
            // a pinned yaw, a parked capture pose, or a flown waypoint path → hold/drive it, no sway
            sway: std::env::var("MARTIN_YAW").is_err()
                && std::env::var("MARTIN_CAMERAS").is_err()
                && std::env::var("MARTIN_FLY").is_err()
                && std::env::var("MARTIN_COMPOSE").is_err(),
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
                flypath,
                fullscreen_toggle,
                shot_driver,
                fps_log,
                music_director,
                live_end,
                build_composition,
                animate_composition,
                compose_camera,
            ),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    seq: Res<Sequence>,
    comp: Res<Composition>,
    score_res: Res<ScoreRes>,
) {
    // load every referenced splat (by filename in the asset folder); build_sequence /
    // build_composition assemble the shapes once they're all available.
    let mut names: Vec<String> = Vec::new();
    let add = |content: &PartContent, names: &mut Vec<String>| {
        if let PartContent::Splats(list) = content {
            for (n, _) in list {
                if !names.contains(n) {
                    names.push(n.clone());
                }
            }
        }
    };
    for b in &seq.parts {
        add(&b.content, &mut names);
    }
    for o in &comp.objects {
        add(&o.content, &mut names);
    }
    let loads = names
        .iter()
        .map(|n| asset_server.load::<PlanarGaussian3d>(n.clone()))
        .collect();
    commands.insert_resource(SeqState {
        load_names: names,
        loads,
        shapes: Vec::new(),
        sources: Vec::new(),
        transitions: Vec::new(),
        deforms: Vec::new(),
        starts: Vec::new(),
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

    // Live audio: render Cinder's synth on a BACKGROUND THREAD — it takes a few seconds, and doing
    // it here would block startup long enough to stall the first frame and lose the swapchain (the
    // crash). music_director picks up the bytes when ready (audio starts a beat into the show).
    // Skipped while recording (the recorder muxes the WAV separately), for a screenshot, or muted.
    let want_audio = std::env::var("MARTIN_RECORD").is_err()
        && std::env::var("MARTIN_SHOT").is_err()
        && std::env::var("MARTIN_MUTE").is_err();
    if want_audio {
        let score = score_res.0.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(audio::encode_wav(&audio::synth_track(&score)));
        });
        commands.insert_resource(Music {
            rx: std::sync::Mutex::new(rx),
            handle: None,
            entity: None,
            prev_t: 0.0,
        });
        info!("live audio: rendering Cinder's synth in the background (MARTIN_MUTE=1 to silence)");
    }
}
