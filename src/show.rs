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
    Scenes,
    Compose,
    Camera,
}

impl From<&str> for Section {
    fn from(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "reel" | "seq" | "sequence" | "morph" => Section::Seq,
            "scenes" | "arc" => Section::Scenes,
            "stage" | "compose" => Section::Compose,
            "camera" | "cam" => Section::Camera,
            _ => Section::Settings, // `[settings]`, or anything unknown → top-level knobs
        }
    }
}

/// Flatten a `[scenes]` body into a plain `[reel]` body (the domain-driven authoring layer, L2). A
/// `[scenes]` block is the Showbook arc made executable: you group Shots under named **Scenes**, and
/// each Scene's look is inherited by its Shots. It is pure sugar — it compiles to the exact reel the
/// engine already runs, so nothing downstream changes.
///
/// Grammar:
/// ```text
/// scene <name> [@@anchor] [backdrop:NAME] [^deform]   # opens a scene; the rest are its defaults
///   <shot line>                                        # any normal reel Shot (indent optional)
///   <shot line>
/// scene <name2> …
/// ```
/// Inheritance, on flatten: the Scene's `@@anchor` is stamped on its **first** Shot only (the rest
/// flow sequentially after it); the Scene's `backdrop:` / `^deform` are appended to every Shot that
/// doesn't set its own. Content-agnostic — a Shot is any `splat:`/`mesh:`/`wall:`/`image:`/… line.
fn flatten_scenes(body: &str) -> String {
    let mut reel = String::new();
    // The current scene's inherited defaults: (anchor token, backdrop token, deform token).
    let (mut anchor, mut backdrop, mut deform) = (None, None, None);
    let mut first_shot_pending = false; // the scene's anchor goes on its first shot only
    for raw in body.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("scene ").or(line.strip_prefix("scene\t")) {
            // open a scene: pull its inherited-look tokens out of the header.
            (anchor, backdrop, deform) = (None, None, None);
            for tok in rest.split_whitespace() {
                if tok.starts_with("@@") {
                    anchor = Some(tok.to_string());
                } else if tok.starts_with("backdrop:") || tok.starts_with("bg:") {
                    backdrop = Some(tok.to_string());
                } else if tok.starts_with('^') {
                    deform = Some(tok.to_string());
                }
                // the scene name + anything else is just a label — ignored on flatten.
            }
            first_shot_pending = true;
            continue;
        }
        // a Shot line: apply the scene's inherited defaults it didn't override.
        let mut shot = line.to_string();
        if first_shot_pending {
            if let Some(a) = &anchor {
                if !shot.contains("@@") {
                    shot.push(' ');
                    shot.push_str(a);
                }
            }
            first_shot_pending = false;
        }
        if let Some(b) = &backdrop {
            if !shot.contains("backdrop:") && !shot.contains("bg:") {
                shot.push(' ');
                shot.push_str(b);
            }
        }
        if let Some(d) = &deform {
            if !shot.contains('^') {
                shot.push(' ');
                shot.push_str(d);
            }
        }
        reel.push_str(&shot);
        reel.push('\n');
    }
    reel
}

fn parse_and_apply(text: &str) -> Show {
    let mut section = Section::Settings;
    let (mut seq, mut compose) = (String::new(), String::new());
    let mut scenes = String::new();
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
            // `[scenes]` is the domain arc; collected raw, flattened to a reel after the loop.
            Section::Scenes => {
                scenes.push_str(raw);
                scenes.push('\n');
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
    // `[scenes]` flattens into the reel. If a show has both, the explicit `[reel]` wins (don't double).
    if seq.trim().is_empty() && !scenes.trim().is_empty() {
        seq = flatten_scenes(&scenes);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenes_flatten_to_a_reel_with_inheritance() {
        let body = "\
scene opener @@intro backdrop:off
  glb:defeest.glb @8,3 ~morph
scene party @@drop backdrop:plasma ^wave
  splat:galaxy.ply @5,2 ~morph
  splat:knot.ply @5,2 ~morph backdrop:bolt   # own backdrop wins
  text:HELLO @4,2 ~outline ^twist            # own deform wins
";
        let reel: Vec<String> = flatten_scenes(body).lines().map(String::from).collect();
        assert_eq!(reel.len(), 4);
        // scene anchor stamped on the FIRST shot of each scene only.
        assert!(reel[0].contains("@@intro") && reel[0].contains("backdrop:off"));
        assert!(reel[1].contains("@@drop") && reel[1].contains("backdrop:plasma") && reel[1].contains("^wave"));
        assert!(!reel[2].contains("@@")); // second shot of the scene flows sequentially
        // per-shot overrides beat the scene defaults (no double backdrop / deform).
        assert!(reel[2].contains("backdrop:bolt") && !reel[2].contains("backdrop:plasma"));
        assert!(reel[3].contains("^twist") && !reel[3].contains("^wave"));
        assert!(reel[3].contains("backdrop:plasma")); // but it DOES inherit the scene backdrop
    }
}
