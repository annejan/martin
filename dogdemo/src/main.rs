//! dogdemo — Phase 0/1/2: load a Gaussian splat, orbit it, and explode it.
//!
//! Controls:
//!   Up / Down    : zoom in / out
//!   Left / Right : lower / raise the camera
//!   Space        : trigger / reset the explosion
//!
//! Phase 2 MVP (this file): a GPU-side "puff" — the whole cloud expands from its
//! centroid (Transform scale) and fades (CloudSettings.global_opacity), driven by
//! ExplodeState.t. No per-frame re-upload. The ballistic per-Gaussian version
//! (shader fork) layers on top of these same levers next.
//!
//! Debug envs: DOGDEMO_SHOT=/path.png captures the framebuffer at t=4.5s and exits;
//! DOGDEMO_EXPLODE=1 auto-triggers the explosion at t=2s (for headless capture).

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
    CloudSettings, GaussianCamera, GaussianSplattingPlugin, PlanarGaussian3dHandle,
};
use bevy_gaussian_splatting::sort::SortMode;
use std::f32::consts::PI;

/// Tuning for the MVP puff.
const EXPAND_RATE: f32 = 1.5; // cloud scale = 1 + EXPAND_RATE * t
const FRONT_YAW: f32 = 1.4; // camera faces both Martins head-on (single-image splats have no back)
const SWAY: f32 = 0.25; // gentle left-right sway amplitude — never reaches the hollow back

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
    /// Intact until `start`, then COLLAPSES inward (particles + whole cloud drift to
    /// the world center) and fades — feeding the reform, not blasting outward.
    Explode { start: f32, home: Vec3 },
    /// Invisible + scattered until `start`, then particles fly IN and fade in over
    /// `dur` seconds, coalescing into the shape at the center.
    Reform { start: f32, dur: f32, scatter: f32 },
}

#[derive(Component, Clone, Copy)]
struct CloudAnim(AnimRole);

struct CloudCfg {
    name: String,
    offset: Vec3,
    role: AnimRole,
}

/// Splats to load (DOGDEMO_PLY / _PLY2 explode; DOGDEMO_REFORM reforms in place).
#[derive(Resource)]
struct Clouds(Vec<CloudCfg>);

/// True when a reformer is present → camera returns to frame after the blast.
#[derive(Resource)]
struct HasReform(bool);

/// DOGDEMO_YAW=<rad>: pin the camera to a fixed orbit angle (for diagnosing which
/// way a splat faces). None → normal spin/sway.
#[derive(Resource)]
struct CamOverride(Option<f32>);

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
    let has_reform = reform_name.is_some();
    let mut clouds: Vec<CloudCfg> = Vec::new();
    match std::env::var("DOGDEMO_PLY2").ok() {
        // two splats → staggered, side by side; they collapse INWARD to center
        Some(p2) => {
            let l = Vec3::new(-SEP, 0.0, 0.0);
            let r = Vec3::new(SEP, 0.0, 0.0);
            clouds.push(CloudCfg { name: name1, offset: l, role: AnimRole::Explode { start: 0.5, home: l } });
            clouds.push(CloudCfg { name: file_name_of(&p2), offset: r, role: AnimRole::Explode { start: 1.0, home: r } });
        }
        None => clouds.push(CloudCfg { name: name1, offset: Vec3::ZERO, role: AnimRole::Explode { start: 0.0, home: Vec3::ZERO } }),
    }
    // DOGDEMO_REFORM: one dog reforms at center, overlapping the Martins' collapse.
    if let Some(dog) = &reform_name {
        clouds.push(CloudCfg {
            name: dog.clone(),
            offset: Vec3::ZERO,
            role: AnimRole::Reform { start: 1.2, dur: 3.0, scatter: 1.5 },
        });
    }

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

    App::new()
        .add_plugins(plugins)
        .add_plugins(GaussianSplattingPlugin)
        .insert_resource(Clouds(clouds))
        .insert_resource(HasReform(has_reform))
        .insert_resource(CamOverride(
            std::env::var("DOGDEMO_YAW").ok().and_then(|s| s.parse().ok()),
        ))
        .insert_resource(ClearColor(Color::BLACK))
        .init_resource::<ExplodeState>()
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
            (frame_on_load, controls, orbit_camera, animate_clouds, debug_driver, record_driver),
        )
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>, clouds: Res<Clouds>) {
    for cfg in &clouds.0 {
        commands.spawn((
            PlanarGaussian3dHandle(asset_server.load(cfg.name.clone())),
            // CPU (rayon) depth sort: GPU radix sort desyncs the view uniform with
            // multiple clouds → half the splats get culled. CPU sort dodges that.
            CloudSettings { sort_mode: SortMode::Rayon, ..default() },
            Transform::from_translation(cfg.offset).with_rotation(cloud_base_rotation()),
            CloudAnim(cfg.role),
        ));
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
    clouds_res: Res<Clouds>,
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
    if loaded.len() < clouds_res.0.len() {
        return; // wait until every splat has loaded (Aabb present)
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

/// Reform-sequence camera zoom (continuous at t=0): martins framed → pull back
/// for the blast → zoom IN on the reformed central dog (settle 1.0→0.55).
fn reform_zoom(t: f32) -> f32 {
    // pure smooth zoom-IN (no pull-back bump): frame the two Martins (1.0) and ease
    // in to the central dog as everything converges (~0.6). No lurch/shake.
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
        reform_zoom(if explode.active { explode.t } else { 0.0 })
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

/// MVP explosion: expand the cloud from its centroid and fade it out, GPU-side
/// (Transform + CloudSettings.global_opacity) — no per-frame re-upload.
/// Drive each cloud's per-Gaussian displacement (gaussian.wgsl reads CloudSettings.time)
/// + opacity/scale from the master clock `tau`, per its role. Exploders fly apart and
/// fade; the reformer runs its explosion backwards (scatter→0) and fades in.
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
            AnimRole::Reform { start, dur, scatter } => {
                if tau <= start {
                    s.time = scatter; // mildly scattered near center…
                    s.global_opacity = 0.0; // …invisible until it starts reforming
                } else {
                    let p = ((tau - start) / dur).clamp(0.0, 1.0);
                    s.time = scatter * (1.0 - p); // particles converge to the formed shape
                    s.global_opacity = p; // fade in as it coalesces
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
    mut camq: Query<&mut OrbitCam>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
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
