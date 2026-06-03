//! Camera waypoints. Press **M** while flying (see `controls`) to log the live orbit pose; the
//! markers accumulate into a JSON file (`MARTIN_WAYPOINTS`, default `waypoints.json`) so you can
//! author a camera path now and replay it into the demo later. The format is deliberately the
//! OrbitCam's own state (target/dist/yaw/pitch) — a path is just a lerp of these, so playback is
//! trivial when we add it.

use bevy::prelude::*;

/// One logged camera pose — enough to fully reconstruct the `OrbitCam` (its transform is derived
/// from exactly these four). Interpolation-friendly: tween target/dist/yaw/pitch between markers.
#[derive(Clone, Copy)]
pub struct Waypoint {
    pub target: Vec3,
    pub dist: f32,
    pub yaw: f32,
    pub pitch: f32,
}

/// The markers logged this session + the file they're written to. Each `M` press appends one and
/// rewrites the whole file, so it stays valid JSON without an append-and-fix-up dance.
#[derive(Resource)]
pub struct Waypoints {
    pub list: Vec<Waypoint>,
    pub path: String,
}

impl Waypoints {
    pub fn from_env() -> Self {
        Self {
            list: Vec::new(),
            path: std::env::var("MARTIN_WAYPOINTS").unwrap_or_else(|_| "waypoints.json".into()),
        }
    }
}

/// Write the markers to the martin-native waypoints JSON: an array of
/// `{ "target": [x, y, z], "dist", "yaw", "pitch" }`, re-loadable for path playback later.
pub fn save(list: &[Waypoint], path: &str) -> std::io::Result<()> {
    let arr: Vec<serde_json::Value> = list
        .iter()
        .map(|w| {
            serde_json::json!({
                "target": [w.target.x, w.target.y, w.target.z],
                "dist": w.dist,
                "yaw": w.yaw,
                "pitch": w.pitch,
            })
        })
        .collect();
    let text = serde_json::to_string_pretty(&serde_json::Value::Array(arr))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(path, text)
}
