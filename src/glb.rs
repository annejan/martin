//! `MARTIN_GLB=<file.glb>`: load + render a **`KHR_gaussian_splatting`** glTF scene — the standard
//! glTF container for splats (e.g. a TRELLIS single-image→3DGS export, or martin's own scene export).
//!
//! martin's fork drops the `.glb`/`.gltf` extension claim (CHANGES.md §7) so Bevy's native `GltfLoader`
//! owns those for `model:` PBR meshes. The crate's `GaussianSceneLoader` still exists; we invoke it
//! **explicitly by asset type** — a typed `load::<GaussianScene>()` picks the loader by its output type,
//! not the extension, so §7 doesn't get in the way. Spawning a `GaussianSceneHandle` then lets the
//! crate's own `spawn_scene` system instantiate each cloud bundle as a child (`PlanarGaussian3dHandle`
//! + `CloudSettings` + transform), which renders through martin's normal splat pipeline (bloom + sort).
//!
//! No morph/sequence — a standalone scene view. `MARTIN_GLB_SCALE` (default 1.0) sizes the scene and
//! `MARTIN_GLB_DIST` (default 5.0) the orbit distance; the free-orbit camera circles the origin.

use bevy::prelude::*;
use bevy_gaussian_splatting::{GaussianScene, GaussianSceneHandle, PlanarGaussian3dHandle};

use crate::camera::OrbitCam;
use crate::scene::file_name_of;
use crate::scene::sequence::SeqState;

/// Spawn the scene handle once; the crate's `spawn_scene` instantiates the clouds when it's ready.
fn spawn_glb_scene(mut commands: Commands, asset_server: Res<AssetServer>, mut done: Local<bool>) {
    if *done {
        return;
    }
    let Ok(path) = std::env::var("MARTIN_GLB") else {
        return;
    };
    *done = true;
    let scale = env_f32("MARTIN_GLB_SCALE", 1.0);
    let handle: Handle<GaussianScene> = asset_server.load(file_name_of(&path));
    commands.spawn((
        GaussianSceneHandle(handle),
        Transform::from_scale(Vec3::splat(scale)),
    ));
    info!("glb: loading KHR_gaussian_splatting scene {path} (scale {scale})");
}

/// Once the crate has spawned the scene's clouds, frame the camera and mark the show "built" so the
/// recorder starts (the standalone glb path has no morph sequence to do that). In glb-only mode the
/// only `PlanarGaussian3dHandle` entities are the scene's clouds, so their presence == ready.
fn glb_ready(
    clouds: Query<(), With<PlanarGaussian3dHandle>>,
    state: Option<ResMut<SeqState>>,
    mut camq: Query<&mut OrbitCam>,
    mut done: Local<bool>,
) {
    if *done || clouds.is_empty() {
        return;
    }
    if let Some(mut state) = state {
        state.built = true; // record_driver gate (empty sequence never sets this itself)
    }
    let dist = env_f32("MARTIN_GLB_DIST", 5.0);
    for mut c in &mut camq {
        c.target = Vec3::ZERO;
        c.dist = dist;
        c.framed = true;
    }
    info!("glb: scene ready, camera framed (dist {dist})");
    *done = true;
}

fn env_f32(key: &str, default: f32) -> f32 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Active only when `MARTIN_GLB` is set — otherwise martin's normal sequence/compose path runs.
pub(crate) struct GlbScenePlugin;

impl Plugin for GlbScenePlugin {
    fn build(&self, app: &mut App) {
        if std::env::var_os("MARTIN_GLB").is_some() {
            app.add_systems(Startup, spawn_glb_scene)
                .add_systems(Update, glb_ready);
        }
    }
}
