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
use bevy::camera::primitives::Aabb;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::render::view::Hdr;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::camera::visibility::NoFrustumCulling;
use bevy_gaussian_splatting::{
    CloudSettings, GaussianCamera, GaussianSplattingPlugin, PlanarGaussian3dHandle,
};
use std::f32::consts::PI;

/// Tuning for the MVP puff.
const EXPAND_RATE: f32 = 1.5; // cloud scale = 1 + EXPAND_RATE * t
const FADE_RATE: f32 = 0.15; // opacity = 1 - FADE_RATE * t

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
        Self { center: Vec3::ZERO, radius: 5.0, elevation: 1.5, yaw: 0.0, framed: false }
    }
}

#[derive(Resource, Default)]
struct ExplodeState {
    active: bool,
    t: f32,
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

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "dogdemo — splat fly-around".into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(GaussianSplattingPlugin)
        .insert_resource(ClearColor(Color::BLACK))
        .init_resource::<ExplodeState>()
        .insert_resource(Debug {
            shot: std::env::var("DOGDEMO_SHOT").ok(),
            shot_at: std::env::var("DOGDEMO_SHOT_AT").ok().and_then(|s| s.parse().ok()).unwrap_or(4.5),
            auto_explode: std::env::var("DOGDEMO_EXPLODE").is_ok(),
            shot_done: false,
            exploded: false,
        })
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (frame_on_load, controls, orbit_camera, apply_explosion, debug_driver),
        )
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn((
        PlanarGaussian3dHandle(asset_server.load("aegg.ply")),
        CloudSettings::default(),
        Transform::from_rotation(cloud_base_rotation()),
    ));

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
    cloud: Query<(Entity, &Aabb, &Transform), With<PlanarGaussian3dHandle>>,
    mut cam: Query<&mut OrbitCam>,
) {
    let Ok((cloud_entity, aabb, cloud_tf)) = cloud.single() else {
        return;
    };
    for mut c in &mut cam {
        if c.framed {
            continue;
        }
        let center = cloud_tf.transform_point(Vec3::from(aabb.center));
        let bounding_radius = Vec3::from(aabb.half_extents).length().max(0.001);
        c.center = center;
        c.radius = bounding_radius * 1.5;
        c.elevation = bounding_radius * 0.25;
        c.framed = true;
        // Aabb now exists (the crate skips Aabb-insertion for NoFrustumCulling entities),
        // so disable entity-level culling so the expanding blast never pops out of view.
        commands.entity(cloud_entity).insert(NoFrustumCulling);
        info!(
            "framed cloud: center={center:?}  bounding_radius={bounding_radius:.3}  camera_radius={:.3}",
            c.radius
        );
    }
}

fn orbit_camera(explode: Res<ExplodeState>, mut q: Query<(&mut Transform, &OrbitCam)>) {
    // Pull back as the cloud expands, but slower than it grows, so the blast stays in frame.
    let zoom = if explode.active {
        1.0 + EXPAND_RATE * explode.t * 0.6
    } else {
        1.0
    };
    for (mut tf, cam) in &mut q {
        let r = cam.radius * zoom;
        let offset = Vec3::new(r * cam.yaw.cos(), cam.elevation * zoom, r * cam.yaw.sin());
        tf.translation = cam.center + offset;
        tf.look_at(cam.center, Vec3::Y);
    }
}

fn controls(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut explode: ResMut<ExplodeState>,
    mut q: Query<&mut OrbitCam>,
) {
    let dt = time.delta_secs();
    for mut cam in &mut q {
        cam.yaw += dt * 0.5;
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
fn apply_explosion(
    explode: Res<ExplodeState>,
    mut cloud: Query<&mut CloudSettings, With<PlanarGaussian3dHandle>>,
) {
    let Ok(mut settings) = cloud.single_mut() else {
        return;
    };
    if explode.active {
        let t = explode.t;
        settings.time = t; // drives the per-Gaussian ballistic displacement in gaussian.wgsl
        settings.global_scale = 1.0 + 0.3 * t; // splats fatten as they fly → smokier
        settings.global_opacity = (1.0 - FADE_RATE * t).max(0.0);
    } else {
        settings.time = 0.0; // exact reset to the original pose (displacement is a no-op at t=0)
        settings.global_scale = 1.0;
        settings.global_opacity = 1.0;
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
