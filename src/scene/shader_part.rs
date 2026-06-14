//! Shader interludes (`shader:` sequence parts): a fullscreen WGSL effect that plays full-frame
//! between splat scenes. The part's gaussians are a transparent placeholder (so the splats clear and
//! the morph chain stays valid), and this module shows a quad — parented to the camera, in front of
//! the splats — running `assets/shader_part.wgsl`, faded in/out across the part's time window.

use bevy::asset::{load_internal_asset, uuid_handle};
use bevy::pbr::{Material, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::{Shader, ShaderRef};

/// Embedded (root-independent) — the asset root is the show's `.ply` folder; see `background.rs`.
const SHADER_PART: Handle<Shader> = uuid_handle!("c0e2d1b3-8f65-4a2b-8d12-1b2c3d4e5f60");

use crate::background::{ASPECT, FxUniform, camera_fill_quad, mode_index};
use crate::scene::SeqClock;
use crate::scene::beat::Beat;
use crate::scene::content::PartContent;
use crate::scene::sequence::{SeqState, Sequence, show_end};

const FADE: f32 = 0.6; // interlude fade in/out time (s) at each edge of the part window

#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct ShaderPartMaterial {
    #[uniform(0)]
    data: FxUniform, // shared with the background layer; `level` = this part's fade alpha
}

impl Material for ShaderPartMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER_PART.into()
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
    let d = 88.0_f32; // far plane, just in front of the background layer (90) so it wins when active
    for (part, p) in seq.parts.iter().enumerate() {
        let PartContent::Shader(name) = &p.content else {
            continue;
        };
        let mat = mats.add(ShaderPartMaterial {
            data: FxUniform {
                mode: mode_index(name),
                aspect: ASPECT,
                ..default()
            },
        });
        let quad = commands
            .spawn((
                Mesh3d(meshes.add(camera_fill_quad(d))),
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
    let starts = state.starts();
    for (sp, h, mut vis) in &mut q {
        let end = starts
            .get(sp.part + 1)
            .copied()
            .unwrap_or_else(|| show_end(&seq.parts, &starts));
        let alpha = interlude_alpha(&starts, end, sp.part, clock.t);
        *vis = if alpha > 0.001 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if let Some(m) = mats.get_mut(&h.0) {
            m.data.time = clock.t;
            m.data.level = alpha;
            m.data.beat = beat.as_vec4();
        }
    }
}

/// The shader-interlude layer — registers the material + its spawn/drive systems.
pub(crate) struct ShaderPartPlugin;

impl Plugin for ShaderPartPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(
            app,
            SHADER_PART,
            "../../assets/shader_part.wgsl",
            Shader::from_wgsl
        );
        app.add_plugins(MaterialPlugin::<ShaderPartMaterial>::default())
            .add_systems(Update, (spawn_shader_parts, update_shader_parts));
    }
}
