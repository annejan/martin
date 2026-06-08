//! Camera waypoints. Press **M** while flying (see `controls`) to log the live orbit pose; the
//! markers accumulate into a JSON file (`MARTIN_WAYPOINTS`, default `waypoints.json`) so you can
//! author a camera path now and replay it into the demo later. The format is deliberately the
//! OrbitCam's own state (target/dist/yaw/pitch) — a path is just a lerp of these, so playback
//! (`MARTIN_FLY=<secs>`, driven by `flypath` in `main.rs`) is a simple interpolation.

use bevy::prelude::*;

/// One logged camera pose — enough to fully reconstruct the `OrbitCam` (its transform is derived
/// from exactly these four). Interpolation-friendly: tween target/dist/yaw/pitch between markers.
/// `t` is an optional **show-time anchor** (seconds): when *every* waypoint carries one the path is
/// a music-timed **camera track** — played straight off the show clock (`pose_at_time`) instead of
/// the part-window heuristic. `M` stamps the live clock, so an authored path is a track by default.
#[derive(Clone, Copy)]
pub struct Waypoint {
    pub target: Vec3,
    pub dist: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub t: Option<f32>,
}

/// The markers logged this session + the file they're written to. Each `M` press appends one and
/// rewrites the whole file, so it stays valid JSON without an append-and-fix-up dance.
#[derive(Resource)]
pub struct Waypoints {
    pub list: Vec<Waypoint>,
    pub path: String,
    /// `MARTIN_FLY=<secs>`: replay the path instead of free-orbiting; `secs` is the **time per
    /// waypoint leg** (time between markers). `Some(secs)` = enabled.
    pub fly: Option<f32>,
}

impl Waypoints {
    /// Load the path from `MARTIN_WAYPOINTS` (M continues it, `MARTIN_FLY` replays it).
    pub fn from_env() -> Self {
        Self::build(None)
    }

    /// Use an inline track (from a `.show` file's `[camera]` section) instead of loading the file;
    /// the file path / fly settings still come from the env (M-saves still target the file).
    pub fn from_inline(list: Vec<Waypoint>) -> Self {
        Self::build(Some(list))
    }

    fn build(inline: Option<Vec<Waypoint>>) -> Self {
        let path = std::env::var("MARTIN_WAYPOINTS").unwrap_or_else(|_| "waypoints.json".into());
        Self {
            // inline wins; else seed from the file so M *continues* a path and MARTIN_FLY replays it.
            list: inline.unwrap_or_else(|| load(&path)),
            path,
            fly: std::env::var("MARTIN_FLY")
                .ok()
                .map(|s| s.trim().parse::<f32>().unwrap_or(2.0).max(0.05)),
        }
    }
}

/// Write the markers to the martin-native waypoints JSON: an array of
/// `{ "target": [x, y, z], "dist", "yaw", "pitch" }`, re-loadable for path playback later.
pub fn save(list: &[Waypoint], path: &str) -> std::io::Result<()> {
    let arr: Vec<serde_json::Value> = list
        .iter()
        .map(|w| {
            let mut o = serde_json::json!({
                "target": [w.target.x, w.target.y, w.target.z],
                "dist": w.dist,
                "yaw": w.yaw,
                "pitch": w.pitch,
            });
            if let Some(t) = w.t {
                o["t"] = serde_json::json!(t); // only timed waypoints carry the anchor
            }
            o
        })
        .collect();
    let text = serde_json::to_string_pretty(&serde_json::Value::Array(arr))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(path, text)
}

/// Read a waypoints file written by `save` (same JSON shape). Missing / unparseable → empty.
pub fn load(path: &str) -> Vec<Waypoint> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    json.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|w| {
                    let t = w.get("target")?.as_array()?;
                    Some(Waypoint {
                        target: Vec3::new(
                            t.first()?.as_f64()? as f32,
                            t.get(1)?.as_f64()? as f32,
                            t.get(2)?.as_f64()? as f32,
                        ),
                        dist: w.get("dist")?.as_f64()? as f32,
                        yaw: w.get("yaw")?.as_f64()? as f32,
                        pitch: w.get("pitch")?.as_f64()? as f32,
                        t: w.get("t").and_then(|t| t.as_f64()).map(|t| t as f32),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Interpolate the path at normalized progress `p` ∈ [0,1]: choose the leg, ease across it with
/// smoothstep (the camera settles as it passes each marker), lerp target/dist/pitch and take the
/// shortest angular path for yaw. `None` only if the path is empty.
pub fn pose_at(list: &[Waypoint], p: f32) -> Option<Waypoint> {
    match list.len() {
        0 => return None,
        1 => return list.first().copied(),
        _ => {}
    }
    let segs = (list.len() - 1) as f32;
    let x = (p.clamp(0.0, 1.0) * segs).min(segs - 1e-4);
    let i = x.floor() as usize;
    let u = x - i as f32;
    let e = u * u * (3.0 - 2.0 * u); // smoothstep ease across the leg
    let (a, b) = (list[i], list[i + 1]);
    Some(Waypoint {
        target: a.target.lerp(b.target, e),
        dist: a.dist + (b.dist - a.dist) * e,
        yaw: a.yaw + shortest_angle(b.yaw - a.yaw) * e,
        pitch: a.pitch + (b.pitch - a.pitch) * e,
        t: None,
    })
}

/// A path is a **camera track** when every waypoint carries a time anchor (and there are ≥2): the
/// flypath then plays it straight off the show clock via `pose_at_time` instead of the part-window
/// heuristic. A path freshly authored with `M` (which stamps the clock) is therefore a track.
pub fn is_track(list: &[Waypoint]) -> bool {
    list.len() >= 2 && list.iter().all(|w| w.t.is_some())
}

/// Sample a *timed* track at absolute show-time `t` (seconds): find the bracketing pair by their
/// anchors, smoothstep between them, clamp at the ends (hold the first pose before the track starts,
/// the last after it ends). Assumes `is_track(list)` — anchors are taken as monotonically authored.
pub fn pose_at_time(list: &[Waypoint], t: f32) -> Option<Waypoint> {
    if list.len() < 2 {
        return list.first().copied();
    }
    let ta = |w: &Waypoint| w.t.unwrap_or(0.0);
    if t <= ta(&list[0]) {
        return list.first().copied();
    }
    if t >= ta(&list[list.len() - 1]) {
        return list.last().copied();
    }
    let i = list.windows(2).position(|p| t < ta(&p[1])).unwrap_or(0);
    let (a, b) = (list[i], list[i + 1]);
    let span = (ta(&b) - ta(&a)).max(1e-4);
    let u = ((t - ta(&a)) / span).clamp(0.0, 1.0);
    let e = u * u * (3.0 - 2.0 * u); // smoothstep — settle through each marker
    Some(Waypoint {
        target: a.target.lerp(b.target, e),
        dist: a.dist + (b.dist - a.dist) * e,
        yaw: a.yaw + shortest_angle(b.yaw - a.yaw) * e,
        pitch: a.pitch + (b.pitch - a.pitch) * e,
        t: Some(t),
    })
}

/// Wrap an angle delta into [-π, π] so yaw interpolates the short way around.
fn shortest_angle(d: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (d + PI).rem_euclid(TAU) - PI
}
