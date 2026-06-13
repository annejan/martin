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

/// What a show file contributes that *isn't* an env var: the raw `[camera]` track lines. They're
/// parsed later (in `main`, by `waypoints::parse_camera`) once the score exists, so a keyframe can
/// anchor its time to a music section (`t=@@drop`). Everything else is expanded into the env here.
#[derive(Default)]
pub struct Show {
    pub camera: Vec<String>,
}

/// Expand `MARTIN_SHOW` into the env and return its `[camera]` track. A no-op (empty `Show`) when
/// `MARTIN_SHOW` is unset or unreadable. Call this **first** in `main`, before anything reads the
/// env (the score, the sequence, the composition).
pub fn apply() -> Show {
    let Ok(spec) = std::env::var("MARTIN_SHOW") else {
        return Show::default();
    };
    // A path OR inline show text (same convention as MARTIN_SEQ/_COMPOSE) — so the bundled build can
    // pre-seed the baked-in show text directly, with no temp file.
    let text = std::fs::read_to_string(&spec).unwrap_or(spec);
    parse_and_apply(&text)
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
            "reel" | "seq" | "sequence" | "morph" => Section::Seq,
            "stage" | "compose" => Section::Compose,
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
            // keep the raw camera lines (comment-stripped); parsed after the score is built.
            Section::Camera => {
                let s = line.split('#').next().unwrap_or("").trim();
                if !s.is_empty() {
                    camera.push(s.to_string());
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
        // SAFETY: the show is parsed at startup, single-threaded, before the Bevy app spawns threads.
        unsafe { std::env::set_var("MARTIN_FLY", "1") };
    }
    Show { camera }
}

/// Canonicalise a settings key: the DOMAIN.md domain spelling → the env var the engine reads. Keeps
/// the old keys working (an alias, not a replacement). E.g. `budget = 200000` → `MARTIN_MORPH_COUNT`.
fn canonical_key(key: &str) -> &str {
    match key.to_ascii_lowercase().as_str() {
        "budget" => "morph_count", // splat budget (DOMAIN.md): the real meaning of morph_count
        "backdrop" => "bg",        // default backdrop shader
        _ => key,
    }
}

/// `key = value` → set `MARTIN_<KEY>=value`, but only if that env var isn't already set, so an
/// explicit CLI env var wins over the file.
fn set_if_absent(key: &str, value: &str) {
    if key.is_empty() {
        return;
    }
    let var = format!("MARTIN_{}", canonical_key(key).to_ascii_uppercase());
    if std::env::var(&var).is_err() {
        // SAFETY: show settings are applied at startup, single-threaded, before any threads spawn.
        unsafe { std::env::set_var(var, value) };
    }
}
