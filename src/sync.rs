//! `[sync]` — a music-timed **look track**, the GNU-Rocket-style automation layer (DOMAIN.md §9
//! Stage 4 / §6 SyncTrack). Where `[camera]` keyframes the camera pose over the score clock, `[sync]`
//! keyframes the *global look knobs* — `flash`, `bg_dim`, `beat` — so the bloom can swell into the
//! drop, the backdrop dim through the breakdown, the beat-reactivity ramp to the climax, etc.,
//! interpolated (smoothstep) between keyframes instead of one static value for the whole show.
//!
//! **Generic knob storage:** any knob name is accepted — `flash`, `beat`, `bg_dim`, `exposure`, `fov`,
//! `tint_music`, and `bg` have typed convenience accessors, but any float-valued knob can be keyframed
//! and read via [`at`](SyncTrack::at). No unknown-knob warnings — everything stores and serves.
//!
//! Syntax (same grammar as `[camera]`): one keyframe per line, many knobs per keyframe:
//! ```text
//! [sync]
//! t=@@intro     flash=0.0  bg_dim=0.5  beat=0.4
//! t=@@drop      flash=0.6  bg_dim=0.3  beat=1.3
//! t=@@climax    flash=0.8  bg_dim=0.15 beat=1.6
//! ```
//! A knob omitted on a line is simply not a keyframe for it (each knob has its own keyframe list).
//! No `[sync]` section (or a knob with no keyframes) → the static `MARTIN_*` value is used, unchanged.

use std::collections::HashMap;

use bevy::prelude::*;

/// Per-knob keyframe lists `(time_s, value)`, each sorted by time. Stored in a generic HashMap so any
/// knob name is accepted — typed accessors below are conveniences for the known set.
///
/// Resolved per frame by the consumer systems (`shot_director` for `flash`, `update_beat` for `beat`,
/// `update_bg` for `bg_dim`) — each reads its own knob at its own point, so there's no cross-system
/// ordering to get wrong.
#[derive(Resource, Default)]
pub struct SyncTrack {
    /// `knob_name → sorted keyframes`. The generic core; typed methods delegate to it.
    data: HashMap<String, Vec<(f32, f32)>>,
}

impl SyncTrack {
    /// Smoothstep-interpolated value for `knob` at time `t`. `None` if the knob has no keyframes.
    pub fn at(&self, knob: &str, t: f32) -> Option<f32> {
        eval(self.data.get(knob)?, t)
    }

    /// STEP lookup for `knob` at time `t`: last value ≤ `t` (no interpolation). `None` before the
    /// first keyframe, so a default or env var can win until then.
    pub fn step_at(&self, knob: &str, t: f32) -> Option<f32> {
        let keys = self.data.get(knob)?;
        step(keys, t)
    }

    /// Typed convenience — delegates to [`at`](SyncTrack::at).
    pub fn flash_at(&self, t: f32) -> Option<f32> {
        self.at("flash", t)
    }
    pub fn bg_dim_at(&self, t: f32) -> Option<f32> {
        self.at("bg_dim", t)
    }
    pub fn beat_at(&self, t: f32) -> Option<f32> {
        self.at("beat", t)
    }
    pub fn exposure_at(&self, t: f32) -> Option<f32> {
        self.at("exposure", t)
    }
    pub fn fov_at(&self, t: f32) -> Option<f32> {
        self.at("fov", t)
    }
    pub fn tint_music_at(&self, t: f32) -> Option<f32> {
        self.at("tint_music", t)
    }
    /// Backdrop mode — a discrete STEP (lerping between mode indices is meaningless). `None` before
    /// the first `bg=` keyframe (→ the env / per-part backdrop wins).
    pub fn bg_at(&self, t: f32) -> Option<f32> {
        self.step_at("bg", t)
    }

    /// True when no knob has any keyframes at all.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// `(knob_name, keyframes_slice)` for every stored knob — for the `MARTIN_VALIDATE` dump.
    pub fn knobs(&self) -> Vec<(String, &[(f32, f32)])> {
        let mut pairs: Vec<_> = self
            .data
            .iter()
            .map(|(k, v)| (k.clone(), v.as_slice()))
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        pairs
    }

    /// Insert a keyframe into the generic store. Public so other parsers (e.g. `[scenes]` → `[sync]`
    /// expansion) can push keyframes without touching the parser. Keeps the knob's list sorted.
    #[allow(dead_code)]
    pub fn push(&mut self, knob: impl Into<String>, time: f32, value: f32) {
        let name = knob.into();
        self.data
            .entry(name.clone())
            .or_default()
            .push((time, value));
        // keep sorted (lists are tiny — this is fine per-insert).
        if let Some(v) = self.data.get_mut(&name) {
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
    }

    fn insert(&mut self, knob: &str, time: f32, value: f32) {
        self.data
            .entry(knob.to_string())
            .or_default()
            .push((time, value));
    }

    fn sort_all(&mut self) {
        for v in self.data.values_mut() {
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
    }
}

/// Evaluate a keyframe list at time `t`: clamp before the first / after the last, smoothstep between.
fn eval(keys: &[(f32, f32)], t: f32) -> Option<f32> {
    match keys {
        [] => None,
        [(_, v)] => Some(*v),
        _ => {
            if t <= keys[0].0 {
                return Some(keys[0].1);
            }
            if t >= keys[keys.len() - 1].0 {
                return Some(keys[keys.len() - 1].1);
            }
            let i = keys.partition_point(|&(kt, _)| kt <= t).max(1);
            let (t0, v0) = keys[i - 1];
            let (t1, v1) = keys[i];
            let f = ((t - t0) / (t1 - t0).max(1e-6)).clamp(0.0, 1.0);
            let s = f * f * (3.0 - 2.0 * f); // smoothstep
            Some(v0 + (v1 - v0) * s)
        }
    }
}

/// STEP lookup: the value of the last keyframe at or before `t` (no interpolation — for discrete
/// values like a mode index). `None` before the first keyframe, so a default can win until then.
fn step(keys: &[(f32, f32)], t: f32) -> Option<f32> {
    if keys.is_empty() || t < keys[0].0 {
        return None;
    }
    Some(keys[keys.partition_point(|&(kt, _)| kt <= t) - 1].1)
}

/// Parse `[sync]` lines (same time grammar as `[camera]`): `t=<secs|@@anchor> knob=value …`. The
/// score resolves the anchors. **Any knob name is accepted** — the generic store handles it. `bg`/
/// `backdrop` values are mode NAMES mapped through [`crate::background::mode_index`]; all others
/// must be parseable floats.
pub fn parse_sync(lines: &[String], score: &crate::score::Score) -> SyncTrack {
    let mut track = SyncTrack::default();
    for line in lines {
        let s = line.split('#').next().unwrap_or("").trim();
        if s.is_empty() {
            continue;
        }
        // resolve the keyframe time first (the `t=`/`time=` token).
        let time = s.split_whitespace().find_map(|tok| {
            let (k, v) = tok.split_once('=')?;
            if k != "t" && k != "time" {
                return None;
            }
            match v.strip_prefix("@@") {
                Some(a) => score.anchor_seconds(a),
                None => v.parse().ok(),
            }
        });
        let Some(time) = time else {
            eprintln!("sync: line without a valid `t=` time — skipped: {s}");
            continue;
        };
        for (k, v) in s.split_whitespace().filter_map(|t| t.split_once('=')) {
            match k {
                "t" | "time" => {}
                "bg" | "backdrop" => {
                    // backdrop mode name → index (e.g. "fractal" → 8.0)
                    track.insert("bg", time, crate::background::mode_index(v) as f32);
                }
                _ => {
                    let Ok(val) = v.parse::<f32>() else { continue };
                    track.insert(k, time, val);
                }
            }
        }
    }
    track.sort_all();
    track
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_clamps_and_interpolates() {
        let k = vec![(0.0, 0.0), (10.0, 1.0)];
        assert_eq!(eval(&k, -1.0), Some(0.0)); // before → first
        assert_eq!(eval(&k, 11.0), Some(1.0)); // after → last
        assert_eq!(eval(&k, 5.0), Some(0.5)); // midpoint, smoothstep(0.5)=0.5
        assert_eq!(eval(&[], 3.0), None); // no keys → fall back to static
    }

    #[test]
    fn eval_picks_the_right_segment_with_many_keyframes() {
        let k = vec![(0.0, 0.0), (10.0, 1.0), (20.0, 0.0)]; // up then down
        assert_eq!(eval(&k, 5.0), Some(0.5)); // first segment midpoint
        assert_eq!(eval(&k, 10.0), Some(1.0)); // exactly on a keyframe
        assert_eq!(eval(&k, 15.0), Some(0.5)); // second segment midpoint
        assert_eq!(eval(&k, 25.0), Some(0.0)); // past the end → last
    }

    #[test]
    fn knobs_lists_only_the_keyframed_channels() {
        let score = crate::score::Score::builtin();
        let tr = parse_sync(
            &["t=0 flash=0.2".to_string(), "t=5 beat=1.0".to_string()],
            &score,
        );
        let named: Vec<String> = tr.knobs().into_iter().map(|(n, _)| n).collect();
        assert_eq!(named, vec!["beat".to_string(), "flash".to_string()]);
        assert!(!tr.is_empty());
        assert!(SyncTrack::default().is_empty());
    }

    #[test]
    fn parse_resolves_time_and_knobs() {
        let score = crate::score::Score::builtin();
        let lines = vec![
            "t=0 flash=0.1 bg_dim=0.5 exposure=1.0 fov=1.0".to_string(),
            "t=10 flash=0.9 beat=1.5 exposure=1.6 fov=0.6".to_string(),
        ];
        let tr = parse_sync(&lines, &score);
        assert_eq!(tr.flash_at(0.0), Some(0.1));
        assert_eq!(tr.flash_at(10.0), Some(0.9));
        assert_eq!(tr.beat_at(10.0), Some(1.5));
        assert_eq!(tr.bg_dim_at(0.0), Some(0.5));
        assert!(tr.bg_dim_at(100.0).is_some()); // clamps to last bg_dim keyframe
        assert_eq!(tr.exposure_at(10.0), Some(1.6)); // exposure knob
        assert_eq!(tr.fov_at(10.0), Some(0.6)); // fov lens-slam knob
        assert!(tr.fov_at(-1.0) == Some(1.0)); // clamps to the first fov keyframe
    }

    #[test]
    fn generic_at_works_for_any_knob() {
        let score = crate::score::Score::builtin();
        let tr = parse_sync(
            &["t=0 flash=0.2".to_string(), "t=5 flash=0.8".to_string()],
            &score,
        );
        assert_eq!(tr.at("flash", 2.5), Some(0.5));
        assert_eq!(tr.at("flash", 10.0), Some(0.8));
        assert_eq!(tr.at("nonexistent", 0.0), None);
    }

    #[test]
    fn bg_keyframe_is_a_name_mapped_step() {
        let score = crate::score::Score::builtin();
        // bg takes a mode NAME → index, and switches as a STEP (no lerp), None before the first key.
        let tr = parse_sync(&["t=10 bg=fractal".to_string()], &score);
        assert_eq!(tr.bg_at(5.0), None); // before → default backdrop wins
        assert_eq!(tr.bg_at(10.0), Some(8.0)); // fractal = mode 8
        assert_eq!(tr.bg_at(50.0), Some(8.0)); // holds (step)
        // two keys → hard switch at the second, no fractional mode between
        let tr = parse_sync(
            &["t=0 bg=stars".to_string(), "t=10 bg=clouds".to_string()],
            &score,
        );
        assert_eq!(tr.bg_at(3.0), Some(2.0)); // stars
        assert_eq!(tr.bg_at(9.9), Some(2.0)); // still stars (no lerp toward clouds)
        assert_eq!(tr.bg_at(10.0), Some(9.0)); // clouds = mode 9
    }

    #[test]
    fn step_at_returns_none_before_first_key_and_holds_after() {
        let mut track = SyncTrack::default();
        // directly push two step-mode keyframes (push keeps sorted)
        track.push("stage", 10.0, 2.0);
        track.push("stage", 5.0, 1.0); // insert before the first → verifies sort
        assert_eq!(track.step_at("stage", 0.0), None);
        assert_eq!(track.step_at("stage", 5.0), Some(1.0));
        assert_eq!(track.step_at("stage", 7.0), Some(1.0));
        assert_eq!(track.step_at("stage", 10.0), Some(2.0));
        assert_eq!(track.step_at("stage", 99.0), Some(2.0));
        // nonexistent knob
        assert_eq!(track.step_at("nope", 0.0), None);
    }

    #[test]
    fn unknown_knob_name_stored_without_warning() {
        let score = crate::score::Score::builtin();
        let tr = parse_sync(&["t=0 flash=0.5 density=0.8".to_string()], &score);
        assert_eq!(tr.flash_at(0.0), Some(0.5));
        assert_eq!(tr.at("density", 0.0), Some(0.8));
        assert_eq!(tr.at("density", 5.0), Some(0.8)); // clamps
        assert_eq!(tr.knobs().len(), 2);
    }
}
