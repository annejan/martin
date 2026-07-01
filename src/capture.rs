//! Headless capture + live exit: the deterministic frame recorder (`MARTIN_RECORD`), the
//! single screenshot (`MARTIN_SHOT`), the FPS/splat metrics log, and the live show's auto-exit.

use std::f32::consts::PI;

use bevy::app::AppExit;
use bevy::asset::RenderAssetUsages;
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use crate::camera::{FRONT_YAW, OrbitCam, SWAY};
use crate::scene::SeqClock;
use crate::scene::compose::Composition;
use crate::scene::sequence::{SeqState, Sequence, show_end};

/// Offscreen render target for recording: the camera renders the show into this image and the
/// recorder screenshots *it* — so frames don't depend on the OS window being visible/focused
/// (a background or unfocused window screenshots black on many compositors). Only set when recording.
#[derive(Resource)]
pub(crate) struct RecordTarget(pub Handle<Image>);

/// The offscreen render resolution: `MARTIN_RES=WxH` (e.g. `1920x1080`, `2560x1440`), default 720p.
/// **Keep it 16:9** — the fullscreen background/interlude quads and the camera framing assume that
/// aspect; a non-16:9 size warns and renders letterboxed/cropped. Shared by record + the serve view.
pub(crate) fn render_size() -> (u32, u32) {
    let Ok(spec) = std::env::var("MARTIN_RES") else {
        return (1280, 720);
    };
    let parsed = spec
        .split_once(['x', 'X', '*'])
        .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)));
    match parsed {
        Some((w, h)) if w >= 16 && h >= 16 => {
            if (w as f32 / h as f32 - 16.0 / 9.0).abs() > 0.02 {
                warn!("MARTIN_RES={spec}: not 16:9 — quads/framing assume 16:9, expect bars");
            }
            (w, h)
        }
        _ => {
            warn!("MARTIN_RES={spec}: expected WxH (e.g. 1920x1080) — using 1280x720");
            (1280, 720)
        }
    }
}

/// Create the offscreen image (`MARTIN_RES`-sized) when recording OR screenshotting, before the camera
/// is retargeted to it — so `MARTIN_SHOT`/`MARTIN_SHOTS` grab the image (truly headless) instead of the
/// OS window (which renders black / panics acquiring its swapchain on RADV when unfocused).
fn setup_record_target(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    if std::env::var_os("MARTIN_RECORD").is_none()
        && std::env::var_os("MARTIN_SHOT").is_none()
        && std::env::var_os("MARTIN_SHOTS").is_none()
    {
        return;
    }
    let (width, height) = render_size();
    let size = Extent3d {
        width,
        height,
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
        // RenderTarget is a component (0.19) — insert it to point this camera at the image.
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
fn record_driver(
    mut rec: ResMut<RecordState>,
    time: Res<Time>,
    real: Res<Time<Real>>,
    seq: Option<Res<Sequence>>,
    state: Option<Res<SeqState>>,
    comp: Option<Res<Composition>>,
    score: Option<Res<crate::music::ScoreRes>>,
    target: Option<Res<RecordTarget>>,
    marks: Option<Res<crate::waypoints::Waypoints>>,
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
        // Backstop: never hang a record/bench forever waiting to build+frame (any loader/build edge
        // case CI must survive). No legitimate build takes 120 s — exit NON-ZERO so a wedge is a CI
        // failure, not a silent zero-frame "success".
        if real.elapsed_secs() > 120.0 {
            error!(
                "record: scene not built+framed after 120 s — aborting (a missing asset / build hang?)"
            );
            exit.write(AppExit::error());
        }
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
            show_end(&seq.parts, state.starts()) + 1.0
        }
        _ => 0.0,
    };
    let comp_dur = if comp_built {
        comp.as_ref().map(|c| c.record_secs()).unwrap_or(0.0)
    } else {
        0.0
    };
    // Run to the end of the MUSIC too: a music-synced show (e.g. a compose stage whose [sync]/[camera]
    // tracks key off @@outro) is shorter by its object timeline than the track, so without this the
    // clip would cut before the finale. The video should never end before the song.
    let score_dur = score.as_ref().map(|s| s.0.demo_len() + 1.0).unwrap_or(0.0);
    let dur = seq_dur.max(comp_dur).max(score_dur).max(12.0);
    // Belt-and-braces: a non-finite or absurd duration (an `inf` hold/anchor that slipped a parser)
    // would make `(dur/dt).ceil() as u32` saturate to u32::MAX → the recorder never reaches `total` and
    // wedges forever, filling the disk. Clamp to a sane finite ceiling (30 min — far past any demo).
    let dur = if dur.is_finite() {
        dur.min(1800.0)
    } else {
        warn!("record: non-finite show duration (a bad @timing / anchor?) — capping at 1800s");
        1800.0
    };
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
        if written >= total as usize {
            info!("recording complete: {total} frames ({written} on disk) -> {dir}");
            exit.write(AppExit::Success);
        } else if rec.grace > 1200 {
            // Grace cap hit with frames STILL missing → the PNG writer stalled (disk full / write
            // error), not "just slow". Exit NON-ZERO so record.sh + CI don't mux a truncated clip and
            // report success — the old code declared "complete" + AppExit::Success here regardless.
            error!(
                "record: PNG writer stalled at {written}/{total} frames after the grace cap — the disk \
                 likely filled or a write failed. Aborting (a truncated dump must not look complete); \
                 free space or point TMPDIR at a bigger filesystem and re-run."
            );
            exit.write(AppExit::error());
        }
        return;
    }
    let i = rec.i;
    clock.t = i as f32 * rec.dt;
    // gentle front-sway for object showcases; hold the framed yaw when MARTIN_YAW pins a scene. A
    // fully-timed `[camera]` track is authoritative (flypath drives it) → no sway, or it'd fight the
    // authored yaw.
    let track = marks
        .as_ref()
        .map(|m| crate::waypoints::is_track(&m.list))
        .unwrap_or(false);
    if rec.sway && !track {
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
/// MARTIN_SHOTS=<s1,s2,…>: a whole CONTACT SHEET in ONE run — seek to each time, screenshot to
/// `<path>_<t>.png`, then exit. Amortizes the cold start (one boot/load instead of N).
#[derive(Resource)]
pub(crate) struct ShotConfig {
    pub path: Option<String>,
    pub ats: Vec<f32>, // shot times (MARTIN_SHOTS csv, else [MARTIN_SHOT_AT])
    pub idx: usize,    // current shot index
    pub done: bool,    // current frame captured
}

/// `/tmp/x.png` + t=8 → `/tmp/x_8.png` (only when shooting a multi-frame sheet; single shot keeps `path`).
fn shot_path(path: &str, at: f32, multi: bool) -> String {
    if !multi {
        return path.to_string();
    }
    let tag = format!("_{}", (at * 10.0).round() / 10.0).replace(['.', '-'], "_");
    match path.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}{tag}.{ext}"),
        None => format!("{path}{tag}"),
    }
}

#[allow(clippy::too_many_arguments)] // ECS system params + the file-wait Locals
fn shot_driver(
    mut shot: ResMut<ShotConfig>,
    mut clock: ResMut<SeqClock>,
    state: Option<Res<SeqState>>,
    comp: Option<Res<crate::scene::compose::Composition>>,
    target: Option<Res<RecordTarget>>,
    camq: Query<&OrbitCam>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
    mut frames: Local<u32>,
    mut last_out: Local<String>,
) {
    let Some(path) = shot.path.clone() else {
        return;
    };
    if shot.idx >= shot.ats.len() {
        exit.write(AppExit::Success);
        return;
    }
    let at = shot.ats[shot.idx];
    // SEEK straight to the shot time (don't simulate the whole timeline to get there — a late
    // MARTIN_SHOT_AT used to take ~that-many seconds). advance_seq_clock is gated off in shot mode,
    // so setting the clock here holds the scene + camera at `at`.
    clock.t = at;
    // wait until the show is actually built (assets loaded → composition/sequence assembled), then a
    // few more frames so the held pose + sort settle before grabbing the frame.
    // wait for the show to be built AND the camera to be framed — else the screenshot grabs the
    // offscreen image before the camera is positioned + a frame has rendered into it (a black frame).
    let built = state.map(|s| s.built).unwrap_or(false) || comp.map(|c| c.built).unwrap_or(false);
    if !built || !camq.iter().any(|c| c.framed) {
        return;
    }
    // settle generously after the SEEK before grabbing: a jump re-positions the morph + churns the
    // radix sort for several frames, so an early grab catches a half-sorted (near-black) cloud — most
    // visible in a fade/morph window. 40 frames is still milliseconds in the headless run-loop.
    *frames += 1;
    if !shot.done && *frames >= 40 {
        let out = shot_path(&path, at, shot.ats.len() > 1);
        // shoot the offscreen image (headless, window-independent); fall back to the window only if the
        // target wasn't set up (e.g. a live windowed session that happens to request a shot).
        let grab = match &target {
            Some(t) => Screenshot::image(t.0.clone()),
            None => Screenshot::primary_window(),
        };
        commands.spawn(grab).observe(save_to_disk(out.clone()));
        shot.done = true;
        *last_out = out.clone();
        info!("auto-screenshot @ t={at:.1} -> {out}");
    }
    // WAIT for the screenshot file to actually land on disk before re-seeking / exiting — the
    // screenshot is an async GPU readback saved by an observer a few render frames later, so a fixed
    // frame count raced it (the file never finished writing before AppExit / the next sheet seek). Poll
    // for the file; a large frame cap is just a backstop so a failed save can't hang the process.
    if shot.done && (std::path::Path::new(&*last_out).exists() || *frames >= 1200) {
        // next frame in the sheet (re-seek), or exit after the last
        shot.idx += 1;
        shot.done = false;
        *frames = 0;
        if shot.idx >= shot.ats.len() {
            exit.write(AppExit::Success);
        }
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
    if rec.dir.is_some()
        || shot.path.is_some()
        || std::env::var("MARTIN_LOOP").is_ok()
        || crate::serve::is_serving()
    {
        return; // recorder/screenshot exit on their own; MARTIN_LOOP / the serve bridge stay up
    }
    let (Some(seq), Some(state)) = (seq, state) else {
        return;
    };
    if state.built && clock.t > show_end(&seq.parts, state.starts()) + 2.5 {
        exit.write(AppExit::Success);
    }
}

/// FPS + splat-count metrics. `MARTIN_FPS=1` logs every ~0.5 s; the **`I`** key toggles that live
/// and logs one snapshot immediately. `smoothed_fps`/`smoothed_ms` are updated every frame so the
/// on-screen overlay (`fps_overlay`) reads them without needing `FrameTimeDiagnosticsPlugin`.
#[derive(Resource)]
pub(crate) struct FpsLog {
    pub enabled: bool,
    pub accum: f32,
    pub frames: u32,
    pub smoothed_fps: f32,
    pub smoothed_ms: f32,
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
    // running smoothed values for the on-screen overlay (always updated, not just on log).
    f.smoothed_fps = f.frames as f32 / f.accum.max(1e-6);
    f.smoothed_ms = 1000.0 * f.accum / f.frames.max(1) as f32;
    if (f.enabled && f.accum >= 0.5) || snap {
        // gaussians rendered per part (the morph budget; 0 = each part's native count).
        let splats = seq.map(|s| s.budget).unwrap_or(0);
        info!(
            "metrics: {:.1} fps ({:.1} ms/frame) · {splats} splats/part · t={:.2}",
            f.smoothed_fps, f.smoothed_ms, clock.t
        );
        f.accum = 0.0;
        f.frames = 0;
    }
}

/// On-screen FPS HUD — the engine's own live perf readout (smoothed FPS + frame-time + splat budget +
/// clock), in the top-left corner. The **`I`** key toggles it (alongside the console metric);
/// `MARTIN_FPS_OVERLAY=1` (or `[settings] fps_overlay`) starts it shown. **Live windows only** — never
/// spawned for `--record`/`--shot`, so the HUD can't bake into recorded frames.
#[derive(Component)]
pub(crate) struct FpsOverlay;

fn setup_fps_overlay(mut commands: Commands) {
    // A video RECORD renders the UI into the offscreen image too → the HUD would bake into the frames.
    // Never spawn it there. (A single `--shot` may show it — hidden unless MARTIN_FPS_OVERLAY is set —
    // so you can grab a HUD still on purpose without it ever creeping into a deliverable video.)
    if std::env::var("MARTIN_RECORD").is_ok() {
        return;
    }
    let visible = std::env::var("MARTIN_FPS_OVERLAY").is_ok();
    commands.spawn((
        FpsOverlay,
        Text::new("…"),
        TextFont {
            font_size: 20.0.into(), // 0.19: TextFont::font_size is FontSize
            ..default()
        },
        TextColor(Color::srgb(0.5, 1.0, 0.65)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(6.0),
            left: Val::Px(8.0),
            padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)), // a panel so it reads over any scene
        GlobalZIndex(i32::MAX),                             // draw over captions / credits
        if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        },
    ));
}

/// Toggle the HUD on `I`; while shown, refresh it from `FpsLog` (no `FrameTimeDiagnosticsPlugin`
/// dependency — runs with or without `MARTIN_DIAG`).
fn fps_overlay(
    keys: Res<ButtonInput<KeyCode>>,
    fps_log: Res<FpsLog>,
    clock: Res<SeqClock>,
    seq: Option<Res<Sequence>>,
    mut q: Query<(&mut Visibility, &mut Text), With<FpsOverlay>>,
) {
    let Ok((mut vis, mut text)) = q.single_mut() else {
        return;
    };
    if keys.just_pressed(KeyCode::KeyI) {
        *vis = if *vis == Visibility::Visible {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
    }
    if *vis != Visibility::Visible {
        return;
    }
    let fps = fps_log.smoothed_fps;
    let ms = fps_log.smoothed_ms;
    // gaussians per part (the morph budget; 0 = each part's native count, as the console metric reports).
    let splats = seq.map(|s| s.budget).unwrap_or(0);
    let splats = if splats == 0 {
        "native".to_string()
    } else {
        format!("{splats}")
    };
    text.0 = format!(
        "{fps:5.1} fps   {ms:4.1} ms\n{splats} splats/part   t={:.1}s",
        clock.t
    );
}

/// The frame recorder, the single screenshot, the metrics log, and the live auto-exit.
pub(crate) struct CapturePlugin;

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        // MARTIN_PREVIEW_FPS=<n>: render at n fps instead of 60 — fewer frames for a FAST preview
        // (n=6 → 1/10 the frames, full-length but choppy). Timing + sway period stay constant (both
        // dt and yaw_step scale with fps). record.sh muxes at the same fps. Default 60 (full quality).
        let fps: f32 = std::env::var("MARTIN_PREVIEW_FPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|&f: &f32| f >= 1.0)
            .unwrap_or(60.0);
        app.insert_resource(RecordState {
            dir: std::env::var("MARTIN_RECORD").ok(),
            dt: 1.0 / fps,
            yaw_step: 2.0 * PI / (8.0 * fps), // ~8s gentle sway period at any fps
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
            ats: std::env::var("MARTIN_SHOTS")
                .ok()
                .map(|s| {
                    s.split(',')
                        .filter_map(|x| x.trim().parse::<f32>().ok())
                        .collect::<Vec<_>>()
                })
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| {
                    vec![
                        std::env::var("MARTIN_SHOT_AT")
                            .ok()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(6.0),
                    ]
                }),
            idx: 0,
            done: false,
        })
        .insert_resource(FpsLog {
            enabled: std::env::var("MARTIN_FPS").is_ok(),
            accum: 0.0,
            frames: 0,
            smoothed_fps: 0.0,
            smoothed_ms: 0.0,
        })
        .add_systems(Startup, (setup_record_target, setup_fps_overlay))
        .add_systems(
            Update,
            (
                attach_record_target,
                record_driver,
                shot_driver,
                live_end,
                fps_log,
                fps_overlay,
            ),
        );
    }
}
