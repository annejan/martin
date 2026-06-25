//! martin — fly a camera around Gaussian splats while they morph and reassemble.
//!
//! Two ways to stage content, authored in a `.show` file (CLI: `martin <show>` / `--production`):
//!   * the **morph timeline** (`scene::sequence`, a `[reel]`) — a chain of parts that each assemble out
//!     of a source cloud and morph into the next; and
//!   * the **composition stage** (`scene::compose`, a `[stage]`/`[compose]`) — many objects on one
//!     stage at once, placed and animated, with the camera flowing among them.
//!
//! A `.show` expands (`show::apply`) into `MARTIN_*` env vars — the internal IR every parser reads;
//! CLI flags (`cli`) compile to the same env with overwrite. There is no config file beyond the show.
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
mod cli;
mod envvar;
mod fourd;
mod glb;
mod loader;
mod mcp;
mod mesh;
mod morph;
mod music;
mod particles;
mod post;
mod scene;
mod score;
mod serve;
mod show;
mod splat_image;
mod splatgen;
mod sync;
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

/// Which content path a run takes — extracted from `main`'s inline if/else so it's pure + unit-tested.
/// `glb_alone`: a standalone `glb:`/`4D` scene (no morph track); `Seq`: the morph timeline (or the
/// default demo); `ComposeOnly`: a placed-objects stage with no morph track.
#[derive(Debug, PartialEq, Eq)]
enum ContentMode {
    GlbAlone,
    Seq,
    ComposeOnly,
}

/// Decide the content path from the three signals. `glb_or_4d` alone (no explicit seq, no compose) is a
/// standalone glb/4D scene; an explicit seq — or nothing at all — runs the morph track; otherwise it's
/// a compose-only stage. Pure (no env) so the precedence is locked by tests.
fn choose_mode(glb_or_4d: bool, explicit_seq: bool, has_compose: bool) -> ContentMode {
    if glb_or_4d && !explicit_seq && !has_compose {
        ContentMode::GlbAlone
    } else if explicit_seq || !has_compose {
        ContentMode::Seq
    } else {
        ContentMode::ComposeOnly
    }
}

/// Run headless (no window, drive the schedule ourselves, render to an offscreen image): any output
/// mode — recording, the perf bench, single/contact-sheet screenshots. Pure (bool inputs) → tested.
fn is_headless(record: bool, bench: bool, shot: bool, shots: bool) -> bool {
    record || bench || shot || shots
}

fn main() {
    let cli = <cli::Cli as clap::Parser>::parse();

    // `martin mcp` (or $MARTIN_MCP) → run the stdio MCP server (proxy to a --serve bridge) and exit —
    // no Bevy, so stdout stays clean JSON-RPC. Before anything else touches the engine.
    if let Some(cli::Commands::Mcp { port }) = &cli.command {
        if let Some(p) = port {
            // SAFETY: top of main(), single-threaded, before any threads spawn.
            unsafe { std::env::set_var("MARTIN_MCP_PORT", p.to_string()) };
        }
        mcp::run();
        return;
    }
    if std::env::var_os("MARTIN_MCP").is_some() {
        mcp::run();
        return;
    }

    // Compile each present CLI flag into its MARTIN_* env var with OVERWRITE — so a flag beats both an
    // existing env var and the .show file (which expand with set-if-absent below). The whole precedence
    // rule: CLI flag > env > .show [settings] > built-in default.
    for (key, value) in cli::apply_cli(&cli) {
        // SAFETY: top of main(), single-threaded, before the Bevy app (and its threads) start.
        unsafe { std::env::set_var(key, value) };
    }

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

    // MARTIN_COMPOSE: the composition stage (placed objects). It can run TOGETHER with the morph
    // timeline — the morph track is the "hero", the compose objects are placed around it (tracks).
    // Compose ALONE (no explicit MARTIN_SEQ/_TEXT/_PLY*) → no morph track. So:
    //   compose + an explicit seq → both;  compose only → compose;  neither → the default demo.
    let composition = std::env::var("MARTIN_COMPOSE")
        .ok()
        .map(|spec| parse_compose(&spec, &score));
    // a morph reel is requested by MARTIN_SEQ (the .show `[reel]`/`[seq]` body, expanded by show::apply).
    let explicit_seq = std::env::var("MARTIN_SEQ").is_ok();
    let glb_or_4d = std::env::var("MARTIN_GLB").is_ok() || std::env::var("MARTIN_4D_TEST").is_ok();
    let (sequence, asset_root) = match choose_mode(glb_or_4d, explicit_seq, composition.is_some()) {
        ContentMode::GlbAlone => {
            // MARTIN_GLB alone: a standalone KHR_gaussian_splatting scene (glb::GlbScenePlugin spawns
            // it) — no morph track. Asset root = the .glb's folder so the typed GaussianScene load
            // resolves. COMBINED with a seq/compose show, the glb is set dressing instead: the normal
            // branches run and the .glb must sit in that show's asset root (e.g. assets/).
            // (MARTIN_4D_TEST rides the same standalone branch — fourd.rs frames + builds itself.)
            (
                Sequence {
                    parts: Vec::new(),
                    budget: 0,
                },
                std::env::var("MARTIN_GLB").ok().and_then(parent_dir),
            )
        }
        ContentMode::Seq => sequence_from_env(&score), // the morph track (or the default demo)
        ContentMode::ComposeOnly => (
            // compose-only: no morph track.
            Sequence {
                parts: Vec::new(),
                budget: 0,
            },
            std::env::var("MARTIN_PLY").ok().and_then(parent_dir),
        ),
    };
    // The camera waypoints: a `.show` inline `[camera]` track (parsed now the score exists, so its
    // keyframes can anchor to music sections), else the MARTIN_WAYPOINTS file.
    let waypoints = if show.camera.is_empty() {
        waypoints::Waypoints::from_env()
    } else {
        waypoints::Waypoints::from_inline(waypoints::parse_camera(&show.camera, &score))
    };
    // The `[sync]` look-track: keyframed global knobs (flash/bg_dim/beat) over the music clock —
    // parsed now the score exists so its keyframes can anchor to sections (`t=@@drop`).
    let sync_track = sync::parse_sync(&show.sync, &score);
    // The `[caption]` track: screen-anchored titles/credits, parsed now the score exists.
    let captions = scene::caption::parse_captions(&show.caption, &score);

    // MARTIN_VALIDATE=1: a dry run — print the parsed timeline (with the parse diagnostics already
    // on stderr) and exit, no window/render. A fast authoring check.
    if std::env::var_os("MARTIN_VALIDATE").is_some() {
        validate::report(
            &sequence,
            composition.as_deref().unwrap_or(&[]),
            &waypoints,
            &sync_track,
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
    // Headless = no window, drive the schedule ourselves, render the camera into an offscreen image:
    // recording, the perf bench, AND single/contact-sheet screenshots all need it (the RADV window
    // renders black / panics acquiring its swapchain when unfocused). Live runs keep a normal window.
    let headless = is_headless(
        std::env::var("MARTIN_RECORD").is_ok(),
        std::env::var("MARTIN_BENCH").is_ok(),
        std::env::var("MARTIN_SHOT").is_ok(),
        std::env::var("MARTIN_SHOTS").is_ok(),
    );
    let fullscreen = std::env::var("MARTIN_FULLSCREEN").is_ok() && !headless;
    let mut plugins = DefaultPlugins.set(WindowPlugin {
        primary_window: (!headless).then(|| Window {
            // `MARTIN_TITLE` (or `.show [settings] title = …`, which expands to it) overrides the
            // default window title — so a bundled demo can name its own window.
            title: std::env::var("MARTIN_TITLE")
                .unwrap_or_else(|_| "martin — splat fly-around".into()),
            resolution: (1280, 720).into(), // fixed size so recorded frames are uniform
            mode: if fullscreen {
                WindowMode::BorderlessFullscreen(MonitorSelection::Current)
            } else {
                WindowMode::Windowed
            },
            ..default()
        }),
        exit_condition: if headless {
            bevy::window::ExitCondition::DontExit
        } else {
            bevy::window::ExitCondition::OnAllClosed
        },
        ..default()
    });
    // Point Bevy's AssetServer at the SAME (absolute) root martin's std::fs reads use.
    plugins = plugins.set(AssetPlugin {
        file_path: asset_root_path.to_string_lossy().into_owned(),
        // The bundled binary self-extracts its assets to /tmp and loads music.wav by ABSOLUTE path;
        // Bevy 0.18 rejects out-of-root absolute paths by default (UnapprovedPathMode::Deny), so the
        // WAV never loads, the AudioGate never opens, and the loader hangs forever. martin only ever
        // loads its OWN local files (never untrusted web input), so allow them.
        unapproved_path_mode: bevy::asset::UnapprovedPathMode::Allow,
        ..default()
    });
    if headless {
        plugins = plugins.disable::<bevy::winit::WinitPlugin>();
    }

    let mut app = App::new();
    app.add_plugins(plugins)
        .add_plugins(GaussianSplattingPlugin);
    if headless {
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
        .insert_resource(sync_track)
        .insert_resource(scene::caption::Captions(captions))
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
            crate::post::PostPlugin,
            crate::particles::ParticlesPlugin,
            crate::scene::caption::CaptionPlugin,
        ))
        .run();
}

#[cfg(test)]
mod tests {
    use super::{ContentMode, choose_mode, is_headless};

    #[test]
    fn choose_mode_truth_table() {
        use ContentMode::*;
        // glb/4D alone (no seq, no compose) → standalone glb scene
        assert_eq!(choose_mode(true, false, false), GlbAlone);
        // an explicit seq always wins → the morph track (even with a glb or compose present)
        assert_eq!(choose_mode(false, true, false), Seq);
        assert_eq!(choose_mode(true, true, false), Seq); // glb is set-dressing, not standalone
        assert_eq!(choose_mode(true, true, true), Seq);
        assert_eq!(choose_mode(false, true, true), Seq);
        // nothing at all → the default demo runs through the seq path
        assert_eq!(choose_mode(false, false, false), Seq);
        // compose only (no seq, no standalone glb) → the placed-objects stage
        assert_eq!(choose_mode(false, false, true), ComposeOnly);
        // glb + compose but no seq → compose stage (the glb is dressing, not standalone)
        assert_eq!(choose_mode(true, false, true), ComposeOnly);
    }

    #[test]
    fn is_headless_any_output_mode() {
        assert!(!is_headless(false, false, false, false)); // live windowed
        assert!(is_headless(true, false, false, false)); // record
        assert!(is_headless(false, true, false, false)); // bench
        assert!(is_headless(false, false, true, false)); // shot
        assert!(is_headless(false, false, false, true)); // shots (contact sheet)
    }
}
