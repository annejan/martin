//! Additive **particle layer** (`MARTIN_PARTICLES=embers|confetti|sparks|fireworks`): glowing points
//! drifting through the scene, in front of / among the splats — a second, independent motion layer
//! that reads as instant "demo energy". The HDR `Bloom` makes the additive points glow. The KIND is
//! the env value: `embers` (warm sparks rising, the default), `confetti` (tumbling coloured flakes
//! falling), `sparks` (fast hot specks bursting from a core), `fireworks` (rise-then-bloom shells).
//! All beat-reactive — the kick scatters/pops them (embers stay calm to keep legacy bakes identical).
//!
//! RADV-safe by construction: a transparent/Blend/Add **custom-shader** material crashes the splat
//! render pipeline on RADV (see `scene::shader_part`), so this uses plain `StandardMaterial`
//! (`AlphaMode::Add`, unlit, emissive) on real quad geometry — the same class `gl_dissolve` blends
//! over the splats successfully — with the motion done CPU-side per entity. Soft round glow comes
//! from a generated radial-gradient texture (no shader needed).
//!
//! Deterministic / record-safe: each particle's transform is a pure function of its fixed seed + the
//! show clock (`SeqClock.t`) + the clock-driven beat envelope, and the billboard copies the
//! (frame-deterministic) camera rotation. No RNG per frame, no wall-clock — a recording bakes identically.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::scene::SeqClock;
use crate::scene::beat::Beat;

/// Which particle effect. Parsed from the `MARTIN_PARTICLES` VALUE; anything unrecognised (incl. the
/// legacy `MARTIN_PARTICLES=1`/`embers`) is `Embers`, so old shows are unchanged.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Embers,
    Confetti,
    Sparks,
    Fireworks,
}

impl Kind {
    fn parse(v: &str) -> Self {
        match v.trim().to_ascii_lowercase().as_str() {
            "confetti" => Kind::Confetti,
            "sparks" => Kind::Sparks,
            "fireworks" => Kind::Fireworks,
            _ => Kind::Embers,
        }
    }
}

/// One particle — its fixed per-particle seed (3 pseudo-random scalars baked once) + the effect kind.
#[derive(Component)]
struct Particle {
    s: [f32; 3],
    kind: Kind,
}

/// Half-extent of the box the particles live within (around the normalized content at the origin).
const FIELD: f32 = 2.6;
const PARTICLE_FADE: f32 = 1.5; // glow ramp-in duration (s)

/// `MARTIN_PARTICLE_AT=<secs|@@anchor>`: ramp the glow in starting at this show-time (a campfire's
/// sparks appear WITH the fire). Holds every material's full base/emissive so the ramp scales them.
#[derive(Resource)]
struct ParticleFade {
    mats: Vec<(Handle<StandardMaterial>, [f32; 3], [f32; 3])>, // (handle, base rgb, emissive rgb)
    start: f32,
}

/// The colour palette (base rgb, emissive rgb) for a kind. Embers = one warm spark; sparks = one hot
/// white-gold; fireworks = a few hot hues; confetti = a bright multi-hue palette (one per flake).
fn palette(kind: Kind) -> Vec<([f32; 3], [f32; 3])> {
    match kind {
        Kind::Embers => vec![([1.0, 0.55, 0.2], [2.5, 1.1, 0.35])],
        Kind::Sparks => vec![([1.0, 0.9, 0.55], [3.0, 2.4, 1.0])],
        Kind::Fireworks => vec![
            ([1.0, 0.3, 0.3], [3.0, 0.6, 0.6]),
            ([0.4, 0.6, 1.0], [0.8, 1.4, 3.0]),
            ([1.0, 0.9, 0.4], [3.0, 2.4, 0.8]),
            ([0.6, 1.0, 0.6], [1.2, 3.0, 1.2]),
        ],
        Kind::Confetti => vec![
            ([1.0, 0.25, 0.35], [1.6, 0.4, 0.55]),
            ([0.3, 0.7, 1.0], [0.5, 1.1, 1.6]),
            ([1.0, 0.85, 0.3], [1.6, 1.35, 0.5]),
            ([0.5, 1.0, 0.5], [0.8, 1.6, 0.8]),
            ([0.85, 0.4, 1.0], [1.35, 0.6, 1.6]),
            ([1.0, 0.6, 0.85], [1.6, 0.95, 1.35]),
        ],
    }
}

/// Spawn the particle field once at startup when `MARTIN_PARTICLES` is set. `MARTIN_PARTICLE_COUNT`
/// sets the number (default 200); they share one mesh + one gradient texture + the kind's material(s).
fn spawn_particles(
    mut commands: Commands,
    score: Res<crate::music::ScoreRes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let kind = match std::env::var("MARTIN_PARTICLES") {
        Ok(v) if !v.is_empty() => Kind::parse(&v),
        _ => return, // unset → no particle layer (off by default)
    };
    let count = crate::envvar::or("MARTIN_PARTICLE_COUNT", 200usize).clamp(1, 5000);

    // Soft round glow: a small radial-gradient texture (white centre → black edge). Black edges add
    // nothing under AlphaMode::Add, so a square quad reads as a round dot.
    let gradient = images.add(radial_glow(32));
    let quad = meshes.add(Rectangle::new(0.12, 0.12));
    let pal = palette(kind);
    let handles: Vec<(Handle<StandardMaterial>, [f32; 3], [f32; 3])> = pal
        .iter()
        .map(|&(base, emit)| {
            let h = mats.add(StandardMaterial {
                base_color: Color::srgb(base[0], base[1], base[2]),
                base_color_texture: Some(gradient.clone()),
                emissive: LinearRgba::rgb(emit[0], emit[1], emit[2]), // HDR → blooms
                unlit: true,
                alpha_mode: AlphaMode::Add, // additive (StandardMaterial → RADV-safe)
                ..default()
            });
            (h, base, emit)
        })
        .collect();

    // When does the glow ramp in? `MARTIN_PARTICLE_AT` (seconds or `@@anchor`); unset → 0 (on from t0).
    let start = std::env::var("MARTIN_PARTICLE_AT")
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|s| score.0.anchor_seconds(s.trim_start_matches("@@")))
        .unwrap_or(0.0);
    commands.insert_resource(ParticleFade {
        mats: handles.clone(),
        start,
    });

    for i in 0..count {
        let s = [
            hash01(i as u32 * 3),
            hash01(i as u32 * 3 + 1),
            hash01(i as u32 * 3 + 2),
        ];
        let mat = handles[i % handles.len()].0.clone(); // confetti spreads across its palette
        commands.spawn((
            Mesh3d(quad.clone()),
            MeshMaterial3d(mat),
            Transform::from_translation(particle_xform(kind, s, 0.0, 0.0).0),
            Particle { s, kind },
        ));
    }
    let name = match kind {
        Kind::Embers => "embers",
        Kind::Confetti => "confetti",
        Kind::Sparks => "sparks",
        Kind::Fireworks => "fireworks",
    };
    info!("particles: {count} additive {name} (MARTIN_PARTICLES)");
}

/// Each frame: place + billboard every particle from its seed, the clock, and the beat. `burst` =
/// the clock-driven kick envelope (record-safe) — the kick scatters/pops the lively kinds.
fn animate_particles(
    clock: Res<SeqClock>,
    beat: Option<Res<Beat>>,
    cam: Query<&Transform, (With<Camera3d>, Without<Particle>)>,
    mut q: Query<(&Particle, &mut Transform), Without<Camera3d>>,
) {
    let Ok(cam) = cam.single() else { return };
    let face = cam.rotation; // screen-aligned billboard (deterministic: camera pose is clock-driven)
    let t = clock.t;
    let burst = beat.map(|b| b.kick * b.intensity).unwrap_or(0.0);
    for (p, mut tf) in &mut q {
        let (pos, spin, scale) = particle_xform(p.kind, p.s, t, burst);
        tf.translation = pos;
        tf.rotation = face * spin;
        tf.scale = Vec3::splat(scale);
    }
}

/// A particle's (position, extra-rotation, scale) from its seed `s`, clock `t` and kick `burst`. Pure
/// (no RNG, no wall-clock). Embers ignore `burst` so legacy embers bakes are byte-identical.
fn particle_xform(kind: Kind, s: [f32; 3], t: f32, burst: f32) -> (Vec3, Quat, f32) {
    use std::f32::consts::TAU;
    let span = 2.0 * FIELD;
    match kind {
        // rises in +Y (wrapping through the box) with a per-particle speed, sways in x/z — unchanged.
        Kind::Embers => {
            let speed = 0.25 + 0.6 * s[0];
            let y = (s[1] * span + t * speed).rem_euclid(span) - FIELD;
            let x = (s[2] * span - FIELD) + (t * 0.5 + s[0] * TAU).sin() * 0.25;
            let z = (s[0] * span - FIELD) + (t * 0.4 + s[1] * TAU).cos() * 0.25;
            (Vec3::new(x, y, z), Quat::IDENTITY, 1.0)
        }
        // tumbling coloured flakes FALLING (top→bottom, wraps); flutter widens + a kick-shove on the beat.
        Kind::Confetti => {
            let fall = 0.4 + 0.5 * s[0];
            let y = FIELD - (s[1] * span + t * fall).rem_euclid(span);
            let flut = 0.35 * (1.0 + burst);
            let x = (s[2] * span - FIELD) + (t * 1.3 + s[0] * TAU).sin() * flut;
            let z = (s[0] * span - FIELD) + (t * 1.1 + s[1] * TAU).cos() * flut;
            let axis = Vec3::new(s[0] - 0.5, s[1] - 0.5, s[2] - 0.5).normalize_or_zero();
            let axis = if axis == Vec3::ZERO { Vec3::Y } else { axis };
            let spin = Quat::from_axis_angle(axis, t * (2.0 + 4.0 * s[2]) + s[0] * TAU);
            (Vec3::new(x, y + burst * 0.4, z), spin, 1.0)
        }
        // fast hot specks: a short life loop, radial outward from a low core; fades + shrinks as it flies.
        Kind::Sparks => {
            let life = (t * (1.5 + s[0]) + s[1]).fract();
            let reach = (1.2 + 1.2 * s[2]) * (1.0 + burst);
            let dir =
                Vec3::new((s[0] * TAU).cos(), 0.3 + s[1], (s[2] * TAU).sin()).normalize_or_zero();
            let pos = Vec3::new(0.0, -FIELD * 0.5, 0.0) + dir * (life * reach);
            (pos, Quat::IDENTITY, (1.0 - life).max(0.0))
        }
        // rise-then-burst shells: launch up, then explode radially with a gravity droop; pops on the kick.
        Kind::Fireworks => {
            let phase = (t * (0.35 + 0.3 * s[0]) + s[1]).fract();
            let lx = (s[2] * 2.0 - 1.0) * FIELD * 0.7;
            let lz = (s[0] * 2.0 - 1.0) * FIELD * 0.7;
            let apex = Vec3::new(lx, -FIELD + FIELD * 1.2, lz);
            if phase < 0.4 {
                let rise = phase / 0.4;
                (
                    Vec3::new(lx, -FIELD + rise * FIELD * 1.2, lz),
                    Quat::IDENTITY,
                    0.5,
                )
            } else {
                let e = (phase - 0.4) / 0.6;
                let dir = Vec3::new(
                    (s[0] * TAU).cos() * (s[1] * std::f32::consts::PI).sin(),
                    (s[1] * std::f32::consts::PI).cos(),
                    (s[2] * TAU).sin() * (s[1] * std::f32::consts::PI).sin(),
                );
                let pos = apex + dir * (e * 2.2 * (1.0 + burst)) + Vec3::Y * (-2.0 * e * e);
                (pos, Quat::IDENTITY, (1.0 - e).max(0.0) * 1.3)
            }
        }
    }
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

/// The additive particle layer — spawns when `MARTIN_PARTICLES` is set, animates deterministically.
pub(crate) struct ParticlesPlugin;

/// Ramp the glow 0 → full over `PARTICLE_FADE`, starting at `ParticleFade.start` — so the sparks fade
/// in WITH the fire. No-op (full from t0) when `start <= 0`; stops once fully in. Ramps EVERY material
/// (confetti's palette too). Deterministic: a pure function of the show clock.
fn fade_particles(
    clock: Res<SeqClock>,
    ctl: Option<Res<ParticleFade>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut done: Local<bool>,
) {
    let Some(ctl) = ctl else { return };
    if *done || ctl.start <= 0.0 {
        return;
    }
    let x = ((clock.t - ctl.start) / PARTICLE_FADE).clamp(0.0, 1.0);
    let f = x * x * (3.0 - 2.0 * x); // smoothstep
    for (h, base, emit) in &ctl.mats {
        if let Some(m) = mats.get_mut(h) {
            m.base_color = Color::srgb(base[0] * f, base[1] * f, base[2] * f);
            m.emissive = LinearRgba::rgb(emit[0] * f, emit[1] * f, emit[2] * f);
        }
    }
    if x >= 1.0 {
        *done = true;
    }
}

impl Plugin for ParticlesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_particles)
            .add_systems(Update, (animate_particles, fade_particles));
    }
}
