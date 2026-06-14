//! Data types of the morph-timeline + the pure cue-timeline functions.
//!
//! A show is a list of `Part`s that each assemble in from a source cloud (ball/fade/explode/… or a
//! per-particle shader transition) and then hold, morphing into the next. `SeqState` holds the
//! loaded handles + the per-part built shapes (index-parallel `Vec`s). The cue-timeline fns
//! (`active_part`/`part_starts`/`show_end`) derive absolute part start/end times — shared by the
//! builder, the director, the camera, and the `MARTIN_VALIDATE` dry-run.

use bevy::prelude::*;
use bevy_gaussian_splatting::{PlanarGaussian3d, RasterizeMode};

use crate::scene::content::PartContent;
use crate::scene::effects::{Deform, Departure, Transition};

/// One part morphs in from the previous (or, for part 0, from a ball), then holds.
#[derive(Clone)]
pub(crate) struct Part {
    pub content: PartContent,
    pub hold: f32,                      // seconds held after arriving
    pub morph: f32,                     // seconds to morph in
    pub bulge: f32,                     // ball-pulse explosiveness (Morph transition only)
    pub transition: Option<Transition>, // None = default (Ball for part 0, else Morph)
    pub anchor: Option<f32>,            // absolute start (s) on the music clock; None = relative
    pub deform: Option<Deform>, // persistent deform while held (None = none / MARTIN_DEFORM)
    pub out: Option<Departure>, // how the part LEAVES (`out:name`); None = cross-morph to the next
    pub rot: Option<Quat>,      // per-part orientation (`rot:rx,ry,rz` deg), baked into the shape
    pub cluster: Option<usize>, // `cluster:N` → N scattered, randomly-rotated copies (a "serving")
    pub bg: Option<u32>,        // `bg:<name>` → background mode from this part on (BG_OFF hides)
    pub raster: Option<RasterizeMode>, // `raster:<mode>` → debug shading for this part (None = MARTIN_RASTER)
}

/// The whole show: a list of parts + the gaussian budget every part is resampled to.
#[derive(Resource)]
pub(crate) struct Sequence {
    pub parts: Vec<Part>,
    pub count: usize,
}

/// MARTIN_FLASH=<strength>: over-bright bloom pulse on each part cut (0 = off, the default).
#[derive(Resource)]
pub(crate) struct FlashStrength(pub f32);

/// Loaded splat handles + the per-part built shapes (all `count` gaussians) + each part's
/// morph-in source cloud + its resolved transition.
#[derive(Resource)]
pub(crate) struct SeqState {
    pub load_names: Vec<String>,
    pub loads: Vec<Handle<PlanarGaussian3d>>,
    pub shapes: Vec<Handle<PlanarGaussian3d>>,
    pub sources: Vec<Option<Handle<PlanarGaussian3d>>>, // per-part lhs source (None = morph from prev)
    pub out_clouds: Vec<Option<Handle<PlanarGaussian3d>>>, // per-part `out:` departure cloud (None = none)
    pub transitions: Vec<Transition>,                      // resolved transition per part
    pub deforms: Vec<Option<Deform>>,                      // resolved persistent deform per part
    pub rasters: Vec<RasterizeMode>,                       // resolved raster mode per part (raster:/MARTIN_RASTER)
    pub starts: Vec<f32>,                                  // absolute start time (s) of each part
    pub built: bool,
    pub entity: Option<Entity>,
}

/// Index of the active part at time `t`: the last part whose absolute start (from the cue
/// timeline — `@@anchor` or laid end-to-end) has arrived. Shared by `part_director` and `flypath`.
pub(crate) fn active_part(starts: &[f32], t: f32) -> usize {
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

/// Absolute start time (s) of each part: its `@@anchor` (locked to the music clock) if set, else
/// laid end-to-end after the previous part (`prev.start + prev.morph + prev.hold`). The cue
/// timeline — shared by `build_sequence` and the `MARTIN_VALIDATE` dry-run.
pub(crate) fn part_starts(parts: &[Part]) -> Vec<f32> {
    let mut starts = Vec::with_capacity(parts.len());
    let mut cursor = 0.0_f32;
    for (i, part) in parts.iter().enumerate() {
        let start = part.anchor.unwrap_or(if i == 0 { 0.0 } else { cursor });
        starts.push(start);
        cursor = start + part.morph + part.hold;
    }
    starts
}

/// End of the cue timeline: the latest part's `start + morph + hold` (anchors can push it past a
/// simple sum). The recorder uses this (+ a tail) for the clip length; `flypath` spreads the
/// camera path across it while recording.
pub(crate) fn show_end(parts: &[Part], starts: &[f32]) -> f32 {
    parts
        .iter()
        .zip(starts)
        .map(|(p, &start)| start + p.morph + p.hold)
        .fold(0.0_f32, f32::max)
}
