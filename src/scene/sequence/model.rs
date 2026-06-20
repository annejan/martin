//! Data types of the morph-timeline + the pure cue-timeline functions.
//!
//! A show is a list of `Shot`s that each assemble in from a source cloud (ball/fade/explode/… or a
//! per-particle shader entrance) and then hold, morphing into the next. `SeqState` holds the
//! loaded handles + the per-shot built shapes (one `BuiltShot` per shot). The cue-timeline fns
//! (`active_shot`/`shot_starts`/`show_end`) derive absolute shot start/end times — shared by the
//! builder, the director, the camera, and the `MARTIN_VALIDATE` dry-run.

use bevy::prelude::*;
use bevy_gaussian_splatting::{PlanarGaussian3d, RasterizeMode};

use crate::scene::content::PartContent;
use crate::scene::effects::{Deform, Departure, Ease, Entrance};

/// One shot morphs in from the previous (or, for shot 0, from a ball), then holds.
#[derive(Clone)]
pub(crate) struct Shot {
    pub content: PartContent,
    pub hold: f32,                                  // seconds held after arriving
    pub morph: f32,                                 // seconds to morph in
    pub bulge: f32,                 // ball-pulse explosiveness (Morph entrance only)
    pub entrance: Option<Entrance>, // None = default (Ball for shot 0, else Morph)
    pub anchor: Option<f32>,        // absolute start (s) on the music clock; None = relative
    pub deform: Option<Deform>,     // persistent deform while held (None = none / MARTIN_DEFORM)
    pub exit: Option<Departure>, // how the shot LEAVES (`exit:name`, was `out:`); None = cross-morph to next
    pub rot: Option<Quat>,       // per-shot orientation (`rot:rx,ry,rz` deg), baked into the shape
    pub flock: Option<usize>, // `flock:N` (was `cluster:`) → N scattered, randomly-rotated copies
    pub backdrop: Option<u32>, // `backdrop:<name>` (was `bg:`) → background mode from this shot on (BG_OFF hides)
    pub raster: Option<RasterizeMode>, // `raster:<mode>` → debug shading for this shot (None = MARTIN_RASTER)
    pub flash: Option<f32>, // `flash:<strength>` → cut-bloom on THIS shot's entry (None = MARTIN_FLASH)
    pub deform_amp: Option<f32>, // `^name:<amp>` → this shot's deform strength scale (None = 1.0)
    pub beat: Option<f32>, // `beat:<scale>` → this shot's beat-bounce reaction (0 = still; None = 1.0)
    pub tint: Option<crate::scene::colorize::Tint>, // `tint:fry|rainbow|brand` → recolour this shape
    pub ease: Ease, // `ease:<name>` → shape the morph curve (default Smoothstep = unchanged)
    pub freeze: Option<f32>, // `freeze:N` → quantize the deform animation to N steps/bar (stop-motion)
    pub ground: Option<f32>, // `ground:<y>` → seat this part's LOWEST splat at world-Y (rest it on a
    // [stage] plate); None = centred at the origin (floats), the default
    pub disk: Option<f32>, // `disk:<f>` → per-part mesh splat-disk overlap (overrides MARTIN_MESH_SPLAT);
                           // smaller = crisper edges on a sharp graphic (the logo), None = global default
}

impl Shot {
    /// A shot with `content` and every effect/modifier at its default (no entrance/deform/tint/…).
    /// Callers spell only the fields they set and fill the rest with `..Shot::base(content)`, so a new
    /// `Shot` field doesn't ripple into every construction site (the timing defaults match `parse_seq`).
    pub(crate) fn base(content: PartContent) -> Self {
        Self {
            content,
            hold: 1.5,
            morph: 3.0,
            bulge: 0.9,
            entrance: None,
            anchor: None,
            deform: None,
            exit: None,
            rot: None,
            flock: None,
            backdrop: None,
            raster: None,
            flash: None,
            deform_amp: None,
            beat: None,
            tint: None,
            ease: Ease::Smoothstep,
            freeze: None,
            ground: None,
            disk: None,
        }
    }
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
    pub exit_cloud: Option<Handle<PlanarGaussian3d>>, // `exit:` departure cloud (None = none)
    pub entrance: Entrance,
    pub deform: Option<Deform>,
    pub deform_amp: Option<f32>, // per-shot deform strength scale (`^name:amp`); None = 1.0
    pub flash: Option<f32>, // per-shot cut-bloom strength (`flash:N`); None = global MARTIN_FLASH
    pub beat: Option<f32>,  // per-shot beat-bounce scale (`beat:N`); None = 1.0, 0 = still
    pub raster: RasterizeMode,
    pub ease: Ease, // morph-curve shaping (`ease:`); applied to the blend factor in the director
    pub freeze: Option<f32>, // `freeze:N` deform quantization grid (steps/bar); None = smooth
    pub start: f32, // absolute start time (s) of this shot
    // copied from the source Shot so the director needs only BuiltShot. (`hold` lives only on the
    // source `Shot` — the cue timeline reads it there via `show_end`; the director never needs it.)
    pub morph: f32,
    pub bulge: f32,
    pub exit: Option<Departure>,
    pub is_gl_mesh: bool, // was `matches!(shot.content, PartContent::GlMesh(_))`
}

/// Loaded splat handles + the per-shot built shots (each resampled to the budget, with its
/// morph-in origin cloud + resolved entrance/deform/raster + cue start).
#[derive(Resource)]
pub(crate) struct SeqState {
    pub load_names: Vec<String>,
    pub loads: Vec<Handle<PlanarGaussian3d>>,
    pub shots: Vec<BuiltShot>,
    /// Each built shot's absolute start time (s) — the cue timeline, collected ONCE in
    /// `build_sequence` (immutable after) so the per-frame readers borrow it via `starts()` instead
    /// of re-allocating a Vec every frame.
    pub starts: Vec<f32>,
    pub built: bool,
    pub entity: Option<Entity>,
}

impl SeqState {
    /// The cue timeline (each shot's absolute start, s) as a flat slice — borrowed from the cached
    /// `starts` field, for the readers that want it (the cut-flash loop, `gl_mesh_alpha`, the
    /// shader-interlude window, `active_shot`). No per-frame allocation.
    pub(crate) fn starts(&self) -> &[f32] {
        &self.starts
    }
}

/// Index of the active shot at time `t`: the last shot whose absolute start (from the cue
/// timeline — `@@anchor` or laid end-to-end) has arrived. Shared by `shot_director` and `flypath`.
pub(crate) fn active_shot(starts: &[f32], t: f32) -> usize {
    debug_assert!(!starts.is_empty(), "active_shot: empty timeline");
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
