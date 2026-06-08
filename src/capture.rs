//! Headless capture + live exit: the deterministic frame recorder (`MARTIN_RECORD`), the
//! single screenshot (`MARTIN_SHOT`), the FPS/splat metrics log, and the live show's auto-exit.

use std::f32::consts::PI;

use bevy::app::AppExit;
use bevy::asset::RenderAssetUsages;
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

use crate::camera::{OrbitCam, FRONT_YAW, SWAY};
use crate::scene::compose::Composition;
use crate::scene::sequence::{show_end, SeqState, Sequence};
use crate::scene::SeqClock;

/// Offscreen render target for recording: the camera renders the show into this image and the
/// recorder screenshots *it* — so frames don't depend on the OS window being visible/focused
/// (a background or unfocused window screenshots black on many compositors). Only set when recording.
#[derive(Resource)]
pub(crate) struct RecordTarget(pub Handle<Image>);

/// Create the offscreen image (window-sized) when recording, before the camera is retargeted to it.
fn setup_record_target(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    if std::env::var_os("MARTIN_RECORD").is_none() {
        return;
    }
    let size = Extent3d {
        width: 1280,
        height: 720,
        depth_or_array_layers: 1,
    };
    let mut image = Image::new_fill(
        size,
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::all(),
    );
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT;
    commands.insert_resource(RecordTarget(images.add(image)));
}

/// Point the orbit camera at the offscreen image (once). After this the window shows nothing while
/// recording — that's fine, the recorder reads the image, not the window.
fn attach_record_target(
    mut commands: Commands,
    target: Option<Res<RecordTarget>>,
    cams: Query<Entity, With<OrbitCam>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(target) = target else { return };
    for e in &cams {
        // RenderTarget is a component in 0.18 — insert it to point this camera at the image.
        commands
            .entity(e)
            .insert(RenderTarget::Image(ImageRenderTarget {
                handle: target.0.clone(),
                scale_factor: 1.0,
            }));
        *done = true;
    }
}

/// MARTIN_RECORD=<dir>: dump one PNG per frame across the whole timeline, then exit.
#[derive(Resource)]
pub(crate) struct RecordState {
    pub dir: Option<String>,
    pub dt: f32,       // timeline seconds advanced per frame
    pub yaw_step: f32, // camera sway radians per frame
    pub sway: bool,    // gentle front-sway (true) vs hold the framed/pinned yaw (MARTIN_YAW set)
    pub i: u32,
    pub grace: u32,
    pub bench: Option<u32>, // MARTIN_BENCH=<frames>: render-only fps (no PNG save), then exit
    pub bench_t0: f32,
}

/// Deterministic recorder: total duration = the cue timeline's end (last part's
/// `start + morph + hold`) + tail; set the clock per frame, sway the camera, screenshot, then
/// exit. Frame-indexed → smooth regardless of render speed.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn record_driver(
    mut rec: ResMut<RecordState>,
    time: Res<Time>,
    seq: Option<Res<Sequence>>,
    state: Option<Res<SeqState>>,
    comp: Option<Res<Composition>>,
    target: Option<Res<RecordTarget>>,
    mut clock: ResMut<SeqClock>,
    mut camq: Query<&mut OrbitCam>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    if rec.dir.is_none() && rec.bench.is_none() {
        return;
    }
    // Record either show: wait for the morph sequence OR the composition stage to be built + framed.
    let seq_built = state.as_ref().map(|s| s.built).unwrap_or(false);
    let comp_built = comp.as_ref().map(|c| c.built).unwrap_or(false);
    if (!seq_built && !comp_built) || !camq.iter().any(|c| c.framed) {
        return;
    }
    // MARTIN_BENCH=<frames>: render-only throughput — advance the clock + render each frame but skip
    // the screenshot/PNG entirely, so the timing isolates the render (no disk I/O), then exit.
    if let Some(n) = rec.bench {
        if rec.i == 0 {
            rec.bench_t0 = time.elapsed_secs();
        }
        clock.t = rec.i as f32 * rec.dt;
        rec.i += 1;
        if rec.i >= n {
            let dt = (time.elapsed_secs() - rec.bench_t0).max(1e-3);
            info!(
                "bench: {n} frames in {dt:.2}s = {:.1} render fps (no I/O)",
                n as f32 / dt
            );
            exit.write(AppExit::Success);
        }
        return;
    }
    let Some(dir) = rec.dir.clone() else { return };
    // duration = the longer of the two tracks: the morph timeline's cue end (last part's
    // start+morph+hold) and the compose stage's object timeline (they can run together).
    let seq_dur = match (&seq, &state) {
        (Some(seq), Some(state)) if seq_built && !seq.parts.is_empty() => {
            show_end(&seq.parts, &state.starts) + 1.0
        }
        _ => 0.0,
    };
    let comp_dur = if comp_built {
        comp.as_ref().map(|c| c.record_secs()).unwrap_or(0.0)
    } else {
        0.0
    };
    let dur = seq_dur.max(comp_dur).max(12.0);
    let total = (dur / rec.dt).ceil() as u32;
    if rec.i >= total {
        // Wait for the async PNG writes to actually land before exiting — a fast (release)
        // build outruns the screenshot writer, so a fixed grace count would truncate the clip.
        // Poll the directory until every frame is on disk (with a ~20 s safety cap).
        rec.grace += 1;
        let written = std::fs::read_dir(&dir)
            .map(|d| {
                d.filter_map(Result::ok)
                    .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
                    .count()
            })
            .unwrap_or(total as usize);
        if written >= total as usize || rec.grace > 1200 {
            info!("recording complete: {total} frames ({written} on disk) -> {dir}");
            exit.write(AppExit::Success);
        }
        return;
    }
    let i = rec.i;
    clock.t = i as f32 * rec.dt;
    // gentle front-sway for object showcases; hold the framed yaw when MARTIN_YAW pins a scene.
    if rec.sway {
        let yaw = FRONT_YAW + SWAY * (i as f32 * rec.yaw_step).sin();
        for mut c in &mut camq {
            c.yaw = yaw;
        }
    }
    // Screenshot the offscreen image the camera renders into (window-independent); fall back to the
    // window only if the target wasn't set up.
    let shot = match &target {
        Some(t) => Screenshot::image(t.0.clone()),
        None => Screenshot::primary_window(),
    };
    commands
        .spawn(shot)
        .observe(save_to_disk(format!("{dir}/frame_{i:05}.png")));
    rec.i += 1;
}

/// MARTIN_SHOT=<path> [MARTIN_SHOT_AT=<s>]: one headless screenshot at time `s`, then exit.
#[derive(Resource)]
pub(crate) struct ShotConfig {
    pub path: Option<String>,
    pub at: f32,
    pub done: bool,
}

fn shot_driver(
    time: Res<Time>,
    mut shot: ResMut<ShotConfig>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = shot.path.clone() else {
        return;
    };
    let el = time.elapsed_secs();
    if !shot.done && el >= shot.at {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
        shot.done = true;
        info!("auto-screenshot -> {path}");
    }
    if shot.done && el >= shot.at + 2.0 {
        exit.write(AppExit::Success);
    }
}

/// In a live window (not recording / screenshotting), **exit when the show is done** instead of
/// sitting on the last part forever. `Space` restarts; `MARTIN_LOOP=1` keeps it up (for tuning).
fn live_end(
    rec: Res<RecordState>,
    shot: Res<ShotConfig>,
    seq: Option<Res<Sequence>>,
    state: Option<Res<SeqState>>,
    clock: Res<SeqClock>,
    mut exit: MessageWriter<AppExit>,
) {
    if rec.dir.is_some() || shot.path.is_some() || std::env::var("MARTIN_LOOP").is_ok() {
        return; // the recorder/screenshot exit on their own; MARTIN_LOOP = stay up
    }
    let (Some(seq), Some(state)) = (seq, state) else {
        return;
    };
    if state.built && clock.t > show_end(&seq.parts, &state.starts) + 2.5 {
        exit.write(AppExit::Success);
    }
}

/// FPS + splat-count metrics. `MARTIN_FPS=1` logs every ~0.5 s; the **`I`** key toggles that live
/// and logs one snapshot immediately.
#[derive(Resource)]
pub(crate) struct FpsLog {
    pub enabled: bool,
    pub accum: f32,
    pub frames: u32,
}

fn fps_log(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    clock: Res<SeqClock>,
    seq: Option<Res<Sequence>>,
    mut f: ResMut<FpsLog>,
) {
    let snap = keys.just_pressed(KeyCode::KeyI); // `I` → toggle logging + log one snapshot now
    if snap {
        f.enabled = !f.enabled;
    }
    f.accum += time.delta_secs();
    f.frames += 1;
    if (f.enabled && f.accum >= 0.5) || snap {
        let fps = f.frames as f32 / f.accum.max(1e-6);
        let ms = 1000.0 * f.accum / f.frames.max(1) as f32;
        // gaussians rendered per part (the morph budget; 0 = each part's native count).
        let splats = seq.map(|s| s.count).unwrap_or(0);
        info!(
            "metrics: {fps:.1} fps ({ms:.1} ms/frame) · {splats} splats/part · t={:.2}",
            clock.t
        );
        f.accum = 0.0;
        f.frames = 0;
    }
}

/// The frame recorder, the single screenshot, the metrics log, and the live auto-exit.
pub(crate) struct CapturePlugin;

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RecordState {
            dir: std::env::var("MARTIN_RECORD").ok(),
            dt: 1.0 / 60.0,
            yaw_step: 2.0 * PI / 480.0, // ~8s gentle sway period
            // a pinned yaw, a parked capture pose, or a flown waypoint path → hold/drive it, no sway
            sway: std::env::var("MARTIN_YAW").is_err()
                && std::env::var("MARTIN_CAMERAS").is_err()
                && std::env::var("MARTIN_FLY").is_err()
                && std::env::var("MARTIN_COMPOSE").is_err(),
            i: 0,
            grace: 0,
            bench: std::env::var("MARTIN_BENCH")
                .ok()
                .and_then(|s| s.parse().ok()),
            bench_t0: 0.0,
        })
        .insert_resource(ShotConfig {
            path: std::env::var("MARTIN_SHOT").ok(),
            at: std::env::var("MARTIN_SHOT_AT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(6.0),
            done: false,
        })
        .insert_resource(FpsLog {
            enabled: std::env::var("MARTIN_FPS").is_ok(),
            accum: 0.0,
            frames: 0,
        })
        .add_systems(Startup, setup_record_target)
        .add_systems(
            Update,
            (
                attach_record_target,
                record_driver,
                shot_driver,
                live_end,
                fps_log,
            ),
        );
    }
}
