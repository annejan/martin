//! dogdemo — Phase 0/1: load a Gaussian splat (.ply) and orbit a camera around it.
//!
//! Controls:
//!   Up / Down    : zoom in / out (orbit radius)
//!   Left / Right : lower / raise camera
//!   Space        : toggle "explode" state  (Phase 2 stub — will drive a
//!                  per-Gaussian compute/vertex-shader displacement)
//!
//! The splat is loaded from `assets/aegg.ply` (swap in a dog later).

use bevy::prelude::*;
use bevy_gaussian_splatting::{CloudSettings, GaussianSplattingPlugin, PlanarGaussian3dHandle};

#[derive(Component)]
struct OrbitCam {
    radius: f32,
    yaw: f32,
    height: f32,
}

#[derive(Resource, Default)]
struct ExplodeState {
    active: bool,
    /// seconds since explosion was triggered — Phase 2 feeds this to the shader.
    t: f32,
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
        .add_systems(Startup, setup)
        .add_systems(Update, (orbit_camera, controls))
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    // The gaussian cloud. CloudSettings + Visibility are auto-added by the plugin.
    commands.spawn((
        PlanarGaussian3dHandle(asset_server.load("aegg.ply")),
        CloudSettings::default(),
    ));

    // Orbiting camera.
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 1.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
        OrbitCam {
            radius: 5.0,
            yaw: 0.0,
            height: 1.0,
        },
    ));
}

/// Place the camera each frame from its orbit params (yaw is advanced in `controls`).
fn orbit_camera(mut q: Query<(&mut Transform, &OrbitCam)>) {
    for (mut tf, cam) in &mut q {
        let x = cam.radius * cam.yaw.cos();
        let z = cam.radius * cam.yaw.sin();
        tf.translation = Vec3::new(x, cam.height, z);
        tf.look_at(Vec3::ZERO, Vec3::Y);
    }
}

/// Keyboard: zoom/height, and the explode toggle stub.
fn controls(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut explode: ResMut<ExplodeState>,
    mut q: Query<&mut OrbitCam>,
) {
    let dt = time.delta_secs();
    for mut cam in &mut q {
        cam.yaw += dt * 0.5; // auto-spin
        if keys.pressed(KeyCode::ArrowUp) {
            cam.radius = (cam.radius - dt * 4.0).max(0.2);
        }
        if keys.pressed(KeyCode::ArrowDown) {
            cam.radius += dt * 4.0;
        }
        if keys.pressed(KeyCode::ArrowLeft) {
            cam.height -= dt * 3.0;
        }
        if keys.pressed(KeyCode::ArrowRight) {
            cam.height += dt * 3.0;
        }
    }

    // Phase 2 hook: this toggle will arm a compute/vertex shader that pushes each
    // Gaussian outward by velocity*t (+ gravity, + noise). For now it just tracks state.
    if keys.just_pressed(KeyCode::Space) {
        explode.active = !explode.active;
        explode.t = 0.0;
        info!(
            "explode -> {} (Phase 2: drive per-Gaussian displacement from ExplodeState.t)",
            explode.active
        );
    }
    if explode.active {
        explode.t += dt;
    }
}
