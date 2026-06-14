//! The scene: shared content + the two ways to stage it — the morph `sequence` timeline and the
//! `compose` stage. `ScenePlugin` wires both, the shared show clock, and the splat loader.

use std::f32::consts::PI;

use bevy::prelude::*;
use bevy_gaussian_splatting::PlanarGaussian3d;

pub mod beat;
pub mod compose;
pub mod content;
pub mod effects;
pub mod gl_dissolve;
pub mod sequence;
pub mod shader_part;

use compose::Composition;
use content::PartContent;
use sequence::{SeqState, Sequence};

use crate::capture::RecordState;

const NORMALIZE_EXTENT: f32 = 2.0; // each part is centered + scaled so its largest dim = this

/// The env vars that request specific content. With none of them set (and no `MARTIN_SHOW`), martin
/// plays the bundled default demo (`assets/demo.show`).
pub(crate) const CONTENT_VARS: [&str; 8] = [
    "MARTIN_SEQ",
    "MARTIN_COMPOSE",
    "MARTIN_TEXT",
    "MARTIN_PLY",
    "MARTIN_PLY2",
    "MARTIN_REFORM",
    "MARTIN_GLB",
    "MARTIN_4D_TEST",
];

/// True when the user requested no specific content — the cue to play the default demo.
pub(crate) fn no_content_requested() -> bool {
    CONTENT_VARS.iter().all(|k| std::env::var(k).is_err())
}

/// `.ply` splats are Y-down → rotate the cloud 180° about X for Y-up. Text is built Y-down
/// too (see `build_text_gaussians`), so one transform makes text *and* splats upright.
pub(crate) fn cloud_base_rotation() -> Quat {
    Quat::from_rotation_x(PI)
}

pub(crate) fn file_name_of(p: &str) -> String {
    std::path::Path::new(p)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "aegg.ply".into())
}

pub(crate) fn parent_dir(p: String) -> Option<String> {
    std::path::Path::new(&p)
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.to_string_lossy().into_owned())
}

/// Folder that `image:` parts (PNG logos) are read from — the `.ply` asset root (default `assets`).
#[derive(Resource)]
pub(crate) struct AssetRoot(pub std::path::PathBuf);

/// Master timeline clock (seconds). Live: accumulates real time; record: frame×dt.
#[derive(Resource, Default)]
pub(crate) struct SeqClock {
    pub t: f32,
}

/// Live clock advance (record mode drives `SeqClock` itself, deterministically).
#[allow(clippy::too_many_arguments)]
fn advance_seq_clock(
    time: Res<Time>,
    rec: Res<RecordState>,
    state: Option<Res<SeqState>>,
    comp: Option<Res<Composition>>,
    seq: Option<Res<crate::scene::sequence::Sequence>>,
    gate: Option<Res<crate::music::AudioGate>>,
    paused: Option<Res<crate::serve::Paused>>,
    mut clock: ResMut<SeqClock>,
    // MARTIN_LOOP=1 wraps the clock at show-end → the reel plays forever (a live kiosk loop). Read
    // once; `None` = not looping, `Some(len)` = wrap modulo len (only when a reel/Score gives a length).
    mut loop_len: Local<Option<Option<f32>>>,
) {
    if rec.dir.is_some() {
        return;
    }
    // the live control bridge can freeze the clock to inspect a moment (seek sets it directly).
    if paused.map(|p| p.0).unwrap_or(false) {
        return;
    }
    // hold for the live synth render (when audio is wanted): picture + music must leave together
    // from t=0 or every @@ anchor lands out of sync. No gate when muted/recording/pre-rendered.
    if gate.map(|g| !g.ready).unwrap_or(false) {
        return;
    }
    // advance once the show is up — the morph sequence OR the composition stage.
    let built = state.map(|s| s.built).unwrap_or(false) || comp.map(|c| c.built).unwrap_or(false);
    if built {
        clock.t += time.delta_secs();
        // resolve the loop length once the reel exists: the cue-timeline end (last shot + hold).
        let len = *loop_len.get_or_insert_with(|| {
            (std::env::var("MARTIN_LOOP").is_ok())
                .then(|| {
                    seq.as_ref().map(|s| {
                        let starts = crate::scene::sequence::shot_starts(&s.parts);
                        crate::scene::sequence::show_end(&s.parts, &starts)
                    })
                })
                .flatten()
                .filter(|&l| l > 0.0)
        });
        if let Some(len) = len {
            if clock.t >= len {
                clock.t -= len; // seamless modulo wrap → the reel restarts (shot 0 from its origin)
            }
        }
    }
}

/// Startup: load every referenced splat (by filename in the asset folder) from both the morph
/// sequence and the composition stage; `build_sequence` / `build_composition` assemble the shapes
/// once they're all available.
fn load_splats(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    seq: Res<Sequence>,
    comp: Res<Composition>,
) {
    let mut names: Vec<String> = Vec::new();
    let add = |content: &PartContent, names: &mut Vec<String>| {
        if let PartContent::Splats(list) = content {
            for (n, _) in list {
                if !names.contains(n) {
                    names.push(n.clone());
                }
            }
        }
    };
    for b in &seq.parts {
        add(&b.content, &mut names);
    }
    for o in &comp.objects {
        add(o.content(), &mut names);
    }
    let loads = names
        .iter()
        .map(|n| asset_server.load::<PlanarGaussian3d>(n.clone()))
        .collect();
    commands.insert_resource(SeqState {
        load_names: names,
        loads,
        shots: Vec::new(),
        built: false,
        entity: None,
    });
}

/// The morph timeline + the composition stage + the shared show clock and splat loader.
pub(crate) struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        beat::plugin(app); // score-driven beat pulses, read by the directors below
        app.init_resource::<SeqClock>()
            .insert_resource(sequence::FlashStrength(
                std::env::var("MARTIN_FLASH")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.0),
            ))
            .add_systems(Startup, load_splats)
            .add_systems(
                Update,
                (
                    sequence::build_sequence,
                    sequence::shot_director,
                    sequence::seq_no_cull,
                    gl_dissolve::sample_gl_mesh,
                    gl_dissolve::animate_seq_model,
                    advance_seq_clock,
                    compose::build_composition,
                    compose::animate_composition,
                    compose::compose_camera,
                ),
            );
    }
}
