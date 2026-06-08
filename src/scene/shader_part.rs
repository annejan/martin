//! Shader interludes (`shader:` sequence parts): a fullscreen WGSL effect that plays full-frame
//! between splat scenes. The part's gaussians are a transparent placeholder (so the splats clear and
//! the morph chain stays valid), and this module shows a quad — parented to the camera, in front of
//! the splats — running `assets/shader_part.wgsl`, faded in/out across the part's time window.

use bevy::pbr::{Material, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use crate::background::mode_index;
use crate::scene::beat::Beat;
use crate::scene::content::PartContent;
use crate::scene::sequence::{show_end, SeqState, Sequence};
use crate::scene::SeqClock;

const FADE: f32 = 0.6; // interlude fade in/out time (s) at each edge of the part window

/// Uniform fed to `shader_part.wgsl` (std140: a 16-byte scalar slot + a vec4).
#[derive(ShaderType, Clone, Default)]
struct FxData {
    time: f32,
    mode: u32,
    aspect: f32,
    alpha: f32, // fade across the part window (0 at edges, 1 while held)
    beat: Vec4, // x=kick y=snare z=hat w=intensity
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct ShaderPartMaterial {
    #[uniform(0)]
    data: FxData,
}

impl Material for ShaderPartMaterial {
    fn fragment_shader() -> ShaderRef {
        "shader_part.wgsl".into()
    }
    // Opaque at the far plane, exactly like the background layer (a transparent/Blend custom material
    // crashes the splat render pipeline on RADV). The effect fades to BLACK via col*alpha; the splats
    // render over it, so as a part's splats clear/return they crossfade naturally with the effect.
}

/// One interlude quad, tagged with the sequence part it plays on.
#[derive(Component)]
struct ShaderPart {
    part: usize,
}

/// The interlude's opacity at time `t`: ramp up over `FADE` after the part starts (the splats are
/// clearing then), hold, ramp down over `FADE` before the next part takes over.
fn interlude_alpha(starts: &[f32], end: f32, p: usize, t: f32) -> f32 {
    let start = starts[p];
    let fin = ((t - start) / FADE).clamp(0.0, 1.0);
    let fout = ((end - t) / FADE).clamp(0.0, 1.0);
    fin * fout
}

/// Once the sequence is built, spawn a hidden fullscreen quad (parented to the camera, just in front
/// of the splats) for every `shader:` part; `update_shader_parts` reveals + drives each in its window.
fn spawn_shader_parts(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<ShaderPartMaterial>>,
    seq: Option<Res<Sequence>>,
    state: Option<Res<SeqState>>,
    cam: Query<Entity, With<Camera3d>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let (Some(seq), Some(state), Ok(cam)) = (seq, state, cam.single()) else {
        return;
    };
    if !state.built {
        return;
    }
    let aspect = 16.0 / 9.0;
    let d = 88.0_f32; // far plane, just in front of the background layer (90) so it wins when active
    let h = 2.0 * d * (std::f32::consts::FRAC_PI_8).tan() * 1.06;
    let w = h * aspect;
    for (part, p) in seq.parts.iter().enumerate() {
        let PartContent::Shader(name) = &p.content else {
            continue;
        };
        let mat = mats.add(ShaderPartMaterial {
            data: FxData {
                mode: mode_index(name),
                aspect,
                ..default()
            },
        });
        let quad = commands
            .spawn((
                Mesh3d(meshes.add(Rectangle::new(w, h))),
                MeshMaterial3d(mat),
                Transform::from_xyz(0.0, 0.0, -d),
                Visibility::Hidden,
                ShaderPart { part },
            ))
            .id();
        commands.entity(cam).add_child(quad);
    }
    *done = true;
}

/// Drive each interlude quad: its fade alpha + time/beat, and show it only while on screen.
fn update_shader_parts(
    clock: Res<SeqClock>,
    beat: Res<Beat>,
    seq: Option<Res<Sequence>>,
    state: Option<Res<SeqState>>,
    mut mats: ResMut<Assets<ShaderPartMaterial>>,
    mut q: Query<(
        &ShaderPart,
        &MeshMaterial3d<ShaderPartMaterial>,
        &mut Visibility,
    )>,
) {
    let (Some(seq), Some(state)) = (seq, state) else {
        return;
    };
    if !state.built {
        return;
    }
    for (sp, h, mut vis) in &mut q {
        let end = state
            .starts
            .get(sp.part + 1)
            .copied()
            .unwrap_or_else(|| show_end(&seq.parts, &state.starts));
        let alpha = interlude_alpha(&state.starts, end, sp.part, clock.t);
        *vis = if alpha > 0.001 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if let Some(m) = mats.get_mut(&h.0) {
            m.data.time = clock.t;
            m.data.alpha = alpha;
            m.data.beat = Vec4::new(beat.kick, beat.snare, beat.hat, beat.intensity);
        }
    }
}

/// The shader-interlude layer — registers the material + its spawn/drive systems.
pub(crate) struct ShaderPartPlugin;

impl Plugin for ShaderPartPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<ShaderPartMaterial>::default())
            .add_systems(Update, (spawn_shader_parts, update_shader_parts));
    }
}
