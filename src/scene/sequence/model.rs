//! Data types of the morph-timeline + the pure cue-timeline functions.
//!
//! A show is a list of `Shot`s that each assemble in from a source cloud (ball/fade/explode/… or a
//! per-particle shader transition) and then hold, morphing into the next. `SeqState` holds the
//! loaded handles + the per-shot built shapes (one `BuiltShot` per shot). The cue-timeline fns
//! (`active_shot`/`shot_starts`/`show_end`) derive absolute shot start/end times — shared by the
//! builder, the director, the camera, and the `MARTIN_VALIDATE` dry-run.

use bevy::prelude::*;
use bevy_gaussian_splatting::{PlanarGaussian3d, RasterizeMode};

use crate::scene::content::PartContent;
use crate::scene::effects::{Deform, Departure, Transition};

/// One shot morphs in from the previous (or, for shot 0, from a ball), then holds.
#[derive(Clone)]
pub(crate) struct Shot {
    pub content: PartContent,
    pub hold: f32,                      // seconds held after arriving
    pub morph: f32,                     // seconds to morph in
    pub bulge: f32,                     // ball-pulse explosiveness (Morph transition only)
    pub transition: Option<Transition>, // None = default (Ball for shot 0, else Morph)
    pub anchor: Option<f32>,            // absolute start (s) on the music clock; None = relative
    pub deform: Option<Deform>, // persistent deform while held (None = none / MARTIN_DEFORM)
    pub out: Option<Departure>, // how the shot LEAVES (`out:name`); None = cross-morph to the next
    pub rot: Option<Quat>,      // per-shot orientation (`rot:rx,ry,rz` deg), baked into the shape
    pub cluster: Option<usize>, // `cluster:N` → N scattered, randomly-rotated copies (a "serving")
    pub bg: Option<u32>,        // `bg:<name>` → background mode from this shot on (BG_OFF hides)
    pub raster: Option<RasterizeMode>, // `raster:<mode>` → debug shading for this shot (None = MARTIN_RASTER)
}

/// The whole show: a list of shots + the gaussian budget every shot is resampled to.
#[derive(Resource)]
pub(crate) struct Sequence {
    pub parts: Vec<Shot>,
    pub budget: usize,
}

/// MARTIN_FLASH=<strength>: over-bright bloom pulse on each shot cut (0 = off, the default).
#[derive(Resource)]
pub(crate) struct FlashStrength(pub f32);

/// One fully-built shot: every per-frame input the director needs, collapsed into one record (vs the
/// old index-parallel `Vec`s). Built by `build_sequence`; consumed by `shot_director`.
pub(crate) struct BuiltShot {
    pub shape: Handle<PlanarGaussian3d>,
    pub origin: Option<Handle<PlanarGaussian3d>>, // lhs source cloud (None = morph from prev shape)
    pub out_cloud: Option<Handle<PlanarGaussian3d>>, // `out:` departure cloud (None = none)
    pub transition: Transition,
    pub deform: Option<Deform>,
    pub raster: RasterizeMode,
    pub start: f32, // absolute start time (s) of this shot
    // copied from the source Shot so the director needs only BuiltShot. (`hold` lives only on the
    // source `Shot` — the cue timeline reads it there via `show_end`; the director never needs it.)
    pub morph: f32,
    pub bulge: f32,
    pub out: Option<Departure>,
    pub is_gl_mesh: bool, // was `matches!(shot.content, PartContent::GlMesh(_))`
}

/// Loaded splat handles + the per-shot built shots (each resampled to the budget, with its
/// morph-in origin cloud + resolved transition/deform/raster + cue start).
#[derive(Resource)]
pub(crate) struct SeqState {
    pub load_names: Vec<String>,
    pub loads: Vec<Handle<PlanarGaussian3d>>,
    pub shots: Vec<BuiltShot>,
    pub built: bool,
    pub entity: Option<Entity>,
}

impl SeqState {
    /// Each built shot's absolute start time (s) — the cue timeline, collected for the readers that
    /// want a flat slice (the cut-flash loop, `gl_mesh_alpha`, the shader-interlude window).
    pub(crate) fn starts(&self) -> Vec<f32> {
        self.shots.iter().map(|s| s.start).collect()
    }
}

/// Index of the active shot at time `t`: the last shot whose absolute start (from the cue
/// timeline — `@@anchor` or laid end-to-end) has arrived. Shared by `shot_director` and `flypath`.
pub(crate) fn active_shot(starts: &[f32], t: f32) -> usize {
    let mut idx = 0;
    for (i, &start) in starts.iter().enumerate() {
        if t >= start {
            idx = i;
        } else {
            break;
        }
    }
    idx
}

/// Absolute start time (s) of each shot: its `@@anchor` (locked to the music clock) if set, else
/// laid end-to-end after the previous shot (`prev.start + prev.morph + prev.hold`). The cue
/// timeline — shared by `build_sequence` and the `MARTIN_VALIDATE` dry-run.
pub(crate) fn shot_starts(parts: &[Shot]) -> Vec<f32> {
    let mut starts = Vec::with_capacity(parts.len());
    let mut cursor = 0.0_f32;
    for (i, part) in parts.iter().enumerate() {
        let start = part.anchor.unwrap_or(if i == 0 { 0.0 } else { cursor });
        starts.push(start);
        cursor = start + part.morph + part.hold;
    }
    starts
}

/// End of the cue timeline: the latest shot's `start + morph + hold` (anchors can push it past a
/// simple sum). The recorder uses this (+ a tail) for the clip length; `flypath` spreads the
/// camera path across it while recording.
pub(crate) fn show_end(parts: &[Shot], starts: &[f32]) -> f32 {
    parts
        .iter()
        .zip(starts)
        .map(|(p, &start)| start + p.morph + p.hold)
        .fold(0.0_f32, f32::max)
}
