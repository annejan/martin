//! Optional fullscreen **background shader** layer (`MARTIN_BG=<mode>`): a custom-material quad
//! parented to the camera (so it tracks the view) at the far plane, opaque — the transparent splats
//! (bloom on black) blend straight over it. Fed time / beat / aspect, so the demoscene classic — a
//! plasma, a raymarched tunnel, a starfield — runs behind the morphing splats, beat-reactive. The
//! WGSL lives in `assets/bg.wgsl` (a `mode` uniform switches effects; edit it / add your own).

use bevy::asset::{load_internal_asset, uuid_handle};
use bevy::pbr::{Material, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::{Shader, ShaderRef};

/// `bg.wgsl` is **embedded** (a fixed handle), not loaded from the asset root — because that root is
/// the show's `.ply` folder (which is `assets/` only by default; an `austin_run/exports/` capture
/// breaks a relative load). Same reason the fonts are `include_bytes`'d (see `text.rs`).
const BG_SHADER: Handle<Shader> = uuid_handle!("b9d1c0a2-7e54-4f3a-9c21-0a1b2c3d4e5f");

use crate::scene::SeqClock;
use crate::scene::beat::Beat;
use crate::scene::sequence::{SeqState, Sequence, active_shot};

/// The 16:9 record/window aspect — fed to the effect uniform and used to size the fullscreen quad.
pub(crate) const ASPECT: f32 = 16.0 / 9.0;

/// Uniform block for a fullscreen WGSL effect, shared by the background layer (`bg.wgsl`) and the
/// `shader:` interlude (`scene::shader_part`, `shader_part.wgsl`) — both feed the same std140 layout
/// (a 16-byte scalar slot + a vec4). ONE type so the Rust and WGSL sides can't drift out of sync.
#[derive(ShaderType, Clone, Default)]
pub(crate) struct FxUniform {
    pub time: f32,
    pub mode: u32,
    pub aspect: f32,
    /// Output multiplier the shader applies to its colour: the background uses it as `MARTIN_BG_DIM`
    /// brightness (default 1.0), the interlude as its fade-to-black alpha (0 at the edges of the part).
    pub level: f32,
    pub beat: Vec4, // x=kick y=snare z=hat w=intensity
}

/// The fullscreen quad that fills the default-FOV (π/4) frustum at distance `d`, with a little
/// overscan. Both effect layers parent one of these (opaque, at the far plane) to the camera.
pub(crate) fn camera_fill_quad(d: f32) -> Rectangle {
    let h = 2.0 * d * std::f32::consts::FRAC_PI_8.tan() * 1.06;
    Rectangle::new(h * ASPECT, h)
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct BgMaterial {
    #[uniform(0)]
    data: FxUniform,
}

impl Material for BgMaterial {
    fn fragment_shader() -> ShaderRef {
        BG_SHADER.into() // embedded (root-independent); see BG_SHADER above
    }
}

#[derive(Component)]
struct BgQuad;

/// The show-wide default mode: `MARTIN_BG` if set, else hidden until a part's `bg:` token speaks.
#[derive(Resource)]
struct BgDefault(Option<u32>);

/// Sentinel for `bg:off` — hide the background layer entirely (a part on pure black).
pub(crate) const BG_OFF: u32 = u32::MAX;

/// Named modes → the `mode` uniform `bg.wgsl` switches on (a number also works: `MARTIN_BG=2`).
/// Shared with the shader-part interlude (`scene::shader_part`), which uses the same effect set,
/// and with the per-part `bg:<name>` seq token (`bg:off` → `BG_OFF`).
pub(crate) fn bg_token(name: &str) -> u32 {
    if name.eq_ignore_ascii_case("off") {
        BG_OFF
    } else {
        mode_index(name)
    }
}

pub(crate) fn mode_index(name: &str) -> u32 {
    match name.trim().to_ascii_lowercase().as_str() {
        "plasma" => 0,
        "tunnel" => 1,
        "stars" | "starfield" => 2,
        "warp" => 3,
        "rings" => 4,
        "grid" => 5,
        "kaleido" | "kaleidoscope" => 6,
        "bolt" | "lightning" => 7,
        other => other.parse().unwrap_or_else(|_| {
            warn!("shader effect '{other}' unknown — using plasma (try plasma/tunnel/stars/warp/rings/grid/kaleido/bolt)");
            0
        }),
    }
}

/// Spawn the background quad once, as a child of the camera so it tracks the view. Sized to fill the
/// default-FOV frustum at a far distance (opaque → the splats render over it). Spawned when
/// `MARTIN_BG` is set OR any seq part carries a `bg:` token; starts hidden without a default mode.
fn spawn_bg(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<BgMaterial>>,
    cam: Query<Entity, With<Camera3d>>,
    seq: Option<Res<Sequence>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let env_mode = std::env::var("MARTIN_BG").ok().map(|s| mode_index(&s));
    let seq_uses_bg = seq
        .map(|s| s.parts.iter().any(|p| p.bg.is_some()))
        .unwrap_or(false);
    if env_mode.is_none() && !seq_uses_bg {
        *done = true; // no background anywhere in this show — nothing to spawn
        return;
    }
    let Ok(cam) = cam.single() else { return };
    let d = 90.0_f32; // far plane, behind the interlude layer (88) so the interlude wins when active
    let dim = std::env::var("MARTIN_BG_DIM")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let mat = mats.add(BgMaterial {
        data: FxUniform {
            mode: env_mode.unwrap_or(0),
            aspect: ASPECT,
            level: dim,
            ..default()
        },
    });
    let quad = commands
        .spawn((
            Mesh3d(meshes.add(camera_fill_quad(d))),
            MeshMaterial3d(mat),
            Transform::from_xyz(0.0, 0.0, -d), // local -Z = in front of the camera, facing it
            if env_mode.is_some() {
                Visibility::Visible
            } else {
                Visibility::Hidden // wait for the first part with a bg: token
            },
            BgQuad,
        ))
        .id();
    commands.entity(cam).add_child(quad);
    commands.insert_resource(BgDefault(env_mode));
    *done = true;
    info!(
        "background: shader layer behind the splats (default mode {env_mode:?}, per-part bg: overrides)"
    );
}

/// Feed the show clock + beat into the background material each frame, and resolve the ACTIVE
/// mode: the last `bg:` token at-or-before the active part wins (sticky), else the `MARTIN_BG`
/// default, else hidden. `bg:off` hides the layer for that stretch.
fn update_bg(
    clock: Res<SeqClock>,
    beat: Res<Beat>,
    default_mode: Option<Res<BgDefault>>,
    seq: Option<Res<Sequence>>,
    state: Option<Res<SeqState>>,
    mut mats: ResMut<Assets<BgMaterial>>,
    mut q: Query<(&MeshMaterial3d<BgMaterial>, &mut Visibility), With<BgQuad>>,
) {
    let mut mode = default_mode.and_then(|d| d.0);
    if let (Some(seq), Some(state)) = (seq, state)
        && state.built
    {
        let active = active_shot(&state.starts(), clock.t);
        if let Some(m) = seq.parts[..=active.min(seq.parts.len().saturating_sub(1))]
            .iter()
            .rev()
            .find_map(|p| p.bg)
        {
            mode = Some(m);
        }
    }
    for (h, mut vis) in &mut q {
        *vis = match mode {
            Some(m) if m != BG_OFF => Visibility::Visible,
            _ => Visibility::Hidden,
        };
        if let Some(m) = mats.get_mut(&h.0) {
            if let Some(md) = mode
                && md != BG_OFF
            {
                m.data.mode = md;
            }
            m.data.time = clock.t;
            m.data.beat = beat.as_vec4();
        }
    }
}

/// The background shader layer — active when `MARTIN_BG` is set or a seq part uses `bg:<name>`.
pub(crate) struct BackgroundPlugin;

impl Plugin for BackgroundPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, BG_SHADER, "../assets/bg.wgsl", Shader::from_wgsl);
        app.add_plugins(MaterialPlugin::<BgMaterial>::default())
            .add_systems(Update, (spawn_bg, update_bg));
    }
}

#[cfg(test)]
mod tests {
    use super::mode_index;

    #[test]
    fn mode_index_maps_names_aliases_numbers_and_unknown() {
        assert_eq!(mode_index("plasma"), 0);
        assert_eq!(mode_index("warp"), 3);
        assert_eq!(mode_index("rings"), 4);
        assert_eq!(mode_index("kaleidoscope"), 6); // alias
        assert_eq!(mode_index("lightning"), 7); // alias for bolt
        assert_eq!(mode_index("5"), 5); // a number passes through
        assert_eq!(mode_index("wat"), 0); // unknown → plasma
    }
}
