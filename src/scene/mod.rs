//! The scene: shared content + the two ways to stage it — the morph `sequence` timeline and the
//! `compose` stage. `ScenePlugin` wires both, the shared show clock, and the splat loader.

use std::f32::consts::PI;

use bevy::prelude::*;
use bevy_gaussian_splatting::PlanarGaussian3d;

pub mod beat;
pub mod caption;
pub mod colorize;
pub mod compose;
pub mod content;
pub mod effects;
pub mod gl_dissolve;
pub mod sequence;
pub mod shader_part;
pub mod spectrum;

use compose::Composition;
use content::PartContent;
use sequence::{SeqState, Sequence};

use crate::capture::RecordState;

const NORMALIZE_EXTENT: f32 = 2.0; // each part is centered + scaled so its largest dim = this

/// Warn when a scene's estimated PEAK resident gaussian count is high enough to risk a GPU OOM on the
/// dev iGPU (Radeon 860M OOMs ~2.5M; safe ~1.2M). The build holds every reel shot's clouds (shape +
/// `origin`/`exit` source clouds) AND every compose prop (+ its entrance source cloud) resident at
/// once — so a big `budget`/`MARTIN_MORPH_COUNT` × many parts silently crosses the line and a long
/// record dies mid-render with a wgpu buffer "Validation Error" (no panic, frames just stop). This is
/// a pure heads-up logged at build (before the slow render) so the author can lower the count first.
/// Soft cap defaults to 2.0M (the 860M renders ~2M fine but OOMs ~2.5M, so this flags the danger zone
/// without false-alarming working scenes); override with `MARTIN_SPLAT_WARN`.
pub(crate) fn warn_splat_budget(label: &str, splats: usize) {
    let cap = crate::envvar::or("MARTIN_SPLAT_WARN", 2_000_000usize);
    if splats > cap {
        warn!(
            "{label}: ~{:.2}M gaussians resident — over the {:.2}M soft cap (MARTIN_SPLAT_WARN). A long \
             record may OOM the GPU mid-render; lower MARTIN_MORPH_COUNT or the .show `budget`.",
            splats as f32 / 1e6,
            cap as f32 / 1e6,
        );
    } else {
        info!(
            "{label}: ~{:.2}M gaussians resident (cap {:.2}M)",
            splats as f32 / 1e6,
            cap as f32 / 1e6
        );
    }
}

/// The env vars that request specific content. With none of them set (and no `MARTIN_SHOW`), martin
/// plays the bundled intro. (`MARTIN_PLY` is NOT here — it's only the asset ROOT now, not content, so a
/// bare `MARTIN_PLY` still falls through to the intro.)
pub(crate) const CONTENT_VARS: [&str; 4] = [
    "MARTIN_SEQ",
    "MARTIN_COMPOSE",
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

/// `MARTIN_SORT_BITS` (16|24|32) → the radix depth-key width for the per-frame GPU sort. Fewer bits =
/// fewer LSD digit passes (16→2, 24→3, 32→4), so a cheaper sort every frame — at the cost of coarser
/// depth buckets. Synthetic morph/text content has shallow depth complexity and rarely reorders
/// visibly at 16/24; real captures can mis-order near-coplanar splats, so the default stays Bits32.
pub(crate) fn sort_depth_bits() -> bevy_gaussian_splatting::RadixSortDepthBits {
    sort_bits_of(crate::envvar::or("MARTIN_SORT_BITS", 32u32))
}

/// Map a `MARTIN_SORT_BITS` width (16|24|32) to the radix depth enum; any other value → Bits32 (the
/// safe default). Split out so the mapping is unit-testable without touching the env.
fn sort_bits_of(n: u32) -> bevy_gaussian_splatting::RadixSortDepthBits {
    use bevy_gaussian_splatting::RadixSortDepthBits as B;
    match n {
        16 => B::Bits16,
        24 => B::Bits24,
        _ => B::Bits32,
    }
}

#[cfg(test)]
mod tests {
    use bevy_gaussian_splatting::RadixSortDepthBits as B;

    #[test]
    fn sort_bits_maps_known_widths_else_default_32() {
        assert_eq!(super::sort_bits_of(16), B::Bits16);
        assert_eq!(super::sort_bits_of(24), B::Bits24);
        assert_eq!(super::sort_bits_of(32), B::Bits32);
        assert_eq!(super::sort_bits_of(17), B::Bits32, "unknown → safe default");
        assert_eq!(super::sort_bits_of(0), B::Bits32);
    }
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
    shot: Option<Res<crate::capture::ShotConfig>>,
    mut clock: ResMut<SeqClock>,
    // MARTIN_LOOP=1 wraps the clock at show-end → the reel plays forever (a live kiosk loop). Read
    // once; `None` = not looping, `Some(len)` = wrap modulo len (only when a reel/Score gives a length).
    mut loop_len: Local<Option<Option<f32>>>,
    // MARTIN_HOLD_T pin (profiling): cached once; outer None = unread, inner None = not set.
    mut hold_t: Local<Option<Option<f32>>>,
) {
    if rec.dir.is_some() {
        return;
    }
    // the live control bridge can freeze the clock to inspect a moment (seek sets it directly).
    if paused.map(|p| p.0).unwrap_or(false) {
        return;
    }
    // MARTIN_SHOT mode SEEKS the clock to the shot time (shot_driver) — don't tick it forward here,
    // or a late shot would slowly sim its way there instead of jumping.
    if shot.map(|s| s.path.is_some()).unwrap_or(false) {
        return;
    }
    // hold for the live synth render (when audio is wanted): picture + music must leave together
    // from t=0 or every @@ anchor lands out of sync. No gate when muted/recording/pre-rendered.
    if gate.map(|g| !g.ready).unwrap_or(false) {
        return;
    }
    // advance once the show is up — the morph sequence OR the composition stage.
    let built = state.map(|s| s.built).unwrap_or(false) || comp.map(|c| c.built).unwrap_or(false);
    // MARTIN_HOLD_T=<s>: PIN the live timeline at a fixed time (don't advance) — a profiling aid so a
    // windowed perf sweep renders byte-identical content every frame (the real-time advance otherwise
    // makes a faster GPU sample a different moment, confounding count/res/scale comparisons). Read once.
    let hold_t = *hold_t.get_or_insert_with(|| crate::envvar::opt::<f32>("MARTIN_HOLD_T"));
    if let Some(ht) = hold_t {
        if built {
            clock.t = ht;
        }
        return;
    }
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
    root: Res<AssetRoot>,
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
    // Procedural shapes (galaxy/knot/lsystem/…) are GENERATED here if missing, rather than shipped:
    // a fresh clone, and the single-binary bundle (which no longer bakes them), synthesize them into
    // the asset root on first launch — the loader covers the ~sub-second cost. Real captures are left
    // alone (→ they fail loudly at load if absent, as before).
    let made = crate::splatgen::ensure_splats(&root.0, &names);
    if !made.is_empty() {
        info!("generated {} procedural splat(s): {made:?}", made.len());
    }
    let loads = names
        .iter()
        .map(|n| asset_server.load::<PlanarGaussian3d>(n.clone()))
        .collect();
    commands.insert_resource(SeqState {
        load_names: names,
        loads,
        shots: Vec::new(),
        starts: Vec::new(),
        built: false,
        entity: None,
    });
}

/// The morph timeline + the composition stage + the shared show clock and splat loader.
pub(crate) struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        beat::plugin(app); // score-driven beat pulses, read by the directors below
        spectrum::plugin(app); // FFT of the rendered track → band table, read by the bg + interlude layers
        app.init_resource::<SeqClock>()
            .init_resource::<sequence::SeqBuildTask>()
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
