//! dogdemo — fly a camera around Gaussian splats while they morph, explode, and
//! reassemble. Everything is driven by `DOGDEMO_*` env vars — see `USAGE.md` (and the
//! repo README) for the full reference; this header is just the map of the file.
//!
//! Effects (one mode is chosen per run, in precedence order):
//!   * Sequence (`DOGDEMO_SEQ`)  — a timeline of beats that morph into one another.
//!   * Splat-text (`DOGDEMO_TEXT`) — a string assembles out of a ball cloud.
//!   * Morph (`DOGDEMO_REFORM`)   — source splat(s) turn into a target splat.
//!   * Explode (default)          — a splat collapses inward.
//!
//! How the morph works: the source/target are paired per-gaussian by a Morton (Z-order)
//! spatial sort (`corresponded`), blended on the GPU by the crate's `GaussianInterpolate`
//! compute pass, and routed onto a fuzzy "ball cloud" at the midpoint by a `sin(pi*t)`
//! pulse injected into the vendored render shader (amplitude = `CloudSettings.bulge`).
//! Splat-text (`build_text_gaussians`) rasterizes glyph coverage into flat gaussians, so
//! text is just another morph source. The depth sort is GPU radix (reads live morphed
//! positions → no holes). HDR `Bloom` on black makes bright splats glow.
//!
//! Live controls: ↑/↓ zoom · ←/→ raise/lower · Space = trigger/reset. The camera only
//! sways across the front (single-image splats have a hollow back).

use bevy::prelude::*;
use bevy::app::AppExit;
use bevy::asset::AssetPlugin;
use bevy::camera::primitives::Aabb;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::render::view::Hdr;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::camera::visibility::NoFrustumCulling;
use bevy_gaussian_splatting::{
    CloudSettings, Gaussian3d, GaussianCamera, GaussianSplattingPlugin, PlanarGaussian3d,
    PlanarGaussian3dHandle, SphericalHarmonicCoefficients,
};
use bevy_gaussian_splatting::morph::interpolate::GaussianInterpolate;
use bevy_gaussian_splatting::sort::SortMode;
use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use std::f32::consts::PI;

/// Bundled bold TTF for splat-text (include_bytes, not the asset server — main() repoints
/// AssetPlugin.file_path to the .ply's parent dir, which would break a relative font load).
static FONT: &[u8] = include_bytes!("../assets/font.ttf");

/// Camera tuning.
const EXPAND_RATE: f32 = 1.5; // explode mode: camera zoom = 1 + EXPAND_RATE * t * 0.6
const FRONT_YAW: f32 = 1.4; // camera faces the subject head-on (single-image splats have no back)
const SWAY: f32 = 0.25; // gentle left-right sway amplitude — never reaches the hollow back
/// Morph duration (seconds of explode-clock) — shared by drive_morph (blend) and the
/// camera recoil so the two stay in lockstep.
const MORPH_DUR: f32 = 3.5;
/// Camera recoil during the morph's midpoint: radius *= (1 + PULLBACK*sin(pi*t)), a
/// gentle pull-back to frame the ball cloud (small — the ball stays ~object-sized).
const PULLBACK: f32 = 0.3;

/// Brush/COLMAP .ply is Y-down/Z-forward → rotate the cloud 180° about X for Y-up.
fn cloud_base_rotation() -> Quat {
    Quat::from_rotation_x(PI)
}

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

#[derive(Resource, Default)]
struct ExplodeState {
    active: bool,
    t: f32,
}

/// Per-cloud animation role, driven by the master clock `tau` (ExplodeState.t).
#[derive(Clone, Copy)]
enum AnimRole {
    /// Intact until `start`, then COLLAPSES inward — particles implode toward the centre
    /// while the whole cloud drifts to the world origin, fading out. (Used by the explode
    /// and two-splat modes; the morph/reform path is handled separately by GaussianInterpolate.)
    Explode { start: f32, home: Vec3 },
}

#[derive(Component, Clone, Copy)]
struct CloudAnim(AnimRole);

struct CloudCfg {
    name: String,
    offset: Vec3,
    role: AnimRole,
}

/// Clouds spawned in EXPLODE mode (DOGDEMO_PLY / _PLY2). Empty in morph/text/sequence
/// modes, which build a single GaussianInterpolate entity instead.
#[derive(Resource)]
struct Clouds(Vec<CloudCfg>);

/// True for morph / text modes → the camera zooms in to frame the reformed result.
#[derive(Resource)]
struct HasReform(bool);

/// DOGDEMO_YAW=<rad>: pin the camera to a fixed orbit angle (for diagnosing which
/// way a splat faces). None → normal spin/sway.
#[derive(Resource)]
struct CamOverride(Option<f32>);

/// How many rendered clouds frame_on_load should wait for (explode: N clouds; morph: 1).
#[derive(Resource)]
struct Expected(usize);

/// DOGDEMO_TEXT="…": splat-text mode — the string assembles out of a ball cloud (a morph
/// whose rhs is the text and whose lhs is the same gaussians scattered onto a sphere).
#[derive(Resource)]
struct TitleText(String);

/// MORPH mode config: source splats (+ side-by-side offset) and the target splat.
#[derive(Resource, Clone)]
struct MorphCfg {
    sources: Vec<(String, Vec3)>,
    target: String,
}

/// Handles for the morph sources/target while they load; build_morph consumes them.
#[derive(Resource)]
struct MorphAssets {
    sources: Vec<(Handle<PlanarGaussian3d>, Vec3)>,
    target: Handle<PlanarGaussian3d>,
    built: bool,
}

/// DOGDEMO_FPS=1: log smoothed FPS + frame-time + morph clock every ~0.5s, so I can
/// read real-time performance across the morph (esp. the dispersed ball cloud) on the
/// target iGPU without the screenshot I/O of record mode skewing the numbers.
#[derive(Resource)]
struct FpsLog {
    enabled: bool,
    accum: f32,
    frames: u32,
}

fn fps_log(time: Res<Time>, explode: Res<ExplodeState>, mut f: ResMut<FpsLog>) {
    if !f.enabled {
        return;
    }
    f.accum += time.delta_secs();
    f.frames += 1;
    if f.accum >= 0.5 {
        let ms = 1000.0 * f.accum / f.frames as f32;
        info!(
            "FPS {:.1} ({ms:.1} ms/frame) morph_t={:.2}",
            f.frames as f32 / f.accum,
            if explode.active { explode.t } else { 0.0 },
        );
        f.accum = 0.0;
        f.frames = 0;
    }
}

fn file_name_of(p: &str) -> String {
    std::path::Path::new(p)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "aegg.ply".into())
}

/// Debug driver (env-gated): auto-explode + framebuffer screenshot + exit.
#[derive(Resource)]
struct Debug {
    shot: Option<String>,
    shot_at: f32,
    auto_explode: bool,
    shot_done: bool,
    exploded: bool,
}

/// Deterministic frame-sequence recorder (env-gated by DOGDEMO_RECORD=<dir>):
/// per-frame explosion clock + gentle orbit, one PNG per frame, then exit.
#[derive(Resource)]
struct RecordState {
    dir: Option<String>,
    frames: u32,
    hold: u32,     // frames to hold the intact object before detonating
    dt: f32,       // explosion-clock seconds advanced per frame
    yaw_step: f32, // camera orbit radians per frame
    i: u32,
    grace: u32,
}

fn main() {
    // DOGDEMO_PLY=<abs path> loads any splat (e.g. a TRELLIS subject), sidestepping
    // the assets/ symlink; DOGDEMO_PLY2 adds a second splat beside it (same dir).
    // Falls back to assets/aegg.ply.
    let primary = std::env::var("DOGDEMO_PLY").ok();
    let asset_root = primary.as_deref().and_then(|p| {
        std::path::Path::new(p)
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .map(|d| d.to_string_lossy().into_owned())
    });
    let name1 = primary
        .as_deref()
        .map(file_name_of)
        .unwrap_or_else(|| "aegg.ply".into());
    const SEP: f32 = 1.2;
    let reform_name = std::env::var("DOGDEMO_REFORM").ok().map(|p| file_name_of(&p));
    // DOGDEMO_TEXT: splat-text mode (a title assembles out of a ball cloud). Takes
    // precedence over the explode/morph paths; needs the reform camera+drive_morph.
    let title = std::env::var("DOGDEMO_TEXT").ok();
    // DOGDEMO_SEQ: the composable timeline ("mix and match all the effects") — one env var
    // of `;`-separated beats (text:/splat:) that morph into one another. Overrides the rest.
    let sequence = std::env::var("DOGDEMO_SEQ").ok().map(|s| {
        let count = std::env::var("DOGDEMO_MORPH_COUNT")
            .ok()
            .and_then(|x| x.parse().ok())
            .unwrap_or(200_000usize);
        Sequence { beats: parse_seq(&s), count }
    });
    let seq_mode = sequence.is_some();
    let has_reform = reform_name.is_some() || title.is_some();
    // source splats (the Martins) with their side-by-side offsets
    let placements: Vec<(String, Vec3)> = match std::env::var("DOGDEMO_PLY2").ok() {
        Some(p2) => vec![
            (name1, Vec3::new(-SEP, 0.0, 0.0)),
            (file_name_of(&p2), Vec3::new(SEP, 0.0, 0.0)),
        ],
        None => vec![(name1, Vec3::ZERO)],
    };
    // MORPH mode when a reform target is set: a true per-gaussian splat-to-splat
    // morph (GaussianInterpolate) — particles literally become the dog. Otherwise
    // EXPLODE mode (in-place inward collapse, no target).
    let morph_cfg = reform_name
        .clone()
        .map(|target| MorphCfg { sources: placements.clone(), target });
    let explode_clouds: Vec<CloudCfg> = if morph_cfg.is_some() {
        Vec::new()
    } else {
        placements
            .iter()
            .enumerate()
            .map(|(i, (name, off))| CloudCfg {
                name: name.clone(),
                offset: *off,
                role: AnimRole::Explode { start: 0.5 * i as f32, home: *off },
            })
            .collect()
    };
    // frame_on_load waits for this many rendered clouds (morph + text each build one).
    let expected = if title.is_some() || morph_cfg.is_some() { 1 } else { explode_clouds.len() };
    // DOGDEMO_SEQ overrides every single-shot mode (it spawns + frames its own entity).
    let (title, has_reform, morph_cfg, explode_clouds, expected) = if seq_mode {
        (None, false, None, Vec::new(), 1usize)
    } else {
        (title, has_reform, morph_cfg, explode_clouds, expected)
    };

    let mut plugins = DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "dogdemo — splat fly-around".into(),
            resolution: (1280, 720).into(), // fixed size so recorded frames are uniform
            ..default()
        }),
        ..default()
    });
    if let Some(root) = asset_root {
        plugins = plugins.set(AssetPlugin { file_path: root, ..default() });
    }

    let mut app = App::new();
    app.add_plugins(plugins)
        .add_plugins(GaussianSplattingPlugin)
        .insert_resource(Clouds(explode_clouds))
        .insert_resource(Expected(expected))
        .insert_resource(HasReform(has_reform))
        .insert_resource(CamOverride(
            std::env::var("DOGDEMO_YAW").ok().and_then(|s| s.parse().ok()),
        ))
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(FpsLog {
            enabled: std::env::var("DOGDEMO_FPS").is_ok(),
            accum: 0.0,
            frames: 0,
        })
        .init_resource::<ExplodeState>()
        .init_resource::<SeqClock>()
        .insert_resource(Debug {
            shot: std::env::var("DOGDEMO_SHOT").ok(),
            shot_at: std::env::var("DOGDEMO_SHOT_AT").ok().and_then(|s| s.parse().ok()).unwrap_or(4.5),
            auto_explode: std::env::var("DOGDEMO_EXPLODE").is_ok(),
            shot_done: false,
            exploded: false,
        })
        .insert_resource({
            let frames: u32 = std::env::var("DOGDEMO_FRAMES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(220);
            RecordState {
                dir: std::env::var("DOGDEMO_RECORD").ok(),
                frames,
                hold: 60, // ~2 s intact intro before detonating
                dt: 1.0 / 60.0, // half-speed explosion clock → slower, smoother motion
                yaw_step: 2.0 * PI / frames as f32,
                i: 0,
                grace: 0,
            }
        })
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                build_morph,
                build_sequence,
                beat_director,
                advance_seq_clock,
                seq_no_cull,
                seq_record_driver,
                frame_on_load,
                controls,
                orbit_camera,
                animate_clouds,
                drive_morph,
                debug_driver,
                record_driver,
                fps_log,
            ),
        );
    // MORPH mode: hand build_morph the source/target config to assemble on load.
    if let Some(m) = morph_cfg {
        app.insert_resource(m);
    }
    // TEXT mode: setup builds the title→ball morph synchronously (no asset wait).
    if let Some(t) = title {
        app.insert_resource(TitleText(t));
    }
    // SEQUENCE mode: setup loads the referenced splats; build_sequence assembles on load.
    if let Some(s) = sequence {
        app.insert_resource(s);
    }
    app.run();
}

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut assets: ResMut<Assets<PlanarGaussian3d>>,
    clouds: Res<Clouds>,
    morph: Option<Res<MorphCfg>>,
    title: Option<Res<TitleText>>,
    seq: Option<Res<Sequence>>,
) {
    if let Some(seq) = seq {
        // SEQUENCE mode: load every referenced splat (by filename in the asset dir);
        // build_sequence assembles the per-beat shapes once they're all available.
        let mut names: Vec<String> = Vec::new();
        for b in &seq.beats {
            if let BeatContent::Splats(list) = &b.content {
                for (n, _) in list {
                    if !names.contains(n) {
                        names.push(n.clone());
                    }
                }
            }
        }
        let loads = names
            .iter()
            .map(|n| asset_server.load::<PlanarGaussian3d>(n.clone()))
            .collect();
        commands.insert_resource(SeqState {
            load_names: names,
            loads,
            shapes: Vec::new(),
            built: false,
            entity: None,
        });
    } else if let Some(t) = title {
        // TEXT mode: a PS1-cyan title assembling out of a ball cloud (built once, here).
        // Sub-1 colour leaves HDR-bloom headroom so the text glows without blobbing out.
        let rgb = [0.40_f32, 0.85, 1.0].map(|c| c * 0.8);
        let text = build_text_gaussians(&t.0, rgb, 3.0, 2, 0.012);
        let (lhs, rhs) = assemble_from_ball(&text, 1.6);
        let lhs_h = assets.add(PlanarGaussian3d::from(lhs));
        let rhs_h = assets.add(PlanarGaussian3d::from(rhs));
        commands.spawn((
            GaussianInterpolate::<Gaussian3d> {
                lhs: PlanarGaussian3dHandle(lhs_h),
                rhs: PlanarGaussian3dHandle(rhs_h),
            },
            // bulge 0: the ball IS the lhs scatter, not a shader pulse. Radix sorts the
            // live GPU positions the compute writes → correct order, no holes.
            CloudSettings {
                sort_mode: SortMode::Radix,
                time: 0.0,
                time_start: 0.0,
                time_stop: 1.0,
                bulge: 0.0,
                ..default()
            },
            Transform::from_rotation(cloud_base_rotation()), // flip Y-down text upright
        ));
    } else if let Some(m) = morph {
        // MORPH mode: load the sources + target as DATA only (no render entity yet);
        // build_morph assembles the corresponded morph clouds once they're available.
        let sources = m
            .sources
            .iter()
            .map(|(name, off)| (asset_server.load::<PlanarGaussian3d>(name.clone()), *off))
            .collect();
        let target = asset_server.load::<PlanarGaussian3d>(m.target.clone());
        commands.insert_resource(MorphAssets { sources, target, built: false });
    } else {
        for cfg in &clouds.0 {
            commands.spawn((
                PlanarGaussian3dHandle(asset_server.load(cfg.name.clone())),
                // CPU (rayon) depth sort: GPU radix sort desyncs the view uniform with
                // multiple clouds → half the splats get culled. CPU sort dodges that.
                // Empty interp range (start==stop) tells the shader `time` is an explode
                // clock, not a morph blend factor (see gaussian.wgsl).
                CloudSettings {
                    sort_mode: SortMode::Rayon,
                    time_start: 0.0,
                    time_stop: 0.0,
                    ..default()
                },
                Transform::from_translation(cfg.offset).with_rotation(cloud_base_rotation()),
                CloudAnim(cfg.role),
            ));
        }
    }

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

/// Frame the camera to the cloud's bounding sphere once its Aabb is known.
fn frame_on_load(
    mut commands: Commands,
    expected: Res<Expected>,
    cloud_q: Query<(Entity, &Aabb, &Transform), With<PlanarGaussian3dHandle>>,
    mut cam: Query<&mut OrbitCam>,
) {
    // world center + radius of every loaded cloud
    let loaded: Vec<(Entity, Vec3, f32)> = cloud_q
        .iter()
        .map(|(e, aabb, tf)| {
            (
                e,
                tf.transform_point(Vec3::from(aabb.center)),
                Vec3::from(aabb.half_extents).length().max(0.001),
            )
        })
        .collect();
    if expected.0 == 0 || loaded.len() < expected.0 {
        return; // wait until every expected cloud has loaded (Aabb present)
    }
    for mut c in &mut cam {
        if c.framed {
            continue;
        }
        // combined bounding sphere over all clouds
        let n = loaded.len() as f32;
        let center = loaded.iter().fold(Vec3::ZERO, |a, (_, cn, _)| a + *cn) / n;
        let radius = loaded
            .iter()
            .fold(0.001_f32, |m, (_, cn, r)| m.max(cn.distance(center) + r));
        c.center = center;
        c.radius = radius * 1.6;
        c.elevation = radius * 0.25;
        c.framed = true;
        for (e, _, _) in &loaded {
            // crate skips Aabb-insertion for NoFrustumCulling entities, so add it now
            commands.entity(*e).insert(NoFrustumCulling);
        }
        info!(
            "framed {} cloud(s): center={center:?} radius={radius:.3} cam_radius={:.3}",
            loaded.len(),
            c.radius
        );
    }
}

/// Morph-mode camera zoom (continuous from t=0): frame the source (1.0) and smoothly
/// zoom IN as it converges onto the reformed target (settles ~0.6). No lurch/shake.
fn reform_zoom(t: f32) -> f32 {
    // pure smooth zoom-IN (no pull-back bump), eased over the morph clock.
    let p = (t / 4.5).clamp(0.0, 1.0);
    let ease = p * p * (3.0 - 2.0 * p); // smoothstep
    1.0 - 0.4 * ease
}

fn orbit_camera(
    explode: Res<ExplodeState>,
    has_reform: Res<HasReform>,
    cam_override: Res<CamOverride>,
    mut q: Query<(&mut Transform, &OrbitCam)>,
) {
    // evaluate the same curve in hold (t=0) and active (t) so there's no step at onset
    let zoom = if has_reform.0 {
        let t = if explode.active { explode.t } else { 0.0 };
        // recoil from the blast: pull back at the morph midpoint (same sin(pi*eased)
        // pulse the shader bulges with), then return to frame the dog.
        let raw = (t / MORPH_DUR).clamp(0.0, 1.0);
        let eased = raw * raw * (3.0 - 2.0 * raw);
        let pulse = (eased * PI).sin();
        reform_zoom(t) * (1.0 + PULLBACK * pulse)
    } else if explode.active {
        1.0 + EXPAND_RATE * explode.t * 0.6
    } else {
        1.0
    };
    for (mut tf, cam) in &mut q {
        let yaw = cam_override.0.unwrap_or(cam.yaw);
        let r = cam.radius * zoom;
        let offset = Vec3::new(r * yaw.cos(), cam.elevation * zoom, r * yaw.sin());
        tf.translation = cam.center + offset;
        tf.look_at(cam.center, Vec3::Y);
    }
}

fn controls(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    rec: Res<RecordState>,
    mut explode: ResMut<ExplodeState>,
    mut q: Query<&mut OrbitCam>,
) {
    if rec.dir.is_some() {
        return; // record_driver drives the animation deterministically while recording
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
        explode.active = !explode.active;
        explode.t = 0.0;
        info!("explode -> {}", explode.active);
    }
    if explode.active {
        explode.t += dt;
    }
}

/// EXPLODE mode (no morph target): drive each cloud's per-gaussian displacement — the
/// vendored gaussian.wgsl reads `CloudSettings.time` as an explode clock — plus opacity and
/// whole-cloud drift, from the master clock `tau`. Each cloud collapses INWARD toward the
/// centre and fades. (The morph/reform path is separate — `build_morph`/`drive_morph`.)
fn animate_clouds(
    explode: Res<ExplodeState>,
    mut q: Query<(&mut CloudSettings, &mut Transform, &CloudAnim)>,
) {
    const COLLAPSE_DUR: f32 = 2.6;
    let tau = if explode.active { explode.t } else { 0.0 };
    for (mut s, mut tf, anim) in &mut q {
        match anim.0 {
            AnimRole::Explode { start, home } => {
                if tau <= start {
                    s.time = 0.0;
                    s.global_opacity = 1.0;
                    tf.translation = home;
                } else {
                    let k = ((tau - start) / COLLAPSE_DUR).clamp(0.0, 1.0);
                    s.time = -1.2 * k; // negative = particles implode toward the centroid
                    s.global_opacity = (1.0 - k).max(0.0); // fade as it collapses
                    tf.translation = home.lerp(Vec3::ZERO, k); // whole cloud drifts to center
                }
                s.global_scale = 1.0;
            }
        }
    }
}

fn debug_driver(
    time: Res<Time>,
    mut dbg: ResMut<Debug>,
    mut explode: ResMut<ExplodeState>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let el = time.elapsed_secs();
    if dbg.auto_explode && !dbg.exploded && el >= 2.0 {
        explode.active = true;
        explode.t = 0.0;
        dbg.exploded = true;
        info!("debug: auto-explode triggered at t={el:.1}");
    }
    if let Some(path) = dbg.shot.clone() {
        if !dbg.shot_done && el >= dbg.shot_at {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path.clone()));
            dbg.shot_done = true;
            info!("auto-screenshot -> {path}");
        }
        if dbg.shot_done && el >= dbg.shot_at + 2.0 {
            exit.write(AppExit::Success);
        }
    }
}

/// Deterministic recorder: once framed, drive a fixed per-frame orbit + explosion
/// clock and dump one PNG per frame to DOGDEMO_RECORD, then exit (frame-indexed, so
/// the output is smooth regardless of render speed).
fn record_driver(
    mut rec: ResMut<RecordState>,
    mut explode: ResMut<ExplodeState>,
    seq: Option<Res<Sequence>>,
    mut camq: Query<&mut OrbitCam>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    if seq.is_some() {
        return; // seq_record_driver owns recording in sequence mode
    }
    let Some(dir) = rec.dir.clone() else {
        return;
    };
    // wait until the camera has framed the cloud before capturing
    if !camq.iter().any(|c| c.framed) {
        return;
    }

    if rec.i >= rec.frames {
        rec.grace += 1; // let async PNG writes flush
        if rec.grace > 30 {
            info!("recording complete: {} frames -> {dir}", rec.frames);
            exit.write(AppExit::Success);
        }
        return;
    }

    let i = rec.i;
    // same gentle front sway as the live camera (never orbit to the splats' missing back)
    let yaw = FRONT_YAW + SWAY * (i as f32 * rec.yaw_step).sin();
    for mut c in &mut camq {
        c.yaw = yaw;
    }
    if i >= rec.hold {
        explode.active = true;
        explode.t = (i - rec.hold) as f32 * rec.dt;
    } else {
        explode.active = false;
        explode.t = 0.0;
    }

    let path = format!("{dir}/frame_{i:05}.png");
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
    rec.i += 1;
}

/// Spread a 10-bit integer so its bits occupy every 3rd position (for Morton/Z-order).
fn part1by2(mut n: u32) -> u32 {
    n &= 0x3ff;
    n = (n | (n << 16)) & 0x030000ff;
    n = (n | (n << 8)) & 0x0300f00f;
    n = (n | (n << 4)) & 0x030c30c3;
    n = (n | (n << 2)) & 0x09249249;
    n
}

/// 30-bit Morton (Z-order) code of a position normalized into [lo, hi] per axis.
/// Sorting by this key orders points so spatial neighbours stay adjacent — which is
/// what makes the morph *flow* (nearby source gaussians map to nearby dog gaussians)
/// instead of teleporting.
fn morton3(p: [f32; 3], lo: [f32; 3], inv: [f32; 3]) -> u32 {
    let q = |k: usize| -> u32 {
        (((p[k] - lo[k]) * inv[k]).clamp(0.0, 1.0) * 1023.0) as u32
    };
    part1by2(q(0)) | (part1by2(q(1)) << 1) | (part1by2(q(2)) << 2)
}

/// Build corresponded `lhs` (sources) / `rhs` (target) gaussian lists of EQUAL length
/// for GaussianInterpolate. Both sets are Morton-sorted so the pairing is spatially
/// coherent, then the sources are resampled to the target count. rhs is returned as the
/// (reordered) target — visually identical, just index-aligned to its lhs partner.
fn corresponded(
    mut src: Vec<Gaussian3d>,
    mut dog: Vec<Gaussian3d>,
    target: usize,
) -> (Vec<Gaussian3d>, Vec<Gaussian3d>) {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for g in src.iter().chain(dog.iter()) {
        let p = g.position_visibility.position;
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let inv = [
        1.0 / (hi[0] - lo[0]).max(1e-6),
        1.0 / (hi[1] - lo[1]).max(1e-6),
        1.0 / (hi[2] - lo[2]).max(1e-6),
    ];
    src.sort_by_key(|g| morton3(g.position_visibility.position, lo, inv));
    dog.sort_by_key(|g| morton3(g.position_visibility.position, lo, inv));
    let nm = src.len().max(1);
    let nd = dog.len().max(1);
    // Output count: `target` if given, else the LARGER input (denser set stays full-res).
    // Both sets are resampled to it — the smaller is upsampled (gaussians repeat,
    // overlapping dups render fine), the larger downsampled. Lower count = faster
    // (fewer to sort + render) at the cost of source density → the real-time knob.
    let l = if target > 0 { target } else { nm.max(nd) };
    let lhs: Vec<Gaussian3d> = (0..l).map(|i| src[((i * nm) / l).min(nm - 1)]).collect();
    let rhs: Vec<Gaussian3d> = (0..l).map(|i| dog[((i * nd) / l).min(nd - 1)]).collect();
    (lhs, rhs)
}

// ---------------------------------------------------------------------------------------
// Sequence ("mix and match all the effects"): DOGDEMO_SEQ is a list of beats that morph
// into one another. Each beat is splat-text or one-or-more splats; consecutive beats are
// Morton-paired and blended (with the ball pulse), so anything flows into anything.
// ---------------------------------------------------------------------------------------

#[derive(Clone)]
enum BeatContent {
    Text(String),
    /// one or more splats (filename in the asset dir, world offset) combined into one shape
    Splats(Vec<(String, Vec3)>),
}

#[derive(Clone)]
struct Beat {
    content: BeatContent,
    hold: f32,  // seconds held after arriving
    morph: f32, // seconds to morph in from the previous beat
    bulge: f32, // ball-cloud explosiveness during the morph-in
}

/// DOGDEMO_SEQ present → the composable timeline mode.
#[derive(Resource)]
struct Sequence {
    beats: Vec<Beat>,
    count: usize, // every beat is resampled to this fixed gaussian count
}

/// Loaded splat handles + the per-beat built shapes (all `count` gaussians).
#[derive(Resource)]
struct SeqState {
    load_names: Vec<String>,
    loads: Vec<Handle<PlanarGaussian3d>>,
    shapes: Vec<Handle<PlanarGaussian3d>>,
    built: bool,
    entity: Option<Entity>,
}

/// Master timeline clock (seconds). Live: accumulates real time; record: frame×dt.
#[derive(Resource, Default)]
struct SeqClock {
    t: f32,
}

/// Parse DOGDEMO_SEQ: a file path OR an inline string. Beats are `;`/newline-separated.
/// Each beat: `text:STRING` or `splat:a.ply` (or `splat:a.ply+b.ply` for side-by-side),
/// optional trailing `@hold,morph,bulge`. `#` comments and blank lines are skipped.
fn parse_seq(spec: &str) -> Vec<Beat> {
    let raw = std::fs::read_to_string(spec).unwrap_or_else(|_| spec.to_string());
    let mut beats = Vec::new();
    for line in raw.split(|c| c == ';' || c == '\n') {
        let s = line.trim();
        if s.is_empty() || s.starts_with('#') {
            continue;
        }
        let (head, timing) = match s.split_once('@') {
            Some((h, t)) => (h.trim(), Some(t.trim())),
            None => (s, None),
        };
        let (mut hold, mut morph, mut bulge) = (1.5_f32, 3.0_f32, 0.9_f32);
        if let Some(t) = timing {
            let nums: Vec<f32> = t.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if let Some(v) = nums.first() { hold = *v; }
            if let Some(v) = nums.get(1) { morph = *v; }
            if let Some(v) = nums.get(2) { bulge = *v; }
        }
        let content = if let Some(txt) = head.strip_prefix("text:") {
            BeatContent::Text(txt.to_string())
        } else if let Some(p) = head.strip_prefix("splat:") {
            const SEP: f32 = 1.2;
            let parts: Vec<&str> = p.split('+').map(str::trim).filter(|x| !x.is_empty()).collect();
            let n = parts.len();
            let splats = parts
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let x = if n <= 1 { 0.0 } else { -SEP + 2.0 * SEP * (i as f32) / ((n - 1) as f32) };
                    (file_name_of(name), Vec3::new(x, 0.0, 0.0))
                })
                .collect();
            BeatContent::Splats(splats)
        } else {
            continue;
        };
        beats.push(Beat { content, hold, morph, bulge });
    }
    beats
}

/// Morton-sort a single gaussian set over its own bounds and resample to exactly `n` (so
/// consecutive beats pair k-th↔k-th in spatial order → coherent flow).
fn resample_morton(mut v: Vec<Gaussian3d>, n: usize) -> Vec<Gaussian3d> {
    if v.is_empty() || n == 0 {
        return Vec::new();
    }
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for g in &v {
        let p = g.position_visibility.position;
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let inv = [
        1.0 / (hi[0] - lo[0]).max(1e-6),
        1.0 / (hi[1] - lo[1]).max(1e-6),
        1.0 / (hi[2] - lo[2]).max(1e-6),
    ];
    v.sort_by_key(|g| morton3(g.position_visibility.position, lo, inv));
    let m = v.len();
    (0..n).map(|i| v[((i * m) / n).min(m - 1)]).collect()
}

/// 3DGS degree-0 encode: rendered color ≈ 0.5 + 0.2820948*dc, so invert for a target sRGB.
fn dc(c: f32) -> f32 {
    (c - 0.5) / 0.282_094_79
}

/// Rasterize `s` to flat gaussians on the z=0 plane (y-up, centered at origin), scaled so the
/// block spans `world_width`. One small gaussian per sampled glyph-coverage pixel. `splat` =
/// LINEAR per-gaussian half-extent (the planar render path does NOT exp scale); opacity =
/// coverage; color from `rgb` (kept <1 so HDR bloom doesn't blow text into a blob).
fn build_text_gaussians(s: &str, rgb: [f32; 3], world_width: f32, stride: usize, splat: f32) -> Vec<Gaussian3d> {
    let font = FontRef::try_from_slice(FONT).expect("font.ttf");
    let px = 64.0_f32;
    let sf = font.as_scaled(PxScale::from(px));
    let line_h = sf.height() + sf.line_gap();

    // layout: pen positions (baseline) per glyph, with kerning + newlines
    let mut placed: Vec<(f32, f32, char)> = Vec::new();
    let (mut pen_x, mut pen_y, mut max_x) = (0.0_f32, sf.ascent(), 0.0_f32);
    let mut prev: Option<char> = None;
    for ch in s.chars() {
        if ch == '\n' {
            pen_x = 0.0;
            pen_y += line_h;
            prev = None;
            continue;
        }
        if let Some(p) = prev {
            pen_x += sf.kern(font.glyph_id(p), font.glyph_id(ch));
        }
        placed.push((pen_x, pen_y, ch));
        pen_x += sf.h_advance(font.glyph_id(ch));
        max_x = max_x.max(pen_x);
        prev = Some(ch);
    }
    let block_h = pen_y + sf.descent().abs();
    let scale = world_width / max_x.max(1.0);
    let (cx, cy) = (max_x * 0.5, block_h * 0.5);

    let mut sh = SphericalHarmonicCoefficients::default();
    sh.set(0, dc(rgb[0]));
    sh.set(1, dc(rgb[1]));
    sh.set(2, dc(rgb[2]));

    let mut out: Vec<Gaussian3d> = Vec::new();
    let mut i: u32 = 0;
    for (gx0, gy0, ch) in &placed {
        let glyph = font.glyph_id(*ch).with_scale_and_position(px, point(*gx0, *gy0));
        let Some(o) = font.outline_glyph(glyph) else { continue }; // spaces -> no outline
        let bb = o.px_bounds();
        let (w, h) = (bb.width().ceil() as usize + 1, bb.height().ceil() as usize + 1);
        let mut cov = vec![0f32; w * h];
        o.draw(|dx, dy, c| {
            let (x, y) = (dx as usize, dy as usize);
            if x < w && y < h {
                cov[y * w + x] = c;
            }
        });
        for yy in (0..h).step_by(stride) {
            for xx in (0..w).step_by(stride) {
                let c = cov[yy * w + xx];
                if c < 0.35 {
                    continue; // coverage threshold → clean edges
                }
                // cheap deterministic jitter inside the cell (no rng dep)
                let j = |k: u32| ((k.wrapping_mul(2_654_435_761) >> 8) & 0xff) as f32 / 255.0 - 0.5;
                let gpx = bb.min.x + xx as f32 + j(i) * stride as f32;
                let gpy = bb.min.y + yy as f32 + j(i ^ 0x9e37) * stride as f32;
                i = i.wrapping_add(1);
                let wx = (gpx - cx) * scale;
                // keep glyph space Y-DOWN; the entity's cloud_base_rotation (flip X by π)
                // turns it upright, exactly as it does for the Y-down .ply splats — so text
                // and splats share one transform and morph between each other cleanly.
                let wy = (gpy - cy) * scale;
                out.push(Gaussian3d {
                    position_visibility: [wx, wy, 0.0, 1.0].into(),
                    spherical_harmonic: sh,
                    rotation: [0.0, 0.0, 0.0, 1.0].into(),
                    scale_opacity: [splat, splat, splat, c].into(),
                });
            }
        }
    }
    out
}

/// "Assemble from a ball": lhs = each target gaussian scattered onto a fuzzy sphere shell
/// (so it flies from the ball to its glyph slot as the morph runs); rhs = the target itself.
/// Per-gaussian pairing (lhs[i]↔rhs[i]), so no shader bulge needed — the ball IS the lhs.
fn assemble_from_ball(target: &[Gaussian3d], shell_r: f32) -> (Vec<Gaussian3d>, Vec<Gaussian3d>) {
    let rhs = target.to_vec();
    let lhs: Vec<Gaussian3d> = target
        .iter()
        .enumerate()
        .map(|(idx, g)| {
            let k = idx as u32;
            let h = |s: u32| ((k.wrapping_mul(s) >> 8) & 0xffff) as f32 / 65535.0; // [0,1)
            // random direction on the sphere (z in [-1,1], azimuth), fuzzy radius
            let z = h(2_654_435_761) * 2.0 - 1.0;
            let a = h(40_503) * std::f32::consts::TAU;
            let rxy = (1.0 - z * z).max(0.0).sqrt();
            let r = shell_r * (0.45 + 0.55 * h(2_246_822_519));
            let p = Vec3::new(rxy * a.cos(), rxy * a.sin(), z) * r;
            let mut s = *g;
            s.position_visibility = [p.x, p.y, p.z, 1.0].into();
            s
        })
        .collect();
    (lhs, rhs)
}

/// SEQUENCE mode: once every referenced splat has loaded, build each beat's shape
/// (resampled to the fixed count), spawn ONE interpolate entity, and frame the union.
fn build_sequence(
    mut commands: Commands,
    mut assets: ResMut<Assets<PlanarGaussian3d>>,
    seq: Option<Res<Sequence>>,
    state: Option<ResMut<SeqState>>,
    mut cam: Query<&mut OrbitCam>,
) {
    let (Some(seq), Some(mut state)) = (seq, state) else { return };
    if state.built || seq.beats.is_empty() {
        return;
    }
    if state.loads.iter().any(|h| assets.get(h).is_none()) {
        return; // wait for every referenced splat
    }

    let n = seq.count;
    let mut shapes = Vec::new();
    let mut union_lo = Vec3::splat(f32::MAX);
    let mut union_hi = Vec3::splat(f32::MIN);
    for beat in &seq.beats {
        let raw: Vec<Gaussian3d> = match &beat.content {
            BeatContent::Text(s) => {
                let rgb = [0.40_f32, 0.85, 1.0].map(|c| c * 0.8); // glowing cyan
                build_text_gaussians(s, rgb, 3.0, 2, 0.012)
            }
            BeatContent::Splats(list) => {
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
        };
        for g in &raw {
            let p = Vec3::from_array(g.position_visibility.position);
            union_lo = union_lo.min(p);
            union_hi = union_hi.max(p);
        }
        shapes.push(assets.add(PlanarGaussian3d::from(resample_morton(raw, n))));
    }

    let entity = commands
        .spawn((
            GaussianInterpolate::<Gaussian3d> {
                lhs: PlanarGaussian3dHandle(shapes[0].clone()),
                rhs: PlanarGaussian3dHandle(shapes[0].clone()),
            },
            CloudSettings {
                sort_mode: SortMode::Radix,
                time: 1.0,
                time_start: 0.0,
                time_stop: 1.0,
                bulge: 0.0,
                ..default()
            },
            Transform::from_rotation(cloud_base_rotation()), // Y-down text + Y-down plys → upright
        ))
        .id();

    // frame the union of all beats once (so the camera never pops between beats); apply
    // the same flip to the center so the camera looks at the post-transform world center.
    let center = cloud_base_rotation() * ((union_lo + union_hi) * 0.5);
    let radius = ((union_hi - union_lo) * 0.5).length().max(0.1);
    for mut c in &mut cam {
        c.center = center;
        c.radius = radius * 1.7;
        c.elevation = radius * 0.2;
        c.framed = true;
    }

    state.shapes = shapes;
    state.entity = Some(entity);
    state.built = true;
    info!("sequence built: {} beats × {n} gaussians", state.shapes.len());
}

/// Drive the sequence from SeqClock.t: pick the active beat, set the interpolate entity's
/// lhs/rhs (only on change), blend factor, and ball bulge.
fn beat_director(
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
    let beats = &seq.beats;

    // beat 0 = hold only; beat i≥1 = morph-in (morph_i) then hold_i.
    let mut t = clock.t;
    let (mut idx, mut morphing, mut factor) = (0usize, false, 1.0_f32);
    if t >= beats[0].hold {
        t -= beats[0].hold;
        let mut done = false;
        for i in 1..beats.len() {
            let seg = beats[i].morph + beats[i].hold;
            if t < seg {
                idx = i;
                if t < beats[i].morph {
                    morphing = true;
                    factor = (t / beats[i].morph.max(1e-3)).clamp(0.0, 1.0);
                }
                done = true;
                break;
            }
            t -= seg;
        }
        if !done {
            idx = beats.len() - 1; // clamp at the end
        }
    }

    let (lhs_i, rhs_i) = if idx == 0 { (0, 0) } else { (idx - 1, idx) };
    if interp.lhs.0.id() != state.shapes[lhs_i].id() {
        interp.lhs = PlanarGaussian3dHandle(state.shapes[lhs_i].clone());
    }
    if interp.rhs.0.id() != state.shapes[rhs_i].id() {
        interp.rhs = PlanarGaussian3dHandle(state.shapes[rhs_i].clone());
    }
    let eased = factor * factor * (3.0 - 2.0 * factor);
    cs.time = if idx == 0 { 1.0 } else { eased };
    cs.bulge = if morphing { beats[idx].bulge } else { 0.0 };
}

/// Live clock advance (record path drives SeqClock itself, deterministically).
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

/// Add NoFrustumCulling to the sequence entity once its Aabb exists (so morph/ball
/// particles that briefly leave the framed view don't pop out).
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

/// Deterministic recorder for sequences: total duration = beat0.hold + Σ(morph+hold), one
/// PNG per frame, front sway, then exit.
fn seq_record_driver(
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
        return;
    }
    let dur = seq.beats[0].hold
        + seq.beats.iter().skip(1).map(|b| b.morph + b.hold).sum::<f32>()
        + 1.0; // tail
    let total = (dur / rec.dt).ceil() as u32;
    if rec.i >= total {
        rec.grace += 1;
        if rec.grace > 30 {
            info!("sequence recording complete: {} frames -> {dir}", rec.i);
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

/// MORPH mode: once the source + target splats have loaded, assemble the corresponded
/// `lhs`/`rhs` clouds and spawn the single GaussianInterpolate entity that renders the
/// blend. Runs every frame but no-ops after the one-time build.
fn build_morph(
    mut commands: Commands,
    mut assets: ResMut<Assets<PlanarGaussian3d>>,
    morph: Option<ResMut<MorphAssets>>,
) {
    let Some(mut morph) = morph else {
        return;
    };
    if morph.built {
        return;
    }
    if assets.get(&morph.target).is_none()
        || morph.sources.iter().any(|(h, _)| assets.get(h).is_none())
    {
        return; // wait until every source + target splat has loaded
    }

    let dog: Vec<Gaussian3d> = assets.get(&morph.target).unwrap().iter().collect();
    if dog.is_empty() {
        return;
    }
    // combine the source splats into one set, baking each one's side-by-side offset
    // into its positions so at t=0 they appear where the two Martins stood.
    let mut src: Vec<Gaussian3d> = Vec::new();
    for (h, off) in &morph.sources {
        for mut g in assets.get(h).unwrap().iter() {
            let p = g.position_visibility.position;
            g.position_visibility.position = [p[0] + off.x, p[1] + off.y, p[2] + off.z];
            src.push(g);
        }
    }
    let (n_src, n_dog) = (src.len(), dog.len());
    // DOGDEMO_MORPH_COUNT=<n>: output gaussian count. Default 0 = max input (~1.15M):
    // crisp full-density Martins ("the full million-plus blobbies"). Lower values trade
    // density for speed on the iGPU (500k ≈ 40fps, 250k ≈ locked 60fps).
    let target = std::env::var("DOGDEMO_MORPH_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0usize);
    let (lhs, rhs) = corresponded(src, dog, target);
    let lhs_h = assets.add(PlanarGaussian3d::from(lhs));
    let rhs_h = assets.add(PlanarGaussian3d::from(rhs));
    // DOGDEMO_BULGE=<r>: ball-cloud radius scale at the morph midpoint, in units of the
    // object radius (0 = clean "puzzle-box" reorder, ~0.9 = ball ≈ object-sized). The
    // ball stays compact on purpose, for real-time performance.
    let bulge = std::env::var("DOGDEMO_BULGE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.9);
    commands.spawn((
        GaussianInterpolate::<Gaussian3d> {
            lhs: PlanarGaussian3dHandle(lhs_h),
            rhs: PlanarGaussian3dHandle(rhs_h),
        },
        // GPU radix sort (NOT Rayon): the interpolate compute writes the morphed
        // positions to the GPU buffer only — the CPU-side copy stays at lhs (Martins),
        // so a CPU sort orders by stale positions → wrong back-to-front blend → dark
        // holes. Radix sorts the live GPU positions. Safe here: it's a single cloud (the
        // multi-cloud view-uniform desync that forced Rayon doesn't apply).
        CloudSettings {
            sort_mode: SortMode::Radix,
            time: 0.0,
            time_start: 0.0,
            time_stop: 1.0,
            bulge,
            ..default()
        },
        Transform::from_rotation(cloud_base_rotation()),
    ));

    morph.built = true;
    morph.sources.clear(); // drop the source/target handles → free their assets
    morph.target = Handle::default();
    info!("morph built: {n_src} source gaussians -> {n_dog} target gaussians");
}

/// Drive the morph blend factor from the master clock: ease 0→1 over MORPH_DUR so the
/// Martins flow into the dog (then hold). Mirrors reform_zoom's timing.
fn drive_morph(
    explode: Res<ExplodeState>,
    has_reform: Res<HasReform>,
    mut q: Query<&mut CloudSettings, With<GaussianInterpolate<Gaussian3d>>>,
) {
    if !has_reform.0 {
        return;
    }
    let tau = if explode.active { explode.t } else { 0.0 };
    let raw = (tau / MORPH_DUR).clamp(0.0, 1.0);
    let eased = raw * raw * (3.0 - 2.0 * raw); // smoothstep
    for mut s in &mut q {
        s.time = eased; // time_start=0, time_stop=1 → interpolation factor = eased
    }
}
