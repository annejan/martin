//! martin — fly a camera around Gaussian splats while they morph and reassemble.
//!
//! Two ways to stage content, both driven by `MARTIN_*` env vars (no config file):
//!   * the **morph timeline** (`scene::sequence`) — a chain of parts that each assemble out of a
//!     source cloud and morph into the next (`MARTIN_SEQ`, or the `MARTIN_PLY/_TEXT/_REFORM`
//!     shorthands); and
//!   * the **composition stage** (`scene::compose`) — many objects on one stage at once
//!     (`MARTIN_COMPOSE`), placed and animated, with the camera flowing among them.
//!
//! Rendering: the vendored `bevy_gaussian_splatting` fork (GPU blend + radix depth sort + HDR
//! bloom on black). This file is just the wiring — each feature lives behind a plugin:
//! `CameraPlugin`, `ScenePlugin`, `CapturePlugin`, `MusicPlugin`. See `USAGE.md` for the env
//! reference and `vendor/.../CHANGES.md` for the shader edits.

use std::sync::Arc;

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, WindowMode};
use bevy_gaussian_splatting::GaussianSplattingPlugin;

mod audio;
mod camera;
mod capture;
mod mesh;
mod morph;
mod music;
mod scene;
mod score;
mod splat_image;
mod text;
mod waypoints;

use crate::camera::CameraPlugin;
use crate::capture::CapturePlugin;
use crate::music::{MusicPlugin, ScoreRes};
use crate::scene::compose::{parse_compose, Composition};
use crate::scene::sequence::{sequence_from_env, Sequence};
use crate::scene::{parent_dir, AssetRoot, ScenePlugin};

fn main() {
    // MARTIN_SCORE_DUMP=path: export the built-in score as an editable tracker file, then exit —
    // a ready-to-edit starting point (round-trips through MARTIN_SCORE).
    if let Ok(path) = std::env::var("MARTIN_SCORE_DUMP") {
        match std::fs::write(&path, score::Score::builtin().to_dsl()) {
            Ok(()) => eprintln!("score: built-in written to {path}"),
            Err(e) => eprintln!("score dump error: {e}"),
        }
        return;
    }

    // The score (MARTIN_SCORE file, else built-in) drives both the synth AND the @@anchor times.
    let score = score::Score::from_env();

    // MARTIN_SYNTH_WAV=path: render the synth to a WAV and exit (record.sh muxes it onto the
    // frames). Done before the Bevy app so it needs no window/GPU.
    if let Ok(path) = std::env::var("MARTIN_SYNTH_WAV") {
        let track = audio::synth_track(&score);
        match audio::write_wav(&track, &path) {
            Ok(()) => eprintln!(
                "synth: {} samples ({:.1}s) -> {path}",
                track.len(),
                track.len() as f32 / audio::SAMPLE_RATE as f32
            ),
            Err(e) => eprintln!("synth wav error: {e}"),
        }
        return;
    }

    // MARTIN_COMPOSE: the composition stage (many objects at once). When set it IS the show — the
    // morph timeline is left empty (build_sequence no-ops) and build_composition drives everything.
    let composition = std::env::var("MARTIN_COMPOSE")
        .ok()
        .map(|spec| parse_compose(&spec, &score));
    let (sequence, asset_root) = if composition.is_some() {
        let root = std::env::var("MARTIN_PLY").ok().and_then(parent_dir);
        (
            Sequence {
                parts: Vec::new(),
                count: 0,
            },
            root,
        )
    } else {
        sequence_from_env(&score)
    };
    // where `image:` PNG parts are read from — the .ply folder, or `assets` by default.
    let asset_root_path =
        std::path::PathBuf::from(asset_root.clone().unwrap_or_else(|| "assets".to_string()));

    // MARTIN_FULLSCREEN=1 → start borderless-fullscreen (ignored while recording, which
    // needs the fixed 1280×720 window for uniform frames). Toggle live with F11 / F.
    let fullscreen =
        std::env::var("MARTIN_FULLSCREEN").is_ok() && std::env::var("MARTIN_RECORD").is_err();
    let mut plugins = DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "martin — splat fly-around".into(),
            resolution: (1280, 720).into(), // fixed size so recorded frames are uniform
            mode: if fullscreen {
                WindowMode::BorderlessFullscreen(MonitorSelection::Current)
            } else {
                WindowMode::Windowed
            },
            ..default()
        }),
        ..default()
    });
    if let Some(root) = asset_root {
        plugins = plugins.set(AssetPlugin {
            file_path: root,
            ..default()
        });
    }

    App::new()
        .add_plugins(plugins)
        .add_plugins(GaussianSplattingPlugin)
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(sequence)
        .insert_resource(Composition {
            objects: composition.unwrap_or_default(),
            built: false,
        })
        .insert_resource(AssetRoot(asset_root_path))
        .insert_resource(ScoreRes(Arc::new(score)))
        .add_plugins((CameraPlugin, ScenePlugin, CapturePlugin, MusicPlugin))
        .run();
}
