//! The unified scene file (`MARTIN_SHOW=<file>.show`): one file that pulls a whole show together —
//! top-level settings, the morph `[seq]`, the `[compose]` stage, and a music-timed `[camera]` track
//! — instead of a scatter of `MARTIN_*` env vars plus a separate `waypoints.json`.
//!
//! It is deliberately pure sugar over the existing env-driven pipeline: `apply()` *expands* the file
//! into the env (each top-level `key = value` → `MARTIN_<KEY>`, the `[seq]`/`[compose]` section
//! bodies → `MARTIN_SEQ` / `MARTIN_COMPOSE` verbatim) so every existing parser runs unchanged. The
//! env vars are simply the "compiled" form of a show. An explicit CLI env var always **wins** over
//! the file (we only set what isn't already set), so `MARTIN_DEFORM=turb MARTIN_SHOW=x.show …` still
//! overrides one knob. The only thing that isn't an env var — the inline `[camera]` track — is
//! returned for `main` to hand to the camera.

use bevy::math::Vec3;

use crate::waypoints::Waypoint;

/// What a show file contributes that *isn't* an env var: the inline camera track. Everything else
/// has already been expanded into the env by `apply` by the time this is returned.
#[derive(Default)]
pub struct Show {
    pub camera: Vec<Waypoint>,
}

/// Expand `MARTIN_SHOW` into the env and return its `[camera]` track. A no-op (empty `Show`) when
/// `MARTIN_SHOW` is unset or unreadable. Call this **first** in `main`, before anything reads the
/// env (the score, the sequence, the composition).
pub fn apply() -> Show {
    let Ok(path) = std::env::var("MARTIN_SHOW") else {
        return Show::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_and_apply(&text),
        Err(e) => {
            eprintln!("show: cannot read {path}: {e}");
            Show::default()
        }
    }
}

/// Which section a body line belongs to. Lines before the first `[header]` are top-level settings.
enum Section {
    Settings,
    Seq,
    Compose,
    Camera,
}

impl From<&str> for Section {
    fn from(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "seq" | "sequence" | "morph" => Section::Seq,
            "compose" | "stage" => Section::Compose,
            "camera" | "cam" => Section::Camera,
            _ => Section::Settings, // `[settings]`, or anything unknown → top-level knobs
        }
    }
}

fn parse_and_apply(text: &str) -> Show {
    let mut section = Section::Settings;
    let (mut seq, mut compose) = (String::new(), String::new());
    let mut camera = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = Section::from(name.trim());
            continue;
        }
        match section {
            // `key = value` → `MARTIN_<KEY>` (a `#` comment ends the value).
            Section::Settings => {
                let s = line.split('#').next().unwrap_or("").trim();
                if let Some((k, v)) = s.split_once('=') {
                    set_if_absent(k.trim(), v.trim());
                }
            }
            // Section bodies pass VERBATIM to their existing parsers (which split on `;`/newline and
            // strip `#` comments), so the body is exactly today's `.seq` / `.compose` syntax.
            Section::Seq => {
                seq.push_str(raw);
                seq.push('\n');
            }
            Section::Compose => {
                compose.push_str(raw);
                compose.push('\n');
            }
            Section::Camera => {
                if let Some(w) = parse_waypoint(line) {
                    camera.push(w);
                }
            }
        }
    }
    if !seq.trim().is_empty() {
        set_if_absent("seq", seq.trim());
    }
    if !compose.trim().is_empty() {
        set_if_absent("compose", compose.trim());
    }
    // An inline `[camera]` section is a track → make sure it actually plays (`MARTIN_FLY` may still
    // override the pace; for a timed track the value is ignored anyway, see `flypath`).
    if !camera.is_empty() && std::env::var("MARTIN_FLY").is_err() {
        std::env::set_var("MARTIN_FLY", "1");
    }
    Show { camera }
}

/// `key = value` → set `MARTIN_<KEY>=value`, but only if that env var isn't already set, so an
/// explicit CLI env var wins over the file.
fn set_if_absent(key: &str, value: &str) {
    if key.is_empty() {
        return;
    }
    let var = format!("MARTIN_{}", key.to_ascii_uppercase());
    if std::env::var(&var).is_err() {
        std::env::set_var(var, value);
    }
}

/// One `[camera]` line: order-free `key=value` tokens — `t` (show-time s; omit → untimed marker),
/// `pos` (look-at `x,y,z`), `dist`, `yaw`, `pitch` (radians). Defaults match the front-on framing.
fn parse_waypoint(line: &str) -> Option<Waypoint> {
    let s = line.split('#').next().unwrap_or("").trim();
    if s.is_empty() {
        return None;
    }
    let mut w = Waypoint {
        target: Vec3::ZERO,
        dist: 5.0,
        yaw: crate::camera::FRONT_YAW,
        pitch: crate::camera::DEFAULT_PITCH,
        t: None,
    };
    for tok in s.split_whitespace() {
        let Some((k, v)) = tok.split_once('=') else {
            continue;
        };
        match k {
            "t" | "time" => w.t = v.parse().ok(),
            "dist" | "d" => w.dist = v.parse().unwrap_or(w.dist),
            "yaw" => w.yaw = v.parse().unwrap_or(w.yaw),
            "pitch" => w.pitch = v.parse().unwrap_or(w.pitch),
            "pos" | "target" => w.target = parse_vec3(v).unwrap_or(w.target),
            _ => {}
        }
    }
    Some(w)
}

/// `x,y,z` → `Vec3` (all three required).
fn parse_vec3(s: &str) -> Option<Vec3> {
    let mut it = s.split(',').map(|c| c.trim().parse::<f32>());
    Some(Vec3::new(
        it.next()?.ok()?,
        it.next()?.ok()?,
        it.next()?.ok()?,
    ))
}
