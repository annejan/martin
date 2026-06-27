//! The free-orbit inspection camera + its live controls, the waypoint flypath, and fullscreen
//! toggle. `CameraPlugin` spawns the camera and runs these each frame.

use bevy::camera::Hdr; // moved bevy_render → bevy_camera in 0.19
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
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

/// Seed the free-orbit camera to frame a built sequence. `center` is the framed world centre and
/// `content_radius`/`frame_factor` come from the builder's `frame_of`. `MARTIN_ZOOM` scales the
/// distance (>1 = closer), `MARTIN_YAW`/`MARTIN_PITCH` (radians) seed the orbit angle so a found
/// viewpoint bakes into a render, and `MARTIN_CAMERAS=<cameras.json>` parks the camera at a real
/// capture pose (the only viewpoint a raw 360° scene renders coherently) — transformed through the
/// same `entity_rot` + part-0 `scene_norm` (center, scale) as the gaussians. `MARTIN_CAM_INDEX`
/// picks which shot (default 0). Moved here from `build_sequence` so the builder stays cloud-math.
pub(crate) fn seed_orbit_framing(
    cam: &mut OrbitCam,
    center: Vec3,
    content_radius: f32,
    frame_factor: f32,
    entity_rot: Quat,
    scene_norm: (Vec3, f32),
) {
    use crate::envvar::or as env;
    let zoom = env("MARTIN_ZOOM", 1.0_f32);
    let zoom = if zoom > 0.0 { zoom } else { 1.0 }; // a non-positive zoom is meaningless → default
    let (mut yaw, mut pitch, mut dist) = (
        env("MARTIN_YAW", FRONT_YAW),
        env("MARTIN_PITCH", DEFAULT_PITCH),
        content_radius * frame_factor / zoom,
    );
    if let Ok(cpath) = std::env::var("MARTIN_CAMERAS") {
        let positions = crate::scene::sequence::load_camera_positions(&cpath);
        if positions.is_empty() {
            warn!("MARTIN_CAMERAS: no camera positions in {cpath}");
        } else {
            let idx = std::env::var("MARTIN_CAM_INDEX")
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0)
                .min(positions.len() - 1);
            let (c0, s0) = scene_norm;
            let dir = entity_rot * ((positions[idx] - c0) * s0) - center;
            let len = dir.length().max(1e-4);
            yaw = dir.z.atan2(dir.x);
            pitch = (dir.y / len).asin();
            dist = len / zoom;
            info!(
                "camera: capture pose {idx}/{} from {cpath}",
                positions.len()
            );
        }
    }
    cam.target = center;
    cam.dist = dist;
    cam.yaw = yaw;
    cam.pitch = pitch;
    cam.framed = true;
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
                cut: false,       // a glide by default; add `cut` by hand in the [camera] track
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
    // MARTIN_CAM_SPLINE / `.show` `cam_spline=1`: Catmull-Rom through the [camera] keys (a flowing,
    // never-stopping glide) instead of the default per-leg smoothstep (settles at each key). Read once.
    mut spline: Local<Option<bool>>,
    mut q: Query<&mut OrbitCam>,
) {
    // the live control bridge drives the camera by hand — the authored track stands down.
    if crate::serve::is_serving() {
        return;
    }
    let spline =
        *spline.get_or_insert_with(|| crate::envvar::or("MARTIN_CAM_SPLINE", 0.0_f32) > 0.5);
    // A fully-timed `[camera]` TRACK is AUTHORITATIVE: play it straight off the show clock (same curve
    // live + recording), ALWAYS — no `MARTIN_FLY` needed. (`MARTIN_FLY` only replays an M-key waypoint
    // path: the part-window mode below.) This is how a `.show` `[camera]` (or a Blender-authored camera)
    // drives BOTH compose and reel shows; without it, the build_* auto-frame + record sway own the camera
    // and the authored pose is ignored.
    // Only an INLINE (`.show [camera]`) camera is authoritative-always; a file you're M-AUTHORING must
    // not auto-play (else the 2nd timed waypoint forms a track + snaps the camera back, blocking flying).
    // A file replays only via `MARTIN_FLY` (the part-window mode below).
    // `is_timed` (≥1 timed key), NOT `is_track` (≥2): a SINGLE inline `[camera]` keyframe is a valid
    // held static pose — `pose_at_time` returns it for all `t`. (Gating this on `is_track` silently
    // dropped a lone keyframe, handing the camera to the build_* auto-frame — a sharp authoring footgun.)
    if marks.inline && waypoints::is_timed(&marks.list) {
        if let Some(w) = waypoints::pose_at_time(&marks.list, clock.t, spline) {
            for mut cam in &mut q {
                cam.target = w.target;
                cam.dist = w.dist;
                cam.yaw = w.yaw;
                cam.pitch = w.pitch;
            }
        }
        return;
    }
    let Some(secs) = marks.fly else { return };
    let n = marks.list.len();
    if n < 2 {
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
        let idx = active_shot(starts, clock.t);
        let part_end = starts
            .get(idx + 1)
            .copied()
            .unwrap_or_else(|| show_end(&seq.parts, starts));
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
    let mut cam = commands.spawn((
        GaussianCamera { warmup: true },
        Camera3d::default(),
        Hdr, // HDR target so bright splats bloom
        // film-grade tonemap: bright splats roll off instead of clipping to flat white
        Tonemapping::TonyMcMapface,
        Transform::default(),
        OrbitCam::default(),
    ));
    // Bloom is the look (HDR glow on black) but costs several fullscreen passes/frame — `MARTIN_BLOOM=0`
    // drops it for weak GPUs / perf profiling (the splats still render, just without the glow).
    if std::env::var("MARTIN_BLOOM").as_deref() != Ok("0") {
        cam.insert(Bloom::NATURAL);
    }
    // MARTIN_POST=chroma → a beat-gated fullscreen post-FX on this camera (default-off: no component).
    if let Some(post) = crate::post::settings_from_env() {
        cam.insert(post);
    }
}

/// Apply the `[sync] exposure` knob to the camera bloom — a music-timed
/// brightness ramp. **Gated**: only touches bloom when an exposure source is present, so the default
/// look (`Bloom::NATURAL`) is byte-identical otherwise. The base intensity is captured once, so the
/// knob *scales* it (exposure 1.0 = unchanged). Deterministic: driven by `clock.t` / the score.
fn update_exposure(
    sync: Option<Res<crate::sync::SyncTrack>>,
    clock: Res<SeqClock>,
    mut base: Local<Option<f32>>,
    mut q: Query<&mut Bloom>,
) {
    // exposure is driven by the `[sync] exposure` keyframe track; no track → leave NATURAL untouched.
    let Some(exposure) = sync.as_ref().and_then(|s| s.exposure_at(clock.t)) else {
        return;
    };
    for mut bloom in &mut q {
        let b = *base.get_or_insert(bloom.intensity); // capture NATURAL's intensity once
        bloom.intensity = (b * exposure).clamp(0.0, 1.0);
    }
}

/// Lens-slam: the `[sync] fov` knob scales the camera FOV (1.0 = the default π/4; `0.6` = punched-in
/// telephoto). **Clamped to ≤ 1.0** — only ever NARROWER (zoom in) — because the fullscreen background
/// / interlude quads are sized to the default FOV, and a wider lens would expose their edges. Gated:
/// the FOV is left at its default without a `fov` keyframe. Deterministic (driven by `clock.t`).
fn update_fov(
    sync: Option<Res<crate::sync::SyncTrack>>,
    clock: Res<SeqClock>,
    mut q: Query<&mut Projection>,
) {
    let Some(scale) = sync.as_ref().and_then(|s| s.fov_at(clock.t)) else {
        return;
    };
    let scale = scale.clamp(0.1, 1.0); // never wider than default → the bg/interlude quads stay covered
    for mut proj in &mut q {
        if let Projection::Perspective(p) = proj.as_mut() {
            p.fov = std::f32::consts::FRAC_PI_4 * scale;
        }
    }
}

/// The orbit camera, its live controls, the waypoint flypath, and fullscreen toggle.
pub(crate) struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        // The `Waypoints` resource is inserted by `main` (it may carry a `.show` inline camera track).
        app.add_systems(Startup, spawn_camera).add_systems(
            Update,
            (
                orbit_camera,
                controls,
                flypath,
                fullscreen_toggle,
                update_exposure,
                update_fov,
            ),
        );
    }
}
