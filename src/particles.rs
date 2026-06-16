//! Additive **particle layer** (`MARTIN_PARTICLES=embers`): glowing warm embers drifting up through
//! the scene, in front of / among the splats — a second, independent motion layer that reads as
//! instant "demo energy". The HDR `Bloom` makes the additive points glow.
//!
//! RADV-safe by construction: a transparent/Blend/Add **custom-shader** material crashes the splat
//! render pipeline on RADV (see `scene::shader_part`), so this uses a plain `StandardMaterial`
//! (`AlphaMode::Add`, unlit, emissive) on real quad geometry — the same class `gl_dissolve` blends
//! over the splats successfully — with the motion done CPU-side per entity. Soft round glow comes
//! from a generated radial-gradient texture (no shader needed).
//!
//! Deterministic / record-safe: each ember's position is a pure function of its fixed seed + the show
//! clock (`SeqClock.t`), and the screen-facing billboard copies the (frame-deterministic) camera
//! rotation. No RNG per frame, no wall-clock — a recording bakes identically.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::scene::SeqClock;

/// One ember — its fixed per-particle seed (3 pseudo-random scalars baked once).
#[derive(Component)]
struct Ember {
    s: [f32; 3],
}

/// Half-extent of the box the embers drift within (around the normalized content at the origin).
const FIELD: f32 = 2.6;

/// Spawn the ember field once, at startup, when `MARTIN_PARTICLES` is set. `MARTIN_PARTICLE_COUNT`
/// sets the number (default 200); they share one mesh + one material + one gradient texture.
fn spawn_particles(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    if std::env::var_os("MARTIN_PARTICLES").is_none() {
        return;
    }
    let count = crate::envvar::or("MARTIN_PARTICLE_COUNT", 200usize).clamp(1, 5000);

    // Soft round glow: a small radial-gradient texture (white centre → black edge). Sampled by the
    // quad's UVs; black edges add nothing under AlphaMode::Add, so a square quad reads as a round dot.
    let gradient = images.add(radial_glow(32));
    let quad = meshes.add(Rectangle::new(0.12, 0.12));
    let mat = mats.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.55, 0.2), // warm ember
        base_color_texture: Some(gradient),
        emissive: LinearRgba::rgb(2.5, 1.1, 0.35), // HDR → blooms
        unlit: true,
        alpha_mode: AlphaMode::Add, // additive (StandardMaterial → RADV-safe, unlike a custom material)
        ..default()
    });

    for i in 0..count {
        let s = [
            hash01(i as u32 * 3),
            hash01(i as u32 * 3 + 1),
            hash01(i as u32 * 3 + 2),
        ];
        commands.spawn((
            Mesh3d(quad.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(ember_pos(s, 0.0)),
            Ember { s },
        ));
    }
    info!("particles: {count} additive embers (MARTIN_PARTICLES)");
}

/// Each frame: place every ember from its seed + the clock, and billboard it to face the camera.
fn animate_particles(
    clock: Res<SeqClock>,
    cam: Query<&Transform, (With<Camera3d>, Without<Ember>)>,
    mut q: Query<(&Ember, &mut Transform), Without<Camera3d>>,
) {
    let Ok(cam) = cam.single() else { return };
    let face = cam.rotation; // screen-aligned billboard (deterministic: camera pose is clock-driven)
    let t = clock.t;
    for (e, mut tf) in &mut q {
        tf.translation = ember_pos(e.s, t);
        tf.rotation = face;
    }
}

/// An ember's position from its seed `s` and clock `t`: rises in +Y (wrapping through the box) with a
/// per-particle speed, and sways gently in x/z. Pure — no RNG, no wall-clock.
fn ember_pos(s: [f32; 3], t: f32) -> Vec3 {
    let span = 2.0 * FIELD;
    let speed = 0.25 + 0.6 * s[0];
    let y = (s[1] * span + t * speed).rem_euclid(span) - FIELD;
    let x = (s[2] * span - FIELD) + (t * 0.5 + s[0] * 6.2831853).sin() * 0.25;
    let z = (s[0] * span - FIELD) + (t * 0.4 + s[1] * 6.2831853).cos() * 0.25;
    Vec3::new(x, y, z)
}

/// Deterministic 0..1 hash (integer → scalar) — the per-particle seed source.
fn hash01(n: u32) -> f32 {
    let mut x = n.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    x ^= x >> 17;
    x = x.wrapping_mul(0xed5a_d4bb);
    x ^= x >> 11;
    (x & 0xff_ffff) as f32 / 0xff_ffff as f32
}

/// A square RGBA texture with a soft radial falloff (white centre → black edge) for round glows.
fn radial_glow(size: u32) -> Image {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    let c = (size as f32 - 1.0) * 0.5;
    for y in 0..size {
        for x in 0..size {
            let dx = (x as f32 - c) / c;
            let dy = (y as f32 - c) / c;
            let d = (dx * dx + dy * dy).sqrt();
            let v = (1.0 - d).clamp(0.0, 1.0);
            let g = (v * v * 255.0) as u8; // squared falloff — a tight soft core
            data.extend_from_slice(&[g, g, g, g]);
        }
    }
    let mut img = Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = ImageSampler::linear();
    img
}

/// The additive ember layer — spawns when `MARTIN_PARTICLES` is set, drifts deterministically.
pub(crate) struct ParticlesPlugin;

impl Plugin for ParticlesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_particles)
            .add_systems(Update, animate_particles);
    }
}
