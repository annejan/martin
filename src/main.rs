//! martin — fly a camera around Gaussian splats while they morph and reassemble.
//!
//! Two ways to stage content, both driven by `MARTIN_*` env vars (no config file):
//!   * the **morph timeline** (`scene::sequence`) — a chain of parts that each assemble out of a
//!     source cloud and morph into the next (`MARTIN_SEQ`, or the `MARTIN_PLY/_TEXT/_REFORM`
//!     shorthands); and
//!   * the **composition stage** (`scene::compose`) — many objects on one stage at once
//!     (`MARTIN_COMPOSE`), placed and animated, with the camera flowing among them.
//!
//! Rendering: our `bevy_gaussian_splatting` fork (GPU blend + radix depth sort + HDR
//! bloom on black), pulled in as a git dep (the `martin` branch of
//! `annejan/bevy_gaussian_splatting`). This file is just the wiring — each feature lives behind
//! a plugin: `CameraPlugin`, `ScenePlugin`, `CapturePlugin`, `MusicPlugin`. See `USAGE.md` for
//! the env reference and the fork's `CHANGES.md` for the shader edits.

// edition-2024 stabilised let-chains, so clippy now suggests collapsing every `if cond { if let … }`
// into one let-chain. That's a pure style call — the nested form reads fine here — so don't enforce
// it crate-wide. (All the correctness/perf/suspicious clippy lints stay on, gated by CI.)
#![allow(clippy::collapsible_if)]

use std::sync::Arc;

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, WindowMode};
use bevy_gaussian_splatting::GaussianSplattingPlugin;

mod audio;
mod background;
#[cfg(feature = "bundle")]
mod bundle;
mod camera;
mod capture;
mod envvar;
mod fourd;
mod glb;
mod loader;
mod mesh;
mod morph;
mod music;
mod scene;
mod score;
mod serve;
mod show;
mod splat_image;
mod text;
mod validate;
mod waypoints;

use crate::camera::CameraPlugin;
use crate::capture::CapturePlugin;
use crate::loader::LoaderPlugin;
use crate::music::{MusicPlugin, ScoreRes};
use crate::scene::compose::{Composition, parse_compose};
use crate::scene::sequence::{Sequence, sequence_from_env};
use crate::scene::{AssetRoot, ScenePlugin, parent_dir};

fn main() {
    // Bundled single-binary build: self-extract the embedded assets + seed the baked-in show into
    // the env BEFORE anything reads it (a no-op without `--features bundle`).
    #[cfg(feature = "bundle")]
    bundle::apply();

    // With nothing requested, play the INTRO production — the same show CI bundles into the single
    // binary, so a fresh `git clone && cargo run` plays exactly the showcase the download does. Its
    // procedural splats are synthesized by build.rs if absent (see DEFAULT_SHOW there), so the clone
    // needs no python/numpy step. Set it as MARTIN_SHOW so it flows through the unified-show path below.
    if std::env::var("MARTIN_SHOW").is_err() && scene::no_content_requested() {
        // SAFETY: top of main(), single-threaded, before the Bevy app (and its threads) start.
        unsafe { std::env::set_var("MARTIN_SHOW", "productions/intro/intro.show") };
    }

    // MARTIN_SHOW=<file>.show: a unified scene file — expand it INTO the env (settings → MARTIN_*,
    // [seq]/[compose] bodies → MARTIN_SEQ/_COMPOSE) so everything below reads it unchanged. Must run
    // before anything reads the env. Returns the inline [camera] track (empty without a show).
    let show = show::apply();

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

    // MARTIN_STREAM_WAV=path: like MARTIN_SYNTH_WAV but via the STREAMING engine — a debug/verify
    // path to A/B the two renderers on real scores. (Live playback streams; this just dumps it.)
    if let Ok(path) = std::env::var("MARTIN_STREAM_WAV") {
        match audio::render_stream_wav(&score, &path) {
            Ok(()) => eprintln!("stream synth -> {path}"),
            Err(e) => eprintln!("stream wav error: {e}"),
        }
        return;
    }

    // MARTIN_COMPOSE: the composition stage (placed objects). It can run TOGETHER with the morph
    // timeline — the morph track is the "hero", the compose objects are placed around it (tracks).
    // Compose ALONE (no explicit MARTIN_SEQ/_TEXT/_PLY*) → no morph track. So:
    //   compose + an explicit seq → both;  compose only → compose;  neither → the default demo.
    let composition = std::env::var("MARTIN_COMPOSE")
        .ok()
        .map(|spec| parse_compose(&spec, &score));
    let explicit_seq = [
        "MARTIN_SEQ",
        "MARTIN_TEXT",
        "MARTIN_PLY",
        "MARTIN_PLY2",
        "MARTIN_REFORM",
    ]
    .iter()
    .any(|k| std::env::var(k).is_ok());
    let glb_alone = (std::env::var("MARTIN_GLB").is_ok()
        || std::env::var("MARTIN_4D_TEST").is_ok())
        && !explicit_seq
        && composition.is_none();
    let (sequence, asset_root) = if glb_alone {
        // MARTIN_GLB alone: a standalone KHR_gaussian_splatting scene (glb::GlbScenePlugin spawns
        // it) — no morph track. Asset root = the .glb's folder so the typed GaussianScene load
        // resolves. COMBINED with a seq/compose show, the glb is set dressing instead: the normal
        // branches below run and the .glb must sit in that show's asset root (e.g. assets/).
        // (MARTIN_4D_TEST rides the same standalone branch — fourd.rs frames + builds itself.)
        (
            Sequence {
                parts: Vec::new(),
                count: 0,
            },
            std::env::var("MARTIN_GLB").ok().and_then(parent_dir),
        )
    } else if explicit_seq || composition.is_none() {
        sequence_from_env(&score) // the morph track (or the default demo when nothing is set)
    } else {
        // compose-only: no morph track.
        (
            Sequence {
                parts: Vec::new(),
                count: 0,
            },
            std::env::var("MARTIN_PLY").ok().and_then(parent_dir),
        )
    };
    // The camera waypoints: a `.show` inline `[camera]` track (parsed now the score exists, so its
    // keyframes can anchor to music sections), else the MARTIN_WAYPOINTS file.
    let waypoints = if show.camera.is_empty() {
        waypoints::Waypoints::from_env()
    } else {
        waypoints::Waypoints::from_inline(waypoints::parse_camera(&show.camera, &score))
    };

    // MARTIN_VALIDATE=1: a dry run — print the parsed timeline (with the parse diagnostics already
    // on stderr) and exit, no window/render. A fast authoring check.
    if std::env::var_os("MARTIN_VALIDATE").is_some() {
        validate::report(
            &sequence,
            composition.as_deref().unwrap_or(&[]),
            &waypoints,
            &score,
            asset_root.as_deref(),
        );
        return;
    }

    // Asset root: the .ply folder, or `assets` by default. Resolve to an ABSOLUTE path so Bevy's
    // AssetServer (glb:/model: loads) and martin's own std::fs reads (mesh:/image:) agree regardless
    // of how the binary is launched (`cargo run` uses CARGO_MANIFEST_DIR; a bare `./target/release/
    // martin` would otherwise resolve Bevy assets next to the executable → "Path not found").
    let asset_root_path = {
        let p =
            std::path::PathBuf::from(asset_root.clone().unwrap_or_else(|| "assets".to_string()));
        std::fs::canonicalize(&p).unwrap_or(p)
    };

    // Recording runs HEADLESS — no window at all. On this AMD/RADV setup the window surface
    // renders black whenever it isn't the focused/visible window, so the recorder renders the
    // camera into an offscreen image (capture.rs) and drives the schedule itself; live runs keep
    // a normal window. MARTIN_FULLSCREEN=1 → borderless fullscreen (live only).
    let recording = std::env::var("MARTIN_RECORD").is_ok() || std::env::var("MARTIN_BENCH").is_ok();
    let fullscreen = std::env::var("MARTIN_FULLSCREEN").is_ok() && !recording;
    let mut plugins = DefaultPlugins.set(WindowPlugin {
        primary_window: (!recording).then(|| Window {
            title: "martin — splat fly-around".into(),
            resolution: (1280, 720).into(), // fixed size so recorded frames are uniform
            mode: if fullscreen {
                WindowMode::BorderlessFullscreen(MonitorSelection::Current)
            } else {
                WindowMode::Windowed
            },
            ..default()
        }),
        exit_condition: if recording {
            bevy::window::ExitCondition::DontExit
        } else {
            bevy::window::ExitCondition::OnAllClosed
        },
        ..default()
    });
    // Point Bevy's AssetServer at the SAME (absolute) root martin's std::fs reads use.
    plugins = plugins.set(AssetPlugin {
        file_path: asset_root_path.to_string_lossy().into_owned(),
        ..default()
    });
    if recording {
        plugins = plugins.disable::<bevy::winit::WinitPlugin>();
    }

    let mut app = App::new();
    app.add_plugins(plugins)
        .add_plugins(GaussianSplattingPlugin);
    if recording {
        // No winit event loop — drive the schedule ourselves; record_driver exits via AppExit.
        app.add_plugins(bevy::app::ScheduleRunnerPlugin::run_loop(
            std::time::Duration::ZERO,
        ));
    } else {
        // Keep rendering even when the window is unfocused (live preview).
        app.insert_resource(bevy::winit::WinitSettings {
            focused_mode: bevy::winit::UpdateMode::Continuous,
            unfocused_mode: bevy::winit::UpdateMode::Continuous,
        });
    }
    app.insert_resource(ClearColor(Color::BLACK))
        .insert_resource(sequence)
        .insert_resource(Composition {
            objects: composition.unwrap_or_default(),
            built: false,
        })
        .insert_resource(AssetRoot(asset_root_path))
        .insert_resource(ScoreRes(Arc::new(score)))
        .insert_resource(waypoints)
        .add_plugins((
            CameraPlugin,
            ScenePlugin,
            CapturePlugin,
            MusicPlugin,
            LoaderPlugin,
            crate::background::BackgroundPlugin,
            crate::scene::shader_part::ShaderPartPlugin,
            crate::glb::GlbScenePlugin,
            crate::fourd::FourDTestPlugin,
            crate::serve::ServePlugin,
        ))
        .run();
}
