//! The one morph-timeline engine: a show is a list of `Part`s that each assemble in from a
//! source cloud (ball/fade/explode/… or a per-particle shader transition) and then hold,
//! morphing into the next. Drives a single `GaussianInterpolate` entity retargeted per part.
//!
//! Split across submodules:
//! - [`model`] — data types (`Part`/`Sequence`/`SeqState`/`FlashStrength`) + the pure cue-timeline
//!   functions (`active_part`/`part_starts`/`show_end`).
//! - [`parse`] — building the `Sequence` from env (`sequence_from_env`/`parse_seq`/…).
//! - [`build`] — `build_sequence` + `seq_no_cull`: assemble the clouds, spawn the entity, frame it.
//! - [`director`] — `part_director`: drive the morph per frame.

mod build;
mod director;
mod model;
mod parse;

pub(crate) use build::{build_sequence, seq_no_cull};
pub(crate) use director::part_director;
pub(crate) use model::{
    FlashStrength, Part, SeqState, Sequence, active_part, part_starts, show_end,
};
pub(crate) use parse::sequence_from_env;
