//! **Experimental** 4D gaussian splat smoke test (`MARTIN_4D_TEST=<n>`): spawn the crate's
//! procedural 4D test cloud (n gaussians, default 4096) with looping temporal playback, to prove
//! the 4D render path (gaussian_4d.wgsl conditional-covariance + spherindrical harmonics) works on
//! this wgpu/RADV stack. A 4D gaussian = a 3D gaussian + a time-centre (`t`) and time-variance
//! (`st`); each frame the shader conditions on `CloudSettings.time`, so gaussians fade in/out and
//! drift as time advances — splat *video*, continuous in time. Real 4D content needs a multi-view
//! video pipeline (4d-gaussian-splatting / EasyVolcap → `.ply4d`); this just validates rendering.

use bevy::prelude::*;
use bevy_gaussian_splatting::gaussian::settings::PlaybackMode;
use bevy_gaussian_splatting::{
    CloudSettings, GaussianMode, PlanarGaussian4d, PlanarGaussian4dHandle,
    random_gaussians_4d_seeded,
};

use crate::camera::OrbitCam;
use crate::scene::sequence::SeqState;

fn spawn_4d_test(
    mut commands: Commands,
    mut clouds: ResMut<Assets<PlanarGaussian4d>>,
    state: Option<ResMut<SeqState>>,
    mut camq: Query<&mut OrbitCam>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let n: usize = std::env::var("MARTIN_4D_TEST")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 1)
        .unwrap_or(4096);
    // seeded → deterministic in record mode
    let cloud = clouds.add(random_gaussians_4d_seeded(n, 42));
    commands.spawn((
        PlanarGaussian4dHandle(cloud),
        CloudSettings {
            gaussian_mode: GaussianMode::Gaussian4d,
            playback_mode: PlaybackMode::Loop,
            time_stop: 1.0,
            time_scale: 0.25, // one temporal sweep ≈ 4 s
            ..default()
        },
    ));
    // standalone harness: mark the show built + frame the camera (like glb.rs standalone mode)
    if let Some(mut state) = state {
        state.built = true;
    }
    for mut c in &mut camq {
        c.target = Vec3::ZERO;
        c.dist = 3.0;
        c.framed = true;
    }
    info!("4d: spawned {n}-gaussian 4D test cloud (looping playback)");
    *done = true;
}

/// Experimental: only active when `MARTIN_4D_TEST` is set (standalone, like glb-alone mode).
pub(crate) struct FourDTestPlugin;

impl Plugin for FourDTestPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var_os("MARTIN_4D_TEST").is_some() {
            app.add_systems(Update, spawn_4d_test);
        }
    }
}
