//! The free-orbit inspection camera + its live controls, the waypoint flypath, and fullscreen
//! toggle. `CameraPlugin` spawns the camera and runs these each frame.

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::Hdr;
use bevy::window::{MonitorSelection, WindowMode};
use bevy_gaussian_splatting::GaussianCamera;

use crate::capture::RecordState;
use crate::scene::SeqClock;
use crate::scene::sequence::{SeqState, Sequence, active_shot, show_end};
use crate::waypoints;

pub(crate) const FRONT_YAW: f32 = 1.4; // camera faces the subject head-on (single-image splats have no back)
pub(crate) const SWAY: f32 = 0.25; // gentle left-right sway amplitude — never reaches the hollow back
pub(crate) const DEFAULT_PITCH: f32 = 0.12; // camera pitch above the horizon (rad) when framing

/// Free-orbit inspection camera: orbit `yaw`/`pitch` at `dist` around a `target` look-at point.
/// `build_sequence` frames it (MARTIN_YAW/PITCH/ZOOM seed it); `controls` flies it live; the
/// recorder sways or holds it deterministically.
#[derive(Component)]
pub(crate) struct OrbitCam {
    pub target: Vec3, // look-at point
    pub dist: f32,    // distance from the target
    pub yaw: f32,     // orbit angle around the vertical (Y) axis
    pub pitch: f32,   // angle above the horizon (0 = eye level, +up looks down)
    pub framed: bool,
}

impl Default for OrbitCam {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            dist: 5.0,
            yaw: FRONT_YAW,
            pitch: DEFAULT_PITCH,
            framed: false,
        }
    }
}

/// Place the camera on a sphere around `target` from `yaw`/`pitch`/`dist`. With `MARTIN_CAM_PUMP=<s>`
/// the kick beat-pumps a transient lunge inward (clean per-frame offset, not stored `dist`, so it
/// bakes identically into recordings). **Off by default** — the camera shake is nauseating on a long
/// loop; opt in (e.g. `0.04`) for a single punchy clip.
fn orbit_camera(
    beat: Option<Res<crate::scene::beat::Beat>>,
    mut q: Query<(&mut Transform, &OrbitCam)>,
    mut amt: Local<Option<f32>>,
) {
    let amt = *amt.get_or_insert_with(|| crate::envvar::or("MARTIN_CAM_PUMP", 0.0_f32));
    let pump = if amt == 0.0 {
        1.0
    } else {
        beat.map(|b| 1.0 - b.kick * amt * b.intensity)
            .unwrap_or(1.0)
    };
    for (mut tf, cam) in &mut q {
        let (sp, cp) = cam.pitch.sin_cos();
        let (sy, cy) = cam.yaw.sin_cos();
        tf.translation = cam.target + Vec3::new(cp * cy, sp, cp * sy) * (cam.dist * pump);
        tf.look_at(cam.target, Vec3::Y);
    }
}

/// Live free-orbit controls (ignored while recording): **arrows** orbit (←/→ yaw, ↑/↓ pitch),
/// **W/S** zoom in/out, **A/D** pan left/right, **Q/E** pan down/up, **M** logs a camera
/// waypoint (→ the waypoints file), **Space** restarts.
fn controls(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    rec: Res<RecordState>,
    mut clock: ResMut<SeqClock>,
    mut marks: ResMut<waypoints::Waypoints>,
    mut q: Query<&mut OrbitCam>,
) {
    if rec.dir.is_some() {
        return; // record_driver drives the camera + clock deterministically while recording
    }
    // path playback owns the camera: while flying a loaded path (MARTIN_FLY) skip live orbit +
    // marking, but keep Space (restart) working so you can re-watch the move.
    if marks.fly.is_some() && marks.list.len() >= 2 {
        if keys.just_pressed(KeyCode::Space) {
            clock.t = 0.0;
        }
        return;
    }
    let dt = time.delta_secs();
    for mut cam in &mut q {
        let orbit = 1.3 * dt; // rad/s
        if keys.pressed(KeyCode::ArrowLeft) {
            cam.yaw -= orbit;
        }
        if keys.pressed(KeyCode::ArrowRight) {
            cam.yaw += orbit;
        }
        if keys.pressed(KeyCode::ArrowUp) {
            cam.pitch = (cam.pitch + orbit).min(1.5);
        }
        if keys.pressed(KeyCode::ArrowDown) {
            cam.pitch = (cam.pitch - orbit).max(-1.5);
        }
        let step = cam.dist.max(0.1) * dt;
        if keys.pressed(KeyCode::KeyW) {
            cam.dist = (cam.dist - step).max(0.05); // zoom in
        }
        if keys.pressed(KeyCode::KeyS) {
            cam.dist += step; // zoom out
        }
        // pan the look-at target: A/D along the camera's horizontal right, Q/E along world up.
        let right = Vec3::new(cam.yaw.sin(), 0.0, -cam.yaw.cos());
        let pan = cam.dist.max(0.1) * 0.6 * dt;
        if keys.pressed(KeyCode::KeyA) {
            cam.target -= right * pan;
        }
        if keys.pressed(KeyCode::KeyD) {
            cam.target += right * pan;
        }
        if keys.pressed(KeyCode::KeyQ) {
            cam.target.y -= pan;
        }
        if keys.pressed(KeyCode::KeyE) {
            cam.target.y += pan;
        }
    }
    // M: drop a camera waypoint — log the live orbit pose into the waypoints file, accumulating a
    // camera path you can replay / author the demo's camera moves from later.
    if keys.just_pressed(KeyCode::KeyM) {
        if let Ok(cam) = q.single() {
            marks.list.push(waypoints::Key {
                target: cam.target,
                dist: cam.dist,
                yaw: cam.yaw,
                pitch: cam.pitch,
                t: Some(clock.t), // stamp the show-time → an authored path is a music-timed track
            });
            match waypoints::save(&marks.list, &marks.path) {
                Ok(()) => info!(
                    "waypoint #{} @ t={:.1}s → {} (yaw {:.3}, pitch {:.3}, dist {:.2}, target [{:.2}, {:.2}, {:.2}])",
                    marks.list.len(),
                    clock.t,
                    marks.path,
                    cam.yaw,
                    cam.pitch,
                    cam.dist,
                    cam.target.x,
                    cam.target.y,
                    cam.target.z,
                ),
                Err(e) => warn!("waypoint save failed: {e}"),
            }
        }
    }
    if keys.just_pressed(KeyCode::Space) {
        clock.t = 0.0; // restart the show
    }
}

/// Triangle wave 0→1→0 over the unit interval — a there-and-back ease for path playback.
fn pingpong(x: f32) -> f32 {
    if x < 0.5 { x * 2.0 } else { 2.0 - x * 2.0 }
}

/// `MARTIN_FLY=<secs>`: fly the camera through the loaded waypoints (the M-key path). While
/// **recording**, the path **fills each part's on-screen window**, **alternating direction**
/// (part 0 first→last, part 1 last→first, …) — so the camera is always moving (it reaches the
/// turn-marker exactly as the morph begins: no dead hold before the transition) and its position
/// is *continuous* across the morph (the next subject reverses from there: no jump). A part's
/// flyby is therefore as long as its `hold`. **Live**, `secs` sets the pace (time per leg) and it
/// ping-pongs the path on a loop for preview. Owns the camera (`controls` + recorder sway stand down).
fn flypath(
    marks: Res<waypoints::Waypoints>,
    rec: Res<RecordState>,
    seq: Option<Res<Sequence>>,
    state: Option<Res<SeqState>>,
    clock: Res<SeqClock>,
    mut q: Query<&mut OrbitCam>,
) {
    // the live control bridge drives the camera by hand — the authored track stands down.
    if crate::serve::is_serving() {
        return;
    }
    let Some(secs) = marks.fly else { return };
    let n = marks.list.len();
    if n < 2 {
        return;
    }
    // a fully-timed path is a CAMERA TRACK: play it straight off the show clock — same curve live
    // and in the recording, no part-window heuristic. (Authored with M, which stamps the clock.)
    if waypoints::is_track(&marks.list) {
        if let Some(w) = waypoints::pose_at_time(&marks.list, clock.t) {
            for mut cam in &mut q {
                cam.target = w.target;
                cam.dist = w.dist;
                cam.yaw = w.yaw;
                cam.pitch = w.pitch;
            }
        }
        return;
    }
    let legs = (n - 1) as f32;
    let p = if rec.dir.is_some() {
        let (Some(seq), Some(state)) = (&seq, &state) else {
            return;
        };
        if !state.built {
            return;
        }
        // recording = the demo: the path fills each part's on-screen window (its slice of the
        // timeline), ALTERNATING direction (even parts first→last, odd parts last→first). Filling
        // the window keeps the camera always moving — it reaches the turn-marker exactly as the
        // morph begins, then reverses through it: no dead hold before the transition, no jump.
        // (So a part's flyby lasts its hold; live still paces by `secs` per leg.)
        let starts = state.starts();
        let idx = active_shot(&starts, clock.t);
        let part_end = starts
            .get(idx + 1)
            .copied()
            .unwrap_or_else(|| show_end(&seq.parts, &starts));
        let local = ((clock.t - starts[idx]) / (part_end - starts[idx]).max(0.1)).clamp(0.0, 1.0);
        if idx.is_multiple_of(2) {
            local
        } else {
            1.0 - local
        }
    } else {
        // live: ping-pong there-and-back at `secs` per leg, looping for preview.
        pingpong((clock.t / (2.0 * secs * legs)).fract())
    };
    let Some(w) = waypoints::pose_at(&marks.list, p) else {
        return;
    };
    for mut cam in &mut q {
        cam.target = w.target;
        cam.dist = w.dist;
        cam.yaw = w.yaw;
        cam.pitch = w.pitch;
    }
}

/// F11 / F: toggle borderless fullscreen at runtime.
fn fullscreen_toggle(keys: Res<ButtonInput<KeyCode>>, mut windows: Query<&mut Window>) {
    if keys.just_pressed(KeyCode::F11) || keys.just_pressed(KeyCode::KeyF) {
        for mut w in &mut windows {
            w.mode = match w.mode {
                WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
                _ => WindowMode::Windowed,
            };
        }
    }
}

/// Spawn the HDR + bloom camera with its `OrbitCam` (framed later by `build_sequence` /
/// `build_composition`).
fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        GaussianCamera { warmup: true },
        Camera3d::default(),
        Hdr, // HDR target so bright splats bloom
        Tonemapping::None,
        Bloom::NATURAL,
        Transform::default(),
        OrbitCam::default(),
    ));
}

/// The orbit camera, its live controls, the waypoint flypath, and fullscreen toggle.
pub(crate) struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        // The `Waypoints` resource is inserted by `main` (it may carry a `.show` inline camera track).
        app.add_systems(Startup, spawn_camera)
            .add_systems(Update, (orbit_camera, controls, flypath, fullscreen_toggle));
    }
}
