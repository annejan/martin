//! dogdemo — Phase 0/1: load a Gaussian splat (.ply) and orbit a camera around it.
//!
//! Controls:
//!   Up / Down    : zoom in / out
//!   Left / Right : lower / raise the camera
//!   Space        : toggle "explode" state (Phase 2 stub)
//!
//! The camera AUTO-FRAMES the splat once it loads (reads the cloud's `Aabb`).
//!
//! Debug: set DOGDEMO_SHOT=/path/frame.png to grab the rendered framebuffer at
//! t=5s and exit (captures exactly what the camera sees, no window manager).

use bevy::prelude::*;
use bevy::app::AppExit;
use bevy::camera::primitives::Aabb;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy_gaussian_splatting::{
    CloudSettings, GaussianCamera, GaussianSplattingPlugin, PlanarGaussian3dHandle,
};

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
        Self {
            center: Vec3::ZERO,
            radius: 5.0,
            elevation: 1.5,
            yaw: 0.0,
            framed: false,
        }
    }
}

#[derive(Resource, Default)]
struct ExplodeState {
    active: bool,
    t: f32,
}

/// Debug auto-screenshot driven by the DOGDEMO_SHOT env var.
#[derive(Resource)]
struct AutoShot {
    path: Option<String>,
    done: bool,
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
        .init_resource::<ExplodeState>()
        .insert_resource(AutoShot {
            path: std::env::var("DOGDEMO_SHOT").ok(),
            done: false,
        })
        .add_systems(Startup, setup)
        .add_systems(Update, (frame_on_load, orbit_camera, controls, auto_screenshot))
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn((
        PlanarGaussian3dHandle(asset_server.load("aegg.ply")),
        CloudSettings::default(),
    ));
    commands.spawn((
        GaussianCamera { warmup: true }, // REQUIRED: splat sort/draw systems only run for cameras with this marker
        Camera3d::default(),
        Tonemapping::None,
        Transform::default(),
        OrbitCam::default(),
    ));
}

/// Once the cloud's `Aabb` exists, frame the camera to its bounding sphere.
fn frame_on_load(cloud: Query<&Aabb, With<PlanarGaussian3dHandle>>, mut cam: Query<&mut OrbitCam>) {
    let Ok(aabb) = cloud.single() else {
        return;
    };
    for mut c in &mut cam {
        if c.framed {
            continue;
        }
        let center = Vec3::from(aabb.center);
        let bounding_radius = Vec3::from(aabb.half_extents).length().max(0.001);
        c.center = center;
        c.radius = bounding_radius * 2.5;
        c.elevation = bounding_radius * 0.6;
        c.framed = true;
        info!(
            "framed cloud: center={center:?}  bounding_radius={bounding_radius:.3}  camera_radius={:.3}",
            c.radius
        );
    }
}

fn orbit_camera(mut q: Query<(&mut Transform, &OrbitCam)>) {
    for (mut tf, cam) in &mut q {
        let offset = Vec3::new(
            cam.radius * cam.yaw.cos(),
            cam.elevation,
            cam.radius * cam.yaw.sin(),
        );
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
        info!("explode -> {} (Phase 2 hook)", explode.active);
    }
    if explode.active {
        explode.t += dt;
    }
}

/// Debug: at t=5s, capture the rendered frame to DOGDEMO_SHOT and exit.
fn auto_screenshot(
    time: Res<Time>,
    mut shot: ResMut<AutoShot>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = shot.path.clone() else {
        return;
    };
    let t = time.elapsed_secs();
    if !shot.done && t >= 5.0 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
        shot.done = true;
        info!("auto-screenshot -> {path}");
    }
    if shot.done && t >= 7.0 {
        exit.write(AppExit::Success);
    }
}
