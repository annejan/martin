//! Optional fullscreen **background shader** layer (`MARTIN_BG=<mode>`): a custom-material quad
//! parented to the camera (so it tracks the view) at the far plane, opaque — the transparent splats
//! (bloom on black) blend straight over it. Fed time / beat / aspect, so the demoscene classic — a
//! plasma, a raymarched tunnel, a starfield — runs behind the morphing splats, beat-reactive. The
//! WGSL lives in `assets/bg.wgsl` (a `mode` uniform switches effects; edit it / add your own).

use bevy::pbr::{Material, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use crate::scene::beat::Beat;
use crate::scene::SeqClock;

/// Uniform block fed to `bg.wgsl` (std140-packed: a 16-byte scalar slot + a vec4).
#[derive(ShaderType, Clone, Default)]
struct BgData {
    time: f32,
    mode: u32,
    aspect: f32,
    _pad: f32,
    beat: Vec4, // x=kick y=snare z=hat w=intensity
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct BgMaterial {
    #[uniform(0)]
    data: BgData,
}

impl Material for BgMaterial {
    fn fragment_shader() -> ShaderRef {
        "bg.wgsl".into() // loaded from the asset root; edit it / add modes
    }
}

#[derive(Component)]
struct BgQuad;

/// Named modes → the `mode` uniform `bg.wgsl` switches on (a number also works: `MARTIN_BG=2`).
/// Shared with the shader-part interlude (`scene::shader_part`), which uses the same effect set.
pub(crate) fn mode_index(name: &str) -> u32 {
    match name.trim().to_ascii_lowercase().as_str() {
        "plasma" => 0,
        "tunnel" => 1,
        "stars" | "starfield" => 2,
        "warp" => 3,
        other => other.parse().unwrap_or(0),
    }
}

/// Spawn the background quad once, as a child of the camera so it tracks the view. Sized to fill the
/// default-FOV frustum at a far distance (opaque → the splats render over it).
fn spawn_bg(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<BgMaterial>>,
    cam: Query<Entity, With<Camera3d>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Ok(cam) = cam.single() else { return };
    let mode = std::env::var("MARTIN_BG")
        .ok()
        .map(|s| mode_index(&s))
        .unwrap_or(0);
    let aspect = 16.0 / 9.0; // the 1280×720 record/window
                             // fill the default perspective FOV (π/4) at distance d, with a little overscan.
    let d = 90.0_f32;
    let h = 2.0 * d * (std::f32::consts::FRAC_PI_8).tan() * 1.06;
    let w = h * aspect;
    let mat = mats.add(BgMaterial {
        data: BgData {
            mode,
            aspect,
            ..default()
        },
    });
    let quad = commands
        .spawn((
            Mesh3d(meshes.add(Rectangle::new(w, h))),
            MeshMaterial3d(mat),
            Transform::from_xyz(0.0, 0.0, -d), // local -Z = in front of the camera, facing it
            BgQuad,
        ))
        .id();
    commands.entity(cam).add_child(quad);
    *done = true;
    info!("background: shader layer (mode {mode}) behind the splats");
}

/// Feed the show clock + beat into the background material each frame.
fn update_bg(
    clock: Res<SeqClock>,
    beat: Res<Beat>,
    mut mats: ResMut<Assets<BgMaterial>>,
    q: Query<&MeshMaterial3d<BgMaterial>, With<BgQuad>>,
) {
    for h in &q {
        if let Some(m) = mats.get_mut(&h.0) {
            m.data.time = clock.t;
            m.data.beat = Vec4::new(beat.kick, beat.snare, beat.hat, beat.intensity);
        }
    }
}

/// The background shader layer — only active when `MARTIN_BG` is set.
pub(crate) struct BackgroundPlugin;

impl Plugin for BackgroundPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var_os("MARTIN_BG").is_some() {
            app.add_plugins(MaterialPlugin::<BgMaterial>::default())
                .add_systems(Update, (spawn_bg, update_bg));
        }
    }
}
